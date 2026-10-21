//! llama.cpp's memory fit (`common_fit_params`), only in the `vulkan` build
//! (v0.6 plan, G-4). Step 83 ships only the link smoke (`orag-vulkan
//! gpu-probe --fit <gguf>`): it proves the `common` wrapper links into the
//! shipped executable with its imports checked, before step 85 uses it to
//! load models.

use std::ffi::CString;
use std::path::Path;

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

/// Runs one fit of `path` with default parameters and a context of
/// `n_ctx` tokens. Every outcome of the fit itself is returned as a value;
/// only a path that cannot be passed to C or a backend that does not start
/// is an error.
pub fn fit_once(path: &Path, n_ctx: u32) -> Result<FitOutcome> {
    backend()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| OragError::Model(format!("model path has a NUL byte: {}", path.display())))?;
    let mut params = std::pin::pin!(LlamaModelParams::default());
    let mut context = LlamaContextParams::default().with_n_ctx(std::num::NonZeroU32::new(n_ctx));
    let mut margins = vec![FIT_MARGIN_BYTES; llama_cpp_2::max_devices()];
    let outcome = params.as_mut().fit_params(
        &c_path,
        &mut context,
        &mut margins,
        n_ctx,
        // Fit messages below a warning go to llama.cpp's debug log.
        llama_cpp_sys_2::GGML_LOG_LEVEL_WARN,
    );
    Ok(match outcome {
        Ok(_) => FitOutcome::Fitted {
            n_gpu_layers: params.n_gpu_layers(),
        },
        Err(FitError::Failure) => FitOutcome::DoesNotFit,
        Err(FitError::Error) => FitOutcome::Failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Option<std::path::PathBuf> {
        let gguf =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/stories260K.gguf");
        if gguf.exists() {
            return Some(gguf);
        }
        assert!(
            std::env::var_os("ORAG_REQUIRE_FIXTURES").is_none(),
            "missing {}",
            gguf.display()
        );
        None
    }

    #[test]
    fn the_fit_links_and_returns_an_outcome_for_the_fixture() {
        let Some(gguf) = fixture() else { return };
        // Any outcome proves the link; on a machine with no GPU device the
        // routine still answers.
        let outcome = fit_once(&gguf, 256).unwrap();
        assert!(!outcome.label().is_empty());
    }

    #[test]
    fn a_missing_model_is_a_fit_error_not_a_panic() {
        let outcome = fit_once(Path::new("/nonexistent/model.gguf"), 256).unwrap();
        assert_eq!(outcome, FitOutcome::Failed);
    }
}
