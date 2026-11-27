//! CORS for origins named by the starting program (`serve --allow-origin`).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::json;

use crate::support::{TestApp, app, get, local, send_full};

const ORIGIN: &str = "tauri://localhost";

fn named() -> TestApp {
    app().map_state(|mut state| {
        state.allowed_origins = Arc::new(vec![ORIGIN.to_string()]);
        state
    })
}

fn preflight(origin: &str, extra: &[(&str, &str)]) -> Request<Body> {
    let mut builder = local(Request::builder().method("OPTIONS").uri("/v1/collections"))
        .header(header::ORIGIN, origin)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type");
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::empty()).unwrap()
}

fn with_origin(uri: &str, origin: &str) -> Request<Body> {
    local(Request::get(uri))
        .header(header::ORIGIN, origin)
        .body(Body::empty())
        .unwrap()
}

fn value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).map(|v| v.to_str().unwrap())
}

fn has_cors_header(headers: &HeaderMap) -> bool {
    headers
        .keys()
        .any(|k| k.as_str().starts_with("access-control-"))
}

#[tokio::test]
async fn preflight_from_allowed_origin_is_answered() {
    let (status, headers, _) = send_full(&named(), preflight(ORIGIN, &[])).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(value(&headers, "access-control-allow-origin"), Some(ORIGIN));
    assert_eq!(
        value(&headers, "access-control-allow-methods"),
        Some("GET, POST, DELETE")
    );
    assert_eq!(
        value(&headers, "access-control-allow-headers"),
        Some("accept, content-type")
    );
    assert_eq!(value(&headers, "access-control-max-age"), Some("600"));
    assert_eq!(value(&headers, "vary"), Some("origin"));
}

#[tokio::test]
async fn simple_response_carries_allow_origin() {
    let (status, headers, _) = send_full(&named(), with_origin("/v1/health", ORIGIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value(&headers, "access-control-allow-origin"), Some(ORIGIN));
}

#[tokio::test]
async fn cors_exposes_retry_after() {
    let (_, headers, _) = send_full(&named(), with_origin("/v1/health", ORIGIN)).await;
    assert_eq!(
        value(&headers, "access-control-expose-headers"),
        Some("retry-after")
    );
}

#[tokio::test]
async fn other_origin_gets_403_and_no_cors_headers() {
    let evil = "http://evil.example";
    let (status, headers, _) = send_full(&named(), with_origin("/v1/health", evil)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!has_cors_header(&headers), "{headers:?}");
    let (status, headers, _) = send_full(&named(), preflight(evil, &[])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!has_cors_header(&headers), "{headers:?}");
}

#[tokio::test]
async fn no_origin_gets_no_cors_headers() {
    let (status, headers, _) = send_full(&named(), get("/v1/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!has_cors_header(&headers), "{headers:?}");
}

#[tokio::test]
async fn sse_answer_streams_to_allowed_origin() {
    let app = named();
    let request = local(Request::post("/v1/collections/1/query"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(
            json!({"query": "iade", "stream": true}).to_string(),
        ))
        .unwrap();
    let (status, headers, _) = send_full(&app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value(&headers, "content-type"), Some("text/event-stream"));
    assert_eq!(value(&headers, "access-control-allow-origin"), Some(ORIGIN));
}

#[tokio::test]
async fn private_network_preflight_is_allowed_for_named_origins() {
    let ask = [("access-control-request-private-network", "true")];
    let (status, headers, _) = send_full(&named(), preflight(ORIGIN, &ask)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        value(&headers, "access-control-allow-private-network"),
        Some("true")
    );
    let (status, headers, _) = send_full(&named(), preflight("http://evil.example", &ask)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        headers
            .get("access-control-allow-private-network")
            .is_none(),
        "{headers:?}"
    );
}
