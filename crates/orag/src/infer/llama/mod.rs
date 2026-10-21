//! llama.cpp integration (D-002). The only module that touches llama-cpp-2
//! types. Pinned to llama-cpp-2 =0.1.158; binding API changes are absorbed here.

#[cfg(feature = "vulkan")]
pub mod devices;
pub mod embedder;
#[cfg(feature = "vulkan")]
pub mod fit;
pub mod generator;
#[cfg(feature = "vulkan")]
pub mod offload;
pub mod reranker;

use std::path::Path;
use std::sync::OnceLock;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::token::LlamaToken;

use crate::error::{OragError, Result};
use crate::gpu::{Offload, Placement};

static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();

/// Process-wide llama.cpp backend; llama.cpp logs are routed to `tracing`.
pub(crate) fn backend() -> Result<&'static LlamaBackend> {
    BACKEND
        .get_or_init(|| {
            llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| OragError::Model(format!("llama.cpp backend init failed: {e}")))
}

/// The thread that owns a model's llama.cpp context. Dropping it waits for
/// the thread, which ends once its job queue is closed: the context (Metal
/// resources) is then freed before the process can exit, since llama.cpp
/// aborts in its static teardown if any are still alive. A worker panic is
/// logged, not lost.
#[derive(Default)]
pub(crate) struct WorkerThread(Option<std::thread::JoinHandle<()>>);

impl WorkerThread {
    pub(crate) fn new(handle: std::thread::JoinHandle<()>) -> Self {
        WorkerThread(Some(handle))
    }
}

impl Drop for WorkerThread {
    fn drop(&mut self) {
        let Some(handle) = self.0.take() else {
            return;
        };
        let name = handle.thread().name().unwrap_or("llama worker").to_string();
        if let Err(panic) = handle.join() {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".into());
            tracing::error!(worker = %name, panic = %message, "model worker thread panicked");
        }
    }
}

pub(crate) fn model_error(err: impl std::fmt::Display) -> OragError {
    OragError::Model(err.to_string())
}

/// A loaded model and what its load did with the GPU (`None` outside
/// `orag-vulkan`, or for [`Placement::BuildDefault`]).
pub(crate) struct Loaded {
    pub(crate) model: LlamaModel,
    pub(crate) offload: Option<Offload>,
}

/// Loads GGUF weights where `placement` says. `BuildDefault` offloads every
/// layer when the build has a GPU backend (Metal), as before v0.6. `context`
/// builds the role's context parameters: `orag-vulkan` fits the model to the
/// GPU's free memory with exactly the context the role then creates.
pub(crate) fn load_model(
    path: &Path,
    context: &dyn Fn() -> LlamaContextParams,
    placement: &Placement,
) -> Result<Loaded> {
    match placement {
        Placement::BuildDefault => Ok(Loaded {
            model: load_with_gpu_layers(path, 999)?,
            offload: None,
        }),
        #[cfg(feature = "vulkan")]
        placed => offload::load_placed(path, context, placed),
        #[cfg(not(feature = "vulkan"))]
        placed => {
            let _ = context;
            Err(OragError::Model(format!(
                "{placed:?} needs orag-vulkan; this build places models itself"
            )))
        }
    }
}

/// Loads where `placement` says, then lets `start` finish the model (its
/// worker context, a first context). In `orag-vulkan` the fit estimates from
/// a snapshot of free memory: when `start` fails on a model with layers on
/// the GPU, the model is loaded again CPU-only (`alloc_failed`) and started
/// once more (v0.6 plan, G-4). Errors of the load itself (a fit's hard
/// error, an unreadable file) are returned as they are. `start`'s flag is
/// the test-only `ORAG_VULKAN_TEST_FIT=context_failure`: fail the first
/// context on the GPU.
pub(crate) fn load_and_start<T>(
    path: &Path,
    context: &dyn Fn() -> LlamaContextParams,
    placement: &Placement,
    start: &mut dyn FnMut(Loaded, bool) -> Result<T>,
) -> Result<T> {
    let loaded = load_model(path, context, placement)?;
    let on_gpu = loaded
        .offload
        .as_ref()
        .is_some_and(|offload| offload.layers_on_gpu > 0);
    if !on_gpu {
        return start(loaded, false);
    }
    match start(loaded, test_context_failure()) {
        Ok(started) => Ok(started),
        Err(err) => {
            tracing::warn!(error = %err, "the model's context failed on the GPU; loading it CPU-only");
            let cpu = load_model(
                path,
                context,
                &Placement::CpuOnly {
                    reason: "alloc_failed",
                },
            )?;
            start(cpu, false)
        }
    }
}

#[cfg(feature = "vulkan")]
fn test_context_failure() -> bool {
    offload::test_context_failure()
}

#[cfg(not(feature = "vulkan"))]
fn test_context_failure() -> bool {
    false
}

/// The test hook's error, in place of the context a worker would create.
pub(crate) fn hooked_context_failure() -> OragError {
    OragError::Model("test hook: context allocation failure".into())
}

pub(crate) fn load_with_gpu_layers(path: &Path, gpu_layers: u32) -> Result<LlamaModel> {
    let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
    LlamaModel::load_from_file(backend()?, path, &params).map_err(model_error)
}

/// Single call site of the binding tokenizer. `parse_special` turns marker
/// text such as `</s>` into control tokens: true only for prompts ORAG
/// renders itself, never for document or query text.
pub(crate) fn tokenize(
    model: &LlamaModel,
    text: &str,
    add_bos: bool,
    parse_special: bool,
) -> Vec<LlamaToken> {
    model
        .vocab()
        .tokenize(text.as_bytes(), add_bos, parse_special)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// Width of the vectors the model's embeddings output has.
    pub n_embd_out: usize,
    pub n_ctx_train: usize,
}

/// Reads shape information from a GGUF file. The weights are memory-mapped
/// and stay on the CPU (no GPU upload). `vocab_only` would be cheaper but
/// skips the hyperparameters: llama.cpp then reports 0 for both values.
pub fn model_info(path: &Path) -> Result<ModelInfo> {
    Ok(ModelInfo::of(&load_with_gpu_layers(path, 0)?))
}

impl ModelInfo {
    pub(crate) fn of(model: &LlamaModel) -> ModelInfo {
        ModelInfo {
            n_embd_out: model.n_embd_out() as usize,
            n_ctx_train: model.n_ctx_train() as usize,
        }
    }
}

/// The 1.2 MB stories260K CI fixture installed as a model with `role` and its
/// TOML section; `None` locally when it was not fetched (CI sets
/// ORAG_REQUIRE_FIXTURES, which turns a missing file into a failure).
#[cfg(test)]
pub(crate) fn fixture_model(
    dir: &Path,
    role: crate::infer::models::ModelRole,
    section: &str,
) -> Option<crate::infer::models::InstalledModel> {
    use crate::infer::models::{MANIFEST_FILE, find_model, import_pack, sha256_file};
    let gguf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/stories260K.gguf");
    if !gguf.exists() {
        assert!(
            std::env::var_os("ORAG_REQUIRE_FIXTURES").is_none(),
            "missing {}",
            gguf.display()
        );
        return None;
    }
    let pack = dir.join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::copy(&gguf, pack.join("m.gguf")).unwrap();
    std::fs::write(pack.join("LICENSE"), "MIT").unwrap();
    let role_name = match role {
        crate::infer::models::ModelRole::Embedding => "embedding",
        crate::infer::models::ModelRole::Generation => "generation",
        crate::infer::models::ModelRole::Reranking => "reranking",
    };
    std::fs::write(
        pack.join(MANIFEST_FILE),
        format!(
            "id = \"tiny\"\nrole = \"{role_name}\"\nfile = \"m.gguf\"\nsha256 = \"{}\"\n\
             license = \"MIT\"\nlicense_file = \"LICENSE\"\nsource = \"s\"\nrevision = \"r\"\n\n{section}",
            sha256_file(&gguf).unwrap()
        ),
    )
    .unwrap();
    let models = dir.join("models");
    import_pack(&pack, &models).unwrap();
    Some(find_model(&models, "tiny", role).unwrap())
}
