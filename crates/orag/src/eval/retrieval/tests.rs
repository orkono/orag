use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::eval::dataset::load_dataset;
use crate::infer::Generator;
use crate::infer::fake::{FakeEmbedder, FakeGenerator};

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
            None,
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

/// A corpus whose only matching chunk for "zebra" is far larger than a
/// small generator's prompt budget, plus small chunks that fit.
fn oversized_corpus(dir: &Path) -> Vec<EvalQuery> {
    let corpus = dir.join("corpus");
    std::fs::create_dir(&corpus).unwrap();
    let big = format!("zebra {}", "kelime ".repeat(300));
    std::fs::write(corpus.join("big.txt"), big).unwrap();
    std::fs::write(corpus.join("small.txt"), "zebra küçük not").unwrap();
    let line = r#"{"id":"q1","lang":"tr","query":"zebra","relevant":[{"document":"big.txt","contains":"zebra kelime"}],"answerable":true}"#;
    let path = dir.join("set.jsonl");
    std::fs::write(&path, format!("{line}\n")).unwrap();
    load_dataset(&path).unwrap()
}

#[test]
fn the_answer_context_skips_a_chunk_too_large_for_the_prompt_budget() {
    let dir = tempfile::tempdir().unwrap();
    let dataset = oversized_corpus(dir.path());
    let run = |generator: Option<Arc<dyn Generator>>, work: &Path| {
        run_retrieval_eval_deep(
            Arc::new(FakeEmbedder::new()),
            &dir.path().join("corpus"),
            &dataset,
            work,
            RetrievalConfig::default(),
            EVAL_DEPTH,
            generator,
        )
        .unwrap()
    };
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let unbudgeted = run(None, a.path());
    // Room for the instructions and a short source, not for 300 words.
    let small: Arc<dyn Generator> = Arc::new(FakeGenerator::new("x").with_context(400, 100));
    let budgeted = run(Some(small), b.path());
    let hybrid = |r: &RetrievalReport| {
        r.strategies
            .iter()
            .find(|s| s.strategy == "hybrid")
            .unwrap()
            .clone()
    };
    let (u, g) = (hybrid(&unbudgeted), hybrid(&budgeted));
    assert_eq!(u.recall_at_10, g.recall_at_10, "ranking must not change");
    assert_eq!(u.in_context, 1.0);
    assert_eq!(
        g.in_context, 0.0,
        "the oversized chunk never reaches the prompt"
    );
    assert!(g.skipped_chunks > 0);
    assert_eq!(budgeted.context_budget.as_deref(), Some("fake-generator"));
    assert_eq!(unbudgeted.context_budget, None);
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
            None,
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
        run_retrieval_eval(Arc::new(FakeEmbedder::new()), dir.path(), &ok, work.path()).unwrap();
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
    assert!(eval_err(only_hidden.path(), &q).contains("no .md, .markdown, .txt, .pdf or .docx"));
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
