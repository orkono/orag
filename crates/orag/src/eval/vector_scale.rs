//! Synthetic dense-search latency benchmark for the v1 scale promise (D-003):
//! 100k chunks × 1024 dims, warm p95 < 250 ms on the reference laptop.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::domain::CollectionId;
use crate::domain::space::{EmbeddingSpace, SpaceDescriptor};
use crate::error::{OragError, Result};
use crate::eval::metrics::percentile;
use crate::infer::{VectorIndex, l2_normalize};
use crate::store::Store;
use crate::store::search::SqliteVecIndex;

pub const TARGET_P95_MS: f64 = 250.0;
/// Fewer samples would make p95 the maximum of a handful of queries.
pub const MIN_QUERIES: usize = 20;
pub const MAX_QUERIES: usize = 10_000;
/// 1M × 1024 dims is about 4 GB of vectors; beyond that is not a v1 question.
pub const MAX_CHUNKS: usize = 1_000_000;
const INSERT_BATCH: usize = 5_000;
const WARMUP_QUERIES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorScaleConfig {
    pub chunks: usize,
    pub dimensions: usize,
    pub queries: usize,
    pub k: usize,
    pub seed: u64,
    /// The gate; `TARGET_P95_MS` except in tests.
    pub target_p95_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VectorScaleReport {
    /// `<os>-<arch>`, e.g. `macos-aarch64`, `linux-x86_64`.
    pub platform: String,
    pub version: &'static str,
    pub chunks: usize,
    pub dimensions: usize,
    pub queries: usize,
    pub k: usize,
    /// Time spent in SQLite inserts only (vector generation is not counted).
    pub insert_seconds: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub target_p95_ms: f64,
    pub passed: bool,
}

impl VectorScaleReport {
    pub fn to_markdown(&self) -> String {
        format!(
            "| platform | version | chunks | dims | k | queries | insert s | p50 ms | p95 ms | max ms | target p95 | result |\n\
             |---|---|---|---|---|---|---|---|---|---|---|---|\n\
             | {} | {} | {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0} | {} |\n",
            self.platform,
            self.version,
            self.chunks,
            self.dimensions,
            self.k,
            self.queries,
            self.insert_seconds,
            self.p50_ms,
            self.p95_ms,
            self.max_ms,
            self.target_p95_ms,
            if self.passed { "PASS" } else { "FAIL" }
        )
    }
}

pub(crate) struct XorShift(pub(crate) u64);

impl XorShift {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }
}

/// A random unit vector; an all-zero draw (possible with one dimension) is
/// drawn again, so normalization cannot fail.
pub(crate) fn random_unit(rng: &mut XorShift, dimensions: usize) -> Vec<f32> {
    loop {
        let raw: Vec<f32> = (0..dimensions).map(|_| rng.next_f32()).collect();
        if let Ok(unit) = l2_normalize(&raw) {
            return unit;
        }
    }
}

pub(crate) fn synthetic_descriptor(dimensions: usize) -> SpaceDescriptor {
    SpaceDescriptor {
        model_id: "synthetic-benchmark".into(),
        model_sha256: "0".repeat(64),
        pooling: "none".into(),
        query_prefix: String::new(),
        document_prefix: String::new(),
        dimensions,
        normalized: true,
        max_tokens: 512,
        require_trailing_eos: false,
    }
}

fn validate(cfg: &VectorScaleConfig) -> Result<()> {
    let problem = if cfg.chunks == 0 || cfg.chunks > MAX_CHUNKS {
        Some(format!("chunks must be 1-{MAX_CHUNKS}"))
    } else if !(MIN_QUERIES..=MAX_QUERIES).contains(&cfg.queries) {
        Some(format!("queries must be {MIN_QUERIES}-{MAX_QUERIES}"))
    } else if cfg.k == 0 {
        Some("k must be positive".to_string())
    } else {
        None
    };
    problem.map_or(Ok(()), |p| Err(OragError::InvalidInput(p)))
}

/// One query, timed. A query that returns fewer hits than it must is an
/// error: a broken search path is never reported as a fast PASS.
fn timed_search(
    index: &SqliteVecIndex,
    space: &EmbeddingSpace,
    collection_id: CollectionId,
    query: &[f32],
    k: usize,
    expected: usize,
) -> Result<f64> {
    let started = Instant::now();
    let hits = index.search(space, collection_id, query, k)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if hits.len() != expected {
        return Err(OragError::Internal(format!(
            "vector search returned {} of {expected} expected hits",
            hits.len()
        )));
    }
    Ok(elapsed_ms)
}

/// Measures exact k-NN as the server runs it: each search opens its own
/// read connection, so connection setup is part of every sample.
pub fn run_vector_scale(cfg: &VectorScaleConfig, work_dir: &Path) -> Result<VectorScaleReport> {
    validate(cfg)?;
    let store = Arc::new(Store::open(&work_dir.join("vector-scale.db"))?);
    let space = store.bind_space(1, &synthetic_descriptor(cfg.dimensions))?;
    let mut rng = XorShift(cfg.seed.max(1));
    let mut insert_seconds = 0.0;
    let mut inserted = 0usize;
    while inserted < cfg.chunks {
        let batch = INSERT_BATCH.min(cfg.chunks - inserted);
        let vectors: Vec<Vec<f32>> = (0..batch)
            .map(|_| random_unit(&mut rng, cfg.dimensions))
            .collect();
        let started = Instant::now();
        store.bulk_insert_vectors(&space, 1, inserted as i64 + 1, &vectors)?;
        insert_seconds += started.elapsed().as_secs_f64();
        inserted += batch;
    }
    let index = SqliteVecIndex::new(store);
    let expected = cfg.k.min(cfg.chunks);
    for _ in 0..WARMUP_QUERIES {
        let query = random_unit(&mut rng, cfg.dimensions);
        timed_search(&index, &space, 1, &query, cfg.k, expected)?;
    }
    let mut latencies = Vec::with_capacity(cfg.queries);
    for _ in 0..cfg.queries {
        let query = random_unit(&mut rng, cfg.dimensions);
        latencies.push(timed_search(&index, &space, 1, &query, cfg.k, expected)?);
    }
    let p95_ms = percentile(&latencies, 95.0);
    Ok(VectorScaleReport {
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        version: crate::version::VERSION,
        chunks: cfg.chunks,
        dimensions: cfg.dimensions,
        queries: cfg.queries,
        k: cfg.k,
        insert_seconds,
        p50_ms: percentile(&latencies, 50.0),
        p95_ms,
        max_ms: percentile(&latencies, 100.0),
        target_p95_ms: cfg.target_p95_ms,
        passed: p95_ms < cfg.target_p95_ms,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::infer::VectorIndex;
    use crate::store::search::SqliteVecIndex;

    fn cfg(chunks: usize, dimensions: usize, queries: usize) -> VectorScaleConfig {
        VectorScaleConfig {
            chunks,
            dimensions,
            queries,
            k: 50,
            seed: 7,
            target_p95_ms: TARGET_P95_MS,
        }
    }

    #[test]
    fn small_run_reports_consistent_numbers() {
        let work = tempfile::tempdir().unwrap();
        let report = run_vector_scale(&cfg(2_000, 64, MIN_QUERIES), work.path()).unwrap();
        assert_eq!(
            (report.chunks, report.dimensions, report.queries),
            (2_000, 64, MIN_QUERIES)
        );
        assert!(
            report.p50_ms > 0.0 && report.p50_ms <= report.p95_ms && report.p95_ms <= report.max_ms
        );
        assert_eq!(report.target_p95_ms, TARGET_P95_MS);
        assert!(report.passed);
    }

    #[test]
    fn the_gate_compares_p95_with_the_target() {
        let work = tempfile::tempdir().unwrap();
        let strict = VectorScaleConfig {
            target_p95_ms: 0.0,
            ..cfg(200, 8, MIN_QUERIES)
        };
        let report = run_vector_scale(&strict, work.path()).unwrap();
        assert!(!report.passed, "{report:?}");
        assert!(report.to_markdown().contains("| FAIL |"));
    }

    #[test]
    fn sizes_outside_the_limits_are_invalid_input() {
        let work = tempfile::tempdir().unwrap();
        for bad in [
            cfg(0, 8, MIN_QUERIES),
            cfg(MAX_CHUNKS + 1, 8, MIN_QUERIES),
            cfg(100, 8, MIN_QUERIES - 1),
            cfg(100, 8, MAX_QUERIES + 1),
            VectorScaleConfig {
                k: 0,
                ..cfg(100, 8, MIN_QUERIES)
            },
        ] {
            let err = run_vector_scale(&bad, work.path()).unwrap_err();
            assert!(
                matches!(err, OragError::InvalidInput(_)),
                "{bad:?}: {err:?}"
            );
        }
    }

    #[test]
    fn a_search_that_returns_too_few_hits_fails_the_run() {
        let work = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&work.path().join("s.db")).unwrap());
        let space = store.bind_space(1, &synthetic_descriptor(8)).unwrap();
        let mut rng = XorShift(3);
        let vectors: Vec<Vec<f32>> = (0..10).map(|_| random_unit(&mut rng, 8)).collect();
        store.bulk_insert_vectors(&space, 1, 1, &vectors).unwrap();
        let index = SqliteVecIndex::new(store);
        // Collection 2 has no vectors: a query that matches nothing is an error.
        let err = timed_search(&index, &space, 2, &vectors[0], 5, 5).unwrap_err();
        assert!(err.to_string().contains("returned 0 of 5"), "{err}");
        assert!(timed_search(&index, &space, 1, &vectors[0], 5, 5).is_ok());
    }

    #[test]
    fn random_unit_vectors_are_never_zero() {
        // One dimension and a component of exactly 0.0 would not normalize.
        let mut rng = XorShift(1);
        for _ in 0..100_000 {
            let v = random_unit(&mut rng, 1);
            assert!((v[0].abs() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn exact_vector_is_its_own_nearest_neighbor() {
        let work = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&work.path().join("s.db")).unwrap());
        let space = store.bind_space(1, &synthetic_descriptor(8)).unwrap();
        let mut rng = XorShift(42);
        let vectors: Vec<Vec<f32>> = (0..100).map(|_| random_unit(&mut rng, 8)).collect();
        store.bulk_insert_vectors(&space, 1, 1, &vectors).unwrap();
        let hits = SqliteVecIndex::new(store)
            .search(&space, 1, &vectors[41], 3)
            .unwrap();
        assert_eq!(hits[0].0, 42); // rowids start at 1
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
    }
}
