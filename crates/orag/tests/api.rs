//! HTTP API integration tests: real router, temp SQLite, fake models.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use orag::infer::fake::{FakeEmbedder, FakeGenerator};
use orag::server::{AppState, router};
use orag::store::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

pub struct TestApp {
    pub _dir: tempfile::TempDir,
    pub state: AppState,
    pub generator: Arc<FakeGenerator>,
}

pub fn app_with(generator: FakeGenerator, max_upload_bytes: usize) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
    let generator = Arc::new(generator);
    let state = AppState::new(
        store,
        Arc::new(FakeEmbedder::new()),
        generator.clone(),
        max_upload_bytes,
    );
    TestApp {
        _dir: dir,
        state,
        generator,
    }
}

pub fn app() -> TestApp {
    app_with(FakeGenerator::new("Cevap [1]."), 1024 * 1024)
}

pub fn get(uri: &str) -> Request<Body> {
    local(Request::get(uri)).body(Body::empty()).unwrap()
}

/// What a local client (curl, the desktop app) sends: a loopback Host, no credentials.
pub fn local(builder: axum::http::request::Builder) -> axum::http::request::Builder {
    builder.header(header::HOST, "127.0.0.1:7613")
}

pub fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    local(Request::builder().method(method).uri(uri))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub async fn send(app: &TestApp, request: Request<Body>) -> (StatusCode, Value) {
    let (status, _, value) = send_full(app, request).await;
    (status, value)
}

/// Like `send`, but also returns the response headers.
pub async fn send_full(
    app: &TestApp,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, headers, value)
}

#[tokio::test]
async fn health_answers_local_requests() {
    let app = app();
    let request = Request::get("/v1/health")
        .header(header::HOST, "localhost:7613")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"status": "ok"}));
}

#[tokio::test]
async fn api_needs_no_credentials_and_reports_version_and_config() {
    let app = app();
    let (status, body) = send(&app, get("/v1/version")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"], orag::version::VERSION);
    assert_eq!(
        body["schema_version"],
        orag::store::migrations::SUPPORTED_SCHEMA_VERSION
    );
    assert!(body["config"].is_object());
}

#[tokio::test]
async fn foreign_host_and_origin_are_rejected() {
    let app = app();
    let rebinding = Request::get("/v1/health")
        .header(header::HOST, "evil.example:7613")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, rebinding).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden_host");
    let cross_origin = local(Request::get("/v1/version"))
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&app, cross_origin).await.1["error"]["code"],
        "forbidden_origin"
    );
    let ipv6 = Request::get("/v1/health")
        .header(header::HOST, "[::1]:7613")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, ipv6).await.0, StatusCode::OK);
}

#[tokio::test]
async fn unknown_routes_and_methods_answer_with_json_errors() {
    let app = app();
    let (status, body) = send(&app, get("/v1/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    let wrong = local(Request::delete("/v1/version"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, wrong).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["error"]["code"], "method_not_allowed");
}

#[tokio::test]
async fn serve_starts_shutdown_for_waiting_requests() {
    let app = app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut watcher = app.state.subscribe_shutdown();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(orag::server::serve(app.state.clone(), listener, async {
        let _ = rx.await;
    }));
    tx.send(()).unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        watcher.wait_for(|stop| *stop),
    )
    .await
    .expect("serve signals shutdown")
    .unwrap();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn waiting_work_ends_with_503_when_shutdown_starts() {
    let app = app();
    let waiter = {
        let state = app.state.clone();
        tokio::spawn(async move {
            state
                .until_shutdown(std::future::pending::<orag::server::errors::ApiResult<()>>())
                .await
        })
    };
    tokio::task::yield_now().await;
    app.state.begin_shutdown();
    let err = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .expect("the waiter is released")
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "shutting_down");
    // Shutdown that started earlier is seen too.
    let late = app
        .state
        .until_shutdown(async { Ok(()) })
        .await
        .unwrap_err();
    assert_eq!(late.code, "shutting_down");
}

#[tokio::test]
async fn cross_site_requests_without_origin_are_rejected() {
    let app = app();
    let blind = local(Request::get("/v1/version"))
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, blind).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden_origin");
    let same = local(Request::get("/v1/version"))
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, same).await.0, StatusCode::OK);
}
