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

/// Hard negatives (added after the v0.2 scoring, after review): (query,
/// relevant passage, negative): the negative is on the same
/// topic and in the same language as the passage, but does not answer the
/// query (another period, another error code, another rule).
const HARD_TRIPLES: [(&str, &str, &str); 10] = [
    (
        "İade süresi kaç gün?",
        "Ürünler teslimattan itibaren 14 gün içinde iade edilebilir.",
        "İade edilen ürünün bedeli 10 iş günü içinde hesabınıza yatırılır.",
    ),
    (
        "Garanti ne kadar sürer?",
        "Cihaz satın alma tarihinden itibaren 24 ay garantilidir.",
        "Garanti, kireç kaynaklı arızaları kapsamaz.",
    ),
    (
        "ORX-E104 ne demek?",
        "ORX-E104 hata kodu lisans süresinin dolduğunu gösterir.",
        "ORX-E105 hata kodu su tankının boş olduğunu gösterir.",
    ),
    (
        "Yıllık izin kaç gündür?",
        "Bir yıldan beş yıla kadar çalışanlara on dört gün yıllık izin verilir.",
        "Yıllık izin, çalışanın talebiyle en fazla üç parçaya bölünebilir.",
    ),
    (
        "Kireç çözme ne sıklıkla yapılmalı?",
        "Makinenin kireci üç ayda bir çözülmelidir.",
        "Kireç çözme için yalnızca sitrik asit bazlı solüsyon kullanın.",
    ),
    (
        "How long is the return period?",
        "Products can be returned within 14 days of delivery.",
        "Refunds are paid to the original card within 10 business days.",
    ),
    (
        "Does the warranty cover limescale damage?",
        "The warranty does not cover damage caused by limescale.",
        "The warranty period is 24 months from the date of purchase.",
    ),
    (
        "How often should the machine be descaled?",
        "Descale the machine every 3 months.",
        "Use only citric-acid-based descaling solution.",
    ),
    (
        "What does error ORX-E104 mean?",
        "Error ORX-E104 means that the licence has expired.",
        "Error ORX-E105 means that the water tank is empty.",
    ),
    (
        "İade süresi kaç gün?",
        "Products can be returned within 14 days of delivery.",
        "İade edilen ürünün bedeli 10 iş günü içinde hesabınıza yatırılır.",
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
        eprintln!("{probe:?}: llama.cpp {counted}, reference {expected}");
        assert!(
            counted.abs_diff(expected) <= 2,
            "{probe:?}: {counted} vs reference {expected}"
        );
    }
}

fn ranking_wins(e: &LlamaEmbedder, triples: &[(&str, &str, &str)]) -> usize {
    let mut wins = 0;
    for (query, positive, negative) in triples {
        let q = e.embed_query(query).unwrap();
        let docs = e
            .embed_documents(&[(*positive).into(), (*negative).into()])
            .unwrap();
        let (p, n) = (cosine(&q, &docs[0]), cosine(&q, &docs[1]));
        eprintln!("{query:?}: positive {p:.3} negative {n:.3}");
        wins += usize::from(p > n);
    }
    wins
}

/// The predeclared v0.2 rule: at least 9 of 10 on `TRIPLES`.
#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn relevant_passages_beat_distractors_in_at_least_nine_of_ten() {
    let (_, e) = embedder();
    let wins = ranking_wins(&e, &TRIPLES);
    assert!(
        wins >= 9,
        "only {wins} of 10 rank the relevant passage first"
    );
}

/// `HARD_TRIPLES` do not separate the v0.2 models: the default
/// (qwen3-embedding-0.6b) and both candidates rank 8 of 10. A candidate must
/// do at least as well as that default.
#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn hard_negatives_rank_at_least_as_well_as_the_default() {
    let (_, e) = embedder();
    let wins = ranking_wins(&e, &HARD_TRIPLES);
    assert!(
        wins >= 8,
        "only {wins} of 10 beat the hard negative (default: 8)"
    );
}

/// Metal vs CPU (part of the qualification): ORAG_TEST_BACKEND_VECTORS is
/// `write:<path>` on one backend (stores the probe vectors, documents and
/// queries) and `compare:<path>` on the other (asserts cosine ≥ 0.999 per
/// vector against the stored ones). Unset is a failure, never a skip.
#[test]
#[ignore = "needs a real embedding model (ORAG_TEST_EMBEDDING_MODEL)"]
fn backend_vectors_agree() {
    let mode = std::env::var("ORAG_TEST_BACKEND_VECTORS")
        .expect("ORAG_TEST_BACKEND_VECTORS=write:<path> or compare:<path> is required");
    let (_, e) = embedder();
    let mut vectors = e.embed_documents(&PROBES.map(String::from)).unwrap();
    for probe in PROBES {
        vectors.push(e.embed_query(probe).unwrap());
    }
    if let Some(path) = mode.strip_prefix("write:") {
        std::fs::write(path, serde_json::to_string(&vectors).unwrap()).unwrap();
    } else if let Some(path) = mode.strip_prefix("compare:") {
        let stored: Vec<Vec<f32>> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(stored.len(), vectors.len(), "different probe set");
        for (i, (a, b)) in stored.iter().zip(&vectors).enumerate() {
            let c = cosine(a, b);
            eprintln!("vector {i}: cosine {c:.5}");
            assert!(c >= 0.999, "vector {i}: cosine {c} below 0.999");
        }
    } else {
        panic!("ORAG_TEST_BACKEND_VECTORS must start with write: or compare:");
    }
}
