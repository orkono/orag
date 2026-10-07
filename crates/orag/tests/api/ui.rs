//! Built-in web UI: embedded assets, their headers, and the origins it may use.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use orag::server::{router, ui, ui_router};
use serde_json::json;
use tower::ServiceExt;

use crate::support::{TestApp, app, drain_jobs, local, multipart};

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

/// A request the page sends: same origin, so it carries the page's `Origin`.
fn from_page(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let builder =
        local(Request::builder().method(method).uri(uri)).header(header::ORIGIN, UI_ORIGIN);
    match body {
        Some(json) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn the_page_can_manage_collections_and_use_the_selected_one() {
    let app = ui_app();
    let call = |request| fetch(ui_router(app.state.clone()), request);
    let (status, _, body) = call(from_page(
        "POST",
        "/v1/collections",
        Some(json!({"name": "Sözleşmeler"})),
    ))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = created["id"].as_i64().unwrap();

    let mut upload = multipart(
        "iade.txt",
        "text/plain",
        "İade süresi 30 gündür.".as_bytes(),
    );
    *upload.uri_mut() = format!("/v1/collections/{id}/documents").parse().unwrap();
    upload
        .headers_mut()
        .insert(header::ORIGIN, UI_ORIGIN.parse().unwrap());
    let (status, _, body) = call(upload).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    drain_jobs(&app);

    let (status, _, body) = call(from_page("GET", "/v1/collections", None)).await;
    assert_eq!(status, StatusCode::OK);
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let mine = listed["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert_eq!(mine["document_count"], 1);

    let query = from_page(
        "POST",
        &format!("/v1/collections/{id}/query"),
        Some(json!({"query": "iade süresi"})),
    );
    let (status, _, body) = call(query).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("iade.txt"),
        "the selected collection is searched: {body}"
    );
    let other = from_page(
        "POST",
        "/v1/collections/1/query",
        Some(json!({"query": "iade süresi"})),
    );
    let (status, _, body) = call(other).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("iade.txt"),
        "other collections are not: {body}"
    );

    let (status, _, body) = call(from_page("DELETE", &format!("/v1/collections/{id}"), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, body) = call(from_page("DELETE", "/v1/collections/1", None)).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the default collection stays: {body}"
    );
}

/// Every element the script looks up exists in the page.
#[test]
fn the_script_only_uses_elements_the_page_has() {
    let ids: Vec<&str> = ui::APP_JS
        .split("$(\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect();
    assert!(ids.len() > 10, "{ids:?}");
    for id in ids {
        assert!(
            ui::INDEX_HTML.contains(&format!("id=\"{id}\"")),
            "#{id} is not in index.html"
        );
    }
}

#[test]
fn the_page_offers_collection_management() {
    for id in [
        "collection",
        "collection-form",
        "collection-name",
        "collection-delete",
    ] {
        assert!(ui::INDEX_HTML.contains(&format!("id=\"{id}\"")), "#{id}");
    }
    // Uploads and questions go to the selected collection, not a fixed one.
    assert!(!ui::APP_JS.contains("/v1/collections/1/"));
    assert!(!ui::APP_JS.contains("COLLECTION = 1"));
}

#[tokio::test]
async fn the_page_can_list_and_delete_documents_of_a_collection() {
    let app = ui_app();
    let call = |request| fetch(ui_router(app.state.clone()), request);
    let mut upload = multipart("not.txt", "text/plain", "Toplantı notu.".as_bytes());
    upload
        .headers_mut()
        .insert(header::ORIGIN, UI_ORIGIN.parse().unwrap());
    let (status, _, body) = call(upload).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    drain_jobs(&app);

    let list = from_page("GET", "/v1/collections/1/documents?limit=50", None);
    let (status, _, body) = call(list).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let doc = &listed["documents"][0];
    assert_eq!(
        (doc["filename"].as_str(), doc["status"].as_str()),
        (Some("not.txt"), Some("ready"))
    );
    let id = doc["id"].as_i64().unwrap();

    let path = format!("/v1/collections/1/documents/{id}");
    let (status, _, body) = call(from_page("DELETE", &path, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, _) = call(from_page("GET", &path, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn the_page_lists_the_documents_of_the_selected_collection() {
    for id in ["documents", "documents-more", "documents-empty"] {
        assert!(ui::INDEX_HTML.contains(&format!("id=\"{id}\"")), "#{id}");
    }
    // Paged with the API's cursor, and each row can be deleted.
    assert!(ui::APP_JS.contains("after_id"));
    assert!(ui::APP_JS.contains("/documents/${doc.id}`"));
    assert!(ui::APP_JS.contains("method: \"DELETE\""));
}

#[test]
fn the_page_offers_a_reindex_of_the_selected_collection() {
    assert!(ui::INDEX_HTML.contains("id=\"collection-reindex\""));
    assert!(ui::APP_JS.contains("/reindex`"));
    // A 409 reindex_required points the user to the button.
    assert!(ui::APP_JS.contains("\"reindex_required\""));
}

#[test]
fn citations_in_the_answer_link_to_their_sources() {
    // Each source has an anchor, and markers in a finished answer link to it.
    assert!(ui::APP_JS.contains("`source-${source.number}`"));
    assert!(ui::APP_JS.contains("function linkCitations("));
    // The server's markers decide what a citation is: no client-side parser.
    assert!(ui::APP_JS.contains("data.citation_markers"));
    assert!(!ui::APP_JS.contains("matchAll("));
    // Opening a source adds no history entry (no `:target` highlight).
    assert!(ui::APP_JS.contains("event.preventDefault()"));
    assert!(!ui::APP_CSS.contains(":target"));
    // HTML sinks are banned in the whole script by
    // `assets_load_nothing_from_the_network_and_never_parse_html`.
}
