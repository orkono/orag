//! Stand-ins for models that are not installed (setup mode, D-023).
//!
//! Every inference call fails with `ModelsMissing`; the HTTP layer refuses
//! that work before it reaches them, so these only guard against a path that
//! forgot the check.

use std::ops::ControlFlow;

use crate::domain::space::SpaceDescriptor;
use crate::error::{OragError, Result};
use crate::infer::{ChatMessage, Embedder, GenerationRequest, GenerationStats, Generator};

fn missing(id: &str) -> OragError {
    OragError::ModelsMissing(format!(
        "model `{id}` is not installed; import the model packs first"
    ))
}

pub struct MissingEmbedder {
    id: String,
    descriptor: SpaceDescriptor,
}

impl MissingEmbedder {
    pub fn new(id: &str) -> MissingEmbedder {
        MissingEmbedder {
            id: id.to_string(),
            // Matches no bound space, so nothing can be written under it.
            descriptor: SpaceDescriptor {
                model_id: format!("missing:{id}"),
                model_sha256: "0".repeat(64),
                pooling: "none".into(),
                query_prefix: String::new(),
                document_prefix: String::new(),
                dimensions: 1,
                normalized: true,
                max_tokens: 1,
                require_trailing_eos: false,
            },
        }
    }
}

impl Embedder for MissingEmbedder {
    fn descriptor(&self) -> &SpaceDescriptor {
        &self.descriptor
    }

    fn count_tokens(&self, text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn embed_documents(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Err(missing(&self.id))
    }

    fn embed_query(&self, _text: &str) -> Result<Vec<f32>> {
        Err(missing(&self.id))
    }
}

pub struct MissingGenerator {
    id: String,
}

impl MissingGenerator {
    pub fn new(id: &str) -> MissingGenerator {
        MissingGenerator { id: id.to_string() }
    }
}

impl Generator for MissingGenerator {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn context_tokens(&self) -> usize {
        0
    }

    fn max_output_tokens(&self) -> usize {
        0
    }

    fn count_prompt_tokens(&self, _messages: &[ChatMessage]) -> Result<usize> {
        Err(missing(&self.id))
    }

    fn generate(
        &self,
        _request: &GenerationRequest,
        _on_token: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<GenerationStats> {
        Err(missing(&self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_inference_call_says_models_missing() {
        let embedder = MissingEmbedder::new("emb");
        assert_eq!(embedder.descriptor().model_id, "missing:emb");
        assert!(matches!(
            embedder.embed_query("x"),
            Err(OragError::ModelsMissing(_))
        ));
        let generator = MissingGenerator::new("gen");
        let request = GenerationRequest {
            messages: Vec::new(),
            max_output_tokens: 1,
            sampler: Default::default(),
        };
        let err = generator
            .generate(&request, &mut |_| ControlFlow::Continue(()))
            .unwrap_err();
        assert!(err.to_string().contains("`gen`"), "{err}");
    }
}
