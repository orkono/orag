//! Built-in web UI: embedded assets, their headers, and the origins it may use.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use orag::server::{router, ui, ui_router};
use serde_json::json;
use tower::ServiceExt;

use crate::support::{TestApp, app, local};

const UI_ORIGIN: &str = "http://127.0.0.1:2442";

/// An app whose allowed origins are the UI's own, as `orag serve` sets them.
fn ui_app() -> TestApp {
    let mut app = app();
    app.state.allowed_origins = Arc::new(ui::origins("127.0.0.1:2442".parse().unwrap()));
    app
}

async fn fetch(
    router: axum::Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = router.oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

fn get_ui(path: &str) -> Request<Body> {
    Request::get(path)
        .header(header::HOST, "127.0.0.1:2442")
        .body(Body::empty())
        .unwrap()
}

#[test]
fn ui_origins_follow_the_bound_address() {
    assert_eq!(
        ui::origins("127.0.0.1:2442".parse().unwrap()),
        vec!["http://127.0.0.1:2442", "http://localhost:2442"]
    );
    assert_eq!(
        ui::origins("[::1]:50123".parse().unwrap()),
        vec!["http://[::1]:50123", "http://localhost:50123"]
    );
}

#[tokio::test]
async fn assets_are_served_with_their_types_and_a_strict_policy() {
    let app = ui_app();
    for (path, content_type, marker) in [
        ("/", "text/html; charset=utf-8", "<!doctype html>"),
        (
            "/app.js",
            "text/javascript; charset=utf-8",
            "\"use strict\"",
        ),
        ("/app.css", "text/css; charset=utf-8", "body"),
        ("/favicon.svg", "image/svg+xml; charset=utf-8", "<svg"),
    ] {
        let (status, headers, body) = fetch(ui_router(app.state.clone()), get_ui(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{path}");
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            "default-src 'self'",
            "{path}"
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
        assert!(body.contains(marker), "{path}: {body}");
    }
}

#[tokio::test]
async fn the_ui_listener_serves_the_api_too() {
    let app = ui_app();
    let (status, _, body) = fetch(ui_router(app.state.clone()), get_ui("/v1/health")).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, r#"{"status":"ok"}"#)
    );
}

#[tokio::test]
async fn the_api_listener_does_not_serve_the_ui() {
    let app = ui_app();
    let (status, _, body) = fetch(router(app.state.clone()), get_ui("/")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn host_checks_apply_to_the_ui() {
    let app = ui_app();
    let rebinding = Request::get("/")
        .header(header::HOST, "evil.example:2442")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = fetch(ui_router(app.state.clone()), rebinding).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("forbidden_host"), "{body}");
}

#[tokio::test]
async fn a_link_from_another_site_opens_the_page_but_not_the_data() {
    let app = ui_app();
    let cross_site = |path: &str| {
        Request::get(path)
            .header(header::HOST, "127.0.0.1:2442")
            .header("sec-fetch-site", "cross-site")
            .body(Body::empty())
            .unwrap()
    };
    let (status, _, body) = fetch(ui_router(app.state.clone()), cross_site("/")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for path in ["/v1/collections", "/v1/collections/1/documents"] {
        let (status, _, body) = fetch(ui_router(app.state.clone()), cross_site(path)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert!(body.contains("forbidden_origin"), "{path}: {body}");
    }
}

fn query_from(origin: &str) -> Request<Body> {
    local(Request::post("/v1/collections/1/query"))
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"query": "iade"}).to_string()))
        .unwrap()
}

#[tokio::test]
async fn only_the_ui_origins_may_call_the_api() {
    let app = ui_app();
    for origin in [UI_ORIGIN, "http://localhost:2442"] {
        let (status, _, body) = fetch(ui_router(app.state.clone()), query_from(origin)).await;
        assert_eq!(status, StatusCode::OK, "{origin}: {body}");
    }
    for origin in [
        "https://evil.example",
        "http://127.0.0.1:2443",
        "http://127.0.0.1:7613",
        "https://127.0.0.1:2442",
        "null",
    ] {
        let (status, _, body) = fetch(ui_router(app.state.clone()), query_from(origin)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
        assert!(body.contains("forbidden_origin"), "{origin}: {body}");
    }
}

#[test]
fn assets_load_nothing_from_the_network_and_never_parse_html() {
    for (name, text) in [
        ("index.html", ui::INDEX_HTML),
        ("app.js", ui::APP_JS),
        ("app.css", ui::APP_CSS),
    ] {
        assert!(
            !text.contains("http://") && !text.contains("https://") && !text.contains("//cdn"),
            "{name} references a network resource"
        );
    }
    // Document text is untrusted: the script renders it as text only.
    for sink in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
    ] {
        assert!(!ui::APP_JS.contains(sink), "app.js uses {sink}");
    }
    assert!(ui::APP_JS.contains("textContent"));
    // CSP forbids inline script and style: everything comes from the two files.
    assert!(!ui::INDEX_HTML.contains("<script>") && !ui::INDEX_HTML.contains("style="));
    assert!(ui::INDEX_HTML.contains(r#"<script src="/app.js" defer></script>"#));
}
