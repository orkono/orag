//! Process wiring for `orag serve`: config → store → models → worker → HTTP.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::domain::chunker::ChunkerConfig;
use crate::infer::fake::{FakeEmbedder, FakeGenerator};
use crate::infer::{Embedder, Generator};
use crate::ingest::worker::{IngestContext, spawn_worker};
use crate::server::{AppState, serve};
use crate::store::Store;

pub const LISTENING_PREFIX: &str = "orag listening on ";
/// After the HTTP drain, how long the worker and blocking tasks (an embedding
/// batch, a generation inside the model) get before the process exits anyway.
const TASK_STOP_LIMIT: Duration = Duration::from_secs(5);
const DEV_REPLY: &str = "This is a development answer from fake models [1].";

#[derive(Debug, Clone, Copy, Default)]
pub struct ServeOptions {
    pub dev_fake_models: bool,
}

pub fn run_server(config: Config, options: ServeOptions) -> anyhow::Result<()> {
    init_logging(&config.log_level);
    tracing::info!(config = %config.config_path().display(), settings = %config.effective(), "configuration loaded (changes need a restart)");
    // Exclusive ownership of the data directory for the whole process lifetime:
    // job recovery and the worker must never run twice against one database.
    let _instance_lock = acquire_instance_lock(&config.home)?;
    crate::ingest::isolate::enable(
        crate::ingest::isolate::self_executable().context("locating the orag executable")?,
    );
    // Bound before the (slow) model load, so a busy port is reported at once.
    let listener = std::net::TcpListener::bind(config.bind)
        .with_context(|| format!("binding {}", config.bind))?;
    listener.set_nonblocking(true)?;
    let store = Arc::new(Store::open(&config.db_path()).context("opening database")?);
    let requeued = store.requeue_running_jobs()?;
    if requeued > 0 {
        tracing::info!(requeued, "resuming interrupted ingestion jobs");
    }
    let (embedder, generator) = if options.dev_fake_models {
        tracing::warn!("serving with fake models (--dev-fake-models): answers are not real");
        (
            Arc::new(FakeEmbedder::new()) as Arc<dyn Embedder>,
            Arc::new(FakeGenerator::new(DEV_REPLY)) as Arc<dyn Generator>,
        )
    } else {
        load_models(&config)?
    };
    // Only weak handles stay here: once serving ends, a live model means a
    // detached blocking task (an embedding batch, a prefill) still uses it.
    let models_in_use = ModelsInUse::watch(&embedder, &generator);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let served = runtime.block_on(async move {
        let listener = TcpListener::from_std(listener)?;
        let local_addr = listener.local_addr()?;
        let mut state = AppState::new(store.clone(), embedder.clone(), generator, config.max_document_bytes());
        let mut effective = config.effective();
        effective["bind"] = serde_json::json!(local_addr.to_string()); // the real port when bind uses :0
        state.effective_config = Arc::new(effective);
        let ingest = Arc::new(IngestContext { store, embedder, chunker: ChunkerConfig::default() });
        ingest.check()?;
        // Registered before the listening line: a signal sent as soon as the
        // line appears is a graceful shutdown, not the default kill.
        // A running parser child stops through the worker's own shutdown
        // signal (AppState), which `serve` starts when this future completes.
        let shutdown = shutdown_signal()?;
        let worker = spawn_worker(ingest, state.ingest_wake.clone(), state.subscribe_shutdown());
        crate::cli::print_stdout(&format!("{LISTENING_PREFIX}http://{local_addr}"))?;
        serve(state, listener, shutdown).await?;
        if tokio::time::timeout(TASK_STOP_LIMIT, worker).await.is_err() {
            tracing::warn!("the ingest worker did not stop within {TASK_STOP_LIMIT:?}; its job resumes on the next start");
        }
        anyhow::Ok(())
    });
    // Never wait without bound for a blocking task: an unfinished job is
    // requeued on the next start, and the instance lock is released on exit.
    runtime.shutdown_timeout(TASK_STOP_LIMIT);
    if models_in_use.any() {
        tracing::warn!(
            "a model is still in use by a task that did not stop; exiting without native teardown"
        );
        crate::exit::exit_without_native_teardown(if served.is_ok() { 0 } else { 1 });
    }
    served
}

/// Weak handles to the loaded models: alive only while something still uses them.
struct ModelsInUse(
    std::sync::Weak<dyn Embedder>,
    std::sync::Weak<dyn Generator>,
);

impl ModelsInUse {
    fn watch(embedder: &Arc<dyn Embedder>, generator: &Arc<dyn Generator>) -> Self {
        ModelsInUse(Arc::downgrade(embedder), Arc::downgrade(generator))
    }

    fn any(&self) -> bool {
        self.0.strong_count() > 0 || self.1.strong_count() > 0
    }
}

fn acquire_instance_lock(home: &std::path::Path) -> anyhow::Result<std::fs::File> {
    let path = home.join("orag.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            anyhow::bail!(
                "another orag instance is already serving {}",
                home.display()
            )
        }
        Err(std::fs::TryLockError::Error(err)) => {
            Err(err).with_context(|| format!("locking {}", path.display()))
        }
    }
}

fn init_logging(level: &str) {
    let filter = tracing_subscriber::EnvFilter::new(level);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Installs the SIGINT/SIGTERM handlers now and returns the future that
/// completes on the first signal. A handler that cannot be installed stops
/// startup: a server that cannot be stopped gracefully must not start.
#[cfg(unix)]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt()).context("installing the SIGINT handler")?;
    let mut terminate =
        signal(SignalKind::terminate()).context("installing the SIGTERM handler")?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
        tracing::info!("shutting down");
    })
}

#[cfg(not(unix))]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
    Ok(async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            // Not a shutdown request: keep serving rather than exit at once.
            tracing::error!(error = %err, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
        tracing::info!("shutting down");
    })
}

fn load_models(config: &Config) -> anyhow::Result<(Arc<dyn Embedder>, Arc<dyn Generator>)> {
    // Both models must be installed before either (slow) load starts.
    #[cfg(feature = "llama")]
    crate::infer::models::find_model(
        &config.models_dir(),
        &config.generation_model,
        crate::infer::models::ModelRole::Generation,
    )?;
    let embedder = load_embedder(config)?;
    let generator = load_generator(config)?;
    Ok((embedder, generator))
}

/// The embedder `serve` runs; `orag eval retrieval` loads the same one.
#[cfg(feature = "llama")]
pub(crate) fn load_embedder(config: &Config) -> anyhow::Result<Arc<dyn Embedder>> {
    use crate::infer::llama::embedder::LlamaEmbedder;
    use crate::infer::models::{ModelRole, find_model};
    let installed = find_model(
        &config.models_dir(),
        &config.embedding_model,
        ModelRole::Embedding,
    )?;
    tracing::info!(embedding = %installed.manifest.id, "loading embedding model");
    Ok(Arc::new(
        LlamaEmbedder::load(&installed).context("loading embedding model")?,
    ))
}

#[cfg(feature = "llama")]
pub(crate) fn load_generator(config: &Config) -> anyhow::Result<Arc<dyn Generator>> {
    use crate::infer::llama::generator::LlamaGenerator;
    use crate::infer::models::{ModelRole, find_model};
    let installed = find_model(
        &config.models_dir(),
        &config.generation_model,
        ModelRole::Generation,
    )?;
    tracing::info!(generation = %installed.manifest.id, "loading generation model");
    Ok(Arc::new(
        LlamaGenerator::load(&installed).context("loading generation model")?,
    ))
}

#[cfg(not(feature = "llama"))]
pub(crate) fn load_embedder(_config: &Config) -> anyhow::Result<Arc<dyn Embedder>> {
    no_backend()
}

#[cfg(not(feature = "llama"))]
pub(crate) fn load_generator(_config: &Config) -> anyhow::Result<Arc<dyn Generator>> {
    no_backend()
}

#[cfg(not(feature = "llama"))]
fn no_backend<T>() -> anyhow::Result<T> {
    anyhow::bail!(
        "this build has no inference backend; rebuild with the `llama` feature or use --dev-fake-models"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_in_use_sees_only_handles_held_elsewhere() {
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
        let generator: Arc<dyn Generator> = Arc::new(FakeGenerator::new("x"));
        let watched = ModelsInUse::watch(&embedder, &generator);
        let detached_task = Arc::clone(&generator);
        drop((embedder, generator));
        assert!(watched.any(), "a task still holds the generator");
        drop(detached_task);
        assert!(!watched.any());
    }
}
