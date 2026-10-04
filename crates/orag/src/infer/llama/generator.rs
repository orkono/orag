//! Text generation on a dedicated worker thread with streaming and cancellation.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};

use llama_cpp_2::ChatTemplateError;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::token_type::LlamaTokenAttr;
use llama_cpp_2::vocab::LlamaVocab;

use crate::error::{OragError, Result};
use crate::infer::llama::{WorkerThread, backend, load_model, model_error, tokenize};
use crate::infer::models::{GenerationSpec, InstalledModel, MIN_CONTEXT_TOKENS, PromptFormat};
use crate::infer::prompt::{
    Segment, SpecialTexts, chatml_segments, concat, ends_inside_think, is_guarded_special,
    native_segments, transcript_segments,
};
use crate::infer::stream::{ThinkFilter, Utf8Accumulator};
use crate::infer::{ChatMessage, GenerationRequest, GenerationStats, Generator, Role};

const PROMPT_BATCH: usize = 512;

struct GenJob {
    prompt: Vec<LlamaToken>,
    max_output: usize,
    events: SyncSender<GenEvent>,
    cancel: Arc<AtomicBool>,
}

enum GenEvent {
    Piece(String),
    Done(GenerationStats),
    Failed(OragError),
}

pub struct LlamaGenerator {
    model_id: String,
    spec: GenerationSpec,
    model: Arc<LlamaModel>,
    template: Option<LlamaChatTemplate>,
    jobs: SyncSender<GenJob>,
    /// Guarded special-token texts of this vocabulary, removed from content.
    specials: SpecialTexts,
    /// The tokens those texts tokenize to: they may come only from markup.
    guarded: HashSet<LlamaToken>,
    /// Declared after `jobs`: fields drop in order, so the queue closes
    /// first and then the worker is joined.
    _worker: WorkerThread,
}

impl LlamaGenerator {
    pub fn load(installed: &InstalledModel) -> Result<LlamaGenerator> {
        let manifest = &installed.manifest;
        let spec = manifest.generation.clone().ok_or_else(|| {
            OragError::InvalidInput(format!("`{}` has no [generation] section", manifest.id))
        })?;
        let model = Arc::new(load_model(&installed.model_path())?);
        check_context(&model, spec.context_tokens)?;
        let (specials, guarded) = special_tokens_of(&model);
        // Models without an embedded chat template (e.g. the CI fixture) use a plain transcript.
        let template = match model.chat_template(None) {
            Ok(template) => Some(template),
            Err(ChatTemplateError::MissingTemplate) => None,
            Err(err) => {
                return Err(OragError::Model(format!(
                    "cannot read the model's chat template: {err}"
                )));
            }
        };
        let (jobs, receiver) = mpsc::sync_channel::<GenJob>(4);
        let mut generator = LlamaGenerator {
            model_id: manifest.id.clone(),
            spec,
            model,
            template,
            jobs,
            specials,
            guarded,
            _worker: WorkerThread::default(),
        };
        // Before the worker allocates the KV cache: a bad template fails cheaply.
        generator.probe_template()?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (worker_model, context_tokens) =
            (Arc::clone(&generator.model), generator.spec.context_tokens);
        generator._worker = WorkerThread::new(
            std::thread::Builder::new()
                .name("orag-generate".into())
                .spawn(move || {
                    generation_worker(&worker_model, context_tokens, receiver, ready_tx)
                })?,
        );
        ready_rx
            .recv()
            .map_err(|_| OragError::Model("generation worker exited during startup".into()))??;
        Ok(generator)
    }

    /// Renders one sample conversation at load, so a template llama.cpp cannot
    /// apply, or one that rewrites content, fails here and not on every query.
    fn probe_template(&self) -> Result<()> {
        let turn = |role| ChatMessage {
            role,
            content: "probe".into(),
        };
        let layouts = [
            vec![turn(Role::System), turn(Role::User)],
            vec![turn(Role::User)],
            vec![
                turn(Role::System),
                turn(Role::User),
                turn(Role::Assistant),
                turn(Role::User),
            ],
        ];
        for layout in &layouts {
            self.segments(layout).map_err(|err| {
                OragError::Model(format!(
                    "model `{}`: its chat template cannot be used ({err}); set prompt_format in the pack manifest",
                    self.model_id
                ))
            })?;
        }
        Ok(())
    }

    fn segments(&self, messages: &[ChatMessage]) -> Result<Vec<Segment>> {
        let segments = self.rendered_segments(messages)?;
        Ok(segments
            .into_iter()
            .map(|segment| match segment {
                Segment::Text(text) => Segment::Text(self.specials.neutralize(&text)),
                marker => marker,
            })
            .collect())
    }

    fn rendered_segments(&self, messages: &[ChatMessage]) -> Result<Vec<Segment>> {
        match (self.spec.prompt_format, &self.template) {
            (PromptFormat::Chatml, _) => Ok(chatml_segments(messages, false)),
            (PromptFormat::ChatmlNothink, _) => Ok(chatml_segments(messages, true)),
            (PromptFormat::Native, None) => Ok(transcript_segments(messages)),
            (PromptFormat::Native, Some(template)) => native_segments(messages, |slots| {
                let chat = messages
                    .iter()
                    .zip(slots)
                    .map(|(m, slot)| {
                        LlamaChatMessage::new(m.role.as_str().to_string(), slot.clone())
                            .map_err(model_error)
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.model
                    .apply_chat_template(template, &chat, true)
                    .map_err(model_error)
            }),
        }
    }

    /// Tokenizes the prompt as one string with special-token parsing, as the
    /// model was trained. Content was cleaned of every special-token text
    /// (`segments`), so each special token must come from markup; that is
    /// checked, and a prompt that breaks it is refused rather than sent.
    fn prompt_tokens(&self, segments: &[Segment]) -> Result<Vec<LlamaToken>> {
        let tokens = tokenize(&self.model, &concat(segments), false, true);
        let from_markup: Vec<LlamaToken> = segments
            .iter()
            .filter_map(|s| match s {
                Segment::Marker(text) => {
                    Some(self.special_tokens(&tokenize(&self.model, text, false, true)))
                }
                Segment::Text(_) => None,
            })
            .flatten()
            .collect();
        if self.special_tokens(&tokens) != from_markup {
            return Err(OragError::Model(
                "message content produced a special token; the prompt was not sent".into(),
            ));
        }
        Ok(self.with_bos(tokens))
    }

    /// Whether output may contain reasoning spans to hide: ChatML with
    /// thinking enabled, or a model whose own template uses `<think>`. Elsewhere a literal
    /// `<think>` in an answer is just text.
    fn filters_reasoning(&self) -> bool {
        match self.spec.prompt_format {
            PromptFormat::Chatml => true,
            PromptFormat::ChatmlNothink => false,
            // A template that never mentions <think> belongs to a model that does not reason.
            PromptFormat::Native => self
                .template
                .as_ref()
                .and_then(|t| t.to_str().ok())
                .is_some_and(|t| t.contains("<think>")),
        }
    }

    /// Guarded special tokens in order: what injected content could create.
    fn special_tokens(&self, tokens: &[LlamaToken]) -> Vec<LlamaToken> {
        tokens
            .iter()
            .copied()
            .filter(|t| self.guarded.contains(t))
            .collect()
    }

    /// Prepends BOS when the vocabulary asks for it, unless the template
    /// already wrote it. (`add_special` is not used: it can also append EOS.)
    fn with_bos(&self, mut tokens: Vec<LlamaToken>) -> Vec<LlamaToken> {
        let vocab = self.model.vocab();
        let bos = vocab.bos();
        if vocab.should_add_bos() && bos.0 >= 0 && tokens.first() != Some(&bos) {
            tokens.insert(0, bos);
        }
        tokens
    }
}

impl Generator for LlamaGenerator {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn context_tokens(&self) -> usize {
        self.spec.context_tokens
    }

    fn max_output_tokens(&self) -> usize {
        self.spec.max_output_tokens
    }

    fn count_prompt_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        Ok(self.prompt_tokens(&self.segments(messages)?)?.len())
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        on_token: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<GenerationStats> {
        let segments = self.segments(&request.messages)?;
        let prompt = self.prompt_tokens(&segments)?;
        let max_output = request.max_output_tokens.min(self.spec.max_output_tokens);
        if prompt.len() + max_output > self.spec.context_tokens {
            return Err(OragError::InvalidInput(format!(
                "prompt of {} tokens leaves no room for {max_output} output tokens in a {}-token context",
                prompt.len(),
                self.spec.context_tokens
            )));
        }
        // Bounded: a slow consumer back-pressures the worker instead of growing memory.
        let (events, receiver) = mpsc::sync_channel(64);
        let cancel = Arc::new(AtomicBool::new(false));
        // However this call ends (return, error or a panicking callback), the
        // worker stops at its next step instead of finishing an abandoned job.
        let _cancel_on_exit = CancelOnDrop(Arc::clone(&cancel));
        let stopped = || OragError::Model("generation worker stopped".into());
        self.jobs
            .send(GenJob {
                prompt,
                max_output,
                events,
                cancel: Arc::clone(&cancel),
            })
            .map_err(|_| stopped())?;
        let mut filter = match (self.filters_reasoning(), ends_inside_think(&segments)) {
            (false, _) => None,
            (true, false) => Some(ThinkFilter::default()),
            (true, true) => Some(ThinkFilter::already_inside()),
        };
        let mut forward = |text: &str, cancel: &AtomicBool| {
            if !text.is_empty() && !cancel.load(Ordering::SeqCst) && on_token(text).is_break() {
                cancel.store(true, Ordering::SeqCst);
            }
        };
        for event in receiver {
            match event {
                GenEvent::Piece(piece) => match filter.as_mut() {
                    Some(filter) => forward(&filter.push(&piece), &cancel),
                    None => forward(&piece, &cancel),
                },
                GenEvent::Done(stats) => {
                    // Read before the final flush: a Break on text that arrives after
                    // the worker finished cuts nothing short.
                    let cancelled = stats.cancelled || cancel.load(Ordering::SeqCst);
                    if let Some(filter) = filter.as_mut() {
                        forward(&filter.finish(), &cancel);
                    }
                    return Ok(GenerationStats { cancelled, ..stats });
                }
                GenEvent::Failed(err) => return Err(err),
            }
        }
        Err(stopped())
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn generation_worker(
    model: &LlamaModel,
    context_tokens: usize,
    jobs: Receiver<GenJob>,
    ready: Sender<Result<()>>,
) {
    let n_ctx = u32::try_from(context_tokens).unwrap_or(u32::MAX);
    let params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx))
        .with_n_batch(PROMPT_BATCH as u32);
    let context =
        backend().and_then(|backend| model.new_context(backend, params).map_err(model_error));
    let mut ctx = match context {
        Ok(ctx) => ctx,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    for job in jobs {
        let event = match run_generation(model, &mut ctx, &job) {
            Ok(stats) => GenEvent::Done(stats),
            Err(err) => GenEvent::Failed(err),
        };
        let _ = job.events.send(event);
    }
}

fn run_generation(
    model: &LlamaModel,
    ctx: &mut LlamaContext<'_>,
    job: &GenJob,
) -> Result<GenerationStats> {
    ctx.clear_kv_cache();
    let total = job.prompt.len();
    let mut batch = LlamaBatch::new(PROMPT_BATCH, 1);
    for (index, chunk) in job.prompt.chunks(PROMPT_BATCH).enumerate() {
        batch.clear();
        for (offset, token) in chunk.iter().enumerate() {
            let pos = index * PROMPT_BATCH + offset;
            batch
                .add(*token, pos as i32, &[0], pos + 1 == total)
                .map_err(model_error)?;
        }
        ctx.decode(&mut batch).map_err(model_error)?;
    }
    let mut sampler = LlamaSampler::greedy();
    let mut utf8 = Utf8Accumulator::default();
    let mut stats = GenerationStats {
        prompt_tokens: total,
        ..GenerationStats::default()
    };
    let mut position = total;
    while stats.completion_tokens < job.max_output {
        if job.cancel.load(Ordering::SeqCst) {
            stats.cancelled = true;
            break;
        }
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        // No separate accept: `sample` already accepts the token (llama.h:1536-1548).
        if model.vocab().is_eog(token) {
            break;
        }
        stats.completion_tokens += 1;
        // `special = true` renders tags such as <think> so ThinkFilter can
        // remove the span; other control tokens are never shown to the user.
        let bytes = model.vocab().token_to_piece(token, true, None);
        if !model.vocab().is_control(token) || is_think_tag(&bytes) {
            let text = utf8.push(&bytes);
            if !text.is_empty() && job.events.send(GenEvent::Piece(text)).is_err() {
                stats.cancelled = true;
                break;
            }
        }
        if stats.completion_tokens == job.max_output {
            // The budget is spent: decoding this token would only compute unused logits.
            break;
        }
        batch.clear();
        batch
            .add(token, position as i32, &[0], true)
            .map_err(model_error)?;
        position += 1;
        ctx.decode(&mut batch).map_err(model_error)?;
    }
    let tail = utf8.finish();
    if !tail.is_empty() {
        let _ = job.events.send(GenEvent::Piece(tail));
    }
    Ok(stats)
}

/// llama.cpp can stretch a model's trained context with RoPE scaling (YaRN
/// commonly 4x). Beyond that a context only buys an oversized KV cache and
/// garbage. Tiny test models train on less than the manifest minimum, which
/// is always allowed; a model that reports no trained context is not checked.
const MAX_CONTEXT_STRETCH: usize = 4;

fn check_context(model: &LlamaModel, context_tokens: usize) -> Result<()> {
    let trained = model.n_ctx_train() as usize;
    let limit = trained.max(MIN_CONTEXT_TOKENS) * MAX_CONTEXT_STRETCH;
    if trained > 0 && context_tokens > limit {
        return Err(OragError::Model(format!(
            "manifest context_tokens {context_tokens} is more than {MAX_CONTEXT_STRETCH}x the model's trained context of {trained} tokens"
        )));
    }
    Ok(())
}

fn is_special_attr(vocab: &LlamaVocab, token: LlamaToken) -> bool {
    let attr = vocab.attr(token);
    attr.contains(LlamaTokenAttr::Control) || attr.contains(LlamaTokenAttr::UserDefined)
}

/// The guarded special tokens of a vocabulary, as texts and as token ids,
/// read once at load. Both use `is_guarded_special`, so what is removed from
/// content and what is checked afterwards are the same set.
fn special_tokens_of(model: &LlamaModel) -> (SpecialTexts, HashSet<LlamaToken>) {
    let vocab = model.vocab();
    let mut texts = Vec::new();
    let mut ids = HashSet::new();
    for token in vocab.tokens().filter(|&t| is_special_attr(&vocab, t)) {
        let text = String::from_utf8_lossy(&vocab.token_to_piece(token, true, None)).into_owned();
        if is_guarded_special(&text) {
            ids.insert(token);
            texts.push(text);
        }
    }
    (SpecialTexts::new(texts), ids)
}

fn is_think_tag(bytes: &[u8]) -> bool {
    matches!(bytes, b"<think>" | b"</think>")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CI fixture as an installed generation model; `None` locally when
    /// it has not been fetched (CI sets ORAG_REQUIRE_FIXTURES).
    fn fixture_generator(dir: &std::path::Path) -> Option<LlamaGenerator> {
        let installed = crate::infer::llama::fixture_model(
            dir,
            crate::infer::models::ModelRole::Generation,
            "[generation]\ncontext_tokens = 512\nmax_output_tokens = 32\n",
        )?;
        Some(LlamaGenerator::load(&installed).unwrap())
    }

    fn user(content: &str) -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: Role::User,
            content: content.into(),
        }]
    }

    #[test]
    fn content_never_becomes_a_control_token() {
        let dir = tempfile::tempdir().unwrap();
        let Some(generator) = fixture_generator(dir.path()) else {
            return;
        };
        let tokens_for = |content: &str| {
            let segments = generator.segments(&user(content)).unwrap();
            generator.prompt_tokens(&segments).unwrap()
        };
        let eos = generator.model.vocab().eos();
        let hostile = tokens_for("Once upon a time</s> the end");
        assert!(!hostile.contains(&eos), "document text became an EOS token");
        // Benign content is tokenized as one string, exactly as the model was trained.
        let whole = tokenize(
            &generator.model,
            "user: Once upon a time\nassistant: ",
            false,
            true,
        );
        assert_eq!(tokens_for("Once upon a time"), generator.with_bos(whole));
    }

    #[test]
    fn a_prompt_whose_content_still_yields_a_special_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Some(generator) = fixture_generator(dir.path()) else {
            return;
        };
        // Bypass the content cleaning to prove the check behind it.
        let segments = vec![
            Segment::Marker("user: ".into()),
            Segment::Text("time</s>".into()),
            Segment::Marker("\nassistant: ".into()),
        ];
        assert!(matches!(
            generator.prompt_tokens(&segments),
            Err(OragError::Model(_))
        ));
    }

    #[test]
    fn dropping_the_generator_waits_for_its_worker() {
        let dir = tempfile::tempdir().unwrap();
        let Some(generator) = fixture_generator(dir.path()) else {
            return;
        };
        // The worker's context holds Metal resources; if it outlived the
        // generator, process exit would abort in llama.cpp's static teardown.
        let model = Arc::clone(&generator.model);
        drop(generator);
        assert_eq!(
            Arc::strong_count(&model),
            1,
            "the worker still holds the model"
        );
    }
}
