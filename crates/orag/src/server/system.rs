//! Liveness and version endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::server::AppState;
use crate::server::errors::{ApiError, ApiResult};
use crate::version::version_info;

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Version information plus the configuration in effect (read at startup)
/// and the GPU state.
pub async fn version(State(state): State<AppState>) -> Json<Value> {
    let mut body = json!(version_info(Some(state.store.schema_version())));
    body["config"] = (*state.effective_config).clone();
    body["gpu"] = json!(*state.gpu);
    Json(body)
}

/// "Try the graphics card again": ends the process with `EXIT_GPU_RETRY`,
/// on which the desktop app clears its fallback and probes the GPU again.
/// Only a CPU service the desktop app started after a GPU fallback offers it;
/// anything else answers 409 and keeps running (`orag start` restarts only
/// on 75, so a 76 would stop it).
pub async fn gpu_retry(State(state): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    if !state.gpu.retry {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "there is no GPU fallback to retry here; only the desktop app retries the graphics card",
        ));
    }
    state.request_gpu_retry();
    Ok((StatusCode::ACCEPTED, Json(json!({"status": "restarting"}))))
}
