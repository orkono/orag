//! Loopback HTTP API (D-013, D-014).

pub mod collections;
pub mod documents;
pub mod errors;
pub mod jobs;
pub mod query;
pub mod security;
pub mod system;
pub mod ui;

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{delete, get, post};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore};

use crate::error::{OragError, Result};
use crate::infer::{Embedder, Generator};
use crate::retrieval::answer::AnswerEngine;
use crate::retrieval::hybrid::{RetrievalConfig, Retriever};
use crate::server::errors::{ApiError, ApiResult};
use crate::store::Store;
use crate::store::search::SqliteVecIndex;

/// Uploads accepted at the same time; more get `429 busy` (D-019).
pub const MAX_CONCURRENT_UPLOADS: usize = 4;
/// Headroom above the document limit for JSON/multipart framing.
const BODY_OVERHEAD_BYTES: usize = 64 * 1024;
/// JSON string escaping can grow text up to 6× (`\u00XX`); the exact
/// `max_document_mb` check runs on the decoded document, not the HTTP body.
const JSON_ESCAPE_FACTOR: usize = 6;
/// Body limit for every route except document upload (queries, collections).
pub const SMALL_BODY_LIMIT: usize = 64 * 1024;

/// Body limit of the upload route only; everything else gets `SMALL_BODY_LIMIT`.
pub fn upload_body_limit(max_upload_bytes: usize) -> DefaultBodyLimit {
    DefaultBodyLimit::max(
        max_upload_bytes
            .saturating_mul(JSON_ESCAPE_FACTOR)
            .saturating_add(BODY_OVERHEAD_BYTES),
    )
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub engine: Arc<AnswerEngine>,
    pub ingest_wake: Arc<Notify>,
    /// One generation at a time: interactive queries own the generator.
    pub generation: Arc<Semaphore>,
    /// Bounds uploads being received/stored at once (D-019 resource budget).
    pub uploads: Arc<Semaphore>,
    pub max_upload_bytes: usize,
    pub allowed_origins: Arc<Vec<String>>,
    /// `Config::effective()` of the running process; `app.rs` sets it.
    pub effective_config: Arc<serde_json::Value>,
    /// The single shutdown signal: `true` once shutdown starts. Private, so
    /// `begin_shutdown()` is the only way to set it.
    shutdown: Arc<tokio::sync::watch::Sender<bool>>,
}

impl AppState {
    pub fn new(
        store: Arc<Store>,
        embedder: Arc<dyn Embedder>,
        generator: Arc<dyn Generator>,
        max_upload_bytes: usize,
    ) -> AppState {
        let retriever = Retriever {
            index: Arc::new(SqliteVecIndex::new(store.clone())),
            store: store.clone(),
            embedder,
            config: RetrievalConfig::default(),
        };
        AppState {
            store,
            engine: Arc::new(AnswerEngine {
                retriever,
                generator,
                sampler: crate::retrieval::answer::ANSWER_SAMPLER,
            }),
            ingest_wake: Arc::new(Notify::new()),
            generation: Arc::new(Semaphore::new(1)),
            uploads: Arc::new(Semaphore::new(MAX_CONCURRENT_UPLOADS)),
            max_upload_bytes,
            allowed_origins: Arc::new(Vec::new()),
            // Set by `app.rs` from `Config::effective()` with the actual bound address.
            effective_config: Arc::new(serde_json::json!({})),
            shutdown: Arc::new(tokio::sync::watch::channel(false).0),
        }
    }

    /// Starts shutdown for this server: generation callbacks, queued queries and
    /// the ingest worker all observe it through `subscribe_shutdown()`.
    pub fn begin_shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    /// A receiver of the shutdown signal. Async code awaits
    /// `wait_for(|stopping| *stopping)` (which also sees an earlier shutdown);
    /// synchronous callbacks read `*receiver.borrow()`.
    pub fn subscribe_shutdown(&self) -> tokio::sync::watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// Runs `work` unless shutdown starts first (or has already started): then
    /// `503 shutting_down`. Every request that waits (a slot, a body) uses this,
    /// so no waiting point can hold up graceful shutdown.
    pub async fn until_shutdown<T>(
        &self,
        work: impl Future<Output = ApiResult<T>>,
    ) -> ApiResult<T> {
        let mut shutdown = self.subscribe_shutdown();
        tokio::select! {
            biased;
            _ = shutdown.wait_for(|stopping| *stopping) => Err(ApiError::shutting_down()),
            result = work => result,
        }
    }
}

/// The API listener: the `/v1` routes only.
pub fn router(state: AppState) -> Router {
    with_layers(api_routes(&state), state)
}

/// The web UI listener: the page's assets plus the same API, so the page
/// calls `/v1/...` on its own origin.
pub fn ui_router(state: AppState) -> Router {
    with_layers(api_routes(&state).merge(ui::routes()), state)
}

fn api_routes(state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/v1/health", get(system::health))
        .route("/v1/version", get(system::version))
        .route(
            "/v1/collections",
            get(collections::list).post(collections::create),
        )
        .route(
            "/v1/collections/{collection_id}",
            delete(collections::remove),
        )
        .route(
            "/v1/collections/{collection_id}/documents",
            get(documents::list)
                .post(documents::upload)
                // Route-level limit overrides the router's SMALL_BODY_LIMIT (axum 0.8):
                // DefaultBodyLimit only sets a request extension and the inner layer
                // writes last. A wrapping limiter (tower-http RequestBodyLimit) would not.
                .layer(upload_body_limit(state.max_upload_bytes)),
        )
        .route(
            "/v1/collections/{collection_id}/documents/{document_id}",
            get(documents::get_one).delete(documents::remove),
        )
        .route(
            "/v1/collections/{collection_id}/reindex",
            post(collections::reindex),
        )
        .route("/v1/jobs/{job_id}", get(jobs::get_one))
        .route("/v1/collections/{collection_id}/query", post(query::query))
}

fn with_layers(routes: Router<AppState>, state: AppState) -> Router {
    routes
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::check_host_and_origin,
        ))
        // Small by default; Task 17 raises it on the upload route only, so no
        // other route can buffer a document-sized body outside the upload permits.
        .layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT))
        .with_state(state)
}

async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "no such endpoint")
}

async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this endpoint does not accept that method",
    )
}

/// Longest graceful drain after shutdown starts; then open connections are closed.
pub const GRACEFUL_SHUTDOWN_LIMIT: Duration = Duration::from_secs(10);

pub async fn serve(
    state: AppState,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    serve_with_ui(state, listener, None, shutdown).await
}

/// Serves the API on `listener` and, if given, the web UI on `ui_listener`.
/// One shutdown stops both; the drain limit covers both together.
pub async fn serve_with_ui(
    state: AppState,
    listener: TcpListener,
    ui_listener: Option<TcpListener>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    // Whatever ends the server also starts the app's own shutdown, so requests
    // waiting on a slot or a body see 503 instead of holding the drain open.
    let stopper = state.clone();
    let trigger = async move {
        shutdown.await;
        stopper.begin_shutdown();
        std::future::pending::<()>().await;
    };
    // The drain is bounded: a peer that stopped reading (a stalled SSE client)
    // cannot keep the server alive after shutdown starts, however it started.
    let stopped = shutdown_started(&state);
    let drain_limit = async move {
        stopped.await;
        tokio::time::sleep(GRACEFUL_SHUTDOWN_LIMIT).await;
    };
    let api = axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(shutdown_started(&state));
    let ui = async {
        match ui_listener {
            Some(listener) => {
                axum::serve(listener, ui_router(state.clone()))
                    .with_graceful_shutdown(shutdown_started(&state))
                    .await
            }
            None => Ok(()),
        }
    };
    let served = async {
        let (api, ui) = tokio::join!(api.into_future(), ui);
        api.and(ui)
    };
    tokio::select! {
        served = served => {
            served.map_err(|err| OragError::Internal(format!("server error: {err}")))
        }
        () = drain_limit => {
            tracing::warn!(
                "open connections did not finish within {GRACEFUL_SHUTDOWN_LIMIT:?}; closing them"
            );
            Ok(())
        }
        () = trigger => unreachable!("the shutdown trigger never completes"),
    }
}

/// Completes once shutdown starts; never if the signal can no longer come.
fn shutdown_started(state: &AppState) -> impl Future<Output = ()> + Send + 'static {
    let mut stopped = state.subscribe_shutdown();
    async move {
        if stopped.wait_for(|stopping| *stopping).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Runs blocking store/inference work off the async workers.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> ApiResult<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(err) => {
            tracing::error!(error = %err, "blocking task failed");
            Err(ApiError::internal())
        }
    }
}
