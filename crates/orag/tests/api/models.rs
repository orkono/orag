//! Setup mode (no models installed) and model import from the page (D-023).

use axum::http::StatusCode;
use serde_json::json;

use crate::support::{
    SETUP_EMBEDDING, SETUP_GENERATION, app, drain_jobs, get, json_request, send, setup_app,
    setup_app_over, setup_app_supervised, write_fixture_pack,
};

#[tokio::test]
async fn setup_mode_refuses_work_with_models_missing() {
    let app = setup_app();
    let (status, body) = send(&app, get("/v1/models")).await;
    assert_eq!(
        (status, body["mode"].clone()),
        (StatusCode::OK, json!("setup")),
        "{body}"
    );
    assert_eq!(
        body["embedding"],
        json!({"id": SETUP_EMBEDDING, "installed": false})
    );
    assert_eq!(
        body["generation"],
        json!({"id": SETUP_GENERATION, "installed": false})
    );
    assert!(
        body["models_dir"].as_str().unwrap().ends_with("models"),
        "{body}"
    );
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "x"})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::SERVICE_UNAVAILABLE, json!("models_missing"))
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains(SETUP_EMBEDDING),
        "{body}"
    );
    let (status, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"filename": "a.md", "content": "# A"}),
        ),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::SERVICE_UNAVAILABLE, json!("models_missing"))
    );
    assert_eq!(send(&app, get("/v1/collections")).await.0, StatusCode::OK);
}

#[tokio::test]
async fn setup_mode_never_says_reindex_for_an_indexed_collection() {
    // Collection 1 holds chunks of the real model (indexed before the models
    // folder went missing): query and reindex must be 503, never 409 or 202.
    let indexed = app();
    let (status, _) = send(
        &indexed,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"filename": "a.md", "content": "# İade\n\nİade süresi 14 gündür."}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    drain_jobs(&indexed);
    let app = setup_app_over(indexed, false);
    for (uri, body) in [
        ("/v1/collections/1/query", json!({"query": "iade"})),
        ("/v1/collections/1/reindex", json!({})),
    ] {
        let (status, err) = send(&app, json_request("POST", uri, body)).await;
        assert_eq!(
            (status, err["error"]["code"].clone()),
            (StatusCode::SERVICE_UNAVAILABLE, json!("models_missing")),
            "{uri}: {err}"
        );
    }
    let (_, doc) = send(&app, get("/v1/collections/1/documents/1")).await;
    assert_eq!(
        doc["status"],
        json!("ready"),
        "reindex must not have queued the document"
    );
    assert!(
        doc["chunk_count"].as_u64().unwrap() > 0,
        "reindex must not have dropped chunks"
    );
}

#[tokio::test]
async fn import_from_folder_with_turkish_name() {
    let packs = tempfile::tempdir().unwrap();
    let folder = packs.path().join("ORAG paketleri Ş");
    write_fixture_pack(&folder.join("emb"), "embedding", SETUP_EMBEDDING);
    write_fixture_pack(&folder.join("gen"), "generation", SETUP_GENERATION);

    let app = setup_app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": folder})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut imported: Vec<String> = serde_json::from_value(body["imported"].clone()).unwrap();
    imported.sort();
    assert_eq!(imported, [SETUP_EMBEDDING, SETUP_GENERATION]);
    assert_eq!(body["restart"], json!("manual"), "not supervised");
    assert!(!app.state.shutdown_started());
    let (_, models) = send(&app, get("/v1/models")).await;
    assert_eq!(models["embedding"]["installed"], json!(true), "{models}");

    let supervised = setup_app_supervised();
    let (_, body) = send(
        &supervised,
        json_request("POST", "/v1/models/import", json!({"path": folder})),
    )
    .await;
    assert_eq!(body["restart"], json!("automatic"), "{body}");
    assert!(supervised.state.shutdown_started());
    assert_eq!(supervised.state.exit_code(), Some(75));
}

#[tokio::test]
async fn importing_only_one_of_two_missing_models_keeps_setup_mode() {
    let packs = tempfile::tempdir().unwrap();
    write_fixture_pack(packs.path(), "embedding", SETUP_EMBEDDING);
    let app = setup_app_supervised();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": packs.path()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["restart"],
        json!("none"),
        "the generation model is still missing"
    );
    assert!(!app.state.shutdown_started());
}

#[tokio::test]
async fn import_of_a_bad_folder_is_a_clear_400() {
    let app = setup_app();
    let missing = std::env::temp_dir().join("orag-no-such-folder-ş");
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": missing})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("orag-no-such-folder-ş"), "{message}");
    let empty = tempfile::tempdir().unwrap();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": empty.path()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no model pack"),
        "{body}"
    );
}

#[tokio::test]
async fn ready_mode_reports_installed_models_and_refuses_page_imports() {
    let app = app(); // fake models, ready mode
    let (_, body) = send(&app, get("/v1/models")).await;
    assert_eq!(body["mode"], json!("ready"), "{body}");
    let packs = tempfile::tempdir().unwrap();
    write_fixture_pack(&packs.path().join("extra"), "embedding", "extra-embedder");
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": packs.path()})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("conflict")),
        "{body}"
    );
    assert!(!app.state.shutdown_started());
    assert!(!app.state.models.models_dir.join("extra-embedder").exists());
}

#[tokio::test]
async fn a_relative_path_is_refused() {
    let app = setup_app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": "orag-packs"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("full path"),
        "{body}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_pack_file_that_is_a_symlink_is_refused() {
    let packs = tempfile::tempdir().unwrap();
    let pack = packs.path().join("emb");
    write_fixture_pack(&pack, "embedding", SETUP_EMBEDDING);
    let elsewhere = packs.path().join("elsewhere.gguf");
    std::fs::rename(pack.join("model.gguf"), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, pack.join("model.gguf")).unwrap();
    let app = setup_app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": pack})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("symbolic link"),
        "{body}"
    );
}

#[tokio::test]
async fn a_leftover_folder_of_a_broken_install_is_named_not_skipped() {
    let packs = tempfile::tempdir().unwrap();
    write_fixture_pack(packs.path(), "embedding", SETUP_EMBEDDING);
    let app = setup_app();
    // An interrupted copy left the folder without a manifest.
    std::fs::create_dir_all(app.state.models.models_dir.join(SETUP_EMBEDDING)).unwrap();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/models/import", json!({"path": packs.path()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("not a working install") && message.contains(SETUP_EMBEDDING),
        "{message}"
    );
}
