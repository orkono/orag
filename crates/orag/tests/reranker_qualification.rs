//! Qualification of a reranker before it is scored (v0.4 plan, Task 2): run
//! with ORAG_MODEL_DIR and ORAG_TEST_RERANKER_MODEL=<installed id>:
//! `cargo test --release --test reranker_qualification -- --ignored`.
//! A candidate that fails here is "not qualified" and never scored.
#![cfg(feature = "llama")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use orag::infer::Reranker;
use orag::infer::llama::reranker::LlamaReranker;
use orag::infer::models::{ModelRole, find_model};

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

fn reranker() -> LlamaReranker {
    let dir = PathBuf::from(std::env::var("ORAG_MODEL_DIR").expect("ORAG_MODEL_DIR is required"));
    let id =
        std::env::var("ORAG_TEST_RERANKER_MODEL").expect("ORAG_TEST_RERANKER_MODEL is required");
    LlamaReranker::load(&find_model(&dir, &id, ModelRole::Reranking).unwrap()).unwrap()
}

fn wins(r: &LlamaReranker, triples: &[(&str, &str, &str)]) -> usize {
    triples
        .iter()
        .filter(|(query, positive, negative)| {
            let s = r
                .score(query, &[(*positive).into(), (*negative).into()], None)
                .unwrap();
            eprintln!("{query:?}: positive {:.3} negative {:.3}", s[0], s[1]);
            assert!(s.iter().all(|v| v.is_finite()), "{s:?}");
            s[0] > s[1]
        })
        .count()
}

#[test]
#[ignore = "needs a real reranker (ORAG_TEST_RERANKER_MODEL)"]
fn ranks_the_answer_above_easy_and_hard_negatives() {
    let r = reranker();
    assert!(wins(&r, &TRIPLES) >= 9, "easy triples");
    assert!(wins(&r, &HARD_TRIPLES) >= 9, "hard triples");
}

#[test]
#[ignore = "needs a real reranker (ORAG_TEST_RERANKER_MODEL)"]
fn long_passages_are_cut_and_scoring_is_repeatable() {
    let r = reranker();
    let long = "Garanti koşulları. ".repeat(4000);
    let passages = vec![long, TRIPLES[0].1.to_string()];
    let first = r.score(TRIPLES[0].0, &passages, None).unwrap();
    assert!(first.iter().all(|v| v.is_finite()));
    assert_eq!(first, r.score(TRIPLES[0].0, &passages, None).unwrap());
    // A passed deadline stops before scoring.
    let late = Instant::now() - Duration::from_secs(1);
    assert!(r.score(TRIPLES[0].0, &passages, Some(late)).is_err());
}
