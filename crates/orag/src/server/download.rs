//! Model download from the setup page (step 76): one press downloads every
//! missing configured model, and setup ends like an import does. Since step
//! 82 the press may name the generator the page offered for this machine;
//! once it is installed its id is written into config.toml before the
//! restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::config_edit::set_generation_model;
use crate::infer::download::{
    Catalog, CatalogEntry, DownloadError, DownloadStatus, Downloader, ErrorCode, Job, Options,
    Restart, RunState, SharedStatus, Tier, builtin_tiers, run,
};
use crate::infer::models::{ModelRole, find_model};
use crate::probe::{Machine, probe};
use crate::server::choice::{check_choice, machine_options, requested_generator, with_generator};
use crate::server::errors::{ApiError, ApiResult};
use crate::server::{AppState, ModelsInfo, blocking};

/// Where the machine facts come from: the live probe, or fixed (tests).
#[derive(Debug, Clone)]
enum MachineSource {
    Probe,
    Fixed(Box<Machine>),
}

/// The download side of the server: settings read once at startup and the
/// one run's shared status.
#[derive(Debug)]
pub struct Downloads {
    /// `models.download` of the config.
    pub enabled: bool,
    pub catalog: Catalog,
    pub downloader: Downloader,
    /// The generators offered by hardware, smallest first.
    pub tiers: Vec<Tier>,
    machine: MachineSource,
    /// `<home>/config.toml`, where a chosen generator is written.
    config_file: Option<PathBuf>,
    status: SharedStatus,
    cancel: Arc<AtomicBool>,
}

impl Downloads {
    pub fn new(enabled: bool, catalog: Catalog, downloader: Downloader) -> Downloads {
        Downloads {
            enabled,
            catalog,
            downloader,
            tiers: builtin_tiers(),
            machine: MachineSource::Probe,
            config_file: None,
            status: SharedStatus::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The built-in catalog from the real hosts.
    pub fn production(enabled: bool) -> Downloads {
        Downloads::new(enabled, Catalog::builtin(), Downloader::production())
    }

    pub fn with_tiers(self, tiers: Vec<Tier>) -> Downloads {
        Downloads { tiers, ..self }
    }

    /// Answers every probe with `machine` (tests).
    pub fn with_machine(self, machine: Machine) -> Downloads {
        Downloads {
            machine: MachineSource::Fixed(Box::new(machine)),
            ..self
        }
    }

    /// The config file a chosen generator is written into.
    pub fn with_config_file(self, path: PathBuf) -> Downloads {
        Downloads {
            config_file: Some(path),
            ..self
        }
    }

    pub fn status(&self) -> DownloadStatus {
        self.status.snapshot()
    }

    /// This machine now; blocking (it reads the free disk).
    pub fn machine(&self, models_dir: &Path) -> Machine {
        match &self.machine {
            MachineSource::Probe => probe(models_dir),
            MachineSource::Fixed(machine) => (**machine).clone(),
        }
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

/// Setup mode with downloads on, or why not.
fn check_can_download(state: &AppState) -> ApiResult<()> {
    if !state.setup {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "the models are already installed; nothing to download",
        ));
    }
    if !state.downloads.enabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "download_disabled",
            "model download is turned off in config.toml (models.download = false); import the model packs instead",
        ));
    }
    Ok(())
}

/// The catalog entries for the missing models, or why there are none. A
/// chosen generator other than the configured one may need no download at
/// all (it is installed already): the run then only writes the config.
fn plan(state: &AppState, missing: &Missing, choice: Option<&str>) -> ApiResult<Vec<CatalogEntry>> {
    check_can_download(state)?;
    let switches = choice.is_some_and(|id| id != state.models.generation);
    if missing.is_empty() && !switches {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "the models are installed; restart orag to load them",
        ));
    }
    missing
        .iter()
        .map(|(id, role)| catalog_entry(&state.downloads.catalog, id, *role))
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

/// The `download` member of `GET /v1/models`: today's availability of the
/// configured models, plus the generator options for this machine.
///
/// A configured generator outside the catalog (`download_unavailable` for
/// the configured models) does not hide the choice: when the selected option
/// can be downloaded the page offers the tiers, `configured_downloadable`
/// says the configured model itself cannot be, and `bytes` is the selected
/// option's download.
pub fn availability(state: &AppState, missing: &Missing, options: &Options) -> Value {
    let configured = plan(state, missing, None);
    let fallback = match &configured {
        Err(err) if err.code == "download_unavailable" => selected_plan(state, missing, options),
        _ => None,
    };
    let mut download = match (configured, fallback) {
        (Ok(entries), _) => available(&entries, true),
        (Err(_), Some(entries)) => available(&entries, false),
        (Err(err), None) => json!({
            "available": false,
            "code": err.code,
            "reason": err.message,
            "bytes": null,
            "configured_downloadable": false,
        }),
    };
    let choice = serde_json::to_value(options).unwrap_or_else(|_| json!({}));
    if let (Some(download), Value::Object(choice)) = (download.as_object_mut(), choice) {
        download.extend(choice);
    }
    download
}

fn available(entries: &[CatalogEntry], configured_downloadable: bool) -> Value {
    json!({
        "available": true,
        "code": null,
        "reason": null,
        "bytes": entries.iter().map(CatalogEntry::total_size).sum::<u64>(),
        "configured_downloadable": configured_downloadable,
    })
}

/// The download of the embedder (if missing) and the selected option (if
/// not installed), when every one of them is in the catalog.
fn selected_plan(
    state: &AppState,
    missing: &Missing,
    options: &Options,
) -> Option<Vec<CatalogEntry>> {
    let selected = options.selected.as_deref()?;
    let option = options.options.iter().find(|o| o.id == selected)?;
    let chosen: Missing = missing
        .iter()
        .filter(|(_, role)| *role != ModelRole::Generation)
        .cloned()
        .chain((!option.installed).then(|| (selected.to_string(), ModelRole::Generation)))
        .collect();
    plan(state, &chosen, Some(selected)).ok()
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

/// Starts downloading every missing configured model (with the chosen
/// generator in place of the configured one), on a task the server owns: a
/// page that reloads or leaves neither stops the download nor frees the
/// one-import permit early.
pub async fn start(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<DownloadStatus>)> {
    let choice = requested_generator(&body)?;
    let Ok(permit) = state.imports.clone().try_acquire_owned() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "a model download or import is already running; wait for it to finish",
        ));
    };
    check_can_download(&state)?;
    // The plan is read under the permit: an import that finishes just before
    // this request must not leave it downloading a model now installed.
    let models = match &choice {
        Some(id) => {
            let read = state.clone();
            let (_, options) = blocking(move || Ok(machine_options(&read))).await?;
            check_choice(&options, id)?;
            with_generator(&state.models, id)
        }
        None => (*state.models).clone(),
    };
    let wanted = models.clone();
    let missing = blocking(move || Ok(missing_models(&wanted))).await?;
    let entries = plan(&state, &missing, choice.as_deref())?;
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
    if let Some(id) = &choice {
        tracing::info!(generation_model = %id, "model download started with the chosen generator");
    }
    tokio::spawn(async move {
        let _permit = permit;
        let outcome = run_job(&state, job).await;
        finish(&state, outcome, &models, choice.as_deref()).await;
    });
    Ok((StatusCode::ACCEPTED, Json(started)))
}

/// Runs the download on a blocking thread; shutdown cancels it.
async fn run_job(state: &AppState, job: Job) -> Result<(), DownloadError> {
    if job.models.is_empty() {
        return Ok(()); // the chosen generator is installed already
    }
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
/// A chosen generator is written into config.toml first, and only then.
async fn finish(
    state: &AppState,
    outcome: Result<(), DownloadError>,
    models: &ModelsInfo,
    choice: Option<&str>,
) {
    let status = &state.downloads.status;
    let outcome = match outcome {
        Ok(()) => {
            let models = models.clone();
            let complete = blocking(move || Ok(missing_models(&models).is_empty()))
                .await
                .unwrap_or(false);
            match (complete, choice) {
                (true, Some(id)) => write_choice(state, id).await.map(|()| complete),
                _ => Ok(complete),
            }
        }
        Err(err) => Err(err),
    };
    match outcome {
        Ok(complete) => {
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

/// `generation_model = "<id>"` in config.toml, before any restart.
async fn write_choice(state: &AppState, id: &str) -> Result<(), DownloadError> {
    let Some(path) = state.downloads.config_file.clone() else {
        return Err(DownloadError::internal(
            "this server does not know its config.toml, so the chosen generator cannot be kept",
        ));
    };
    let model = id.to_string();
    let written = tokio::task::spawn_blocking(move || set_generation_model(&path, &model))
        .await
        .unwrap_or_else(|err| Err(crate::error::OragError::Internal(err.to_string())));
    match written {
        Ok(true) => {
            tracing::info!(generation_model = %id, "chosen generator written into config.toml");
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(err) => {
            tracing::error!(error = %err, "could not write the chosen generator into config.toml");
            Err(DownloadError::new(
                ErrorCode::Config,
                format!(
                    "the models are installed, but config.toml could not be updated ({err}); set generation_model = \"{id}\" there and restart orag"
                ),
            ))
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
