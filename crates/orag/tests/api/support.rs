//! Shared helpers: a router over a temp SQLite store with fake models.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use orag::infer::fake::{FakeEmbedder, FakeGenerator};
use orag::server::{AppState, router};
use orag::store::Store;
use serde_json::Value;
use tower::ServiceExt;

use orag::domain::chunker::ChunkerConfig;
use orag::ingest::worker::{IngestContext, run_once};

pub struct TestApp {
    _dir: tempfile::TempDir,
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

pub fn multipart(filename: &str, content_type: &str, body: &[u8]) -> Request<Body> {
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
