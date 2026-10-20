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
use orag::infer::download::options::DISK_HEADROOM_BYTES;
use orag::infer::download::{Catalog, CatalogEntry, Downloader, SpeedHint, Tier, TierSize};
use orag::infer::models::{ModelRole, find_model};
use orag::probe::{CpuFeatures, Machine, X86Features};
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
    let download = &models["download"];
    assert_eq!(
        ["available", "code", "reason", "bytes"].map(|key| download[key].clone()),
        [json!(true), Value::Null, Value::Null, json!(bytes)],
        "{models}"
    );
    // The test catalog has none of the built-in tiers; the configured
    // generator is a catalog generator, so it is the one (current) option.
    let options = download["options"].as_array().unwrap();
    assert_eq!(options.len(), 1, "{models}");
    assert_eq!(
        (options[0]["id"].clone(), options[0]["current"].clone()),
        (json!(SETUP_GENERATION), json!(true))
    );
    assert!(models["machine"]["total_ram_bytes"].is_u64(), "{models}");
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
    // Installed but not loaded yet: nothing is left to download.
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(models["download"]["available"], json!(false), "{models}");
    assert_eq!(models["download"]["code"], json!("conflict"), "{models}");
    let (status, _) = send(&app, post_download()).await;
    assert_eq!(status, StatusCode::CONFLICT);
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
    assert_eq!(models["download"]["code"], json!("conflict"), "{models}");
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
    assert_eq!(models["download"]["code"], json!("download_disabled"));
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
    assert_eq!(models["download"]["code"], json!("download_unavailable"));
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

/* The generator choice (step 82): the machine, the tiers and the config write. */

const LARGE: &str = "gen-large";
const GIB: u64 = 1024 * 1024 * 1024;

/// A machine with `total_gib` of RAM (all of it free) and lots of disk.
fn machine(total_gib: u64) -> Machine {
    Machine {
        os: "test",
        arch: "aarch64",
        total_ram_bytes: Some(total_gib * GIB),
        available_ram_bytes: Some(total_gib * GIB),
        logical_cores: Some(8),
        physical_cores: Some(8),
        cpu_model: Some("Test CPU".into()),
        cpu_features: CpuFeatures::NotApplicable,
        apple_silicon: true,
        metal: true,
        free_disk_bytes: Some(500 * GIB),
        gpu: None,
    }
}

fn tier(id: &str, size: TierSize, need_gib: u64) -> Tier {
    Tier {
        id: id.into(),
        size,
        need_ram_bytes: need_gib * GIB,
        speed: SpeedHint::default(),
        offered: true,
    }
}

/// The configured generator is the default tier (4 GiB), `gen-large` the
/// large one (16 GiB); `gen-small` is in the catalog but not offered.
fn tiers() -> Vec<Tier> {
    let mut small = tier("gen-small", TierSize::Small, 2);
    small.offered = false;
    vec![
        small,
        tier(SETUP_GENERATION, TierSize::Default, 4),
        tier(LARGE, TierSize::Large, 16),
    ]
}

fn tiered_catalog() -> Vec<CatalogEntry> {
    let mut entries = both();
    entries.push(entry(LARGE, ModelRole::Generation));
    entries.push(entry("gen-small", ModelRole::Generation));
    entries
}

/// A setup app on a machine with `total_gib` of RAM, its config file in
/// the app's home.
async fn tiered_app(total_gib: u64, supervised: bool, base: Option<&str>) -> TestApp {
    let catalog = tiered_catalog();
    let base = match base {
        Some(base) => base.to_string(),
        None => file_server(&catalog, open_gate()).await,
    };
    let app = if supervised {
        setup_app_supervised()
    } else {
        setup_app()
    };
    let config = config_path(&app);
    let downloads = Downloads::new(true, Catalog::new(catalog), Downloader::with_base_url(base))
        .with_tiers(tiers())
        .with_machine(machine(total_gib))
        .with_config_file(config);
    app.map_state(|state| state.with_downloads(downloads))
}

fn config_path(app: &TestApp) -> std::path::PathBuf {
    app.state
        .models
        .models_dir
        .parent()
        .unwrap()
        .join("config.toml")
}

fn choose(id: &str) -> HttpRequest<Body> {
    json_request(
        "POST",
        "/v1/models/download",
        json!({"generation_model": id}),
    )
}

#[tokio::test]
async fn the_models_status_reports_the_machine_and_the_tiers_that_fit() {
    let app = tiered_app(8, false, Some("http://127.0.0.1:9")).await;
    let (status, models) = send(&app, get("/v1/models")).await;
    assert_eq!(status, StatusCode::OK, "{models}");
    assert_eq!(
        models["machine"]["total_ram_bytes"],
        json!(8 * GIB),
        "{models}"
    );
    assert_eq!(models["machine"]["metal"], json!(true));
    assert_eq!(models["machine"]["cpu_features"], json!("not_applicable"));
    let download = &models["download"];
    let options = download["options"].as_array().unwrap();
    let listed: Vec<(&str, &str)> = options
        .iter()
        .map(|o| (o["id"].as_str().unwrap(), o["verdict"].as_str().unwrap()))
        .collect();
    assert_eq!(
        listed,
        [(SETUP_GENERATION, "recommended"), (LARGE, "too_big_ram")],
        "the small tier is not offered: {download}"
    );
    assert_eq!(options[0]["label_key"], json!("setup.tier.default"));
    assert_eq!(options[0]["need_ram_bytes"], json!(4 * GIB));
    assert_eq!(
        options[1]["bytes"],
        json!(entry(LARGE, ModelRole::Generation).total_size())
    );
    assert_eq!(
        download["support"],
        json!({"status": "supported", "code": null, "missing_cpu_features": []})
    );
    assert_eq!(download["warnings"], json!(["single_option"]));
    assert_eq!(download["selected"], json!(SETUP_GENERATION));
    assert_eq!(
        download["embedding_bytes"],
        json!(entry(SETUP_EMBEDDING, ModelRole::Embedding).total_size())
    );
    // Today's fields stay.
    assert_eq!(download["available"], json!(true));
}

#[tokio::test]
async fn an_unknown_or_unoffered_generator_is_refused_with_400() {
    let app = tiered_app(32, false, Some("http://127.0.0.1:9")).await;
    for id in ["nope", "gen-small", SETUP_EMBEDDING] {
        let (status, body) = send(&app, choose(id)).await;
        assert_eq!(
            (status, body["error"]["code"].clone()),
            (StatusCode::BAD_REQUEST, json!("unknown_model")),
            "{id}: {body}"
        );
    }
    let (status, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/models/download",
            json!({"generation_model": 4}),
        ),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("invalid_input")),
        "{body}"
    );
    let (_, idle) = send(&app, get("/v1/models/download")).await;
    assert_eq!(idle["state"], json!("idle"));
    assert!(!config_path(&app).exists(), "nothing was written");
}

#[tokio::test]
async fn a_generator_too_big_for_this_machine_is_refused_with_409() {
    let app = tiered_app(8, false, Some("http://127.0.0.1:9")).await;
    let (status, body) = send(&app, choose(LARGE)).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("model_not_eligible")),
        "{body}"
    );
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("too_big_ram"), "{message}");
    let (_, idle) = send(&app, get("/v1/models/download")).await;
    assert_eq!(idle["state"], json!("idle"));
}

#[tokio::test]
async fn the_chosen_generator_is_downloaded_and_written_before_the_restart() {
    let app = tiered_app(32, true, None).await;
    let config = config_path(&app);
    let before = "# mine\nlog_level = \"debug\"\n";
    std::fs::write(&config, before).unwrap();
    let (status, body) = send(&app, choose(LARGE)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ids: Vec<&str> = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [SETUP_EMBEDDING, LARGE],
        "the chosen generator replaces the configured one"
    );
    let done = finished(&app).await;
    assert_eq!(
        (done["state"].clone(), done["restart"].clone()),
        (json!("done"), json!("automatic")),
        "{done}"
    );
    let dir = &app.state.models.models_dir;
    find_model(dir, LARGE, ModelRole::Generation).unwrap();
    assert!(find_model(dir, SETUP_GENERATION, ModelRole::Generation).is_err());
    let written = std::fs::read_to_string(&config).unwrap();
    assert_eq!(written, format!("{before}generation_model = \"{LARGE}\"\n"));
    assert_eq!(app.state.exit_code(), Some(75));
}

#[tokio::test]
async fn a_failed_download_leaves_the_config_untouched() {
    let base = file_server(&[], open_gate()).await; // every path is 404
    let app = tiered_app(32, false, Some(&base)).await;
    let config = config_path(&app);
    let before = b"# mine\r\ngeneration_model = \"gen-setup\"\r\n".to_vec();
    std::fs::write(&config, &before).unwrap();
    assert_eq!(send(&app, choose(LARGE)).await.0, StatusCode::ACCEPTED);
    let done = finished(&app).await;
    assert_eq!(done["state"], json!("failed"), "{done}");
    assert_eq!(std::fs::read(&config).unwrap(), before);
}

#[tokio::test]
async fn choosing_the_configured_generator_downloads_like_before() {
    let app = tiered_app(32, false, None).await;
    let config = config_path(&app);
    let before = format!("generation_model = \"{SETUP_GENERATION}\" # mine\n");
    std::fs::write(&config, &before).unwrap();
    let modified = std::fs::metadata(&config).unwrap().modified().unwrap();
    let (status, body) = send(&app, choose(SETUP_GENERATION)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let done = finished(&app).await;
    assert_eq!(
        (done["state"].clone(), done["restart"].clone()),
        (json!("done"), json!("manual")),
        "{done}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        before,
        "the key already names it"
    );
    assert_eq!(
        std::fs::metadata(&config).unwrap().modified().unwrap(),
        modified
    );
}

/// A setup app on an x86-64 machine of `os` whose CPU lacks AVX2 and FMA.
fn old_cpu_app(os: &'static str) -> TestApp {
    let mut old = machine(32);
    old.os = os;
    old.arch = "x86_64";
    old.cpu_features = CpuFeatures::X86(X86Features {
        avx2: false,
        fma: false,
        f16c: true,
        bmi2: true,
    });
    let downloads = Downloads::new(
        true,
        Catalog::new(tiered_catalog()),
        Downloader::with_base_url("http://127.0.0.1:9"),
    )
    .with_tiers(tiers())
    .with_machine(old);
    setup_app().map_state(|state| state.with_downloads(downloads))
}

#[tokio::test]
async fn an_old_cpu_on_linux_is_only_a_slow_cpu_warning() {
    let app = old_cpu_app("linux");
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(
        models["download"]["support"],
        json!({"status": "supported", "code": null, "missing_cpu_features": ["avx2", "fma"]}),
        "{models}"
    );
    assert_eq!(models["download"]["warnings"], json!(["slow_cpu"]));
    assert_eq!(models["download"]["selected"], json!(SETUP_GENERATION));
}

#[tokio::test]
async fn an_unsupported_cpu_offers_nothing_and_refuses_a_choice() {
    let app = old_cpu_app("windows");
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(
        models["download"]["support"],
        json!({"status": "unsupported", "code": "cpu_unsupported", "missing_cpu_features": ["avx2", "fma"]}),
        "{models}"
    );
    assert_eq!(models["download"]["selected"], Value::Null);
    let (status, body) = send(&app, choose(SETUP_GENERATION)).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("model_not_eligible")),
        "{body}"
    );
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("cpu_unsupported"), "{message}");
}

/// A setup app whose configured generator (`gen-setup`) is a catalog
/// generator but not a tier: the tiers are `gen-default` and `gen-large`.
async fn untiered_app(total_gib: u64) -> TestApp {
    let mut catalog = tiered_catalog();
    catalog.push(entry("gen-default", ModelRole::Generation));
    let base = file_server(&catalog, open_gate()).await;
    let app = setup_app();
    let config = config_path(&app);
    let downloads = Downloads::new(true, Catalog::new(catalog), Downloader::with_base_url(base))
        .with_tiers(vec![
            tier("gen-default", TierSize::Default, 4),
            tier(LARGE, TierSize::Large, 16),
        ])
        .with_machine(machine(total_gib))
        .with_config_file(config);
    app.map_state(|state| state.with_downloads(downloads))
}

#[tokio::test]
async fn a_configured_generator_that_is_not_a_tier_is_the_current_option() {
    let app = untiered_app(32).await;
    let (_, models) = send(&app, get("/v1/models")).await;
    let download = &models["download"];
    let listed: Vec<(&str, &str, bool)> = download["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["id"].as_str().unwrap(),
                o["verdict"].as_str().unwrap(),
                o["current"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (SETUP_GENERATION, "fits", true),
            ("gen-default", "recommended", false),
            (LARGE, "fits", false)
        ],
        "{download}"
    );
    assert_eq!(
        download["options"][0]["label_key"],
        json!("setup.tier.current")
    );
    assert_eq!(
        download["options"][0]["need_ram_bytes"],
        json!(4 * GIB),
        "the default's need"
    );
    assert_eq!(download["selected"], json!(SETUP_GENERATION));
    // Downloading it changes nothing in config.toml: the key already names it.
    let config = config_path(&app);
    let before = format!("generation_model = \"{SETUP_GENERATION}\"\n");
    std::fs::write(&config, &before).unwrap();
    let (status, body) = send(&app, choose(SETUP_GENERATION)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let done = finished(&app).await;
    assert_eq!(done["state"], json!("done"), "{done}");
    find_model(
        &app.state.models.models_dir,
        SETUP_GENERATION,
        ModelRole::Generation,
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
}

#[tokio::test]
async fn the_large_tier_is_never_recommended() {
    let app = tiered_app(32, false, Some("http://127.0.0.1:9")).await;
    let (_, models) = send(&app, get("/v1/models")).await;
    let verdicts: Vec<&str> = models["download"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["verdict"].as_str().unwrap())
        .collect();
    assert_eq!(verdicts, ["recommended", "fits"], "{models}");
    assert_eq!(models["download"]["selected"], json!(SETUP_GENERATION));
}

#[tokio::test]
async fn an_interrupted_download_on_a_nearly_full_disk_can_be_resumed() {
    // Free space for the embedder, the large generator minus what an
    // earlier run already left, and the 512 MiB headroom: nothing more.
    let large = entry(LARGE, ModelRole::Generation);
    let embedder = entry(SETUP_EMBEDDING, ModelRole::Embedding);
    let left = 1000u64;
    let mut tight = machine(32);
    tight.free_disk_bytes =
        Some(DISK_HEADROOM_BYTES + embedder.total_size() + large.total_size() - left);
    let app = setup_app();
    let part_dir = app.state.models.models_dir.join(".download").join(LARGE);
    let downloads = Downloads::new(
        true,
        Catalog::new(tiered_catalog()),
        Downloader::with_base_url("http://127.0.0.1:9"),
    )
    .with_tiers(tiers())
    .with_machine(tight)
    .with_config_file(config_path(&app));
    let app = app.map_state(|state| state.with_downloads(downloads));
    let verdict = |models: &Value| models["download"]["options"][1]["verdict"].clone();
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(verdict(&models), json!("too_big_disk"), "{models}");
    std::fs::create_dir_all(&part_dir).unwrap();
    std::fs::write(
        part_dir.join("model.gguf.part"),
        &weights(LARGE)[..left as usize],
    )
    .unwrap();
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(verdict(&models), json!("fits"), "{models}");
    assert_eq!(
        models["download"]["options"][1]["download_bytes"],
        json!(large.total_size() - left)
    );
    let (status, body) = send(&app, choose(LARGE)).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the retry is accepted: {body}"
    );
}
