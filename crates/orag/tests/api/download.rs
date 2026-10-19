//! Model download from the setup page (step 76): the HTTP API over a local
//! file server standing in for Hugging Face.

use std::collections::HashMap;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Request as HttpRequest, StatusCode};
use axum::response::{IntoResponse, Response};
use orag::infer::download::{Catalog, CatalogEntry, Downloader};
use orag::infer::models::{ModelRole, find_model};
use orag::server::download::Downloads;
use serde_json::{Value, json};
use sha2::Digest;
use tokio::sync::Semaphore;

use crate::support::{
    SETUP_EMBEDDING, SETUP_GENERATION, TestApp, app, get, json_request, send, setup_app,
    setup_app_supervised,
};

const LICENSE: &[u8] = b"license text";

/// Serves fixed files; every response waits for a permit of `gate`.
#[derive(Clone)]
struct Files {
    files: Arc<HashMap<String, Vec<u8>>>,
    gate: Arc<Semaphore>,
}

async fn serve_file(State(files): State<Files>, request: Request) -> Response {
    let _permit = files.gate.acquire().await.unwrap();
    match files.files.get(request.uri().path()) {
        Some(body) => body.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn weights(id: &str) -> Vec<u8> {
    format!("weights of {id}").repeat(1000).into_bytes()
}

fn entry(id: &str, role: ModelRole) -> CatalogEntry {
    let (body, section) = (
        weights(id),
        match role {
            ModelRole::Embedding => {
                "[embedding]\ndimensions = 16\npooling = \"mean\"\nmax_tokens = 128"
            }
            _ => "[generation]\ncontext_tokens = 2048\nmax_output_tokens = 256",
        },
    );
    CatalogEntry {
        id: id.into(),
        role,
        repo: format!("test/{id}"),
        revision: "r1".into(),
        file: "model.gguf".into(),
        sha256: hex::encode(sha2::Sha256::digest(&body)),
        size: body.len() as u64,
        license_url: format!("https://www.example.org/licenses/{id}/LICENSE"),
        license_sha256: hex::encode(sha2::Sha256::digest(LICENSE)),
        license_size: LICENSE.len() as u64,
        role_section: section.into(),
    }
}

fn both() -> Vec<CatalogEntry> {
    vec![
        entry(SETUP_EMBEDDING, ModelRole::Embedding),
        entry(SETUP_GENERATION, ModelRole::Generation),
    ]
}

/// A file server for `entries`, its responses held until `gate` has permits.
async fn file_server(entries: &[CatalogEntry], gate: Arc<Semaphore>) -> String {
    let mut files = HashMap::new();
    for entry in entries {
        let weights_path = format!("/{}/resolve/{}/{}", entry.repo, entry.revision, entry.file);
        files.insert(weights_path, weights(&entry.id));
        files.insert(format!("/licenses/{}/LICENSE", entry.id), LICENSE.to_vec());
    }
    let files = Files {
        files: Arc::new(files),
        gate,
    };
    let router = axum::Router::new().fallback(serve_file).with_state(files);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, router).into_future());
    base
}

fn open_gate() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(Semaphore::MAX_PERMITS))
}

fn with_downloads(app: TestApp, enabled: bool, catalog: Vec<CatalogEntry>, base: &str) -> TestApp {
    let downloads = Downloads::new(
        enabled,
        Catalog::new(catalog),
        Downloader::with_base_url(base),
    );
    app.map_state(|state| state.with_downloads(downloads))
}

fn post_download() -> HttpRequest<Body> {
    json_request("POST", "/v1/models/download", json!({}))
}

/// Polls the status until the run is no longer running.
async fn finished(app: &TestApp) -> Value {
    for _ in 0..1000 {
        let (status, body) = send(app, get("/v1/models/download")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["state"] != json!("running") {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the download did not finish");
}

#[tokio::test]
async fn before_a_start_the_status_is_idle_and_the_download_is_offered() {
    let app = with_downloads(setup_app(), true, both(), "http://127.0.0.1:9");
    let (status, body) = send(&app, get("/v1/models/download")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"state": "idle", "models": [], "error": null, "restart": null})
    );
    let (_, models) = send(&app, get("/v1/models")).await;
    let bytes: u64 = both().iter().map(CatalogEntry::total_size).sum();
    assert_eq!(
        models["download"],
        json!({"available": true, "reason": null, "bytes": bytes}),
        "{models}"
    );
}

#[tokio::test]
async fn one_press_downloads_both_models_and_restarts_under_orag_start() {
    let base = file_server(&both(), open_gate()).await;
    let app = with_downloads(setup_app_supervised(), true, both(), &base);
    let (status, body) = send(&app, post_download()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], json!("running"), "{body}");
    let ids: Vec<&str> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [SETUP_EMBEDDING, SETUP_GENERATION]);
    let done = finished(&app).await;
    assert_eq!(
        (done["state"].clone(), done["restart"].clone()),
        (json!("done"), json!("automatic")),
        "{done}"
    );
    for model in done["models"].as_array().unwrap() {
        assert_eq!(model["state"], json!("installed"), "{done}");
        assert_eq!(model["received"], model["total"], "{done}");
    }
    let dir = &app.state.models.models_dir;
    find_model(dir, SETUP_EMBEDDING, ModelRole::Embedding).unwrap();
    find_model(dir, SETUP_GENERATION, ModelRole::Generation).unwrap();
    assert!(app.state.shutdown_started());
    assert_eq!(app.state.exit_code(), Some(75));
}

#[tokio::test]
async fn without_orag_start_the_restart_is_manual_and_a_bodyless_post_works() {
    let base = file_server(&both(), open_gate()).await;
    let app = with_downloads(setup_app(), true, both(), &base);
    let request = crate::support::local(HttpRequest::post("/v1/models/download"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let done = finished(&app).await;
    assert_eq!(
        (done["state"].clone(), done["restart"].clone()),
        (json!("done"), json!("manual")),
        "{done}"
    );
    assert!(!app.state.shutdown_started());
}

#[tokio::test]
async fn a_second_start_while_running_is_refused_and_delete_cancels() {
    let gate = Arc::new(Semaphore::new(0));
    let base = file_server(&both(), gate.clone()).await;
    let app = with_downloads(setup_app(), true, both(), &base);
    assert_eq!(send(&app, post_download()).await.0, StatusCode::ACCEPTED);
    let (status, body) = send(&app, post_download()).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("conflict")),
        "{body}"
    );
    let import = json_request(
        "POST",
        "/v1/models/import",
        json!({"path": app.state.models.models_dir}),
    );
    assert_eq!(
        send(&app, import).await.0,
        StatusCode::CONFLICT,
        "one permit for imports and downloads"
    );
    let (status, body) = send(
        &app,
        json_request("DELETE", "/v1/models/download", json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    gate.add_permits(Semaphore::MAX_PERMITS);
    let done = finished(&app).await;
    assert_eq!(
        (
            done["state"].clone(),
            done["error"]["code"].clone(),
            done["restart"].clone()
        ),
        (json!("cancelled"), json!("cancelled"), Value::Null),
        "{done}"
    );
    assert!(!app.state.shutdown_started());
}

#[tokio::test]
async fn a_failed_file_is_reported_with_its_code() {
    let base = file_server(&[], open_gate()).await; // every path is 404
    let app = with_downloads(setup_app(), true, both(), &base);
    assert_eq!(send(&app, post_download()).await.0, StatusCode::ACCEPTED);
    let done = finished(&app).await;
    assert_eq!(
        (done["state"].clone(), done["error"]["code"].clone()),
        (json!("failed"), json!("http")),
        "{done}"
    );
    assert_eq!(done["models"][0]["state"], json!("failed"), "{done}");
    assert!(
        done["error"]["message"].as_str().unwrap().contains("404"),
        "{done}"
    );
}

#[tokio::test]
async fn a_server_with_its_models_never_downloads() {
    let app = with_downloads(app(), true, both(), "http://127.0.0.1:9");
    let (status, body) = send(&app, post_download()).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("conflict")),
        "{body}"
    );
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(models["download"]["available"], json!(false), "{models}");
    assert!(models["download"]["reason"].is_string(), "{models}");
}

#[tokio::test]
async fn a_config_without_downloads_refuses_with_403() {
    let app = with_downloads(setup_app(), false, both(), "http://127.0.0.1:9");
    let (status, body) = send(&app, post_download()).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::FORBIDDEN, json!("download_disabled")),
        "{body}"
    );
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(models["download"]["available"], json!(false), "{models}");
    let reason = models["download"]["reason"].as_str().unwrap();
    assert!(reason.contains("models.download"), "{reason}");
    assert_eq!(models["download"]["bytes"], Value::Null);
}

#[tokio::test]
async fn a_model_outside_the_catalog_makes_the_download_unavailable() {
    let only_embedding = vec![entry(SETUP_EMBEDDING, ModelRole::Embedding)];
    let app = with_downloads(setup_app(), true, only_embedding, "http://127.0.0.1:9");
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(models["download"]["available"], json!(false), "{models}");
    let reason = models["download"]["reason"].as_str().unwrap();
    assert!(reason.contains(SETUP_GENERATION), "{reason}");
    let (status, body) = send(&app, post_download()).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("download_unavailable")),
        "{body}"
    );
    let (_, status_body) = send(&app, get("/v1/models/download")).await;
    assert_eq!(status_body["state"], json!("idle"));
}

#[tokio::test]
async fn a_catalog_model_of_the_wrong_role_is_not_offered() {
    let swapped = vec![
        entry(SETUP_EMBEDDING, ModelRole::Generation),
        entry(SETUP_GENERATION, ModelRole::Generation),
    ];
    let app = with_downloads(setup_app(), true, swapped, "http://127.0.0.1:9");
    let (_, models) = send(&app, get("/v1/models")).await;
    let reason = models["download"]["reason"].as_str().unwrap();
    assert!(
        reason.contains(SETUP_EMBEDDING) && reason.contains("embedding"),
        "{reason}"
    );
}
