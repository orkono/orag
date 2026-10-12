//! Qualification of an embedding model before it is scored (Task 44, D-007):
//! run with ORAG_MODEL_DIR and ORAG_TEST_EMBEDDING_MODEL=<installed id>:
//! `cargo test --release --test embedder_qualification -- --ignored`.
//! A candidate that fails here is "not qualified" and never scored.
#![cfg(feature = "llama")]

use std::path::PathBuf;

use orag::infer::Embedder;
use orag::infer::llama::embedder::LlamaEmbedder;
use orag::infer::models::{ModelRole, find_model};

/// Probe strings: Turkish, English and a product code.
const PROBES: [&str; 5] = [
    "İade süresi teslimattan itibaren on dört gündür.",
    "The warranty does not cover damage caused by limescale.",
    "ORX-E104 hata kodu lisans süresinin dolduğunu gösterir.",
    "Kahve makinesinin garanti süresi nedir?",
    "Descale the machine every 3 months.",
];

/// Token counts of `PROBES` without special tokens from each model's own
/// `tokenizer.json` (Python `tokenizers` 0.19.1, `add_special_tokens=False`),
/// pinned at the revisions in `scripts/fetch-model-pack.sh`.
fn reference_tokens(id: &str) -> [usize; 5] {
    match id {
        // Both use the XLM-RoBERTa sentencepiece vocabulary.
        "multilingual-e5-large-instruct-q8_0" | "bge-m3-q8_0" => [11, 13, 14, 8, 8],
        other => panic!("no reference token counts for {other}; add them first"),
    }
}

/// (query, relevant passage, distractor); the distractor shares words or
/// language with the query but not the answer.
const TRIPLES: [(&str, &str, &str); 10] = [
    (
        "İade süresi kaç gün?",
        "Ürünler teslimattan itibaren 14 gün içinde iade edilebilir.",
        "Kargo ücreti 500 TL üzeri siparişlerde ücretsizdir.",
    ),
    (
        "Garanti ne kadar sürer?",
        "Cihaz satın alma tarihinden itibaren 24 ay garantilidir.",
        "Garanti belgesini kaybederseniz faturanızı saklayın.",
    ),
    (
        "ORX-E104 ne demek?",
        "ORX-E104 hata kodu lisans süresinin dolduğunu gösterir.",
        "ORX-7 cihazını prize takmadan önce kılavuzu okuyun.",
    ),
    (
        "Yıllık izin kaç gündür?",
        "Bir yıldan beş yıla kadar çalışanlara on dört gün yıllık izin verilir.",
        "İzin talepleri insan kaynakları sistemine girilir.",
    ),
    (
        "Kireç çözme ne sıklıkla yapılmalı?",
        "Makinenin kireci üç ayda bir çözülmelidir.",
        "Makine kahve çekirdeklerini otomatik olarak öğütür.",
    ),
    (
        "How long is the return period?",
        "Products can be returned within 14 days of delivery.",
        "Shipping is free for orders over 500 TL.",
    ),
    (
        "Does the warranty cover limescale damage?",
        "The warranty does not cover damage caused by limescale.",
        "The warranty card must be kept with the receipt.",
    ),
    (
        "How often should the machine be descaled?",
        "Descale the machine every 3 months.",
        "The machine grinds coffee beans automatically.",
    ),
    (
        "What does error ORX-E104 mean?",
        "Error ORX-E104 means that the licence has expired.",
        "Connect the ORX-7 to power before reading the manual.",
    ),
    (
        "İade süresi kaç gün?",
        "Products can be returned within 14 days of delivery.",
        "Kargo ücreti 500 TL üzeri siparişlerde ücretsizdir.",
    ),
];

fn embedder() -> (String, LlamaEmbedder) {
    let dir = PathBuf::from(std::env::var("ORAG_MODEL_DIR").expect("ORAG_MODEL_DIR is required"));
    let id =
        std::env::var("ORAG_TEST_EMBEDDING_MODEL").expect("ORAG_TEST_EMBEDDING_MODEL is required");
    let installed = find_model(&dir, &id, ModelRole::Embedding).unwrap();
    (id, LlamaEmbedder::load(&installed).unwrap())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn vectors_are_unit_length_finite_and_1024_wide() {
    let (_, e) = embedder();
    let docs = e.embed_documents(&PROBES.map(String::from)).unwrap();
    let query = e.embed_query(PROBES[0]).unwrap();
    for v in docs.iter().chain(std::iter::once(&query)) {
        assert_eq!(v.len(), 1024);
        assert!(v.iter().all(|x| x.is_finite()));
        let norm = v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "norm {norm}");
    }
}

#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn token_counts_match_the_reference_tokenizer() {
    let (id, e) = embedder();
    let reference = reference_tokens(&id);
    for (probe, expected) in PROBES.iter().zip(reference) {
        let counted = e.count_tokens(probe);
        assert!(
            counted.abs_diff(expected) <= 2,
            "{probe:?}: {counted} vs reference {expected}"
        );
    }
}

#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn relevant_passages_beat_distractors_in_at_least_nine_of_ten() {
    let (_, e) = embedder();
    let mut wins = 0;
    for (query, positive, distractor) in TRIPLES {
        let q = e.embed_query(query).unwrap();
        let docs = e
            .embed_documents(&[positive.into(), distractor.into()])
            .unwrap();
        let (p, d) = (cosine(&q, &docs[0]), cosine(&q, &docs[1]));
        eprintln!("{query:?}: positive {p:.3} distractor {d:.3}");
        wins += usize::from(p > d);
    }
    assert!(
        wins >= 9,
        "only {wins} of 10 rank the relevant passage first"
    );
}

/// Writes the probe vectors as JSON to ORAG_TEST_VECTOR_OUT, so a Metal run
/// and a CPU run (Docker) can be compared (cosine ≥ 0.999 per probe).
#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn probe_vectors_for_the_backend_comparison() {
    let Some(out) = std::env::var_os("ORAG_TEST_VECTOR_OUT") else {
        return;
    };
    let (_, e) = embedder();
    let vectors = e.embed_documents(&PROBES.map(String::from)).unwrap();
    std::fs::write(out, serde_json::to_string(&vectors).unwrap()).unwrap();
}
