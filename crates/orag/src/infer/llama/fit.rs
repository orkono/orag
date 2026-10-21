//! llama.cpp's memory fit (`common_fit_params`), only in the `vulkan` build
//! (v0.6 plan, G-4). Not thread-safe: the fit swaps llama.cpp's process-wide
//! logger, so models load one after another and the fit's tests run it in
//! child processes (`tests/cli.rs`).

use std::ffi::CString;
use std::path::Path;
use std::pin::Pin;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::params::{FitError, LlamaModelParams};

use crate::error::{OragError, Result};
use crate::infer::llama::backend;

/// Margin left free on every device, in bytes.
pub(crate) const FIT_MARGIN_BYTES: usize = 512 * 1024 * 1024;

/// What one fit call returned, as the probe prints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitOutcome {
    /// A placement was found; `n_gpu_layers` is what it chose.
    Fitted { n_gpu_layers: i32 },
    /// No placement fits the free memory.
    DoesNotFit,
    /// A hard error (unreadable model and the like).
    Failed,
}

impl FitOutcome {
    pub fn label(self) -> String {
        match self {
            FitOutcome::Fitted { n_gpu_layers } => format!("fitted n_gpu_layers={n_gpu_layers}"),
            FitOutcome::DoesNotFit => "does_not_fit".into(),
            FitOutcome::Failed => "error".into(),
        }
    }
}

/// One margin per device llama.cpp can address.
pub(crate) fn margins() -> Vec<usize> {
    vec![FIT_MARGIN_BYTES; llama_cpp_2::max_devices()]
}

/// Fits `params` (default-valued, restricted to the devices it may use) for
/// `context`, never shrinking the context: `n_ctx_min` is its `n_ctx`.
pub(crate) fn fit(
    params: Pin<&mut LlamaModelParams>,
    path: &Path,
    context: &mut LlamaContextParams,
) -> Result<std::result::Result<(), FitError>> {
    backend()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| OragError::Model(format!("model path has a NUL byte: {}", path.display())))?;
    let n_ctx_min = n_ctx_min(context);
    let mut margins = margins();
    Ok(params
        .fit_params(
            &c_path,
            context,
            &mut margins,
            n_ctx_min,
            // Fit messages below a warning go to llama.cpp's debug log.
            llama_cpp_sys_2::GGML_LOG_LEVEL_WARN,
        )
        .map(|_| ()))
}

/// The smallest context the fit may leave: the context itself, so the fit
/// never shrinks what the model is used with.
pub(crate) fn n_ctx_min(context: &LlamaContextParams) -> u32 {
    context.n_ctx().map_or(0, std::num::NonZeroU32::get)
}

/// Runs one fit of `path` with default parameters and a context of `n_ctx`
/// tokens (the probe's link check). Every outcome of the fit itself is a
/// value; only a bad path or a backend that does not start is an error.
pub fn fit_once(path: &Path, n_ctx: u32) -> Result<FitOutcome> {
    let mut params = std::pin::pin!(LlamaModelParams::default());
    let mut context = LlamaContextParams::default().with_n_ctx(std::num::NonZeroU32::new(n_ctx));
    Ok(match fit(params.as_mut(), path, &mut context)? {
        Ok(()) => FitOutcome::Fitted {
            n_gpu_layers: params.n_gpu_layers(),
        },
        Err(FitError::Failure) => FitOutcome::DoesNotFit,
        Err(FitError::Error) => FitOutcome::Failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // No fit call here: it swaps llama.cpp's process-wide logger, so it runs
    // only in child processes (tests/cli.rs, scripts/check-gpu-offload.sh).

    #[test]
    fn one_margin_of_512_mib_per_device() {
        let margins = margins();
        assert_eq!(margins.len(), llama_cpp_2::max_devices());
        assert!(margins.iter().all(|&m| m == 512 * 1024 * 1024));
    }

    #[test]
    fn the_fit_never_shrinks_the_role_context() {
        use crate::infer::llama::{embedder, generator, reranker};
        use crate::infer::models::{EmbeddingSpec, Pooling, RerankInput, RerankerSpec};
        let embedding = EmbeddingSpec {
            dimensions: 1024,
            pooling: Pooling::Last,
            max_tokens: 8192,
            query_prefix: String::new(),
            document_prefix: String::new(),
            require_trailing_eos: true,
        };
        let rerank = RerankerSpec {
            max_tokens: 1024,
            input: RerankInput::Pair,
            relevant_output: 0,
        };
        for (params, n_ctx) in [
            (embedder::context_params(&embedding), 8192),
            (generator::context_params(16384), 16384),
            (reranker::context_params(&rerank), 1024),
        ] {
            assert_eq!(n_ctx_min(&params), n_ctx);
        }
    }
}
