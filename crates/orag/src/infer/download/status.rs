//! The download's status body (`GET /v1/models/download`), shared between the
//! downloading thread and the handlers behind one mutex.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Idle,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Waiting,
    Downloading,
    Verifying,
    Installed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorCode {
    Network,
    Http,
    Checksum,
    Disk,
    Cancelled,
    Internal,
}

/// Why a download stopped; `message` is plain English naming the host or file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadError {
    pub code: ErrorCode,
    pub message: String,
}

impl DownloadError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> DownloadError {
        DownloadError {
            code,
            message: message.into(),
        }
    }

    pub fn cancelled() -> DownloadError {
        DownloadError::new(
            ErrorCode::Cancelled,
            "the download was cancelled; start it again to continue where it stopped",
        )
    }

    pub fn internal(message: impl Into<String>) -> DownloadError {
        DownloadError::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// How setup ends once every model is installed (the import's values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Restart {
    None,
    Automatic,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelProgress {
    pub id: String,
    pub state: ModelState,
    pub received: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DownloadStatus {
    pub state: RunState,
    pub models: Vec<ModelProgress>,
    pub error: Option<DownloadError>,
    pub restart: Option<Restart>,
}

impl DownloadStatus {
    pub fn idle() -> DownloadStatus {
        DownloadStatus {
            state: RunState::Idle,
            models: Vec::new(),
            error: None,
            restart: None,
        }
    }
}

/// The status, cheap to clone and to read.
#[derive(Debug, Clone)]
pub struct SharedStatus(Arc<Mutex<DownloadStatus>>);

impl Default for SharedStatus {
    fn default() -> Self {
        SharedStatus(Arc::new(Mutex::new(DownloadStatus::idle())))
    }
}

impl SharedStatus {
    /// A copy of the current status.
    pub fn snapshot(&self) -> DownloadStatus {
        self.lock().clone()
    }

    /// Replaces the whole status.
    pub fn set(&self, status: DownloadStatus) {
        *self.lock() = status;
    }

    /// Changes the progress of one model; an unknown id is ignored.
    pub fn update_model(&self, id: &str, change: impl FnOnce(&mut ModelProgress)) {
        if let Some(model) = self.lock().models.iter_mut().find(|m| m.id == id) {
            change(model);
        }
    }

    /// Ends the run: its final state, error and restart in one step.
    pub fn finish(&self, state: RunState, error: Option<DownloadError>, restart: Option<Restart>) {
        let mut status = self.lock();
        status.state = state;
        status.error = error;
        status.restart = restart;
    }

    /// A poisoned lock still holds a usable status: progress counters only.
    fn lock(&self) -> MutexGuard<'_, DownloadStatus> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_body_has_the_contract_shape() {
        let status = SharedStatus::default();
        assert_eq!(
            serde_json::to_value(status.snapshot()).unwrap(),
            serde_json::json!({"state": "idle", "models": [], "error": null, "restart": null})
        );
        status.set(DownloadStatus {
            state: RunState::Running,
            models: vec![ModelProgress {
                id: "m".into(),
                state: ModelState::Downloading,
                received: 1,
                total: 2,
            }],
            error: None,
            restart: None,
        });
        status.update_model("m", |m| m.received = 2);
        status.finish(
            RunState::Failed,
            Some(DownloadError::new(ErrorCode::Checksum, "x")),
            Some(Restart::None),
        );
        assert_eq!(
            serde_json::to_value(status.snapshot()).unwrap(),
            serde_json::json!({
                "state": "failed",
                "models": [{"id": "m", "state": "downloading", "received": 2, "total": 2}],
                "error": {"code": "checksum", "message": "x"},
                "restart": "none"
            })
        );
    }
}
