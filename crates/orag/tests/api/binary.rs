//! DOCX and PDF uploads: indexing, signature checks and the multipart-only rule.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::support::{TestApp, app, drain_jobs, get, json_request, multipart, send};

#[tokio::test]
async fn docx_upload_is_indexed_and_searchable() {
    let app = app();
    let record = upload_and_index(
        &app,
        "politika.docx",
        "application/octet-stream",
        &fixture("sample.docx"),
    )
    .await;
    assert_eq!(record["status"], "ready", "{record}");
    assert_eq!(record["format"], "docx");
    assert_eq!(record["title"], "Kargo Politikası");
    let (_, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/query",
            json!({"query": "iade kac gun"}),
        ),
    )
    .await;
    assert!(
        body["sources"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("14 gün"),
        "{body}"
    );
}

#[tokio::test]
async fn pdf_upload_is_indexed_with_turkish_text() {
    let app = app();
    let record = upload_and_index(
        &app,
        "politika.pdf",
        "application/pdf",
        &fixture("sample-tr.pdf"),
    )
    .await;
    assert_eq!(record["status"], "ready", "{record}");
    assert_eq!(record["warnings"], json!([]));
    let (_, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/query",
            json!({"query": "İade süresi"}),
        ),
    )
    .await;
    assert!(
        body["sources"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("İade süresi 14 gündür"),
        "{body}"
    );
}

#[tokio::test]
async fn mislabeled_binary_uploads_are_rejected_before_storage() {
    let app = app();
    let (status, body) = send(
        &app,
        multipart("fake.pdf", "application/pdf", b"<html>hi</html>"),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("invalid_input"))
    );
    let (status, _) = send(
        &app,
        multipart("fake.docx", "application/octet-stream", b"plain text"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, list) = send(&app, get("/v1/collections/1/documents")).await;
    assert_eq!(list["documents"], json!([]));
}

#[tokio::test]
async fn binary_formats_cannot_be_sent_as_json_text() {
    let app = app();
    let cases = [
        (
            json!({"filename": "a.pdf", "format": "text", "content": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"filename": "a.pdf", "content": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"filename": "a.docx", "content": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"format": "pdf", "content": "x"}),
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (body, expected) in cases {
        let (status, response) = send(
            &app,
            json_request("POST", "/v1/collections/1/documents", body.clone()),
        )
        .await;
        assert_eq!(status, expected, "{body} -> {response}");
    }
}

#[tokio::test]
async fn json_binary_rejection_explains_multipart() {
    let app = app();
    let (status, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"filename": "a.pdf", "content": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("multipart"),
        "{body}"
    );
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

async fn upload_and_index(app: &TestApp, name: &str, mime: &str, bytes: &[u8]) -> Value {
    let (status, queued) = send(app, multipart(name, mime, bytes)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    drain_jobs(app);
    send(
        app,
        get(&format!(
            "/v1/collections/1/documents/{}",
            queued["document_id"]
        )),
    )
    .await
    .1
}
