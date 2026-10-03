//! llama.cpp integration (D-002). The only module that touches llama-cpp-2
//! types. Pinned to llama-cpp-2 =0.1.158; binding API changes are absorbed here.

pub mod embedder;

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
