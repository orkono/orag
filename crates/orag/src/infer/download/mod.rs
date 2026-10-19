//! In-app model download (step 76, amends D-012): only after the user asks,
//! only pinned Hugging Face revisions, every file SHA-256 checked, resumable.
//! Files land in `<models_dir>/.download/<id>/<file>.part` and are installed
//! like `orag models import` installs a pack.

pub mod catalog;
mod fetch;
mod guard;
pub mod http;
mod install;
pub mod status;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tls_tests;
mod verify;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use catalog::{Catalog, CatalogEntry};
pub use http::HttpSettings;
pub use status::{
    DownloadError, DownloadStatus, ErrorCode, ModelProgress, ModelState, Restart, RunState,
    SharedStatus,
};

use crate::infer::download::fetch::{Fetch, fetch};
use crate::infer::download::install::{check_free_space, install, part_path};
use crate::infer::download::verify::verify;

/// The folder of unfinished downloads inside the models folder; its leading
/// dot keeps `orag models list` from reading it as a model.
pub const DOWNLOAD_DIR: &str = ".download";

/// Where and how files are fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloader {
    pub http: HttpSettings,
    /// Replaces the scheme and host of every catalog URL (tests serve the
    /// files from a local server); `None` in the service.
    pub base_url: Option<String>,
}

impl Downloader {
    /// The real hosts over HTTPS, proxies from the environment.
    pub fn production() -> Downloader {
        Downloader {
            http: HttpSettings::production(),
            base_url: None,
        }
    }

    /// Every file from `base_url` (plain HTTP allowed, no proxy).
    pub fn with_base_url(base_url: impl Into<String>) -> Downloader {
        Downloader {
            http: HttpSettings {
                env_proxy: false,
                https_only: false,
                idle_timeout: std::time::Duration::from_secs(10),
            },
            base_url: Some(base_url.into()),
        }
    }

    /// `https://host/path` as fetched: unchanged, or the path on `base_url`.
    pub fn url(&self, url: &str) -> String {
        let Some(base) = &self.base_url else {
            return url.to_string();
        };
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        let path = rest.find('/').map_or("/", |at| &rest[at..]);
        format!("{}{path}", base.trim_end_matches('/'))
    }
}

/// One run: the models to download and where its progress goes.
#[derive(Debug, Clone)]
pub struct Job {
    pub models_dir: PathBuf,
    pub models: Vec<CatalogEntry>,
    pub status: SharedStatus,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    /// The status a run starts with: every model waiting, nothing received.
    pub fn initial_status(&self) -> DownloadStatus {
        DownloadStatus {
            state: RunState::Running,
            models: self
                .models
                .iter()
                .map(|entry| ModelProgress {
                    id: entry.id.clone(),
                    state: ModelState::Waiting,
                    received: 0,
                    total: entry.total_size(),
                })
                .collect(),
            error: None,
            restart: None,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// `Err(cancelled)` once the run was asked to stop.
    fn check_cancel(&self) -> Result<(), DownloadError> {
        if self.cancelled() {
            return Err(DownloadError::cancelled());
        }
        Ok(())
    }
}

/// Downloads and installs every model of `job`, one file at a time. Blocking:
/// run it on a blocking thread. The run's final state is the caller's.
///
/// Every path written is under the canonical models folder, and the download
/// subtree must hold no links (see `guard`). A run stopped while the cancel
/// flag is set ends as `Cancelled`, whatever the interrupted step reported.
pub fn run(downloader: &Downloader, job: &Job) -> Result<(), DownloadError> {
    let models_dir = guard::anchor(&job.models_dir)?;
    let work = guard::child(&models_dir, DOWNLOAD_DIR)?;
    guard::ensure_dir(&work)?;
    check_free_space(&work, still_needed(job, &work))?;
    let agent = http::agent(&downloader.http, job.cancel.clone());
    for entry in &job.models {
        download_model(downloader, &agent, job, &models_dir, entry)
            .map_err(|err| {
                if job.cancelled() {
                    DownloadError::cancelled()
                } else {
                    err
                }
            })
            .inspect_err(|err| {
                let state = match err.code {
                    ErrorCode::Cancelled => ModelState::Waiting,
                    _ => ModelState::Failed,
                };
                job.status.update_model(&entry.id, |m| m.state = state);
            })?;
    }
    Ok(())
}

/// Bytes still to download: what earlier runs left in `.part` files counts
/// (a link counts as nothing; the download refuses it later).
fn still_needed(job: &Job, work: &Path) -> u64 {
    job.models
        .iter()
        .flat_map(|entry| {
            let dir = work.join(&entry.id);
            entry.files().map(move |file| {
                let have = part_path(&dir, &file.name)
                    .and_then(|part| guard::existing(&part))
                    .ok()
                    .flatten()
                    .map_or(0, |meta| meta.len());
                file.size.saturating_sub(have)
            })
        })
        .sum()
}

fn download_model(
    downloader: &Downloader,
    agent: &ureq::Agent,
    job: &Job,
    models_dir: &Path,
    entry: &CatalogEntry,
) -> Result<(), DownloadError> {
    // Checked again for every model: an install before this one removes
    // `.download` once it is empty.
    let work = guard::child(models_dir, DOWNLOAD_DIR)?;
    guard::ensure_dir(&work)?;
    let dir = guard::child(&work, &entry.id)?;
    guard::ensure_dir(&dir)?;
    let mut done = 0;
    for file in entry.files() {
        job.check_cancel()?;
        let url = downloader.url(&file.url);
        let part = part_path(&dir, &file.name)?;
        job.status
            .update_model(&entry.id, |m| m.state = ModelState::Downloading);
        tracing::info!(model = %entry.id, file = %file.name, host = %fetch::host(&url), "model download started");
        let progress = |have: u64| {
            job.status
                .update_model(&entry.id, |m| m.received = done + have)
        };
        fetch(&Fetch {
            agent,
            url: &url,
            file: &file,
            part: &part,
            cancel: &job.cancel,
            progress: &progress,
        })?;
        job.status
            .update_model(&entry.id, |m| m.state = ModelState::Verifying);
        verify(&part, &file.name, &url, &file.sha256, &job.cancel)?;
        tracing::info!(model = %entry.id, file = %file.name, "model download finished and verified");
        done += file.size;
    }
    // The last moment a cancel can stop the run: an install is not undone.
    job.check_cancel()?;
    install(models_dir, entry, &dir)?;
    job.status
        .update_model(&entry.id, |m| m.state = ModelState::Installed);
    tracing::info!(model = %entry.id, "downloaded model installed");
    Ok(())
}
