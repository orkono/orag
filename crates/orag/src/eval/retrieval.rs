//! Indexes a corpus into a fresh store and compares lexical, dense and hybrid retrieval.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::domain::chunker::ChunkerConfig;
use crate::error::{OragError, Result};
use crate::eval::dataset::{EvalQuery, Relevant};
use crate::eval::metrics::{label_ranks, percentile};
use crate::eval::scoring::{
    GroupReport, GroupSums, QuestionScore, group_reports, question_found, score_question,
};
use crate::infer::Embedder;
use crate::ingest::format::SourceFormat;
use crate::ingest::worker::{IngestContext, run_once};
use crate::retrieval::hybrid::{Candidate, RetrievalConfig, Retriever, Strategy};
use crate::store::Store;
use crate::store::documents::{DocumentStatus, NewDocument};
use crate::store::search::SqliteVecIndex;

const EVAL_DEPTH: usize = 10;
/// Deepest `--rank-depth`: the lexical and dense lists hold 50 hits each, so
/// a deeper hybrid rank would only show the fused tail of the same lists.
pub const MAX_RANK_DEPTH: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StrategyReport {
    pub strategy: &'static str,
    pub queries: usize,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr_at_10: f64,
    pub ndcg_at_10: f64,
    /// Share of queries found within the first `context_chunks` candidates,
    /// the most the answer prompt can hold (D-005): any one label, or every
    /// label for `require_all` questions. An approximation of what the
    /// generator sees: the answer path can still skip a candidate that does
    /// not fit its token budget.
    pub in_context: f64,
    /// Ids of the queries that miss it, so one subgroup cannot hide in a mean.
    pub missed_in_context: Vec<String>,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub by_category: BTreeMap<String, GroupReport>,
    pub by_split: BTreeMap<String, GroupReport>,
    /// Every question with its label ranks, so two runs can be compared per
    /// question instead of by their means.
    pub per_query: Vec<QueryRanks>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryRanks {
    pub id: String,
    pub category: String,
    pub split: String,
    /// 1-based rank of each label within the retrieved candidates.
    pub ranks: Vec<Option<usize>>,
    pub in_context: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalReport {
    pub embedding_model: String,
    pub lexical_query: String,
    pub corpus_documents: usize,
    pub answerable: usize,
    pub unanswerable: usize,
    /// At most this many chunks reach the answer prompt (D-005).
    pub context_chunks: usize,
    /// `per_query` ranks reach this deep; metrics stay at depth 10.
    pub rank_depth: usize,
    pub strategies: Vec<StrategyReport>,
}

impl RetrievalReport {
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "Embedding model: `{}` · lexical query: `{}` · documents: {} · answerable: {} · unanswerable: {}\n\n\
             | strategy | recall@5 | recall@10 | MRR@10 | nDCG@10 | in context (top {}) | p50 ms | p95 ms |\n\
             |---|---|---|---|---|---|---|---|\n",
            self.embedding_model,
            self.lexical_query,
            self.corpus_documents,
            self.answerable,
            self.unanswerable,
            self.context_chunks
        );
        for s in &self.strategies {
            out.push_str(&format!(
                "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} | {:.1} |\n",
                s.strategy,
                s.recall_at_5,
                s.recall_at_10,
                s.mrr_at_10,
                s.ndcg_at_10,
                s.in_context,
                s.p50_ms,
                s.p95_ms
            ));
        }
        // Older sets have one group each; a breakdown of one row repeats the table.
        let groups = |pick: fn(&StrategyReport) -> &BTreeMap<String, GroupReport>| {
            self.strategies.first().map_or(0, |s| pick(s).len())
        };
        if groups(|s| &s.by_category) > 1 {
            out.push_str(&self.breakdown("By category", "category", |s| &s.by_category));
        }
        if groups(|s| &s.by_split) > 1 {
            out.push_str(&self.breakdown("By split", "split", |s| &s.by_split));
        }
        out
    }

    /// One row per group: in-context share for every strategy, then the
    /// hybrid (served) recall and MRR.
    fn breakdown(
        &self,
        title: &str,
        column: &str,
        pick: fn(&StrategyReport) -> &BTreeMap<String, GroupReport>,
    ) -> String {
        let names: Vec<&str> = self.strategies.iter().map(|s| s.strategy).collect();
        let mut out = format!("\n### {title}\n\n| {column} | questions |");
        for name in &names {
            out.push_str(&format!(" {name} in context |"));
        }
        out.push_str(" hybrid recall@10 | hybrid MRR@10 |\n|---|---|");
        out.push_str(&"---|".repeat(names.len() + 2));
        out.push('\n');
        let hybrid = self.strategies.iter().find(|s| s.strategy == "hybrid");
        let Some(first) = self.strategies.first() else {
            return out;
        };
        for (group, report) in pick(first) {
            out.push_str(&format!("| {group} | {} |", report.queries));
            for strategy in &self.strategies {
                let value = pick(strategy).get(group).map_or(0.0, |g| g.in_context);
                out.push_str(&format!(" {value:.3} |"));
            }
            let served = hybrid
                .and_then(|h| pick(h).get(group))
                .cloned()
                .unwrap_or_default();
            out.push_str(&format!(
                " {:.3} | {:.3} |\n",
                served.recall_at_10, served.mrr_at_10
            ));
        }
        out
    }
}

pub fn run_retrieval_eval(
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
    dataset: &[EvalQuery],
    work_dir: &Path,
) -> Result<RetrievalReport> {
    run_retrieval_eval_with(
        embedder,
        corpus,
        dataset,
        work_dir,
        RetrievalConfig::default(),
    )
}

/// `run_retrieval_eval` with another retrieval configuration (experiments).
pub fn run_retrieval_eval_with(
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
    dataset: &[EvalQuery],
    work_dir: &Path,
    config: RetrievalConfig,
) -> Result<RetrievalReport> {
    run_retrieval_eval_deep(embedder, corpus, dataset, work_dir, config, EVAL_DEPTH)
}

/// `run_retrieval_eval_with`, reporting each label's rank down to `rank_depth`
/// (10..=50). Scores stay at depth 10 and the answer context (diagnosis);
/// latencies then include the deeper retrieval.
pub fn run_retrieval_eval_deep(
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
    dataset: &[EvalQuery],
    work_dir: &Path,
    config: RetrievalConfig,
    rank_depth: usize,
) -> Result<RetrievalReport> {
    if !(EVAL_DEPTH..=MAX_RANK_DEPTH).contains(&rank_depth) {
        return Err(OragError::InvalidInput(format!(
            "rank depth must be between {EVAL_DEPTH} and {MAX_RANK_DEPTH}, got {rank_depth}"
        )));
    }
    let db = work_dir.join("eval.db");
    if db.exists() {
        return Err(OragError::InvalidInput(format!(
            "{} already exists; use an empty work dir",
            db.display()
        )));
    }
    let store = Arc::new(Store::open(&db)?);
    let corpus_documents = index_corpus(&store, embedder.clone(), corpus)?;
    check_labels(&store, dataset)?;
    let retriever = Retriever {
        index: Arc::new(SqliteVecIndex::new(store.clone())),
        store,
        embedder: embedder.clone(),
        config,
    };
    let answerable: Vec<&EvalQuery> = dataset.iter().filter(|q| q.answerable).collect();
    let strategies = [Strategy::Lexical, Strategy::Dense, Strategy::Hybrid]
        .into_iter()
        .map(|strategy| evaluate_strategy(&retriever, strategy, &answerable, rank_depth))
        .collect::<Result<Vec<_>>>()?;
    Ok(RetrievalReport {
        embedding_model: embedder.descriptor().model_id.clone(),
        lexical_query: retriever.config.lexical_query.name(),
        corpus_documents,
        answerable: answerable.len(),
        unanswerable: dataset.len() - answerable.len(),
        context_chunks: retriever.config.max_context_chunks,
        rank_depth,
        strategies,
    })
}

/// File extensions indexed from a corpus directory (case-insensitive).
/// Every format ORAG ingests; PDF and DOCX go through the production parsers
/// (in process here), so the corpus measures what `orag serve` indexes.
const CORPUS_EXTENSIONS: [&str; 5] = ["md", "markdown", "txt", "pdf", "docx"];

/// Byte-based, so a dotfile whose name is not UTF-8 is still hidden.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.as_encoded_bytes().first() == Some(&b'.'))
}

/// Indexes every corpus file. Anything that would silently score 0 instead
/// is an error: an empty corpus, an unreadable entry, a file that fails to
/// index, or two files with identical bytes (stored as one document).
pub(crate) fn index_corpus(
    store: &Arc<Store>,
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
) -> Result<usize> {
    let in_corpus = |err: std::io::Error| {
        OragError::Io(std::io::Error::new(
            err.kind(),
            format!("corpus {}: {err}", corpus.display()),
        ))
    };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(corpus).map_err(in_corpus)? {
        let path = entry.map_err(in_corpus)?.path();
        // Dotfiles (AppleDouble `._name`, editor backups) and directories
        // are never corpus.
        if is_hidden(&path) || path.is_dir() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if extension.is_some_and(|e| CORPUS_EXTENSIONS.contains(&e.as_str())) {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(OragError::InvalidInput(format!(
            "corpus {} has no .md, .markdown, .txt, .pdf or .docx files",
            corpus.display()
        )));
    }
    let mut documents: HashMap<i64, String> = HashMap::new();
    for path in &files {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let in_file = |err: OragError| OragError::InvalidInput(format!("{filename}: {err}"));
        let format = SourceFormat::detect(Some(&filename), None).map_err(in_file)?;
        let bytes = std::fs::read(path).map_err(|e| in_file(e.into()))?;
        let enqueued = store
            .enqueue_document(
                1,
                NewDocument {
                    filename: Some(filename.clone()),
                    format,
                    bytes,
                },
            )
            .map_err(in_file)?;
        if let Some(first) = documents.insert(enqueued.document_id, filename.clone()) {
            return Err(OragError::InvalidInput(format!(
                "{filename} has the same content as {first}; remove one, labels can only name the first"
            )));
        }
    }
    let ctx = IngestContext {
        store: store.clone(),
        embedder,
        chunker: ChunkerConfig::default(),
    };
    while run_once(&ctx)? {}
    let mut failed = Vec::new();
    for (&id, filename) in &documents {
        let document = store.get_document(1, id)?;
        if document.status != DocumentStatus::Ready {
            failed.push(format!(
                "{filename}: {}",
                document.error.unwrap_or_default()
            ));
        }
    }
    if !failed.is_empty() {
        failed.sort();
        return Err(OragError::InvalidInput(format!(
            "corpus documents failed to index: {}",
            failed.join("; ")
        )));
    }
    Ok(documents.len())
}

/// Every label must be satisfiable: its document is in the corpus and one of
/// that document's chunks contains the text (parsed text, whitespace aside).
/// Otherwise a typo would look like a retrieval failure.
fn check_labels(store: &Store, dataset: &[EvalQuery]) -> Result<()> {
    let chunks: Vec<(String, String)> = store
        .chunk_texts(1)?
        .into_iter()
        .map(|(filename, text)| (filename.unwrap_or_default(), text))
        .collect();
    let mut problems = Vec::new();
    for query in dataset {
        for (label, rank) in query
            .relevant
            .iter()
            .zip(label_ranks(&chunks, &query.relevant))
        {
            if rank.is_some() {
                continue;
            }
            let known = chunks
                .iter()
                .any(|(document, _)| *document == label.document);
            problems.push(if known {
                format!(
                    "{}: {:?} is not in the parsed text of {}",
                    query.id, label.contains, label.document
                )
            } else if label.document.starts_with('.') {
                format!(
                    "{}: {} is not a corpus document (hidden files are ignored)",
                    query.id, label.document
                )
            } else {
                format!("{}: {} is not a corpus document", query.id, label.document)
            });
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(OragError::InvalidInput(format!(
        "labels that can never match (labels use the parsed text, without Markdown syntax): {}",
        problems.join("; ")
    )))
}

/// For each label, the 1-based rank of the first candidate that satisfies it.
pub fn candidate_ranks(candidates: &[Candidate], labels: &[Relevant]) -> Vec<Option<usize>> {
    let ranked: Vec<(String, String)> = candidates
        .iter()
        .map(|c| {
            (
                c.chunk.filename.clone().unwrap_or_default(),
                c.chunk.text.clone(),
            )
        })
        .collect();
    label_ranks(&ranked, labels)
}

/// Every label found within the first `limit` candidates.
pub fn all_in_context(ranks: &[Option<usize>], limit: usize) -> bool {
    question_found(ranks, limit, true)
}

fn evaluate_strategy(
    retriever: &Retriever,
    strategy: Strategy,
    queries: &[&EvalQuery],
    rank_depth: usize,
) -> Result<StrategyReport> {
    let context = retriever.config.max_context_chunks;
    // Deep enough for the metrics, the context check and the reported ranks;
    // `score_question` caps every metric itself.
    let depth = EVAL_DEPTH.max(context).max(rank_depth);
    let mut latencies = Vec::new();
    let mut scored: Vec<(&EvalQuery, QuestionScore, Vec<Option<usize>>)> = Vec::new();
    for query in queries {
        let started = Instant::now();
        let outcome = retriever.retrieve(1, &query.query, strategy, depth)?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let ranks = candidate_ranks(&outcome.candidates, &query.relevant);
        scored.push((
            query,
            score_question(&ranks, context, query.require_all),
            ranks,
        ));
    }
    let mut all = GroupSums::default();
    scored.iter().for_each(|(_, score, _)| all.add(score));
    let all = all.report();
    Ok(StrategyReport {
        strategy: strategy.as_str(),
        queries: queries.len(),
        recall_at_5: all.recall_at_5,
        recall_at_10: all.recall_at_10,
        mrr_at_10: all.mrr_at_10,
        ndcg_at_10: all.ndcg_at_10,
        in_context: all.in_context,
        missed_in_context: scored
            .iter()
            .filter(|(_, score, _)| !score.in_context)
            .map(|(query, _, _)| query.id.clone())
            .collect(),
        p50_ms: percentile(&latencies, 50.0),
        p95_ms: percentile(&latencies, 95.0),
        by_category: group_reports(scored.iter().map(|(q, s, _)| (q.category_name(), s))),
        by_split: group_reports(scored.iter().map(|(q, s, _)| (q.split_name(), s))),
        per_query: scored
            .into_iter()
            .map(|(query, score, ranks)| QueryRanks {
                id: query.id.clone(),
                category: query.category_name().to_string(),
                split: query.split_name().to_string(),
                ranks,
                in_context: score.in_context,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::eval::dataset::load_dataset;
    use crate::infer::fake::FakeEmbedder;

    #[test]
    fn deeper_ranks_change_no_metric() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/seed.jsonl")).unwrap();
        let run = |depth| {
            let work = tempfile::tempdir().unwrap();
            run_retrieval_eval_deep(
                Arc::new(FakeEmbedder::new()),
                &root.join("corpus/seed"),
                &dataset,
                work.path(),
                RetrievalConfig::default(),
                depth,
            )
            .unwrap()
        };
        let (shallow, deep) = (run(EVAL_DEPTH), run(50));
        assert_eq!((shallow.rank_depth, deep.rank_depth), (10, 50));
        for (a, b) in shallow.strategies.iter().zip(&deep.strategies) {
            assert_eq!(
                (
                    a.recall_at_5,
                    a.recall_at_10,
                    a.mrr_at_10,
                    a.ndcg_at_10,
                    a.in_context
                ),
                (
                    b.recall_at_5,
                    b.recall_at_10,
                    b.mrr_at_10,
                    b.ndcg_at_10,
                    b.in_context
                ),
                "{}",
                a.strategy
            );
            for (qa, qb) in a.per_query.iter().zip(&b.per_query) {
                for (ra, rb) in qa.ranks.iter().zip(&qb.ranks) {
                    match ra {
                        Some(r) => assert_eq!(rb, &Some(*r), "{}", qa.id),
                        None => assert!(rb.is_none_or(|r| r > 10), "{}", qa.id),
                    }
                }
            }
        }
        assert!(
            deep.strategies.iter().any(|s| s
                .per_query
                .iter()
                .any(|q| q.ranks.iter().any(|r| r.is_some_and(|r| r > 10)))),
            "the deep run should find some label below rank 10 with the fake embedder"
        );
    }

    #[test]
    fn rank_depth_outside_10_to_50_is_refused() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/seed.jsonl")).unwrap();
        for depth in [9, 51] {
            let work = tempfile::tempdir().unwrap();
            let err = run_retrieval_eval_deep(
                Arc::new(FakeEmbedder::new()),
                &root.join("corpus/seed"),
                &dataset,
                work.path(),
                RetrievalConfig::default(),
                depth,
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("rank depth"), "{err}");
        }
    }

    #[test]
    fn seed_eval_reports_all_strategies_with_sane_metrics() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/seed.jsonl")).unwrap();
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            &root.join("corpus/seed"),
            &dataset,
            work.path(),
        )
        .unwrap();
        assert_eq!(report.corpus_documents, 4);
        assert_eq!((report.answerable, report.unanswerable), (14, 2));
        let names: Vec<&str> = report.strategies.iter().map(|s| s.strategy).collect();
        assert_eq!(names, vec!["lexical", "dense", "hybrid"]);
        for s in &report.strategies {
            for value in [s.recall_at_5, s.recall_at_10, s.mrr_at_10, s.ndcg_at_10] {
                assert!((0.0..=1.0).contains(&value), "{s:?}");
            }
        }
        // Exact identifiers must be found lexically even with a toy embedder.
        assert!(
            report.strategies[0].recall_at_10 >= 0.5,
            "{:?}",
            report.strategies[0]
        );
        assert!(report.to_markdown().contains("| hybrid |"));
    }

    #[test]
    fn anayasa_labels_match_and_lexical_finds_the_circumflex_case() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/anayasa-tr.jsonl")).unwrap();
        let work = tempfile::tempdir().unwrap();
        // Fails when a label no longer sits inside one chunk.
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            &root.join("corpus/anayasa"),
            &dataset,
            work.path(),
        )
        .unwrap();
        assert_eq!((report.answerable, report.unanswerable), (20, 2));
        let lexical = &report.strategies[0];
        // `milli marsimiz` finds `Millî marşı` only through the circumflex
        // fold, `resmi dil` finds `resmî dili` only through prefix terms (D-006).
        for id in ["ana-016", "ana-001"] {
            assert!(
                !lexical.missed_in_context.iter().any(|m| m == id),
                "{id} missed lexically: {lexical:?}"
            );
        }
    }

    #[test]
    fn inflection_labels_match_the_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/anayasa-cekim-tr.jsonl")).unwrap();
        let work = tempfile::tempdir().unwrap();
        // Fails when a label no longer sits inside one chunk.
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            &root.join("corpus/anayasa"),
            &dataset,
            work.path(),
        )
        .unwrap();
        assert_eq!((report.answerable, report.unanswerable), (30, 0));
        assert_eq!(report.lexical_query, "prefix:3+stem:7");
    }

    fn corpus(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    fn query(id: &str, document: &str, contains: &str) -> EvalQuery {
        EvalQuery {
            id: id.into(),
            lang: "tr".into(),
            query: "iade".into(),
            relevant: vec![crate::eval::dataset::Relevant {
                document: document.into(),
                contains: contains.into(),
            }],
            answerable: true,
            category: None,
            split: None,
            family: None,
            require_all: false,
        }
    }

    fn eval_err(dir: &Path, dataset: &[EvalQuery]) -> String {
        let work = tempfile::tempdir().unwrap();
        run_retrieval_eval(Arc::new(FakeEmbedder::new()), dir, dataset, work.path())
            .unwrap_err()
            .to_string()
    }

    const DOC: &str = "# İade\n\nÜrünler **14 gün** içinde `iade` edilebilir.";

    #[test]
    fn labels_that_can_never_match_are_rejected_by_id() {
        let dir = corpus(&[("a.md", DOC)]);
        let err = eval_err(dir.path(), &[query("q-typo", "typo.md", "14 gün")]);
        assert!(err.contains("q-typo") && err.contains("typo.md"), "{err}");
        // Copied with Markdown syntax: the parsed text has no `**`.
        let err = eval_err(dir.path(), &[query("q-md", "a.md", "**14 gün**")]);
        assert!(err.contains("q-md"), "{err}");
        // The parsed text matches, whitespace aside.
        let work = tempfile::tempdir().unwrap();
        let ok = [query("q-ok", "a.md", "14 gün içinde iade\n edilebilir")];
        let report =
            run_retrieval_eval(Arc::new(FakeEmbedder::new()), dir.path(), &ok, work.path())
                .unwrap();
        assert_eq!(report.strategies[0].recall_at_10, 1.0);
    }

    #[test]
    fn corpus_problems_are_errors_not_zero_scores() {
        let q = [query("q", "a.md", "14 gün")];
        let empty = corpus(&[]);
        assert!(eval_err(empty.path(), &q).contains("no .md, .markdown, .txt, .pdf or .docx"));
        let twins = corpus(&[("a.md", DOC), ("b.md", DOC)]);
        let err = eval_err(twins.path(), &q);
        assert!(err.contains("a.md") && err.contains("b.md"), "{err}");
        let blank = corpus(&[("a.md", DOC), ("bos.md", "")]);
        assert!(eval_err(blank.path(), &q).contains("bos.md"));
        // Extensions are case-insensitive and include .markdown.
        let mixed = corpus(&[("A.MD", DOC), ("b.markdown", "# B\n\nikinci belge")]);
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            mixed.path(),
            &[query("q", "A.MD", "14 gün")],
            work.path(),
        )
        .unwrap();
        assert_eq!(report.corpus_documents, 2);
    }

    #[test]
    fn hidden_files_are_not_part_of_the_corpus() {
        // macOS copies to FAT/exFAT media leave AppleDouble `._name` files
        // (binary, not UTF-8) next to every document; dotfiles are never corpus.
        let dir = corpus(&[("a.md", DOC), (".notes.md", DOC)]);
        std::fs::write(dir.path().join("._a.md"), b"\x00\x05\x16\x07\xff\xfe").unwrap();
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            dir.path(),
            &[query("q", "a.md", "14 gün")],
            work.path(),
        )
        .unwrap();
        assert_eq!(report.corpus_documents, 1);
        let only_hidden = corpus(&[(".a.md", DOC)]);
        let q = [query("q", ".a.md", "14 gün")];
        assert!(
            eval_err(only_hidden.path(), &q).contains("no .md, .markdown, .txt, .pdf or .docx")
        );
        // A label naming a hidden file says why it is not a corpus document.
        let labelled = corpus(&[("a.md", DOC), (".faq.md", DOC)]);
        assert!(eval_err(labelled.path(), &q).contains("hidden files are ignored"));
    }

    #[test]
    fn directories_named_like_documents_are_skipped() {
        let dir = corpus(&[("a.md", DOC)]);
        std::fs::create_dir(dir.path().join("drafts.md")).unwrap();
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            dir.path(),
            &[query("q", "a.md", "14 gün")],
            work.path(),
        )
        .unwrap();
        assert_eq!(report.corpus_documents, 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn hidden_files_with_non_utf8_names_are_skipped() {
        use std::os::unix::ffi::OsStrExt;
        let dir = corpus(&[("a.md", DOC)]);
        let name = std::ffi::OsStr::from_bytes(b"._\xff.md");
        std::fs::write(dir.path().join(name), b"\x00\x05").unwrap();
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            dir.path(),
            &[query("q", "a.md", "14 gün")],
            work.path(),
        )
        .unwrap();
        assert_eq!(report.corpus_documents, 1);
    }

    #[test]
    fn context_inclusion_needs_every_label_within_the_limit() {
        assert!(all_in_context(&[Some(1), Some(8)], 8));
        assert!(!all_in_context(&[Some(1), Some(9)], 8));
        assert!(!all_in_context(&[Some(1), None], 8));
    }

    #[test]
    fn seed_report_names_the_queries_missing_from_the_context() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let dataset = load_dataset(&root.join("datasets/seed.jsonl")).unwrap();
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            &root.join("corpus/seed"),
            &dataset,
            work.path(),
        )
        .unwrap();
        assert_eq!(
            report.context_chunks,
            RetrievalConfig::default().max_context_chunks
        );
        for s in &report.strategies {
            let found = s.queries - s.missed_in_context.len();
            assert!(
                (s.in_context - found as f64 / s.queries as f64).abs() < 1e-9,
                "{s:?}"
            );
            assert!(
                s.missed_in_context
                    .iter()
                    .all(|id| dataset.iter().any(|q| q.id == *id && q.answerable))
            );
        }
        // The toy embedder misses some queries lexically: they are listed by id.
        assert!(!report.strategies[0].missed_in_context.is_empty());
        assert!(report.to_markdown().contains("in context"));
    }

    #[test]
    fn report_breaks_down_by_category_and_split_and_lists_each_query() {
        let dir = corpus(&[
            ("a.md", DOC),
            ("b.md", "# Kargo\n\nKargo ücreti 49,90 TL olarak uygulanır."),
        ]);
        let mut first = query("q-a", "a.md", "14 gün");
        first.category = Some("factual".into());
        first.split = Some("dev".into());
        let mut second = query("q-b", "b.md", "49,90 TL");
        second.query = "kargo ücreti".into();
        second.category = Some("table".into());
        second.split = Some("holdout".into());
        let mut both = query("q-ab", "a.md", "14 gün");
        both.relevant.push(crate::eval::dataset::Relevant {
            document: "b.md".into(),
            contains: "49,90 TL".into(),
        });
        both.query = "iade kargo ücreti".into();
        both.category = Some("multi_document".into());
        both.require_all = true;
        let work = tempfile::tempdir().unwrap();
        let report = run_retrieval_eval(
            Arc::new(FakeEmbedder::new()),
            dir.path(),
            &[first, second, both],
            work.path(),
        )
        .unwrap();
        let lexical = &report.strategies[0];
        assert_eq!(
            lexical.by_category.keys().collect::<Vec<_>>(),
            vec!["factual", "multi_document", "table"]
        );
        assert_eq!(
            lexical.by_split.keys().collect::<Vec<_>>(),
            vec!["dev", "holdout", "unsplit"]
        );
        assert_eq!(lexical.by_category["table"].queries, 1);
        let ids: Vec<&str> = lexical.per_query.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, vec!["q-a", "q-b", "q-ab"]);
        assert_eq!(lexical.per_query[2].ranks.len(), 2);
        assert_eq!(lexical.per_query[2].category, "multi_document");
        let markdown = report.to_markdown();
        assert!(markdown.contains("By category") && markdown.contains("| multi_document |"));
        assert!(markdown.contains("By split") && markdown.contains("| holdout |"));
    }
}
