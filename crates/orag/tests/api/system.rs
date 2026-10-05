//! Health, version, loopback Host/Origin rules, JSON errors and shutdown.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::json;

use crate::support::{app, get, local, send};

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
