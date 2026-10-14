//! Cross-encoder reranking with llama.cpp rank pooling (v0.4 candidate).
//! The model is shared; each call builds its own small context, so scoring
//! runs on the caller's thread and never waits behind ingestion.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;

use crate::error::{OragError, Result};
use crate::infer::Reranker;
use crate::infer::llama::{backend, load_model, model_error, tokenize};
use crate::infer::models::{InstalledModel, RerankInput, RerankerSpec};
use crate::infer::prompt::neutralize_special_markers;

/// GGUF metadata key of the Qwen3-Reranker prompt.
const TEMPLATE_KEY: &str = "tokenizer.chat_template.rerank";

pub struct LlamaReranker {
    model_id: String,
    spec: RerankerSpec,
    model: Arc<LlamaModel>,
    /// `(before the document, after it)` for template input; the question
    /// is filled in per call.
    template: Option<(String, String)>,
}

impl LlamaReranker {
    pub fn load(installed: &InstalledModel) -> Result<LlamaReranker> {
        let manifest = &installed.manifest;
        let spec = manifest.reranker.clone().ok_or_else(|| {
            OragError::InvalidInput(format!("`{}` has no [reranker] section", manifest.id))
        })?;
        let model = Arc::new(load_model(&installed.model_path())?);
        let template = match spec.input {
            RerankInput::Pair => None,
            RerankInput::GgufTemplate => {
                Some(split_template(&model.meta_val_str(TEMPLATE_KEY).map_err(
                    |e| OragError::Model(format!("`{}` has no {TEMPLATE_KEY}: {e}", manifest.id)),
                )?)?)
            }
        };
        if spec.relevant_output >= model.n_cls_out().max(1) as usize {
            return Err(OragError::Model(format!(
                "`{}`: relevant_output {} but the model has {} classifier outputs",
                manifest.id,
                spec.relevant_output,
                model.n_cls_out()
            )));
        }
        Ok(LlamaReranker {
            model_id: manifest.id.clone(),
            spec,
            model,
            template,
        })
    }

    /// The pair's tokens within `max_tokens`: the passage is cut, the
    /// question and the template's fixed parts never are.
    fn pair_tokens(&self, query: &str, passage: &str) -> Result<Vec<LlamaToken>> {
        let model = self.model.as_ref();
        let query = neutralize_special_markers(query);
        let passage = neutralize_special_markers(passage);
        let doc = tokenize(model, &passage, false, false);
        let (head, tail) = match &self.template {
            None => {
                let vocab = model.vocab();
                let mut head = vec![vocab.bos()];
                head.extend(tokenize(model, &query, false, false));
                head.extend([vocab.eos(), vocab.sep()]);
                (head, vec![vocab.eos()])
            }
            Some((before, after)) => {
                // Only the template's own text may hold control tokens; the
                // question and passage are tokenized as plain text.
                let (lead, rest) = before.split_once("{query}").unwrap_or((before, ""));
                let mut head = tokenize(model, lead, false, true);
                head.extend(tokenize(model, &query, false, false));
                head.extend(tokenize(model, rest, false, true));
                (head, tokenize(model, after, false, true))
            }
        };
        let room = self
            .spec
            .max_tokens
            .checked_sub(head.len() + tail.len())
            .filter(|&room| room > 0)
            .ok_or_else(|| {
                OragError::InvalidInput(format!(
                    "the question is too long for the reranker ({} tokens max)",
                    self.spec.max_tokens
                ))
            })?;
        Ok(head
            .into_iter()
            .chain(doc.into_iter().take(room))
            .chain(tail)
            .collect())
    }

    fn relevance(&self, outputs: &[f32]) -> f32 {
        relevance(outputs, self.spec.relevant_output)
    }
}

impl Reranker for LlamaReranker {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn score(
        &self,
        query: &str,
        passages: &[String],
        deadline: Option<Instant>,
    ) -> Result<Vec<f32>> {
        if passages.is_empty() {
            return Ok(Vec::new());
        }
        let n = u32::try_from(self.spec.max_tokens).unwrap_or(u32::MAX);
        let params = LlamaContextParams::default()
            .with_embeddings(true)
            .with_n_ctx(NonZeroU32::new(n))
            .with_n_batch(n)
            .with_n_ubatch(n)
            .with_pooling_type(LlamaPoolingType::Rank);
        let mut ctx = self
            .model
            .new_context(backend()?, params)
            .map_err(model_error)?;
        let mut scores = Vec::with_capacity(passages.len());
        for passage in passages {
            if deadline.is_some_and(|d| Instant::now() > d) {
                return Err(OragError::Model("reranking ran out of time".into()));
            }
            let tokens = self.pair_tokens(query, passage)?;
            ctx.clear_kv_cache();
            let mut batch = LlamaBatch::new(tokens.len(), 1);
            batch.add_sequence(&tokens, 0, false).map_err(model_error)?;
            ctx.decode(&mut batch).map_err(model_error)?;
            let outputs = ctx.embeddings_seq_ith(0).map_err(model_error)?;
            scores.push(self.relevance(outputs));
        }
        Ok(scores)
    }
}

/// The log-odds of the relevant output against the others. llama.cpp
/// returns probabilities for some models (Qwen3-Reranker: softmax over
/// yes/no) and logits for others; log-odds keeps the ranking resolution that
/// a probability near 1 loses in `f32`. A single output is the score itself.
fn relevance(outputs: &[f32], relevant: usize) -> f32 {
    match outputs {
        [] => f32::NAN,
        [only] => *only,
        many => {
            let sum: f32 = many.iter().sum();
            let probabilities =
                many.iter().all(|v| (0.0..=1.0).contains(v)) && (sum - 1.0).abs() < 1e-3;
            let rest = many
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != relevant)
                .map(|(_, v)| *v);
            if probabilities {
                many[relevant].ln() - rest.sum::<f32>().ln()
            } else {
                let max = rest.clone().fold(f32::NEG_INFINITY, f32::max);
                many[relevant] - max - rest.map(|v| (v - max).exp()).sum::<f32>().ln()
            }
        }
    }
}

/// Splits a rerank template at `{document}`; it must hold `{query}` before it.
fn split_template(template: &str) -> Result<(String, String)> {
    let (before, after) = template
        .split_once("{document}")
        .filter(|(before, _)| before.contains("{query}"))
        .ok_or_else(|| {
            OragError::Model("rerank template needs {query} before {document}".into())
        })?;
    Ok((before.to_string(), after.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_is_the_log_odds_of_the_relevant_output() {
        assert_eq!(relevance(&[2.5], 0), 2.5);
        assert!(relevance(&[], 0).is_nan());
        // Probabilities near 1 still order: 1 - 1e-7 against 1 - 1e-9.
        let a = relevance(&[1.0 - 1e-7, 1e-7], 0);
        let b = relevance(&[1.0, 1e-9], 0);
        assert!(b > a && a > 15.0, "{a} {b}");
        // Logits: log-odds of yes against no.
        assert!((relevance(&[3.0, 1.0], 0) - 2.0).abs() < 1e-6);
        assert!((relevance(&[3.0, 1.0], 1) + 2.0).abs() < 1e-6);
    }

    #[test]
    fn templates_split_at_the_document_after_the_query() {
        let (before, after) = split_template("<Q>: {query}\n<D>: {document}<end>").unwrap();
        assert_eq!(
            (before.as_str(), after.as_str()),
            ("<Q>: {query}\n<D>: ", "<end>")
        );
        assert!(split_template("{document} then {query}").is_err());
        assert!(split_template("no slots").is_err());
    }
}
