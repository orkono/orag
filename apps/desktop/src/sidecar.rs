//! Where the `orag` sidecar executable is.
//!
//! A bundle ships it next to the app executable (Tauri `externalBin`, staged
//! as `binaries/orag-<target triple>` and installed as `orag`). A development
//! build may point elsewhere with `ORAG_DESKTOP_SIDECAR`; release builds
//! refuse that variable so an installed app always runs its own service.

use std::path::{Path, PathBuf};

pub const DEV_SIDECAR_ENV: &str = "ORAG_DESKTOP_SIDECAR";

/// The sidecar's file name next to the app executable.
pub fn bundled_name() -> &'static str {
    if cfg!(windows) { "orag.exe" } else { "orag" }
}

/// Resolves the sidecar path. `dev_override` is the value of
/// `ORAG_DESKTOP_SIDECAR`, honoured only when `allow_override` is true.
pub fn resolve(
    app_exe: &Path,
    dev_override: Option<&str>,
    allow_override: bool,
) -> Result<PathBuf, String> {
    match dev_override.filter(|v| !v.trim().is_empty()) {
        Some(path) if allow_override => return Ok(PathBuf::from(path)),
        Some(_) => {
            return Err(format!(
                "{DEV_SIDECAR_ENV} is only honoured in development builds"
            ));
        }
        None => {}
    }
    let dir = app_exe
        .parent()
        .ok_or_else(|| format!("the app executable {} has no folder", app_exe.display()))?;
    Ok(dir.join(bundled_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_sidecar_sits_next_to_the_app() {
        let exe = Path::new("/Applications/ORAG.app/Contents/MacOS/orag-desktop");
        assert_eq!(
            resolve(exe, None, false).unwrap(),
            Path::new("/Applications/ORAG.app/Contents/MacOS").join(bundled_name())
        );
    }

    #[test]
    fn override_is_used_in_development() {
        let exe = Path::new("/x/orag-desktop");
        assert_eq!(
            resolve(exe, Some("/dev/orag"), true).unwrap(),
            Path::new("/dev/orag")
        );
    }

    #[test]
    fn override_is_refused_in_release() {
        let err = resolve(Path::new("/x/orag-desktop"), Some("/evil/orag"), false).unwrap_err();
        assert!(err.contains(DEV_SIDECAR_ENV), "{err}");
    }

    #[test]
    fn empty_override_is_ignored() {
        let exe = Path::new("/x/orag-desktop");
        assert_eq!(
            resolve(exe, Some("  "), false).unwrap(),
            Path::new("/x").join(bundled_name())
        );
    }
}
