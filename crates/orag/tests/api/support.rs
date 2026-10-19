//! Shared helpers: a router over a temp SQLite store with fake models.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use orag::infer::fake::{FakeEmbedder, FakeGenerator};
use orag::infer::missing::{MissingEmbedder, MissingGenerator};
use orag::server::{AppState, ModelsInfo, router};
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
    let models = ModelsInfo {
        models_dir: dir.path().join("models"),
        ..(*state.models).clone()
    };
    let state = state.with_models(models, false, false);
    TestApp {
        _dir: dir,
        state,
        generator,
    }
}

impl TestApp {
    /// The same app (store, temp folder) with a changed state.
    pub fn map_state(self, change: impl FnOnce(AppState) -> AppState) -> TestApp {
        TestApp {
            state: change(self.state),
            ..self
        }
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

/// Bytes of a file in `tests/fixtures`.
pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Configured model ids of the setup-mode apps below.
pub const SETUP_EMBEDDING: &str = "emb-setup";
pub const SETUP_GENERATION: &str = "gen-setup";

/// An app in setup mode: no models are installed, `models_dir` is empty.
pub fn setup_app() -> TestApp {
    setup_app_over(app(), false)
}

/// Like `setup_app`, but running under `orag start` (`ORAG_SUPERVISED=1`).
pub fn setup_app_supervised() -> TestApp {
    setup_app_over(app(), true)
}

/// Serves `app`'s store (with whatever it has indexed) in setup mode.
pub fn setup_app_over(app: TestApp, supervised: bool) -> TestApp {
    let models = ModelsInfo {
        models_dir: app._dir.path().join("models"),
        embedding: SETUP_EMBEDDING.into(),
        generation: SETUP_GENERATION.into(),
    };
    let state = AppState::new(
        app.state.store.clone(),
        Arc::new(MissingEmbedder::new(SETUP_EMBEDDING)),
        Arc::new(MissingGenerator::new(SETUP_GENERATION)),
        app.state.max_upload_bytes,
    )
    .with_models(models, true, supervised);
    TestApp {
        _dir: app._dir,
        state,
        generator: app.generator,
    }
}

/// A valid model pack at `dir` (tiny fake weights, SHA-256 matching).
pub fn write_fixture_pack(dir: &std::path::Path, role: &str, id: &str) {
    use sha2::Digest;
    std::fs::create_dir_all(dir).unwrap();
    let payload = format!("weights of {id}");
    std::fs::write(dir.join("model.gguf"), &payload).unwrap();
    std::fs::write(dir.join("LICENSE"), "Apache-2.0 text").unwrap();
    let sha = hex::encode(sha2::Sha256::digest(payload.as_bytes()));
    let section = if role == "embedding" {
        "[embedding]\ndimensions = 16\npooling = \"mean\"\nmax_tokens = 128\n"
    } else {
        "[generation]\ncontext_tokens = 2048\nmax_output_tokens = 256\n"
    };
    std::fs::write(
        dir.join("orag-model.toml"),
        format!(
            "id = \"{id}\"\nrole = \"{role}\"\nfile = \"model.gguf\"\nsha256 = \"{sha}\"\n\
             license = \"Apache-2.0\"\nlicense_file = \"LICENSE\"\nsource = \"test\"\nrevision = \"r1\"\n\n{section}"
        ),
    )
    .unwrap();
}
