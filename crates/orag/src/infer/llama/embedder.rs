//! Embeddings on a dedicated worker thread that owns the llama context.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::{LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;

use crate::domain::space::SpaceDescriptor;
use crate::error::{OragError, Result};
use crate::infer::llama::{ModelInfo, backend, load_model, model_error, tokenize};
use crate::infer::models::{EmbeddingSpec, InstalledModel, Pooling};
use crate::infer::tokens::{check_input_length, ensure_single_trailing_eos};
use crate::infer::{Embedder, l2_normalize, require_text};

/// One worker serves queries and ingestion in FIFO order, one text per
/// decode. Known v0.1 limits: a query can wait behind queued ingestion
/// batches, and short chunks are not packed into one batch.
const QUEUE_DEPTH: usize = 8;

struct EmbedJob {
    texts: Vec<String>,
    reply: Sender<Result<Vec<Vec<f32>>>>,
}

pub struct LlamaEmbedder {
    descriptor: SpaceDescriptor,
    spec: EmbeddingSpec,
    model: Arc<LlamaModel>,
    jobs: SyncSender<EmbedJob>,
}

impl LlamaEmbedder {
    pub fn load(installed: &InstalledModel) -> Result<LlamaEmbedder> {
        let manifest = &installed.manifest;
        let spec = manifest.embedding.clone().ok_or_else(|| {
            OragError::InvalidInput(format!("`{}` has no [embedding] section", manifest.id))
        })?;
        let model = Arc::new(load_model(&installed.model_path())?);
        check_model(&model, &spec)?;
        let (jobs, receiver) = mpsc::sync_channel::<EmbedJob>(QUEUE_DEPTH);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (worker_model, worker_spec) = (Arc::clone(&model), spec.clone());
        std::thread::Builder::new()
            .name("orag-embed".into())
            .spawn(move || embed_worker(&worker_model, &worker_spec, receiver, ready_tx))?;
        ready_rx
            .recv()
            .map_err(|_| OragError::Model("embedding worker exited during startup".into()))??;
        let descriptor = SpaceDescriptor {
            model_id: manifest.id.clone(),
            model_sha256: manifest.sha256.clone(),
            pooling: spec.pooling.as_str().into(),
            query_prefix: spec.query_prefix.clone(),
            document_prefix: spec.document_prefix.clone(),
            dimensions: spec.dimensions,
            // llama.cpp pools without normalizing; l2_normalize does it.
            normalized: false,
            max_tokens: spec.max_tokens,
            require_trailing_eos: spec.require_trailing_eos,
        };
        Ok(LlamaEmbedder {
            descriptor,
            spec,
            model,
            jobs,
        })
    }

    fn run(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let (reply, response) = mpsc::channel();
        let stopped = || OragError::Model("embedding worker stopped".into());
        self.jobs
            .send(EmbedJob { texts, reply })
            .map_err(|_| stopped())?;
        response.recv().map_err(|_| stopped())?
    }
}

impl Embedder for LlamaEmbedder {
    fn descriptor(&self) -> &SpaceDescriptor {
        &self.descriptor
    }

    fn count_tokens(&self, text: &str) -> usize {
        tokenize(&self.model, text, false, false).len()
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().try_for_each(|t| require_text(t))?;
        self.run(
            texts
                .iter()
                .map(|t| format!("{}{t}", self.spec.document_prefix))
                .collect(),
        )
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        require_text(text)?;
        let mut vectors = self.run(vec![format!("{}{text}", self.spec.query_prefix)])?;
        vectors
            .pop()
            .ok_or_else(|| OragError::Model("embedding worker returned no vector".into()))
    }
}

/// Rejects a manifest the model cannot honour, before any text is embedded.
fn check_model(model: &LlamaModel, spec: &EmbeddingSpec) -> Result<()> {
    let info = ModelInfo::of(model);
    let width = info.n_embd_out;
    if width != spec.dimensions {
        return Err(OragError::Model(format!(
            "manifest declares {} dimensions but the model outputs {width}",
            spec.dimensions
        )));
    }
    let trained = info.n_ctx_train;
    if spec.max_tokens > trained {
        return Err(OragError::Model(format!(
            "manifest max_tokens {} exceeds the model's trained context of {trained} tokens",
            spec.max_tokens
        )));
    }
    // llama.cpp reports a missing token as -1 (LLAMA_TOKEN_NULL).
    if spec.require_trailing_eos && model.vocab().eos().0 < 0 {
        return Err(OragError::Model(
            "manifest requires a trailing EOS but the model defines none".into(),
        ));
    }
    Ok(())
}

fn pooling_type(pooling: Pooling) -> LlamaPoolingType {
    match pooling {
        Pooling::Mean => LlamaPoolingType::Mean,
        Pooling::Cls => LlamaPoolingType::Cls,
        Pooling::Last => LlamaPoolingType::Last,
    }
}

fn embed_worker(
    model: &LlamaModel,
    spec: &EmbeddingSpec,
    jobs: Receiver<EmbedJob>,
    ready: Sender<Result<()>>,
) {
    let n = u32::try_from(spec.max_tokens).unwrap_or(u32::MAX);
    let params = LlamaContextParams::default()
        .with_embeddings(true)
        .with_n_ctx(NonZeroU32::new(n))
        .with_n_batch(n)
        .with_n_ubatch(n)
        .with_pooling_type(pooling_type(spec.pooling));
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
        let result = job
            .texts
            .iter()
            .map(|text| embed_text(model, &mut ctx, spec, text))
            .collect();
        let _ = job.reply.send(result);
    }
}

fn embed_text(
    model: &LlamaModel,
    ctx: &mut LlamaContext<'_>,
    spec: &EmbeddingSpec,
    text: &str,
) -> Result<Vec<f32>> {
    let eos = model.vocab().eos();
    let tokens = ensure_single_trailing_eos(
        tokenize(model, text, true, false),
        eos,
        spec.require_trailing_eos,
    )?;
    check_input_length(&tokens, spec.max_tokens)?;
    ctx.clear_kv_cache();
    let mut batch = LlamaBatch::new(tokens.len(), 1);
    batch.add_sequence(&tokens, 0, false).map_err(model_error)?;
    ctx.decode(&mut batch).map_err(model_error)?;
    let embedding = ctx.embeddings_seq_ith(0).map_err(model_error)?;
    // `check_model` matched `n_embd_out`, the slice length, to the manifest at load.
    debug_assert_eq!(embedding.len(), spec.dimensions);
    l2_normalize(embedding)
}
