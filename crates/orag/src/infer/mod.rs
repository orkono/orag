//! Inference boundaries. Everything model-specific lives behind these traits;
//! the RAG core never sees llama.cpp types (D-002).

pub mod fake;

use std::ops::ControlFlow;

use serde::Serialize;

use crate::domain::space::{EmbeddingSpace, SpaceDescriptor};
use crate::domain::{ChunkId, CollectionId};
use crate::error::{OragError, Result};

pub trait Embedder: Send + Sync {
    fn descriptor(&self) -> &SpaceDescriptor;
    /// Tokens the model sees for `text`, excluding prefixes and special tokens.
    fn count_tokens(&self, text: &str) -> usize;
    /// Embeds document texts (document prefix applied inside). Output is always
    /// L2-normalized here; `SpaceDescriptor::normalized` records whether the
    /// model's own pooling already normalizes, which is part of the space.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Embeds a query (query prefix applied inside). Output is L2-normalized.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRequest {
    pub messages: Vec<ChatMessage>,
    pub max_output_tokens: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GenerationStats {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cancelled: bool,
}

pub trait Generator: Send + Sync {
    fn model_id(&self) -> &str;
    /// Total context window (prompt + output) in tokens.
    fn context_tokens(&self) -> usize;
    fn max_output_tokens(&self) -> usize;
    /// Exact prompt size after applying the chat template.
    fn count_prompt_tokens(&self, messages: &[ChatMessage]) -> Result<usize>;
    /// Streams text pieces to `on_token`; `ControlFlow::Break(())` cancels.
    fn generate(
        &self,
        request: &GenerationRequest,
        on_token: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<GenerationStats>;
}

pub trait VectorIndex: Send + Sync {
    /// Nearest chunks of `collection_id` in `space`, nearest first, as
    /// `(chunk_id, cosine_similarity)`. The collection filter applies before top-k.
    fn search(
        &self,
        space: &EmbeddingSpace,
        collection_id: CollectionId,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(ChunkId, f32)>>;
}

/// Returns `v / ‖v‖`. A zero, non-finite or overflowing vector is a model
/// error: storing it would silently corrupt the dense ranking.
pub fn l2_normalize(v: &[f32]) -> Result<Vec<f32>> {
    let norm = v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(OragError::Model(format!(
            "embedding has no usable length (norm {norm})"
        )));
    }
    let unit: Vec<f32> = v.iter().map(|x| (f64::from(*x) / norm) as f32).collect();
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_components_do_not_overflow() {
        let v = l2_normalize(&[1e30, 1e30]).unwrap();
        assert!((v[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn normalizing_gives_unit_length() {
        let v = l2_normalize(&[3.0, 4.0]).unwrap();
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn non_finite_or_zero_vectors_are_errors() {
        for bad in [vec![f32::NAN, 1.0], vec![f32::INFINITY], vec![0.0, 0.0]] {
            assert!(l2_normalize(&bad).is_err(), "{bad:?}");
        }
    }
}
