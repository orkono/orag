//! Indexes a corpus into a fresh store and compares lexical, dense and hybrid retrieval.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::domain::chunker::ChunkerConfig;
use crate::error::{OragError, Result};
use crate::eval::dataset::EvalQuery;
use crate::eval::metrics::{label_ranks, mrr_at, ndcg_at, percentile, recall_at};
use crate::infer::Embedder;
use crate::ingest::format::SourceFormat;
use crate::ingest::worker::{IngestContext, run_once};
use crate::retrieval::hybrid::{RetrievalConfig, Retriever, Strategy};
use crate::store::Store;
use crate::store::documents::{DocumentStatus, NewDocument};
use crate::store::search::SqliteVecIndex;

const EVAL_DEPTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StrategyReport {
    pub strategy: &'static str,
    pub queries: usize,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr_at_10: f64,
    pub ndcg_at_10: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalReport {
    pub embedding_model: String,
    pub corpus_documents: usize,
    pub answerable: usize,
    pub unanswerable: usize,
    pub strategies: Vec<StrategyReport>,
}

impl RetrievalReport {
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "Embedding model: `{}` · documents: {} · answerable: {} · unanswerable: {}\n\n\
             | strategy | recall@5 | recall@10 | MRR@10 | nDCG@10 | p50 ms | p95 ms |\n|---|---|---|---|---|---|---|\n",
            self.embedding_model, self.corpus_documents, self.answerable, self.unanswerable
        );
        for s in &self.strategies {
            out.push_str(&format!(
                "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} | {:.1} |\n",
                s.strategy,
                s.recall_at_5,
                s.recall_at_10,
                s.mrr_at_10,
                s.ndcg_at_10,
                s.p50_ms,
                s.p95_ms
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
        config: RetrievalConfig::default(),
    };
    let answerable: Vec<&EvalQuery> = dataset.iter().filter(|q| q.answerable).collect();
    let strategies = [Strategy::Lexical, Strategy::Dense, Strategy::Hybrid]
        .into_iter()
        .map(|strategy| evaluate_strategy(&retriever, strategy, &answerable))
        .collect::<Result<Vec<_>>>()?;
    Ok(RetrievalReport {
        embedding_model: embedder.descriptor().model_id.clone(),
        corpus_documents,
        answerable: answerable.len(),
        unanswerable: dataset.len() - answerable.len(),
        strategies,
    })
}

/// File extensions indexed from a corpus directory (case-insensitive).
const CORPUS_EXTENSIONS: [&str; 3] = ["md", "markdown", "txt"];

/// Indexes every corpus file. Anything that would silently score 0 instead
/// is an error: an empty corpus, an unreadable entry, a file that fails to
/// index, or two files with identical bytes (stored as one document).
fn index_corpus(store: &Arc<Store>, embedder: Arc<dyn Embedder>, corpus: &Path) -> Result<usize> {
    let in_corpus = |err: std::io::Error| {
        OragError::Io(std::io::Error::new(
            err.kind(),
            format!("corpus {}: {err}", corpus.display()),
        ))
    };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(corpus).map_err(in_corpus)? {
        let path = entry.map_err(in_corpus)?.path();
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
            "corpus {} has no .md, .markdown or .txt files",
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

fn evaluate_strategy(
    retriever: &Retriever,
    strategy: Strategy,
    queries: &[&EvalQuery],
) -> Result<StrategyReport> {
    let (mut r5, mut r10, mut mrr, mut ndcg, mut latencies) = (0.0, 0.0, 0.0, 0.0, Vec::new());
    for query in queries {
        let started = Instant::now();
        let outcome = retriever.retrieve(1, &query.query, strategy, EVAL_DEPTH)?;
        latencies.push(started.elapsed().as_secs_f64() * 1000.0);
        let ranked: Vec<(String, String)> = outcome
            .candidates
            .iter()
            .map(|c| {
                (
                    c.chunk.filename.clone().unwrap_or_default(),
                    c.chunk.text.clone(),
                )
            })
            .collect();
        let ranks = label_ranks(&ranked, &query.relevant);
        r5 += recall_at(&ranks, 5);
        r10 += recall_at(&ranks, 10);
        mrr += mrr_at(&ranks, 10);
        ndcg += ndcg_at(&ranks, 10);
    }
    let n = queries.len().max(1) as f64;
    Ok(StrategyReport {
        strategy: strategy.as_str(),
        queries: queries.len(),
        recall_at_5: r5 / n,
        recall_at_10: r10 / n,
        mrr_at_10: mrr / n,
        ndcg_at_10: ndcg / n,
        p50_ms: percentile(&latencies, 50.0),
        p95_ms: percentile(&latencies, 95.0),
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
        assert!(eval_err(empty.path(), &q).contains("no .md, .markdown or .txt"));
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
}
