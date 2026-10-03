//! Job status endpoint.

use axum::Json;
use axum::extract::State;

use crate::server::errors::{ApiPath, ApiResult};
use crate::server::{AppState, blocking};
use crate::store::jobs::JobRecord;

pub async fn get_one(
    State(state): State<AppState>,
    ApiPath(job_id): ApiPath<i64>,
) -> ApiResult<Json<JobRecord>> {
    let store = state.store.clone();
    Ok(Json(blocking(move || store.get_job(job_id)).await?))
}
