//! Deterministic fakes for tests and `orag serve --dev-fake-models`.

use std::ops::ControlFlow;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::domain::normalize::normalize_for_lexical;
use crate::domain::space::SpaceDescriptor;
use crate::error::{OragError, Result};
use crate::infer::{
    ChatMessage, Embedder, GenerationRequest, GenerationStats, Generator, l2_normalize,
};

pub const FAKE_DIMENSIONS: usize = 16;
/// Word-count "tokens" the fake embedder accepts per text.
pub const FAKE_MAX_TOKENS: usize = 4096;

pub struct FakeEmbedder {
    descriptor: SpaceDescriptor,
    fixtures: Vec<(String, Vec<f32>)>,
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeEmbedder {
    pub fn new() -> Self {
        FakeEmbedder {
            descriptor: SpaceDescriptor {
                model_id: "fake-embedder".into(),
                model_sha256: "0".repeat(64),
                pooling: "bag-of-words".into(),
                query_prefix: String::new(),
                document_prefix: String::new(),
                dimensions: FAKE_DIMENSIONS,
                normalized: true,
                max_tokens: FAKE_MAX_TOKENS,
                require_trailing_eos: false,
            },
            fixtures: Vec::new(),
        }
    }

    /// Texts whose normalized form contains `needle` embed to `vector`
    /// (zero-padded to 16 dims, normalized). First matching fixture wins.
    ///
    /// Panics on a needle that normalizes to nothing (it would match every
    /// text), or a vector longer than 16 dims or of zero length.
    pub fn with_fixture(mut self, needle: &str, vector: Vec<f32>) -> Self {
        let words = words_of(&normalize_for_lexical(needle)).join(" ");
        assert!(!words.is_empty(), "fixture needle must contain text");
        let needle = format!(" {words} ");
        assert!(
            vector.len() <= FAKE_DIMENSIONS,
            "fixture vector has more than {FAKE_DIMENSIONS} dims"
        );
        let mut padded = vector;
        padded.resize(FAKE_DIMENSIONS, 0.0);
        let unit = l2_normalize(&padded).expect("fixture vector must be non-zero and finite");
        self.fixtures.push((needle, unit));
        self
    }

    /// Like a real model, refuses text over the descriptor's `max_tokens`.
    fn embed_checked(&self, text: &str) -> Result<Vec<f32>> {
        crate::infer::require_text(text)?;
        let limit = self.descriptor.max_tokens;
        if self.count_tokens(text) > limit {
            return Err(OragError::Model(format!(
                "input has more than {limit} tokens"
            )));
        }
        Ok(self.embed_one(text))
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        let normalized = normalize_for_lexical(text);
        // Whole words only: `orx` must not match inside `korxan`.
        let padded = format!(" {} ", words_of(&normalized).join(" "));
        if let Some((_, vector)) = self
            .fixtures
            .iter()
            .find(|(needle, _)| padded.contains(needle.as_str()))
        {
            return vector.clone();
        }
        let mut vector = vec![0.0f32; FAKE_DIMENSIONS];
        // Every distinct word of the whole text: `lexical_terms` is for queries
        // (first 4096 characters, 32 terms), not for document text.
        let words: std::collections::BTreeSet<&str> = words_of(&normalized).into_iter().collect();
        for term in words {
            vector[(fnv1a(term.as_bytes()) % FAKE_DIMENSIONS as u64) as usize] += 1.0;
        }
        if vector.iter().all(|x| *x == 0.0) {
            vector[0] = 1.0;
        }
        l2_normalize(&vector).expect("word counts are finite and non-zero")
    }
}

fn words_of(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

impl Embedder for FakeEmbedder {
    fn descriptor(&self) -> &SpaceDescriptor {
        &self.descriptor
    }

    fn count_tokens(&self, text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|text| self.embed_checked(text)).collect()
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_checked(text)
    }
}

pub struct FakeGenerator {
    reply: String,
    context_tokens: usize,
    max_output_tokens: usize,
    token_delay: Option<Duration>,
    /// Only the latest request: the fake also backs a long-running dev server.
    last_request: Mutex<Option<GenerationRequest>>,
    emitted: AtomicUsize,
}

impl FakeGenerator {
    pub fn new(reply: &str) -> Self {
        FakeGenerator {
            reply: reply.into(),
            context_tokens: 4096,
            max_output_tokens: 256,
            token_delay: None,
            last_request: Mutex::new(None),
            emitted: AtomicUsize::new(0),
        }
    }

    /// Shrinks the context window (word-count "tokens") to exercise budgeting.
    pub fn with_context(mut self, context_tokens: usize, max_output_tokens: usize) -> Self {
        self.context_tokens = context_tokens;
        self.max_output_tokens = max_output_tokens;
        self
    }

    pub fn with_token_delay(mut self, delay: Duration) -> Self {
        self.token_delay = Some(delay);
        self
    }

    pub fn emitted_tokens(&self) -> usize {
        self.emitted.load(Ordering::SeqCst)
    }

    pub fn last_request(&self) -> Option<GenerationRequest> {
        self.last_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Generator for FakeGenerator {
    fn model_id(&self) -> &str {
        "fake-generator"
    }

    fn context_tokens(&self) -> usize {
        self.context_tokens
    }

    fn max_output_tokens(&self) -> usize {
        self.max_output_tokens
    }

    fn count_prompt_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        Ok(messages
            .iter()
            .map(|m| m.content.split_whitespace().count() + 4)
            .sum())
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        on_token: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<GenerationStats> {
        *self
            .last_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(request.clone());
        let prompt_tokens = self.count_prompt_tokens(&request.messages)?;
        // Like the real backend: output is capped by its own maximum, and a
        // prompt that leaves no room for output is refused.
        let output_budget = request.max_output_tokens.min(self.max_output_tokens);
        if prompt_tokens + output_budget > self.context_tokens {
            return Err(OragError::InvalidInput(format!(
                "prompt of {prompt_tokens} tokens plus {output_budget} output tokens exceeds the \
                 {}-token context",
                self.context_tokens
            )));
        }
        let mut stats = GenerationStats {
            prompt_tokens,
            ..GenerationStats::default()
        };
        for piece in self.reply.split_inclusive(' ').take(output_budget) {
            if let Some(delay) = self.token_delay {
                std::thread::sleep(delay);
            }
            // Counts every generated piece, including one the consumer then
            // rejects with Break, as a real model has already produced it.
            self.emitted.fetch_add(1, Ordering::SeqCst);
            stats.completion_tokens += 1;
            if on_token(piece).is_break() {
                stats.cancelled = true;
                break;
            }
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::Role;
    use std::ops::ControlFlow;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn fixture_vectors_win_and_are_normalized() {
        let e = FakeEmbedder::new().with_fixture("iade", vec![3.0, 4.0]);
        let v = e.embed_query("İade süresi?").unwrap();
        assert_eq!(v.len(), FAKE_DIMENSIONS);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn shared_words_are_more_similar_than_unrelated_text() {
        let e = FakeEmbedder::new();
        let docs = e
            .embed_documents(&[
                "return policy days".into(),
                "coffee machine descaling".into(),
            ])
            .unwrap();
        let q = e.embed_query("return policy").unwrap();
        assert!(cosine(&q, &docs[0]) > cosine(&q, &docs[1]));
    }

    #[test]
    fn generator_streams_reply_and_records_request() {
        let g = FakeGenerator::new("Cevap [1] burada.");
        let mut out = String::new();
        let req = GenerationRequest {
            messages: vec![ChatMessage {
                role: Role::User,
                content: "q".into(),
            }],
            max_output_tokens: 10,
        };
        let stats = g
            .generate(&req, &mut |piece| {
                out.push_str(piece);
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(out, "Cevap [1] burada.");
        assert_eq!(stats.completion_tokens, 3);
        assert!(!stats.cancelled);
        assert_eq!(g.last_request().unwrap(), req);
    }

    #[test]
    fn generator_stops_when_consumer_breaks() {
        let g = FakeGenerator::new("a b c d e");
        let req = GenerationRequest {
            messages: vec![],
            max_output_tokens: 10,
        };
        let mut seen = 0;
        let stats = g
            .generate(&req, &mut |_| {
                seen += 1;
                if seen == 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert!(stats.cancelled);
        assert_eq!(g.emitted_tokens(), 2);
    }

    #[test]
    fn generator_respects_the_output_budget() {
        let g = FakeGenerator::new("a b c d e");
        let req = GenerationRequest {
            messages: vec![],
            max_output_tokens: 2,
        };
        let mut out = String::new();
        let stats = g
            .generate(&req, &mut |piece| {
                out.push_str(piece);
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!((out.as_str(), stats.completion_tokens), ("a b ", 2));
    }

    #[test]
    fn embedder_refuses_text_over_its_token_limit() {
        let e = FakeEmbedder::new();
        let long = "w ".repeat(FAKE_MAX_TOKENS + 1);
        assert!(e.embed_documents(&[long]).is_err());
    }

    #[test]
    #[should_panic(expected = "fixture")]
    fn a_zero_fixture_vector_is_refused() {
        let _ = FakeEmbedder::new().with_fixture("x", vec![0.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "fixture")]
    fn an_oversized_fixture_vector_is_refused() {
        let _ = FakeEmbedder::new().with_fixture("x", vec![1.0; FAKE_DIMENSIONS + 1]);
    }

    #[test]
    fn generator_refuses_a_prompt_that_leaves_no_room() {
        let g = FakeGenerator::new("a b c").with_context(8, 4);
        let long = ChatMessage {
            role: Role::User,
            content: "w ".repeat(10),
        };
        let req = GenerationRequest {
            messages: vec![long],
            max_output_tokens: 256,
        };
        assert!(
            g.generate(&req, &mut |_| ControlFlow::Continue(()))
                .is_err()
        );
    }

    #[test]
    fn generator_caps_output_at_its_own_maximum() {
        let g = FakeGenerator::new("a b c d e").with_context(64, 2);
        let req = GenerationRequest {
            messages: vec![],
            max_output_tokens: 256,
        };
        let stats = g
            .generate(&req, &mut |_| ControlFlow::Continue(()))
            .unwrap();
        assert_eq!(stats.completion_tokens, 2);
    }

    #[test]
    fn a_long_query_is_refused_too() {
        let long = "w ".repeat(FAKE_MAX_TOKENS + 1);
        assert!(FakeEmbedder::new().embed_query(&long).is_err());
    }

    #[test]
    fn fixtures_match_whole_words_only() {
        let e = FakeEmbedder::new().with_fixture("orx", vec![1.0, 0.0]);
        let inside = e.embed_query("korxan bey").unwrap();
        assert!(
            inside[0] < 0.99,
            "a needle inside another word must not match"
        );
        assert!((e.embed_query("Orx nedir").unwrap()[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fake_embedder_rejects_blank_text_like_the_real_one() {
        let embedder = FakeEmbedder::new();
        assert!(matches!(
            embedder.embed_query("  "),
            Err(OragError::InvalidInput(_))
        ));
        assert!(matches!(
            embedder.embed_documents(&["ok".into(), "".into()]),
            Err(OragError::InvalidInput(_))
        ));
    }
}
