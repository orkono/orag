//! Liveness and version endpoints.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::server::AppState;
use crate::version::version_info;

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Version information plus the configuration in effect (read at startup).
pub async fn version(State(state): State<AppState>) -> Json<Value> {
    let mut body = json!(version_info(Some(state.store.schema_version())));
    body["config"] = (*state.effective_config).clone();
    Json(body)
}
