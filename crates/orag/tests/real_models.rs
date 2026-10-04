//! Real-model release checks. Run only via scripts/release-model-check.sh.
//! Missing ORAG_MODEL_DIR is a failure, never a silent skip; the script also
//! requires all of these tests to run (a build without `llama` has none).
#![cfg(feature = "llama")]

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

use orag::domain::chunker::ChunkerConfig;
use orag::infer::llama::embedder::LlamaEmbedder;
use orag::infer::llama::generator::LlamaGenerator;
use orag::infer::models::{ModelRole, find_model};
use orag::infer::{Embedder, Generator};
use orag::ingest::format::SourceFormat;
use orag::ingest::worker::{IngestContext, run_once};
use orag::retrieval::answer::{AnswerEngine, AnswerEvent, AnswerSummary};
use orag::retrieval::hybrid::{RetrievalConfig, Retriever};
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
        assert!(r > u, "{query:?}: relevant {r:.3} <= unrelated {u:.3}");
    };
    prefers("İade süresi kaç gündür?", 0, 1);
    // Cross-lingual, both directions.
    prefers("What is the return period for products?", 0, 1);
    prefers("Kahve makinesinin garanti süresi nedir?", 2, 1);
}

fn answer(question: &str) -> AnswerSummary {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
    let embedder: Arc<dyn Embedder> = Arc::new(embedder());
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../eval/corpus/seed/tr-kargo-politikasi.md");
    store
        .enqueue_document(
            1,
            NewDocument {
                filename: Some("tr-kargo-politikasi.md".into()),
                format: SourceFormat::Markdown,
                bytes: std::fs::read(corpus).unwrap(),
            },
        )
        .unwrap();
    let ctx = IngestContext {
        store: store.clone(),
        embedder: embedder.clone(),
        chunker: ChunkerConfig::default(),
    };
    while run_once(&ctx).unwrap() {}
    // A failed ingest would leave nothing to retrieve, and an empty context
    // abstains without calling the model: that must not pass as a real answer.
    let document = store.get_document(1, 1).unwrap();
    assert_eq!(
        document.status,
        DocumentStatus::Ready,
        "seed document did not index: {:?}",
        document.error
    );
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
