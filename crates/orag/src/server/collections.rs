//! Collection endpoints.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::errors::{ApiError, ApiPath, ApiResult};
use crate::server::{AppState, blocking};
use crate::store::collections::Collection;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCollection {
    pub name: String,
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let store = state.store.clone();
    let collections = blocking(move || store.list_collections()).await?;
    Ok(Json(json!({ "collections": collections })))
}

pub async fn create(
    State(state): State<AppState>,
    body: Result<Json<CreateCollection>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<Collection>)> {
    let Json(body) = body.map_err(|e| ApiError::json_rejection(e.status(), e.body_text()))?;
    let store = state.store.clone();
    let created = blocking(move || store.create_collection(&body.name)).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn remove(
    State(state): State<AppState>,
    ApiPath(collection_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let store = state.store.clone();
    blocking(move || store.delete_collection(collection_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
