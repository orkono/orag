//! Model download from the setup page (step 76): one press downloads every
//! missing configured model, and setup ends like an import does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::infer::download::{
    Catalog, CatalogEntry, DownloadError, DownloadStatus, Downloader, ErrorCode, Job, Restart,
    RunState, SharedStatus, run,
};
use crate::infer::models::{ModelRole, find_model};
use crate::server::errors::{ApiError, ApiResult};
use crate::server::{AppState, ModelsInfo, blocking};

/// The download side of the server: settings read once at startup and the
/// one run's shared status.
#[derive(Debug)]
pub struct Downloads {
    /// `models.download` of the config.
    pub enabled: bool,
    pub catalog: Catalog,
    pub downloader: Downloader,
    status: SharedStatus,
    cancel: Arc<AtomicBool>,
}

impl Downloads {
    pub fn new(enabled: bool, catalog: Catalog, downloader: Downloader) -> Downloads {
        Downloads {
            enabled,
            catalog,
            downloader,
            status: SharedStatus::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The built-in catalog from the real hosts.
    pub fn production(enabled: bool) -> Downloads {
        Downloads::new(enabled, Catalog::builtin(), Downloader::production())
    }

    pub fn status(&self) -> DownloadStatus {
        self.status.snapshot()
    }
}

/// A configured model that is not installed, and the slot it fills.
type Missing = Vec<(String, ModelRole)>;

/// The configured models `find_model` does not accept.
pub fn missing_models(models: &ModelsInfo) -> Missing {
    [
        (&models.embedding, ModelRole::Embedding),
        (&models.generation, ModelRole::Generation),
    ]
    .into_iter()
    .filter(|(id, role)| find_model(&models.models_dir, id, *role).is_err())
    .map(|(id, role)| (id.clone(), role))
    .collect()
}

/// The catalog entries for the missing models, or why there are none.
fn plan(state: &AppState, missing: &Missing) -> ApiResult<Vec<CatalogEntry>> {
    if !state.setup {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "the models are already installed; nothing to download",
        ));
    }
    let downloads = &state.downloads;
    if !downloads.enabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "download_disabled",
            "model download is turned off in config.toml (models.download = false); import the model packs instead",
        ));
    }
    if missing.is_empty() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "the models are installed; restart orag to load them",
        ));
    }
    missing
        .iter()
        .map(|(id, role)| catalog_entry(&downloads.catalog, id, *role))
        .collect()
}

fn catalog_entry(catalog: &Catalog, id: &str, role: ModelRole) -> ApiResult<CatalogEntry> {
    let unavailable =
        |message: String| ApiError::new(StatusCode::BAD_REQUEST, "download_unavailable", message);
    let Some(entry) = catalog.find(id) else {
        return Err(unavailable(format!(
            "`{id}` is not one of the models ORAG can download; build its pack with scripts/fetch-model-pack.sh and import it"
        )));
    };
    if entry.role != role {
        return Err(unavailable(format!(
            "`{id}` is a {} model, but config.toml names it as the {} model",
            entry.role.as_str(),
            role.as_str()
        )));
    }
    Ok(entry.clone())
}

/// The `download` member of `GET /v1/models`.
pub fn availability(state: &AppState, missing: &Missing) -> Value {
    match plan(state, missing) {
        Ok(entries) => json!({
            "available": true,
            "code": null,
            "reason": null,
            "bytes": entries.iter().map(CatalogEntry::total_size).sum::<u64>(),
        }),
        Err(err) => json!({
            "available": false,
            "code": err.code,
            "reason": err.message,
            "bytes": null,
        }),
    }
}

pub async fn status(State(state): State<AppState>) -> Json<DownloadStatus> {
    Json(state.downloads.status())
}

/// Cancels a running download; its `.part` files stay for the next start.
pub async fn cancel(State(state): State<AppState>) -> Json<DownloadStatus> {
    let downloads = &state.downloads;
    if downloads.status().state == RunState::Running {
        tracing::info!("model download cancelled from the page");
        downloads.cancel.store(true, Ordering::SeqCst);
    }
    Json(downloads.status())
}

/// Starts downloading every missing configured model, on a task the server
/// owns: a page that reloads or leaves neither stops the download nor frees
/// the one-import permit early.
pub async fn start(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<DownloadStatus>)> {
    check_empty_body(&body)?;
    let Ok(permit) = state.imports.clone().try_acquire_owned() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "a model download or import is already running; wait for it to finish",
        ));
    };
    // The plan is read under the permit: an import that finishes just before
    // this request must not leave it downloading a model now installed.
    let models = state.models.clone();
    let missing = blocking(move || Ok(missing_models(&models))).await?;
    let entries = plan(&state, &missing)?;
    let downloads = &state.downloads;
    let job = Job {
        models_dir: state.models.models_dir.clone(),
        models: entries,
        status: downloads.status.clone(),
        cancel: downloads.cancel.clone(),
    };
    job.cancel.store(false, Ordering::SeqCst);
    job.status.set(job.initial_status());
    let started = downloads.status();
    tokio::spawn(async move {
        let _permit = permit;
        let outcome = run_job(&state, job).await;
        finish(&state, outcome).await;
    });
    Ok((StatusCode::ACCEPTED, Json(started)))
}

/// No body or `{}`: the request names nothing, the config decides.
fn check_empty_body(body: &[u8]) -> ApiResult<()> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(fields)) if fields.is_empty() => Ok(()),
        _ => Err(ApiError::invalid(
            "send no body or {}; the models to download come from config.toml",
        )),
    }
}

/// Runs the download on a blocking thread; shutdown cancels it.
async fn run_job(state: &AppState, job: Job) -> Result<(), DownloadError> {
    let cancel = job.cancel.clone();
    let downloader = state.downloads.downloader.clone();
    let mut handle = tokio::task::spawn_blocking(move || run(&downloader, &job));
    let mut shutdown = state.subscribe_shutdown();
    let joined = tokio::select! {
        joined = &mut handle => joined,
        // The guard `wait_for` returns is not `Send`; only the outcome is kept.
        _ = async { shutdown.wait_for(|stopping| *stopping).await.map(|_| ()) } => {
            cancel.store(true, Ordering::SeqCst);
            handle.await
        }
    };
    joined.unwrap_or_else(|err| {
        tracing::error!(error = %err, "model download task failed");
        Err(DownloadError::internal(
            "the download stopped unexpectedly; start it again",
        ))
    })
}

/// Records how the run ended; a complete install ends setup like an import.
async fn finish(state: &AppState, outcome: Result<(), DownloadError>) {
    let status = &state.downloads.status;
    match outcome {
        Ok(()) => {
            let models = state.models.clone();
            let complete = blocking(move || Ok(missing_models(&models).is_empty()))
                .await
                .unwrap_or(false);
            let restart = restart_after(state, complete);
            // Recorded first: a page polling the status sees `done` before
            // the restart closes the connection.
            status.finish(RunState::Done, None, Some(restart));
            if restart == Restart::Automatic {
                state.request_restart();
            }
        }
        Err(err) if err.code == ErrorCode::Cancelled => {
            status.finish(RunState::Cancelled, Some(err), None);
        }
        Err(err) => {
            tracing::warn!(code = ?err.code, error = %err, "model download failed");
            status.finish(RunState::Failed, Some(err), None);
        }
    }
}

fn restart_after(state: &AppState, complete: bool) -> Restart {
    match (complete, state.supervised) {
        (false, _) => Restart::None,
        (true, true) => {
            tracing::info!("models downloaded; restarting to load them");
            Restart::Automatic
        }
        (true, false) => {
            tracing::info!("models downloaded; restart orag to load them");
            Restart::Manual
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_empty_body_or_object_is_accepted() {
        assert!(check_empty_body(b"").is_ok());
        assert!(check_empty_body(b" {} ").is_ok());
        assert!(check_empty_body(b"{\"models\": [\"x\"]}").is_err());
        assert!(check_empty_body(b"[]").is_err());
    }
}
