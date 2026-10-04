//! Real-model release checks. Run only via scripts/release-model-check.sh.
//! Missing ORAG_MODEL_DIR is a failure, never a silent skip; the script also
//! requires all of these tests to run (a build without `llama` has none).
#![cfg(feature = "llama")]

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

use orag::domain::chunker::ChunkerConfig;
use orag::eval::retrieval::{all_in_context, candidate_ranks};
use orag::infer::llama::embedder::LlamaEmbedder;
use orag::infer::llama::generator::LlamaGenerator;
use orag::infer::models::{ModelRole, find_model};
use orag::infer::{Embedder, Generator};
use orag::ingest::format::SourceFormat;
use orag::ingest::worker::{IngestContext, run_once};
use orag::retrieval::answer::{AnswerEngine, AnswerEvent, AnswerSummary};
use orag::retrieval::hybrid::{RetrievalConfig, Retriever, Strategy};
use orag::store::Store;
use orag::store::documents::{DocumentStatus, NewDocument};
use orag::store::search::SqliteVecIndex;

fn models_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("ORAG_MODEL_DIR")
            .expect("ORAG_MODEL_DIR must point at installed model packs"),
    )
}

fn embedder() -> LlamaEmbedder {
    LlamaEmbedder::load(
        &find_model(
            &models_dir(),
            "qwen3-embedding-0.6b-q8_0",
            ModelRole::Embedding,
        )
        .unwrap(),
    )
    .unwrap()
}

fn generator() -> LlamaGenerator {
    // Same default the product ships with, so the D-007 fallback is tested automatically.
    let id = std::env::var("ORAG_TEST_GENERATION_MODEL")
        .unwrap_or_else(|_| orag::config::DEFAULT_GENERATION_MODEL.into());
    LlamaGenerator::load(&find_model(&models_dir(), &id, ModelRole::Generation).unwrap()).unwrap()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Sections as the chunker emits them (heading + paragraph from the seed
/// corpus). One-sentence snippets are not used: with them the 0.6B embedder
/// ranked an English question about returns slightly closer to an unrelated
/// English sentence (0.393) than to the Turkish return policy (0.380), while
/// on real sections the margin is clear (0.475 vs 0.288).
const RETURNS_TR: &str = "## İade Koşulları\n\nÜrünler teslim tarihinden itibaren 14 gün içinde iade edilebilir. İade edilen ürünün orijinal ambalajında ve kullanılmamış olması gerekir. Hijyen ürünlerinde iade kabul edilmez.";
const DESCALING_EN: &str = "## Descaling\n\nDescale the machine every 3 months, or when the orange DESCALE light turns on. Use only citric-acid-based descaling solution.";
const WARRANTY_EN: &str = "## Warranty\n\nThe warranty period is 24 months from the date of purchase and does not cover damage caused by limescale.";

#[test]
#[ignore]
fn real_embedding_shape_and_bilingual_similarity() {
    let e = embedder();
    let docs = e
        .embed_documents(&[RETURNS_TR.into(), DESCALING_EN.into(), WARRANTY_EN.into()])
        .unwrap();
    assert!(docs.iter().all(|d| d.len() == 1024));
    let prefers = |query: &str, relevant: usize, unrelated: usize| {
        let q = e.embed_query(query).unwrap();
        let (r, u) = (cosine(&q, &docs[relevant]), cosine(&q, &docs[unrelated]));
        eprintln!("section similarity {query:?}: relevant {r:.4}, unrelated {u:.4}");
        assert!(r > u, "{query:?}: relevant {r:.3} <= unrelated {u:.3}");
    };
    prefers("İade süresi kaç gündür?", 0, 1);
    // Cross-lingual, both directions.
    prefers("What is the return period for products?", 0, 1);
    prefers("Kahve makinesinin garanti süresi nedir?", 2, 1);
}

/// Not a gate: the plan's original one-sentence probe, kept as a record
/// (release-model-check.sh runs it apart from the gated tests). On
/// such short texts the 0.6B embedder can rank by language before topic
/// (measured 2026-10-04: the English question scored 0.380 for the Turkish
/// policy and 0.393 for an unrelated English sentence). See D-005.
#[test]
#[ignore]
fn real_embedding_short_sentence_diagnostic() {
    let e = embedder();
    let docs = e
        .embed_documents(&[
            "Ürünler teslim tarihinden itibaren 14 gün içinde iade edilebilir.".into(),
            "Descale the machine every 3 months.".into(),
        ])
        .unwrap();
    assert_eq!(docs[0].len(), 1024);
    for query in [
        "İade süresi kaç gündür?",
        "What is the return period for products?",
    ] {
        let q = e.embed_query(query).unwrap();
        eprintln!(
            "short-sentence diagnostic {query:?}: returns policy {:.3}, unrelated {:.3}",
            cosine(&q, &docs[0]),
            cosine(&q, &docs[1])
        );
    }
}

/// Topic-near distractors in both languages, so cross-language evidence must
/// beat same-language passages about refunds, deliveries and warranties.
const DISTRACTORS_EN: &str = "# Customer Service Notes\n\n## Refund Processing\n\nRefunds are issued to the original payment method within 10 business days after the returned item has been inspected.\n\n## Delivery Deadlines\n\nStandard orders ship within 2 business days; express orders arrive the next business day.\n\n## Extended Warranty\n\nAn optional extended warranty adds 12 months of coverage for an additional fee.";
const DISTRACTORS_TR: &str = "# Müşteri Hizmetleri Notları\n\n## Para İadesi\n\nPara iadesi, iade edilen ürün incelendikten sonra 10 iş günü içinde ödeme yöntemine yapılır.\n\n## Garanti Kapsamı\n\nCihazlar üretim hatalarına karşı garanti kapsamındadır; kullanıcı hatasından doğan arızalar kapsam dışıdır.\n\n## Günlük Kayıtları\n\nMüşteri hizmetleri görüşme kayıtları 30 gün saklanır.";

/// Dense retrieval alone (no lexical help) over the seed corpus plus the
/// distractors (about 18 chunks): the evidence for each cross-language seed
/// question must rank within the top `CROSS_LANGUAGE_MAX_RANK`. Measured
/// 2026-10-04: ranks 1-2 on Metal and on CPU, so 3 leaves one rank of backend
/// noise while a real drop (to rank 4+) fails; the answer context (8) alone
/// would let the evidence fall behind half of this corpus. Ranks are printed
/// so a drift is visible before it fails.
const CROSS_LANGUAGE_MAX_RANK: usize = 3;

#[test]
#[ignore]
fn real_dense_ranks_cross_language_evidence_near_the_top() {
    let embedder: Arc<dyn Embedder> = Arc::new(embedder());
    let mut files: Vec<(String, Vec<u8>)> = [
        "tr-kargo-politikasi.md",
        "tr-orx7-kurulum.md",
        "en-security-policy.md",
        "en-barista-manual.md",
    ]
    .into_iter()
    .map(seed_file)
    .collect();
    files.push((
        "en-distractors.md".into(),
        DISTRACTORS_EN.as_bytes().to_vec(),
    ));
    files.push((
        "tr-distractors.md".into(),
        DISTRACTORS_TR.as_bytes().to_vec(),
    ));
    let (_dir, store) = indexed(embedder.clone(), &files);
    let retriever = Retriever {
        index: Arc::new(SqliteVecIndex::new(store.clone())),
        store,
        embedder,
        config: RetrievalConfig::default(),
    };
    let context = retriever.config.max_context_chunks;
    let dataset = orag::eval::dataset::load_dataset(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval/datasets/seed.jsonl"),
    )
    .unwrap();
    let cross: Vec<_> = dataset.iter().filter(|q| q.id.starts_with("x-")).collect();
    assert_eq!(cross.len(), 3, "the seed set's cross-language questions");
    let mut misses = Vec::new();
    for query in cross {
        let outcome = retriever
            .retrieve(1, &query.query, Strategy::Dense, context)
            .unwrap();
        let ranks = candidate_ranks(&outcome.candidates, &query.relevant);
        eprintln!("dense rank {} {:?}: {ranks:?}", query.id, query.query);
        if !all_in_context(&ranks, CROSS_LANGUAGE_MAX_RANK) {
            misses.push(query.id.clone());
        }
    }
    assert!(
        misses.is_empty(),
        "below rank {CROSS_LANGUAGE_MAX_RANK}: {misses:?}"
    );
}

/// An indexed store with the given corpus files (`(filename, bytes)`); every
/// document must index, or retrieval checks would silently test nothing.
fn indexed(
    embedder: Arc<dyn Embedder>,
    files: &[(String, Vec<u8>)],
) -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
    let ids: Vec<i64> = files
        .iter()
        .map(|(name, bytes)| {
            let doc = NewDocument {
                filename: Some(name.clone()),
                format: SourceFormat::Markdown,
                bytes: bytes.clone(),
            };
            store.enqueue_document(1, doc).unwrap().document_id
        })
        .collect();
    let ctx = IngestContext {
        store: store.clone(),
        embedder,
        chunker: ChunkerConfig::default(),
    };
    while run_once(&ctx).unwrap() {}
    // A failed ingest would leave nothing to retrieve, and an empty context
    // abstains without calling the model: that must not pass as a real answer.
    for id in ids {
        let document = store.get_document(1, id).unwrap();
        assert_eq!(
            document.status,
            DocumentStatus::Ready,
            "{:?} did not index: {:?}",
            document.filename,
            document.error
        );
    }
    (dir, store)
}

fn seed_file(name: &str) -> (String, Vec<u8>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../eval/corpus/seed")
        .join(name);
    (name.to_string(), std::fs::read(path).unwrap())
}

fn answer(question: &str) -> AnswerSummary {
    let embedder: Arc<dyn Embedder> = Arc::new(embedder());
    let (_dir, store) = indexed(embedder.clone(), &[seed_file("tr-kargo-politikasi.md")]);
    let retriever = Retriever {
        index: Arc::new(SqliteVecIndex::new(store.clone())),
        store,
        embedder,
        config: RetrievalConfig::default(),
    };
    let engine = AnswerEngine {
        retriever,
        generator: Arc::new(generator()) as Arc<dyn Generator>,
    };
    let mut summary = None;
    engine
        .answer(1, question, &mut |event| {
            if let AnswerEvent::Done(done) = event {
                summary = Some(*done);
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    let summary = summary.expect("the answer finished");
    assert!(
        summary.trace.context_chunks > 0 && summary.trace.completion_tokens > 0,
        "the generator must have answered from retrieved context: {:?}",
        summary.trace
    );
    summary
}

#[test]
#[ignore]
fn real_generation_answers_turkish_with_citation() {
    let s = answer("İade süresi kaç gündür?");
    assert!(s.answer.contains("14"), "{}", s.answer);
    assert!(s.citations.contains(&1), "{s:?}");
    assert!(!s.answer.contains("<think>"));
    assert!(!s.abstained);
}

#[test]
#[ignore]
fn real_generation_abstains_when_unsupported() {
    let s = answer("Şirketin kuruluş yılı nedir?");
    assert!(s.abstained, "{}", s.answer);
}
