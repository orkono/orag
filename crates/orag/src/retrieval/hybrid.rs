//! Deterministic hybrid retrieval: BM25 + dense → RRF, each list's #1 first (D-005).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::domain::CollectionId;
use crate::domain::normalize::fts_query;
use crate::domain::rrf::{FusedHit, promote_list_leaders, reciprocal_rank_fusion};
use crate::error::{OragError, Result};
use crate::infer::{Embedder, VectorIndex};
use crate::store::Store;
use crate::store::search::ChunkRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Lexical,
    Dense,
    Hybrid,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Strategy::Lexical => "lexical",
            Strategy::Dense => "dense",
            Strategy::Hybrid => "hybrid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrievalConfig {
    pub lexical_k: usize,
    pub dense_k: usize,
    pub max_context_chunks: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        RetrievalConfig {
            lexical_k: 50,
            dense_k: 50,
            max_context_chunks: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub chunk: ChunkRecord,
    pub hit: FusedHit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RetrievalTrace {
    pub strategy: &'static str,
    pub lexical_hits: usize,
    pub dense_hits: usize,
    pub fused_hits: usize,
    pub embed_ms: u64,
    pub lexical_ms: u64,
    pub dense_ms: u64,
    pub embedding_space: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalOutcome {
    pub candidates: Vec<Candidate>,
    pub trace: RetrievalTrace,
}

pub struct Retriever {
    pub store: Arc<Store>,
    pub index: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn Embedder>,
    pub config: RetrievalConfig,
}

fn timed<T>(slot: &mut u64, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    *slot = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    out
}

impl Retriever {
    pub fn retrieve(
        &self,
        collection_id: CollectionId,
        query: &str,
        strategy: Strategy,
        limit: usize,
    ) -> Result<RetrievalOutcome> {
        self.store.get_collection(collection_id)?;
        let mut trace = RetrievalTrace {
            strategy: strategy.as_str(),
            ..RetrievalTrace::default()
        };
        let lexical = if strategy == Strategy::Dense {
            Vec::new()
        } else {
            self.lexical(collection_id, query, &mut trace)?
        };
        // Lexical search never touches the embedding space, so it keeps
        // working after a model change; only the dense side needs a match.
        let dense = if strategy == Strategy::Lexical {
            Vec::new()
        } else {
            self.dense(collection_id, query, &mut trace)?
        };
        trace.lexical_hits = lexical.len();
        trace.dense_hits = dense.len();
        let fused = promote_list_leaders(reciprocal_rank_fusion(&lexical, &dense));
        trace.fused_hits = fused.len();
        let top: Vec<FusedHit> = fused.into_iter().take(limit).collect();
        let candidates = self.hydrate(collection_id, top)?;
        Ok(RetrievalOutcome { candidates, trace })
    }

    fn lexical(
        &self,
        collection_id: CollectionId,
        query: &str,
        trace: &mut RetrievalTrace,
    ) -> Result<Vec<i64>> {
        timed(&mut trace.lexical_ms, || match fts_query(query) {
            Some(q) => self
                .store
                .lexical_search(collection_id, &q, self.config.lexical_k),
            None => Ok(Vec::new()),
        })
    }

    fn dense(
        &self,
        collection_id: CollectionId,
        query: &str,
        trace: &mut RetrievalTrace,
    ) -> Result<Vec<i64>> {
        check_query_fits(self.embedder.as_ref(), query)?;
        let Some(space) = self
            .store
            .query_space(collection_id, self.embedder.descriptor())?
        else {
            return Ok(Vec::new());
        };
        trace.embedding_space = Some(space.fingerprint.clone());
        let vector = timed(&mut trace.embed_ms, || self.embedder.embed_query(query))?;
        let hits = timed(&mut trace.dense_ms, || {
            self.index
                .search(&space, collection_id, &vector, self.config.dense_k)
        })?;
        Ok(hits.into_iter().map(|(id, _)| id).collect())
    }

    fn hydrate(&self, collection_id: CollectionId, top: Vec<FusedHit>) -> Result<Vec<Candidate>> {
        let ids: Vec<i64> = top.iter().map(|hit| hit.chunk_id).collect();
        let mut chunks: HashMap<i64, ChunkRecord> = self
            .store
            .get_chunks(collection_id, &ids)?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        Ok(top
            .into_iter()
            .filter_map(|hit| {
                chunks
                    .remove(&hit.chunk_id)
                    .map(|chunk| Candidate { chunk, hit })
            })
            .collect())
    }
}

/// Tokens kept for BOS/EOS around the embedded query.
const QUERY_SPECIAL_TOKENS: usize = 4;

/// A question that fits the character limit can still be too long for the
/// embedding model; that is the user's input, not a model failure.
pub fn check_query_fits(embedder: &dyn Embedder, query: &str) -> Result<()> {
    let descriptor = embedder.descriptor();
    let tokens = embedder.count_tokens(&format!("{}{query}", descriptor.query_prefix))
        + QUERY_SPECIAL_TOKENS;
    if tokens > descriptor.max_tokens {
        return Err(OragError::InvalidInput(format!(
            "query is too long for the embedding model ({tokens} tokens, at most {})",
            descriptor.max_tokens
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::OragError;
    use crate::infer::fake::FakeEmbedder;
    use crate::retrieval::testing::indexed;

    const LEXICAL: &str = "# Kargo\n\nORX-E104 hata kodu lisans süresinin dolduğunu gösterir.";
    const SEMANTIC: &str = "# Garanti\n\nCihaz yirmi dört ay boyunca güvence altındadır.";

    fn embedder() -> FakeEmbedder {
        // Dense space: the garanti document and the paraphrased query share a fixture vector.
        FakeEmbedder::new()
            .with_fixture("güvence", vec![0.0, 1.0])
            .with_fixture("ne kadar korunur", vec![0.0, 1.0])
            .with_fixture("orx", vec![1.0, 0.0])
    }

    fn titles(outcome: &RetrievalOutcome) -> Vec<String> {
        outcome
            .candidates
            .iter()
            .map(|c| c.chunk.document_title.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn hybrid_finds_lexical_and_semantic_matches() {
        let (_dir, retriever) = indexed(embedder(), &[("a.md", LEXICAL), ("b.md", SEMANTIC)]);
        let lexical = retriever
            .retrieve(1, "ORX-E104", Strategy::Lexical, 8)
            .unwrap();
        assert_eq!(titles(&lexical), vec!["Kargo"]);
        let dense = retriever
            .retrieve(1, "cihaz ne kadar korunur", Strategy::Dense, 1)
            .unwrap();
        assert_eq!(titles(&dense), vec!["Garanti"]);
        let hybrid = retriever
            .retrieve(1, "ORX-E104 cihaz ne kadar korunur", Strategy::Hybrid, 8)
            .unwrap();
        assert_eq!(hybrid.candidates.len(), 2);
        assert_eq!(hybrid.trace.strategy, "hybrid");
        assert!(hybrid.trace.lexical_hits >= 1 && hybrid.trace.dense_hits >= 1);
        assert!(hybrid.trace.embedding_space.is_some());
    }

    #[test]
    fn a_lexical_only_leader_reaches_a_small_context() {
        // `xq7` is the rare term, so its chunk is lexical #1, but it falls
        // outside the dense top k; the garanti chunks are in both lists and
        // outscore it under plain RRF.
        let embedder = FakeEmbedder::new()
            .with_fixture("garanti", vec![0.0, 1.0])
            .with_fixture("xq7", vec![1.0, 0.0]);
        let (_dir, mut retriever) = indexed(
            embedder,
            &[
                ("a.md", "# Kod\n\nxq7 kodu."),
                ("b.md", "# Garanti 1\n\ngaranti ortak süre bir"),
                ("c.md", "# Garanti 2\n\ngaranti ortak süre iki"),
                ("d.md", "# Garanti 3\n\ngaranti ortak süre üç"),
            ],
        );
        retriever.config.dense_k = 3;
        let outcome = retriever
            .retrieve(1, "xq7 garanti ortak", Strategy::Hybrid, 2)
            .unwrap();
        let kod = outcome
            .candidates
            .iter()
            .find(|c| c.chunk.document_title.as_deref() == Some("Kod"))
            .expect("the lexical #1 chunk is in the top 2");
        assert_eq!((kod.hit.lexical_rank, kod.hit.dense_rank), (Some(1), None));
    }

    #[test]
    fn query_without_lexical_terms_still_uses_dense() {
        let (_dir, retriever) = indexed(embedder(), &[("b.md", SEMANTIC)]);
        let outcome = retriever.retrieve(1, "???", Strategy::Hybrid, 8).unwrap();
        assert_eq!(outcome.trace.lexical_hits, 0);
        assert_eq!(outcome.candidates.len(), 1);
    }

    #[test]
    fn never_indexed_collection_returns_empty_and_unknown_is_not_found() {
        let (_dir, retriever) = indexed(embedder(), &[]);
        let outcome = retriever
            .retrieve(1, "anything", Strategy::Hybrid, 8)
            .unwrap();
        assert!(outcome.candidates.is_empty());
        assert!(matches!(
            retriever.retrieve(77, "x", Strategy::Hybrid, 8),
            Err(OragError::NotFound { .. })
        ));
    }

    #[test]
    fn limit_caps_candidates() {
        let docs: Vec<(String, String)> = (0..5)
            .map(|i| (format!("{i}.md"), format!("# D{i}\n\nortak kelime {i}")))
            .collect();
        let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let (_dir, retriever) = indexed(FakeEmbedder::new(), &refs);
        assert_eq!(
            retriever
                .retrieve(1, "ortak", Strategy::Hybrid, 3)
                .unwrap()
                .candidates
                .len(),
            3
        );
    }

    #[test]
    fn a_query_too_long_for_the_embedder_is_invalid_input() {
        use crate::domain::space::SpaceDescriptor;
        use crate::infer::Embedder;
        struct Small(FakeEmbedder, SpaceDescriptor);
        impl Embedder for Small {
            fn descriptor(&self) -> &SpaceDescriptor {
                &self.1
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.0.count_tokens(t)
            }
            fn embed_documents(&self, t: &[String]) -> Result<Vec<Vec<f32>>> {
                self.0.embed_documents(t)
            }
            fn embed_query(&self, t: &str) -> Result<Vec<f32>> {
                self.0.embed_query(t)
            }
        }
        let inner = FakeEmbedder::new();
        let mut desc = inner.descriptor().clone();
        desc.max_tokens = 8;
        let small = Small(inner, desc);
        assert!(check_query_fits(&small, "kisa soru").is_ok());
        let err = check_query_fits(&small, &"kelime ".repeat(20)).unwrap_err();
        assert!(matches!(err, OragError::InvalidInput(_)), "{err:?}");
    }

    #[test]
    fn lexical_search_works_after_an_embedding_model_change() {
        use crate::domain::space::SpaceDescriptor;
        use crate::infer::Embedder;
        struct Other(FakeEmbedder, SpaceDescriptor);
        impl Embedder for Other {
            fn descriptor(&self) -> &SpaceDescriptor {
                &self.1
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.0.count_tokens(t)
            }
            fn embed_documents(&self, t: &[String]) -> Result<Vec<Vec<f32>>> {
                self.0.embed_documents(t)
            }
            fn embed_query(&self, t: &str) -> Result<Vec<f32>> {
                self.0.embed_query(t)
            }
        }
        let (_dir, retriever) = indexed(
            FakeEmbedder::new(),
            &[("k.md", "# Kargo\n\nKargo ücretsizdir.")],
        );
        let mut desc = FakeEmbedder::new().descriptor().clone();
        desc.model_id = "other".into();
        let other = Retriever {
            embedder: Arc::new(Other(FakeEmbedder::new(), desc)),
            ..retriever
        };
        let lexical = other.retrieve(1, "kargo", Strategy::Lexical, 5).unwrap();
        assert_eq!(lexical.candidates.len(), 1);
        assert!(matches!(
            other.retrieve(1, "kargo", Strategy::Hybrid, 5),
            Err(OragError::ReindexRequired { .. })
        ));
    }
}
