//! Model status and import from the page (setup mode, D-023).

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{OragError, Result};
use crate::infer::models::{MANIFEST_FILE, ModelRole, find_model, import_pack, read_manifest};
use crate::server::choice::machine_options;
use crate::server::download::{availability, missing_models};
use crate::server::errors::{ApiError, ApiResult};
use crate::server::{AppState, ModelsInfo, blocking};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportBody {
    /// A local folder: one pack, or a folder of pack folders.
    pub path: PathBuf,
}

pub async fn status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let read = state.clone();
    let ((embedding, generation), missing, (machine, options)) = blocking(move || {
        let models = &read.models;
        Ok((
            installed(models),
            missing_models(models),
            machine_options(&read),
        ))
    })
    .await?;
    Ok(Json(json!({
        "mode": if state.setup { "setup" } else { "ready" },
        "embedding": { "id": state.models.embedding, "installed": embedding },
        "generation": { "id": state.models.generation, "installed": generation },
        "models_dir": state.models.models_dir,
        "machine": machine,
        "gpu": *state.gpu,
        "download": availability(&state, &missing, &options),
    })))
}

/// Imports every pack in the folder (SHA-256 verified), in setup mode only:
/// once both configured models are installed, setup ends with a restart,
/// automatic under `orag start`, manual otherwise (D-023). A server with its
/// models never imports from the page (`orag models import` does that), and
/// never loads a model while running.
pub async fn import(
    State(state): State<AppState>,
    body: std::result::Result<Json<ImportBody>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    let Json(body) = body.map_err(|e| ApiError::json_rejection(e.status(), e.body_text()))?;
    if !state.setup {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "the models are already installed; import more with `orag models import` and restart orag",
        ));
    }
    let Ok(permit) = state.imports.clone().try_acquire_owned() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "another model import is running; wait for it to finish",
        ));
    };
    let folder = expand_home(&body.path, std::env::home_dir().as_deref());
    if !folder.is_absolute() {
        return Err(ApiError::invalid(format!(
            "{} is not a full path; give the whole folder path (for example ~/orag-packs)",
            folder.display()
        )));
    }
    // The import and its outcome run in their own task: a client that leaves
    // mid-copy (a reloaded page) neither frees the one-import permit early nor
    // loses the restart that ends setup.
    let task = tokio::spawn(async move {
        let _permit = permit;
        let models = state.models.clone();
        let (imported, complete) = blocking(move || {
            let imported = import_folder(&folder, &models)?;
            let (embedding, generation) = installed(&models);
            Ok((imported, embedding && generation))
        })
        .await?;
        let restart = match (complete, state.supervised) {
            (false, _) => "none",
            (true, true) => {
                tracing::info!("model packs installed; restarting to load them");
                state.request_restart();
                "automatic"
            }
            (true, false) => {
                tracing::info!("model packs installed; restart orag to load them");
                "manual"
            }
        };
        Ok(Json(json!({ "imported": imported, "restart": restart })))
    });
    task.await.unwrap_or_else(|err| {
        tracing::error!(error = %err, "model import task failed");
        Err(ApiError::internal())
    })
}

/// The user's home folder for a leading `~` (the page's example).
fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    let (Some(home), Ok(rest)) = (home, path.strip_prefix("~")) else {
        return path.to_path_buf();
    };
    home.join(rest)
}

/// Whether each configured model is installed with the right role.
fn installed(models: &ModelsInfo) -> (bool, bool) {
    let has = |id: &str, role| find_model(&models.models_dir, id, role).is_ok();
    (
        has(&models.embedding, ModelRole::Embedding),
        has(&models.generation, ModelRole::Generation),
    )
}

/// Imports `folder` if it is a pack, else each pack folder directly inside it.
/// A pack that is already installed and working is skipped, so a retry is
/// harmless. Pack errors are `InvalidInput` naming the pack; host errors (a
/// full disk, an unwritable models folder) stay what they are (500).
fn import_folder(folder: &Path, models: &ModelsInfo) -> Result<Vec<String>> {
    let named = || folder.display().to_string();
    if !folder.is_dir() {
        return Err(OragError::InvalidInput(format!(
            "{} is not a folder on this computer",
            named()
        )));
    }
    let packs: Vec<PathBuf> = if folder.join(MANIFEST_FILE).is_file() {
        vec![folder.to_path_buf()]
    } else {
        let mut packs: Vec<PathBuf> = std::fs::read_dir(folder)
            .map_err(|e| OragError::InvalidInput(format!("cannot read {}: {e}", named())))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join(MANIFEST_FILE).is_file())
            .collect();
        packs.sort();
        packs
    };
    if packs.is_empty() {
        return Err(OragError::InvalidInput(format!(
            "{} holds no model pack (a folder with {MANIFEST_FILE})",
            named()
        )));
    }
    let mut imported = Vec::new();
    for pack in packs {
        let in_pack = |err: OragError| match err {
            OragError::InvalidInput(message) => {
                OragError::InvalidInput(format!("{}: {message}", pack.display()))
            }
            other => {
                tracing::error!(pack = %pack.display(), error = %other, "model import failed");
                other
            }
        };
        check_pack_files(&pack, &[MANIFEST_FILE]).map_err(in_pack)?;
        let manifest = read_manifest(&pack).map_err(in_pack)?;
        check_pack_files(&pack, &[&manifest.file, &manifest.license_file]).map_err(in_pack)?;
        match import_pack(&pack, &models.models_dir) {
            Ok(manifest) => {
                tracing::info!(model = %manifest.id, "model pack imported");
                imported.push(manifest.id);
            }
            Err(OragError::Conflict(_)) => {
                // Skipped only when the installed copy works; a leftover folder
                // of an interrupted copy would otherwise block setup forever.
                if find_model(&models.models_dir, &manifest.id, manifest.role).is_err() {
                    return Err(OragError::InvalidInput(format!(
                        "{} already exists but is not a working install of `{}`; delete that folder and import again",
                        models.models_dir.join(&manifest.id).display(),
                        manifest.id
                    )));
                }
            }
            Err(err) => return Err(in_pack(err)),
        }
    }
    Ok(imported)
}

/// The page lets any local process name a folder, so its files must be the
/// server user's own regular files: no symlink to someone else's file (whose
/// hash an error would show), and on unix no file owned by another user.
fn check_pack_files(pack: &Path, files: &[&str]) -> Result<()> {
    let paths = std::iter::once(pack.to_path_buf()).chain(files.iter().map(|name| pack.join(name)));
    for path in paths {
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| OragError::InvalidInput(format!("cannot read {}: {e}", path.display())))?;
        if meta.file_type().is_symlink() {
            return Err(OragError::InvalidInput(format!(
                "{} is a symbolic link; copy the pack instead of linking it",
                path.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: geteuid has no preconditions and cannot fail.
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err(OragError::InvalidInput(format!(
                    "{} belongs to another user; copy the pack into your own folder",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_home(Path::new("~/orag-packs"), Some(home)),
            home.join("orag-packs")
        );
        assert_eq!(expand_home(Path::new("~"), Some(home)), home.to_path_buf());
        assert_eq!(
            expand_home(Path::new("/srv/~packs"), Some(home)),
            Path::new("/srv/~packs")
        );
        assert_eq!(
            expand_home(Path::new("~other/x"), Some(home)),
            Path::new("~other/x")
        );
        assert_eq!(expand_home(Path::new("~/x"), None), Path::new("~/x"));
    }
}
