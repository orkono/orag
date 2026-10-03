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

use orag::domain::chunker::ChunkerConfig;
use orag::ingest::worker::{IngestContext, run_once};

/// Indexes one document into `collection_id` with the fake embedder under
/// another model id, so the collection holds chunks of a previous model.
pub fn index_with_previous_model(app: &TestApp, collection_id: i64) {
    use orag::domain::space::SpaceDescriptor;
    use orag::error::Result;
    use orag::infer::Embedder;
    use orag::ingest::format::SourceFormat;
    use orag::store::documents::NewDocument;

    struct Previous(FakeEmbedder, SpaceDescriptor);
    impl Embedder for Previous {
        fn descriptor(&self) -> &SpaceDescriptor {
            &self.1
        }
        fn count_tokens(&self, text: &str) -> usize {
            self.0.count_tokens(text)
        }
        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0.embed_documents(texts)
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            self.0.embed_query(text)
        }
    }

    let mut other = FakeEmbedder::new().descriptor().clone();
    other.model_id = "previous-model".into();
    let old = IngestContext {
        store: app.state.store.clone(),
        embedder: Arc::new(Previous(FakeEmbedder::new(), other)),
        chunker: ChunkerConfig::default(),
    };
    app.state
        .store
        .enqueue_document(
            collection_id,
            NewDocument {
                filename: Some("a.md".into()),
                format: SourceFormat::Markdown,
                bytes: b"# A\n\nmetin".to_vec(),
            },
        )
        .unwrap();
    assert!(run_once(&old).unwrap());
}

pub fn drain_jobs(app: &TestApp) {
    let ctx = IngestContext {
        store: app.state.store.clone(),
        embedder: app.state.engine.retriever.embedder.clone(),
        chunker: ChunkerConfig::default(),
    };
    while run_once(&ctx).unwrap() {}
}

fn multipart(filename: &str, content_type: &str, body: &[u8]) -> Request<Body> {
    let boundary = "orag-test-boundary";
    let mut payload = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes();
    payload.extend_from_slice(body);
    payload.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    local(Request::post("/v1/collections/1/documents"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(payload))
        .unwrap()
}

#[tokio::test]
async fn collection_lifecycle() {
    let app = app();
    let (status, created) = send(
        &app,
        json_request("POST", "/v1/collections", json!({"name": "Hukuk"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "Hukuk");
    assert_eq!(
        send(
            &app,
            json_request("POST", "/v1/collections", json!({"name": "Hukuk"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/collections", json!({"name": "a/b"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_input");
    let (_, list) = send(&app, get("/v1/collections")).await;
    assert_eq!(list["collections"].as_array().unwrap().len(), 2);
    let id = created["id"].as_i64().unwrap();
    let delete = local(Request::delete(format!("/v1/collections/{id}")))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, delete).await.0, StatusCode::NO_CONTENT);
    let protect = local(Request::delete("/v1/collections/1"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, protect).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn json_upload_rejects_unsupported_document_types_but_accepts_dotted_names() {
    let app = app();
    for name in ["rapor.xlsx", "page.html"] {
        let (status, body) = send(
            &app,
            json_request(
                "POST",
                "/v1/collections/1/documents",
                json!({"filename": name, "content": "x"}),
            ),
        )
        .await;
        assert_eq!(
            (status, body["error"]["code"].clone()),
            (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                json!("unsupported_format")
            ),
            "{name}"
        );
    }
    for (i, name) in ["Toplantı 12.10.2026", "notes.v2", "Q3.final"]
        .iter()
        .enumerate()
    {
        let body = json!({"filename": name, "content": format!("dotted name {i}")});
        let (status, _) = send(
            &app,
            json_request("POST", "/v1/collections/1/documents", body),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::ACCEPTED,
            "{name} is plain text, not an unsupported file"
        );
    }
}

#[tokio::test]
async fn unknown_json_fields_are_rejected() {
    let app = app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/collections", json!({"name": "x", "extra": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[tokio::test]
async fn json_upload_is_queued_indexed_listed_and_deleted() {
    let app = app();
    let doc = json!({"filename": "iade.md", "content": "# İade\n\n14 gün içinde iade."});
    let (status, queued) = send(
        &app,
        json_request("POST", "/v1/collections/1/documents", doc.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(queued["duplicate"], false);
    let (status, again) = send(
        &app,
        json_request("POST", "/v1/collections/1/documents", doc),
    )
    .await;
    assert_eq!(
        (status, again["duplicate"].clone()),
        (StatusCode::OK, json!(true))
    );
    let (_, job) = send(&app, get(&format!("/v1/jobs/{}", queued["job_id"]))).await;
    assert_eq!(job["status"], "queued");
    drain_jobs(&app);
    let doc_uri = format!("/v1/collections/1/documents/{}", queued["document_id"]);
    let (_, record) = send(&app, get(&doc_uri)).await;
    assert_eq!(record["status"], "ready");
    assert_eq!(record["title"], "İade");
    let (_, list) = send(&app, get("/v1/collections/1/documents?limit=10")).await;
    assert_eq!(list["documents"].as_array().unwrap().len(), 1);
    assert_eq!(list["next_after_id"], Value::Null);
    let delete = local(Request::delete(&doc_uri))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, delete).await.0, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, get(&doc_uri)).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn multipart_upload_detects_format() {
    let app = app();
    let (status, queued) = send(
        &app,
        multipart("notes.md", "text/markdown", "# Not\n\nMetin".as_bytes()),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    let (status, body) = send(
        &app,
        multipart(
            "sheet.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            b"PK",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"]["code"], "unsupported_format");
}

#[tokio::test]
async fn invalid_text_fails_the_job_visibly() {
    let app = app();
    let (_, queued) = send(
        &app,
        multipart("bad.txt", "text/plain", &[0xFF, 0xFE, b'a', 0]),
    )
    .await;
    drain_jobs(&app);
    let (_, record) = send(
        &app,
        get(&format!(
            "/v1/collections/1/documents/{}",
            queued["document_id"]
        )),
    )
    .await;
    assert_eq!(record["status"], "failed");
    assert!(record["error"].as_str().unwrap().contains("UTF-8"));
}

#[tokio::test]
async fn oversized_upload_is_rejected_with_413() {
    let app = app_with(FakeGenerator::new("x"), 1024);
    // Just over the limit (explicit check) and far over it (extractor body limit): same error.
    for size in [2 * 1024, 2 * 1024 * 1024] {
        let (status, body) =
            send(&app, multipart("big.txt", "text/plain", &vec![b'a'; size])).await;
        assert_eq!(
            (status, body["error"]["code"].clone()),
            (StatusCode::PAYLOAD_TOO_LARGE, json!("too_large")),
            "{size}"
        );
    }
    let (_, list) = send(&app, get("/v1/collections/1/documents")).await;
    assert_eq!(list["documents"], json!([]), "nothing is stored");
}

#[tokio::test]
async fn concurrent_upload_limit_returns_429() {
    let app = app();
    let held: Vec<_> = (0..orag::server::MAX_CONCURRENT_UPLOADS)
        .map(|_| app.state.uploads.clone().try_acquire_owned().unwrap())
        .collect();
    let (status, headers, body) = send_full(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"content": "x"}),
        ),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::TOO_MANY_REQUESTS, json!("busy"))
    );
    assert_eq!(
        headers[header::RETRY_AFTER],
        "5",
        "429 tells clients when to retry"
    );
    drop(held);
    let (status, _) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"content": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn only_the_upload_route_accepts_large_bodies() {
    let app = app();
    let name = "x".repeat(orag::server::SMALL_BODY_LIMIT);
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections", json!({"name": name})),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let content = "y".repeat(orag::server::SMALL_BODY_LIMIT * 2);
    let (status, _) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/documents",
            json!({"content": content}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test(start_paused = true)]
async fn stalled_upload_times_out_and_frees_its_permit() {
    let app = app();
    // A body that never arrives; paused time jumps straight to the timeout.
    let stalled = Body::from_stream(tokio_stream::pending::<
        Result<axum::body::Bytes, std::io::Error>,
    >());
    let request = local(Request::post("/v1/collections/1/documents"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(stalled)
        .unwrap();
    let (status, body) = send(&app, request).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::REQUEST_TIMEOUT, json!("upload_timeout"))
    );
    assert_eq!(
        app.state.uploads.available_permits(),
        orag::server::MAX_CONCURRENT_UPLOADS
    );
}

#[tokio::test]
async fn shutdown_ends_an_upload_in_transit_with_503() {
    let app = app();
    let stalled = Body::from_stream(tokio_stream::pending::<
        Result<axum::body::Bytes, std::io::Error>,
    >());
    let request = local(Request::post("/v1/collections/1/documents"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(stalled)
        .unwrap();
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    app.state.begin_shutdown();
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .expect("not the 60 s deadline")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        app.state.uploads.available_permits(),
        orag::server::MAX_CONCURRENT_UPLOADS
    );
}

#[tokio::test]
async fn explicit_format_cannot_bypass_filename_rules() {
    let app = app();
    let cases = [
        (
            json!({"filename": "rapor.xlsx", "format": "text", "content": "x"}),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            json!({"filename": "a.md", "format": "text", "content": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"filename": "a.md", "format": "markdown", "content": "# ok"}),
            StatusCode::ACCEPTED,
        ),
        (
            json!({"filename": "Toplantı 12.10.2026", "format": "markdown", "content": "# ok 2"}),
            StatusCode::ACCEPTED,
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
async fn escaped_json_under_the_limit_is_accepted() {
    let app = app_with(FakeGenerator::new("x"), 1024 * 1024);
    // 300k × "ş" = 600 KB of text, but ~1.8 MB of `\u015f` escapes on the wire.
    let body = format!(
        r#"{{"filename":"t.txt","content":"{}"}}"#,
        "\\u015f".repeat(300_000)
    );
    let request = local(Request::post("/v1/collections/1/documents"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    assert_eq!(send(&app, request).await.0, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn documents_of_unknown_collection_are_404() {
    let app = app();
    assert_eq!(
        send(&app, get("/v1/collections/99/documents")).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/99/documents",
            json!({"content": "x"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_into_incompatible_collection_is_refused_up_front() {
    let app = app();
    let collection = app.state.store.create_collection("old").unwrap();
    index_with_previous_model(&app, collection.id);
    let uri = format!("/v1/collections/{}/documents", collection.id);
    let (status, body) = send(&app, json_request("POST", &uri, json!({"content": "x"}))).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("reindex_required"))
    );
}

#[tokio::test]
async fn pagination_reports_next_cursor() {
    let app = app();
    for i in 0..3 {
        send(
            &app,
            json_request(
                "POST",
                "/v1/collections/1/documents",
                json!({"content": format!("doc {i}")}),
            ),
        )
        .await;
    }
    let (_, page) = send(&app, get("/v1/collections/1/documents?limit=2")).await;
    assert_eq!(page["documents"].as_array().unwrap().len(), 2);
    let next = page["next_after_id"].as_i64().unwrap();
    let (_, rest) = send(
        &app,
        get(&format!(
            "/v1/collections/1/documents?limit=2&after_id={next}"
        )),
    )
    .await;
    assert_eq!(rest["documents"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn filename_rules_hold_for_padded_names_and_dotfiles() {
    let app = app();
    for name in ["rapor.xlsx ", " .html", ".json", "a.pdf\t"] {
        let body = json!({"filename": name, "content": "x"});
        let (status, _) = send(
            &app,
            json_request("POST", "/v1/collections/1/documents", body),
        )
        .await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{name:?}");
    }
    let contradicts = json!({"filename": "a.md ", "format": "text", "content": "x"});
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections/1/documents", contradicts),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(&app, multipart("rapor.xlsx ", "text/plain", b"x")).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn bad_path_and_query_values_are_json_errors() {
    let app = app();
    for uri in [
        "/v1/collections/abc/documents",
        "/v1/collections/1/documents?limit=abc",
        "/v1/jobs/foo",
    ] {
        let (status, body) = send(&app, get(uri)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"]["code"], "invalid_input", "{uri}");
    }
}

#[tokio::test]
async fn an_unsupported_json_media_type_is_reported_as_such() {
    let app = app();
    let request = local(Request::post("/v1/collections/1/documents"))
        .header(header::CONTENT_TYPE, "application/jsonl")
        .body(Body::from(r#"{"content":"x"}"#))
        .unwrap();
    let (status, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"]["code"], "unsupported_media_type");
}

#[tokio::test]
async fn a_multipart_file_over_the_limit_is_413_without_reading_it_all() {
    let app = app_with(FakeGenerator::new("x"), 1024);
    // Within the route's body limit (6x), over the document limit.
    let (status, body) = send(&app, multipart("big.txt", "text/plain", &vec![b'a'; 4096])).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "too_large");
}

use std::time::Duration;

async fn seed(app: &TestApp) {
    let doc = json!({"filename": "iade.md", "content": "# Kargo\n\n## İade\n\nÜrünler 14 gün içinde iade edilebilir."});
    let (status, body) = send(
        app,
        json_request("POST", "/v1/collections/1/documents", doc),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    drain_jobs(app);
}

/// Words in the slow fake reply; the generator is allowed to emit all of them,
/// so only cancellation can keep the count below it.
const LONG_REPLY_WORDS: usize = 400;

/// About 0.8 s for a full answer, so an uncancelled run is easy to observe.
fn slow_long_generator() -> FakeGenerator {
    FakeGenerator::new(&"kelime ".repeat(LONG_REPLY_WORDS))
        .with_context(4096, LONG_REPLY_WORDS)
        .with_token_delay(Duration::from_millis(2))
}

/// Generation has stopped: the token count settles below a full answer and
/// the single generation permit is free again.
async fn assert_generation_stopped(app: &TestApp, why: &str) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let settled = app.generator.emitted_tokens();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(app.generator.emitted_tokens(), settled, "{why}");
    assert!(settled < LONG_REPLY_WORDS, "{why}");
    assert!(
        app.state.generation.clone().try_acquire_owned().is_ok(),
        "the generation permit was not released"
    );
}

#[tokio::test]
async fn query_returns_grounded_answer_json() {
    let app = app_with(FakeGenerator::new("14 gün [1]."), 1024 * 1024);
    seed(&app).await;
    let (status, body) = send(
        &app,
        json_request(
            "POST",
            "/v1/collections/1/query",
            json!({"query": "İade süresi kaç gün?"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["answer"], "14 gün [1].");
    assert_eq!(body["citations"], json!([1]));
    assert_eq!(body["sources"][0]["number"], 1);
    assert_eq!(body["trace"]["retrieval"]["strategy"], "hybrid");
    assert_eq!(body["abstained"], false);
}

#[tokio::test]
async fn query_streams_sources_tokens_done_in_order() {
    let app = app_with(FakeGenerator::new("14 gün [1]."), 1024 * 1024);
    seed(&app).await;
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    let sources = text.find("event: sources").expect("sources event");
    let token = text.find("event: token").expect("token event");
    let done = text.find("event: done").expect("done event");
    assert!(sources < token && token < done, "{text}");
}

#[tokio::test]
async fn query_validation_and_missing_collection() {
    let app = app();
    let (status, body) = send(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "  "})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::BAD_REQUEST, json!("invalid_input"))
    );
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections/99/query", json!({"query": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn embedding_model_change_requires_reindex() {
    let app = app();
    // A collection indexed by a previous embedding model.
    let new_collection = app.state.store.create_collection("x").unwrap();
    index_with_previous_model(&app, new_collection.id);
    let uri = format!("/v1/collections/{}/query", new_collection.id);
    let (status, body) = send(&app, json_request("POST", &uri, json!({"query": "iade"}))).await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("reindex_required"))
    );
}

#[tokio::test]
async fn dropping_sse_stream_stops_generation() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    let mut body = response.into_body();
    let _first = body.frame().await.expect("first frame");
    drop(body);
    assert_generation_stopped(&app, "generation kept running after disconnect").await;
    // The generation permit was released: a new query completes.
    let follow_up = tokio::time::timeout(
        Duration::from_secs(5),
        send(
            &app,
            json_request("POST", "/v1/collections/1/query", json!({"query": "iade"})),
        ),
    )
    .await
    .expect("permit released");
    assert_eq!(follow_up.0, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn query_wait_for_the_generation_slot_is_bounded() {
    let app = app();
    let _held = app.state.generation.clone().try_acquire_owned().unwrap();
    // Paused time jumps straight to GENERATION_WAIT; the collection is empty, so no seeding.
    let started = tokio::time::Instant::now();
    let (status, headers, body) = send_full(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "iade"})),
    )
    .await;
    assert_eq!(
        (status, body["error"]["code"].clone()),
        (StatusCode::TOO_MANY_REQUESTS, json!("busy"))
    );
    assert!(
        started.elapsed() >= orag::server::query::GENERATION_WAIT,
        "the query must wait before giving up"
    );
    assert_eq!(headers[header::RETRY_AFTER], "5");
}

#[tokio::test]
async fn query_queued_through_shutdown_does_not_start() {
    let long_reply = "kelime ".repeat(400);
    let app = app_with(FakeGenerator::new(&long_reply), 1024 * 1024);
    seed(&app).await; // retrieval finds chunks, so a query that ran would generate tokens
    let held = app.state.generation.clone().try_acquire_owned().unwrap();
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    // Queued on the slot or still in its pre-checks: both observe the same
    // channel (`wait_for` sees an earlier shutdown too), so the result is 503.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !pending.is_finished(),
        "the query should be waiting for the generation slot"
    );
    app.state.begin_shutdown();
    // Woken by shutdown, not by the slot: `held` is still taken here.
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("shutdown wakes the waiter")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(held);
    // With the slot free, a query arriving during shutdown still starts no generation.
    let (status, _) = send(
        &app,
        json_request("POST", "/v1/collections/1/query", json!({"query": "iade"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        app.generator.emitted_tokens(),
        0,
        "no generation may start during shutdown"
    );
}

/// Waits (bounded) until the fake generator has emitted a token, so a test can
/// start shutdown mid-generation without depending on machine speed.
async fn wait_until_generating(app: &TestApp) {
    for _ in 0..1000 {
        if app.generator.emitted_tokens() > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("generation never started");
}

#[tokio::test]
async fn shutdown_during_generation_never_returns_a_partial_json_answer() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    wait_until_generating(&app).await;
    app.state.begin_shutdown();
    let response = pending.await.unwrap().unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a truncated answer must not be a 200"
    );
    assert!(app.generator.emitted_tokens() > 0);
    assert_generation_stopped(&app, "generation kept running after shutdown").await;
}

#[tokio::test]
async fn shutdown_during_sse_ends_with_an_error_event() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let (state, generator) = (app.state.clone(), app.generator.clone());
    tokio::spawn(async move {
        // >= 2: the counter rises just before a token reaches the stream, so wait
        // for a second one to be sure the first was delivered.
        while generator.emitted_tokens() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        state.begin_shutdown();
    });
    let request = json_request(
        "POST",
        "/v1/collections/1/query",
        json!({"query": "iade", "stream": true}),
    );
    let response = router(app.state.clone()).oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: token"), "{text}");
    assert!(
        text.trim_end().ends_with("}")
            && text.contains("event: error")
            && text.contains("shutting_down"),
        "{text}"
    );
    assert!(!text.contains("event: done"), "{text}");
    assert_eq!(text.matches("event: error").count(), 1, "{text}");
}

#[tokio::test]
async fn abandoned_json_query_stops_generation() {
    let app = app_with(slow_long_generator(), 1024 * 1024);
    seed(&app).await;
    let request = json_request("POST", "/v1/collections/1/query", json!({"query": "iade"}));
    let abandoned = tokio::time::timeout(Duration::from_millis(100), send(&app, request)).await;
    assert!(
        abandoned.is_err(),
        "the request should still be running when abandoned"
    );
    assert_generation_stopped(&app, "generation kept running for a disconnected client").await;
}

#[tokio::test]
async fn a_collection_deleted_while_queued_fails_the_stream_with_a_status() {
    let app = app();
    let collection = app.state.store.create_collection("gecici").unwrap();
    let held = app.state.generation.clone().try_acquire_owned().unwrap();
    let request = json_request(
        "POST",
        &format!("/v1/collections/{}/query", collection.id),
        json!({"query": "iade", "stream": true}),
    );
    let pending = tokio::spawn(router(app.state.clone()).oneshot(request));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!pending.is_finished(), "the query should be queued");
    app.state.store.delete_collection(collection.id).unwrap();
    drop(held);
    let response = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("the queued query runs")
        .unwrap()
        .unwrap();
    // Checked again after the wait: a 404, not a 200 stream with an error event.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
