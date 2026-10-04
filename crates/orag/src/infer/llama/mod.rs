//! llama.cpp integration (D-002). The only module that touches llama-cpp-2
//! types. Pinned to llama-cpp-2 =0.1.158; binding API changes are absorbed here.

pub mod embedder;
pub mod generator;

use std::path::Path;
use std::sync::OnceLock;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::token::LlamaToken;

use crate::error::{OragError, Result};

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

/// Loads GGUF weights, offloading all layers when a GPU backend is compiled in.
pub(crate) fn load_model(path: &Path) -> Result<LlamaModel> {
    load_with_gpu_layers(path, 999)
}

fn load_with_gpu_layers(path: &Path, gpu_layers: u32) -> Result<LlamaModel> {
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
