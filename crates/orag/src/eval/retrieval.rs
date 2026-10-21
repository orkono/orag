//! Indexes a corpus into a fresh store and compares lexical, dense and hybrid retrieval.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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
use crate::infer::{Embedder, Generator};
use crate::ingest::format::SourceFormat;
use crate::ingest::worker::{IngestContext, run_once};
use crate::retrieval::answer::{answer_context, answer_pool};
use crate::retrieval::hybrid::{Candidate, RetrievalConfig, Retriever, Strategy};
use crate::retrieval::rerank::Rerank;
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
    /// Share of queries whose evidence reaches the answer prompt: any one
    /// label, or every label for `require_all` questions. The prompt is chosen
    /// as the answer path chooses it (`answer_context`): from the first
    /// `context_chunks × CANDIDATES_PER_SLOT` candidates, at most
    /// `context_chunks` chunks, skipping any that does not fit the generator's
    /// token budget. Without a generator (`context_budget` is `None`) no chunk
    /// is skipped.
    pub in_context: f64,
    /// Candidates skipped over all queries because they did not fit the budget.
    pub skipped_chunks: usize,
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
    /// With a reranker window: scored chunks read whole because no query key
    /// matched them (step 65).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank_whole: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalReport {
    pub embedding_model: String,
    /// Where the embedder's layers went (`orag-vulkan`; `None` elsewhere):
    /// a GPU run proves it used the GPU with this (v0.6 plan, G-7).
    pub embedding_offload: Option<crate::gpu::Offload>,
    pub lexical_query: String,
    pub corpus_documents: usize,
    pub answerable: usize,
    pub unanswerable: usize,
    /// At most this many chunks reach the answer prompt (D-005).
    pub context_chunks: usize,
    /// The generator whose token budget chose the answer context, or `None`
    /// when the context is simply the first `context_chunks` candidates.
    pub context_budget: Option<String>,
    /// `per_query` ranks reach this deep; metrics stay at depth 10.
    pub rank_depth: usize,
    /// The reranker over hybrid candidates and its depth, if one ran.
    pub reranker: Option<String>,
    pub strategies: Vec<StrategyReport>,
}

impl RetrievalReport {
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "Embedding model: `{}` · lexical query: `{}` · reranker: `{}` · context budget: `{}` · documents: {} · answerable: {} · unanswerable: {}\n\n\
             | strategy | recall@5 | recall@10 | MRR@10 | nDCG@10 | in context (prompt, ≤ {}) | skipped | p50 ms | p95 ms |\n\
             |---|---|---|---|---|---|---|---|---|\n",
            self.embedding_model,
            self.lexical_query,
            self.reranker.as_deref().unwrap_or("none"),
            self.context_budget.as_deref().unwrap_or("none"),
            self.corpus_documents,
            self.answerable,
            self.unanswerable,
            self.context_chunks
        );
        for s in &self.strategies {
            out.push_str(&format!(
                "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {:.1} | {:.1} |\n",
                s.strategy,
                s.recall_at_5,
                s.recall_at_10,
                s.mrr_at_10,
                s.ndcg_at_10,
                s.in_context,
                s.skipped_chunks,
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
    run_retrieval_eval_deep(
        embedder, corpus, dataset, work_dir, config, EVAL_DEPTH, None,
    )
}

/// `run_retrieval_eval_with`, reporting each label's rank down to `rank_depth`
/// (10..=50). Scores stay at depth 10 and the answer context (diagnosis);
/// latencies then include the deeper retrieval. With `generator`, the answer
/// context is budgeted by its token counter, as `orag serve` does.
pub fn run_retrieval_eval_deep(
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
    dataset: &[EvalQuery],
    work_dir: &Path,
    config: RetrievalConfig,
    rank_depth: usize,
    generator: Option<Arc<dyn Generator>>,
) -> Result<RetrievalReport> {
    let run = EvalRun {
        rank_depth,
        generator,
        rerank: None,
        reuse_index: false,
    };
    run_retrieval_eval_run(embedder, corpus, dataset, work_dir, config, run)
}

/// How one experimental eval run differs from the served retrieval.
pub struct EvalRun {
    /// Labels' ranks are reported down to this (10..=50).
    pub rank_depth: usize,
    /// Budgets the answer context by this generator's token counter.
    pub generator: Option<Arc<dyn Generator>>,
    /// Reranks hybrid candidates (v0.4 candidate).
    pub rerank: Option<Rerank>,
    /// Query the index an earlier run left in `work_dir` instead of
    /// building one: its vectors stay as that run's embedder made them
    /// (v0.6: indexed by `orag`, queried by `orag-vulkan`, and the reverse).
    pub reuse_index: bool,
}

/// `run_retrieval_eval_deep` with an optional reranker.
pub fn run_retrieval_eval_run(
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
    dataset: &[EvalQuery],
    work_dir: &Path,
    config: RetrievalConfig,
    run: EvalRun,
) -> Result<RetrievalReport> {
    let EvalRun {
        rank_depth,
        generator,
        rerank,
        reuse_index,
    } = run;
    if !(EVAL_DEPTH..=MAX_RANK_DEPTH).contains(&rank_depth) {
        return Err(OragError::InvalidInput(format!(
            "rank depth must be between {EVAL_DEPTH} and {MAX_RANK_DEPTH}, got {rank_depth}"
        )));
    }
    let db = work_dir.join("eval.db");
    match (db.exists(), reuse_index) {
        (true, false) => {
            return Err(OragError::InvalidInput(format!(
                "{} already exists; use an empty work dir",
                db.display()
            )));
        }
        (false, true) => {
            return Err(OragError::InvalidInput(format!(
                "no index to reuse at {}",
                db.display()
            )));
        }
        _ => {}
    }
    let store = Arc::new(Store::open(&db)?);
    let corpus_documents = if reuse_index {
        check_reused_index(&store, corpus)?
    } else {
        index_corpus(&store, embedder.clone(), corpus)?
    };
    check_labels(&store, dataset)?;
    let retriever = Retriever {
        index: Arc::new(SqliteVecIndex::new(store.clone())),
        store,
        embedder: embedder.clone(),
        config,
        reranker: rerank,
    };
    let answerable: Vec<&EvalQuery> = dataset.iter().filter(|q| q.answerable).collect();
    let strategies = [Strategy::Lexical, Strategy::Dense, Strategy::Hybrid]
        .into_iter()
        .map(|strategy| {
            evaluate_strategy(
                &retriever,
                strategy,
                &answerable,
                rank_depth,
                generator.as_deref(),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(RetrievalReport {
        embedding_model: embedder.descriptor().model_id.clone(),
        embedding_offload: embedder.offload(),
        lexical_query: retriever.config.lexical_query.name(),
        corpus_documents,
        answerable: answerable.len(),
        unanswerable: dataset.len() - answerable.len(),
        context_chunks: retriever.config.max_context_chunks,
        context_budget: generator.map(|g| g.model_id().to_string()),
        rank_depth,
        reranker: retriever.reranker.as_ref().map(|r| {
            let pinned = if r.pin_leaders {
                ", leaders pinned"
            } else {
                ""
            };
            {
                let window = r
                    .window_chars
                    .map(|n| format!(", window {n}"))
                    .unwrap_or_default();
                format!(
                    "{}, depth {}{pinned}{window}",
                    r.reranker.model_id(),
                    r.depth
                )
            }
        }),
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

/// The corpus files, sorted; dotfiles, directories and other extensions
/// are skipped, and a corpus with none is an error.
fn corpus_files(corpus: &Path) -> Result<Vec<PathBuf>> {
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
    Ok(files)
}

/// A reused index is queried as it is: nothing is embedded again. It must
/// hold exactly the corpus files (by content), all ready; anything else
/// would be indexed by the querying backend beside the other one's vectors.
fn check_reused_index(store: &Store, corpus: &Path) -> Result<usize> {
    use sha2::{Digest, Sha256};
    let mut expected = std::collections::BTreeSet::new();
    for path in corpus_files(corpus)? {
        let bytes = std::fs::read(&path)?;
        expected.insert(hex::encode(Sha256::digest(&bytes)));
    }
    let mut stored = std::collections::BTreeSet::new();
    let mut after_id = None;
    loop {
        let page = store.list_documents(
            1,
            crate::store::documents::Page {
                after_id,
                limit: crate::store::documents::MAX_PAGE_SIZE,
            },
        )?;
        let Some(last) = page.last() else { break };
        after_id = Some(last.id);
        for document in &page {
            if document.status != DocumentStatus::Ready {
                return Err(OragError::InvalidInput(format!(
                    "the reused index has a document that is not ready: {}",
                    document.filename.as_deref().unwrap_or("?")
                )));
            }
            stored.insert(document.source_sha256.clone());
        }
    }
    if stored != expected {
        return Err(OragError::InvalidInput(format!(
            "the corpus changed since the index was built ({} files, {} indexed, {} in common); use a new --index-dir",
            expected.len(),
            stored.len(),
            expected.intersection(&stored).count()
        )));
    }
    Ok(stored.len())
}

/// Indexes every corpus file. Anything that would silently score 0 instead
/// is an error: an empty corpus, an unreadable entry, a file that fails to
/// index, or two files with identical bytes (stored as one document).
pub(crate) fn index_corpus(
    store: &Arc<Store>,
    embedder: Arc<dyn Embedder>,
    corpus: &Path,
) -> Result<usize> {
    let files = corpus_files(corpus)?;
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

/// Whether the answer prompt built from `candidates` holds the question's
/// evidence, and how many candidates the budget skipped. The prompt is chosen
/// by `answer_context`, as `orag serve` chooses it; without a generator it is
/// the first `context` candidates. A question `orag serve` would refuse (too
/// long for the prompt) puts nothing in the prompt.
fn prompt_holds_evidence(
    generator: Option<&dyn Generator>,
    query: &EvalQuery,
    candidates: Vec<Candidate>,
    pool: usize,
    context: usize,
) -> Result<(bool, usize)> {
    let pooled: Vec<Candidate> = candidates.into_iter().take(pool).collect();
    let (prompt, skipped) = match generator {
        Some(generator) => match answer_context(generator, &query.query, pooled, context) {
            Ok(chosen) => chosen,
            Err(OragError::InvalidInput(_)) => (Vec::new(), 0),
            Err(other) => return Err(other),
        },
        None => (pooled.into_iter().take(context).collect(), 0),
    };
    let ranks = candidate_ranks(&prompt, &query.relevant);
    Ok((question_found(&ranks, context, query.require_all), skipped))
}

/// A question with its score, label ranks and reranker whole-chunk count.
type Scored<'q> = (
    &'q EvalQuery,
    QuestionScore,
    Vec<Option<usize>>,
    Option<usize>,
);

fn evaluate_strategy(
    retriever: &Retriever,
    strategy: Strategy,
    queries: &[&EvalQuery],
    rank_depth: usize,
    generator: Option<&dyn Generator>,
) -> Result<StrategyReport> {
    let context = retriever.config.max_context_chunks;
    let pool = answer_pool(&retriever.config);
    // Deep enough for the metrics, the answer pool and the reported ranks;
    // `score_question` caps every metric itself.
    let depth = EVAL_DEPTH.max(pool).max(rank_depth);
    let mut latencies = Vec::new();
    let mut skipped_chunks = 0;
    let mut scored: Vec<Scored<'_>> = Vec::new();
    for query in queries {
        let started = Instant::now();
        let outcome = retriever.retrieve(1, &query.query, strategy, depth)?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let ranks = candidate_ranks(&outcome.candidates, &query.relevant);
        let whole = match (&retriever.reranker, &outcome.trace.rerank) {
            (Some(r), Some(trace)) if r.window_chars.is_some() => Some(trace.whole),
            _ => None,
        };
        let mut score = score_question(&ranks, context, query.require_all);
        let (in_context, skipped) =
            prompt_holds_evidence(generator, query, outcome.candidates, pool, context)?;
        score.in_context = in_context;
        skipped_chunks += skipped;
        scored.push((query, score, ranks, whole));
    }
    let mut all = GroupSums::default();
    scored.iter().for_each(|(_, score, _, _)| all.add(score));
    let all = all.report();
    Ok(StrategyReport {
        strategy: strategy.as_str(),
        queries: queries.len(),
        recall_at_5: all.recall_at_5,
        recall_at_10: all.recall_at_10,
        mrr_at_10: all.mrr_at_10,
        ndcg_at_10: all.ndcg_at_10,
        in_context: all.in_context,
        skipped_chunks,
        missed_in_context: scored
            .iter()
            .filter(|(_, score, _, _)| !score.in_context)
            .map(|(query, _, _, _)| query.id.clone())
            .collect(),
        p50_ms: percentile(&latencies, 50.0),
        p95_ms: percentile(&latencies, 95.0),
        by_category: group_reports(scored.iter().map(|(q, s, _, _)| (q.category_name(), s))),
        by_split: group_reports(scored.iter().map(|(q, s, _, _)| (q.split_name(), s))),
        per_query: scored
            .into_iter()
            .map(|(query, score, ranks, rerank_whole)| QueryRanks {
                id: query.id.clone(),
                category: query.category_name().to_string(),
                split: query.split_name().to_string(),
                ranks,
                in_context: score.in_context,
                rerank_whole,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests;
