//! Process wiring for `orag serve`: config → store → models → worker → HTTP.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::domain::chunker::ChunkerConfig;
use crate::infer::fake::{FakeEmbedder, FakeGenerator};
use crate::infer::missing::{MissingEmbedder, MissingGenerator};
use crate::infer::models::{ModelRole, find_model};
use crate::infer::{Embedder, Generator};
use crate::ingest::worker::{IngestContext, spawn_worker};
use crate::server::download::Downloads;
use crate::server::{AppState, ModelsInfo, serve_with_ui};
use crate::store::Store;

pub const LISTENING_PREFIX: &str = "orag listening on ";
/// The second stdout line, printed only when the web UI is on (D-021).
pub const UI_PREFIX: &str = "orag ui on ";
/// After the HTTP drain, how long the worker and blocking tasks (an embedding
/// batch, a generation inside the model) get before the process exits anyway.
const TASK_STOP_LIMIT: Duration = Duration::from_secs(5);
const DEV_REPLY: &str = "This is a development answer from fake models [1].";
/// Set by `orag start` in the environment of the `serve` it supervises, so an
/// import that ends setup mode can restart the process (D-023).
pub const SUPERVISED_ENV: &str = "ORAG_SUPERVISED";

#[derive(Debug, Clone, Copy, Default)]
pub struct ServeOptions {
    pub dev_fake_models: bool,
    /// Stop gracefully when stdin reaches end of file (D-016 sidecar).
    pub exit_on_stdin_close: bool,
    /// The port to try first when `ui_bind` asks for a free one (port 0):
    /// the desktop app's last page port, so its origin stays the same.
    pub ui_port_hint: Option<u16>,
}

/// Runs `orag serve`; returns the process exit code (`EXIT_RESTART` when an
/// import asked `orag start` to run it again).
pub fn run_server(config: Config, options: ServeOptions) -> anyhow::Result<u8> {
    init_logging(&config.log_level);
    tracing::info!(config = %config.config_path().display(), settings = %config.effective(), "configuration loaded (changes need a restart)");
    // Watched from the start, so a parent that dies during the (slow) model
    // load still stops this process once loading ends.
    let stdin_closed = options
        .exit_on_stdin_close
        .then(watch_stdin_close)
        .transpose()?;
    // Exclusive ownership of the data directory for the whole process lifetime:
    // job recovery and the worker must never run twice against one database.
    let supervised = std::env::var(SUPERVISED_ENV).is_ok_and(|v| v == "1");
    let lock_wait = if supervised {
        SUPERVISED_LOCK_WAIT
    } else {
        Duration::ZERO
    };
    let _instance_lock = acquire_instance_lock(&config.home, lock_wait)?;
    crate::ingest::isolate::enable(
        crate::ingest::isolate::self_executable().context("locating the orag executable")?,
    );
    // Bound before the (slow) model load, so a busy port is reported at once.
    let listener = bind_listener(config.bind, || format!("binding {}", config.bind))?;
    // Bound before the model load too, so a busy UI port fails at once.
    let ui_listener = config
        .ui_bind
        .map(|addr| bind_ui_listener(addr, options.ui_port_hint))
        .transpose()?;
    let store = Arc::new(Store::open(&config.db_path()).context("opening database")?);
    let requeued = store.requeue_running_jobs()?;
    if requeued > 0 {
        tracing::info!(requeued, "resuming interrupted ingestion jobs");
    }
    // A build without an inference backend fails at the load below with its
    // own message: setup mode would only lead to importing packs it cannot use.
    let missing = if options.dev_fake_models || cfg!(not(feature = "llama")) {
        Vec::new()
    } else {
        missing_models(&config)?
    };
    let setup = !missing.is_empty();
    let (embedder, generator) = if options.dev_fake_models {
        tracing::warn!("serving with fake models (--dev-fake-models): answers are not real");
        (
            Arc::new(FakeEmbedder::new()) as Arc<dyn Embedder>,
            Arc::new(FakeGenerator::new(DEV_REPLY)) as Arc<dyn Generator>,
        )
    } else if setup {
        tracing::warn!(
            "starting in setup mode: models {} are not installed; import them from the page or with `orag models import`",
            missing.join(", ")
        );
        (
            Arc::new(MissingEmbedder::new(&config.embedding_model)) as Arc<dyn Embedder>,
            Arc::new(MissingGenerator::new(&config.generation_model)) as Arc<dyn Generator>,
        )
    } else {
        load_models(&config)?
    };
    if stdin_closed.as_ref().is_some_and(|closed| *closed.borrow()) {
        tracing::info!("stdin closed during startup; not serving");
        return Ok(0);
    }
    let models = ModelsInfo {
        models_dir: config.models_dir(),
        embedding: config.embedding_model.clone(),
        generation: config.generation_model.clone(),
    };
    // Only weak handles stay here: once serving ends, a live model means a
    // detached blocking task (an embedding batch, a prefill) still uses it.
    let models_in_use = ModelsInUse::watch(&embedder, &generator);
    let gpu = crate::gpu::GpuStatus::from_env(generator.offload().as_ref());
    tracing::info!(?gpu, "gpu status");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let served = runtime.block_on(async move {
        let listener = TcpListener::from_std(listener)?;
        let local_addr = listener.local_addr()?;
        let mut state = AppState::new(store.clone(), embedder.clone(), generator, config.max_document_bytes())
            .with_models(models, setup, supervised)
            .with_gpu(gpu)
            .with_downloads(Downloads::production(config.models.download).with_config_file(config.config_path()));
        let mut effective = config.effective();
        effective["bind"] = serde_json::json!(local_addr.to_string()); // the real port when bind uses :0
        let ui_listener = ui_listener.map(TcpListener::from_std).transpose()?;
        let ui_addr = ui_listener.as_ref().map(TcpListener::local_addr).transpose()?;
        if let Some(ui_addr) = ui_addr {
            effective["ui_bind"] = serde_json::json!(ui_addr.to_string());
            // The page's own origins and nothing else (D-013, D-021).
            let mut origins = (*state.allowed_origins).clone();
            origins.extend(crate::server::ui::origins(ui_addr));
            state.allowed_origins = Arc::new(origins);
        }
        state.effective_config = Arc::new(effective);
        let ingest = Arc::new(IngestContext { store, embedder, chunker: ChunkerConfig::default() });
        // Setup mode runs no worker: there is no model to index with.
        if !setup {
            ingest.check()?;
        }
        // Registered before the listening line: a signal sent as soon as the
        // line appears is a graceful shutdown, not the default kill.
        // A running parser child stops through the worker's own shutdown
        // signal (AppState), which `serve` starts when this future completes.
        let shutdown = or_stdin_closed(shutdown_signal()?, stdin_closed);
        let worker = (!setup)
            .then(|| spawn_worker(ingest, state.ingest_wake.clone(), state.subscribe_shutdown()));
        let exit = state.clone();
        // The listening line stays first: the desktop sidecar and tests read it.
        crate::cli::print_stdout(&format!("{LISTENING_PREFIX}http://{local_addr}"))?;
        if let Some(ui_addr) = ui_addr {
            crate::cli::print_stdout(&format!("{UI_PREFIX}http://{ui_addr}"))?;
        }
        serve_with_ui(state, listener, ui_listener, shutdown).await?;
        if let Some(worker) = worker
            && tokio::time::timeout(TASK_STOP_LIMIT, worker).await.is_err()
        {
            tracing::warn!("the ingest worker did not stop within {TASK_STOP_LIMIT:?}; its job resumes on the next start");
        }
        anyhow::Ok(exit.exit_code().unwrap_or(0))
    });
    // Never wait without bound for a blocking task: an unfinished job is
    // requeued on the next start, and the instance lock is released on exit.
    runtime.shutdown_timeout(TASK_STOP_LIMIT);
    if models_in_use.any() {
        tracing::warn!(
            "a model is still in use by a task that did not stop; exiting without native teardown"
        );
        crate::exit::exit_without_native_teardown(match &served {
            Ok(code) => i32::from(*code),
            Err(_) => 1,
        });
    }
    served
}

/// A non-blocking listener for tokio; `context` names what failed to bind.
fn bind_listener(
    addr: std::net::SocketAddr,
    context: impl FnOnce() -> String,
) -> anyhow::Result<std::net::TcpListener> {
    let listener = std::net::TcpListener::bind(addr).with_context(context)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// The web UI listener. For an ephemeral `ui_bind` (port 0) a `hint` port is
/// tried first, so the page keeps its origin (and the browser storage that
/// goes with it) across restarts; a taken hint falls back to a free port. A
/// port set in config.toml always wins.
fn bind_ui_listener(
    addr: std::net::SocketAddr,
    hint: Option<u16>,
) -> anyhow::Result<std::net::TcpListener> {
    let context = || {
        format!(
            "binding the web UI to {addr} (set ui_bind in config.toml to another port or \"off\")"
        )
    };
    if let Some(port) = hint.filter(|&port| port != 0 && addr.port() == 0) {
        let hinted = std::net::SocketAddr::new(addr.ip(), port);
        match bind_listener(hinted, context) {
            Ok(listener) => return Ok(listener),
            Err(err) => {
                tracing::info!(%hinted, error = %err, "the web UI's previous port is taken; using a free one")
            }
        }
    }
    bind_listener(addr, context)
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

/// How long a supervised `serve` retries a held lock: Windows releases the
/// lock of a process that just ended (or a closed handle) asynchronously, so
/// the restart after exit code 75 can find it still held for a moment.
const SUPERVISED_LOCK_WAIT: Duration = Duration::from_secs(5);

/// The instance lock of `home`, or `None` when another process holds it.
/// Any other failure (unwritable home, a filesystem without locks) is an error.
pub(crate) fn try_instance_lock(home: &std::path::Path) -> anyhow::Result<Option<std::fs::File>> {
    let path = home.join("orag.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(err)) => {
            Err(err).with_context(|| format!("locking {}", path.display()))
        }
    }
}

fn acquire_instance_lock(home: &std::path::Path, wait: Duration) -> anyhow::Result<std::fs::File> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if let Some(file) = try_instance_lock(home)? {
            return Ok(file);
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "another orag instance is already serving {}",
                home.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn init_logging(level: &str) {
    let filter = tracing_subscriber::EnvFilter::new(level);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Reads stdin to its end on a thread of its own; the receiver turns `true`
/// at end of file or on a read error (a parent that went away).
fn watch_stdin_close() -> anyhow::Result<tokio::sync::watch::Receiver<bool>> {
    let (tx, rx) = tokio::sync::watch::channel(false);
    std::thread::Builder::new()
        .name("stdin-close".into())
        .spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            let _ = tx.send(true);
        })
        .context("starting the stdin watcher")?;
    Ok(rx)
}

/// `signal`, or the end of stdin when it is watched.
async fn or_stdin_closed(
    signal: impl Future<Output = ()> + Send + 'static,
    stdin_closed: Option<tokio::sync::watch::Receiver<bool>>,
) {
    let Some(mut closed) = stdin_closed else {
        return signal.await;
    };
    tokio::select! {
        () = signal => {}
        _ = closed.wait_for(|c| *c) => tracing::info!("stdin closed; shutting down"),
    }
}

/// Installs the SIGINT/SIGTERM handlers now and returns the future that
/// completes on the first signal. A handler that cannot be installed stops
/// startup: a server that cannot be stopped gracefully must not start.
#[cfg(unix)]
pub(crate) fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
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

/// Windows: Ctrl-C, Ctrl-Break (what a supervisor sends to a process group),
/// closing the console window, and system shutdown (delivered to services)
/// stop the service gracefully. A logoff event (also only delivered to
/// services) is ignored: another user logging off must not stop it, and
/// without a listener the default handler would end the process at once.
/// Windows gives a closing console a few seconds only, so the drain limits
/// still apply but the process may be ended before they run out; an
/// unfinished job is requeued on the next start.
#[cfg(windows)]
pub(crate) fn shutdown_signal() -> anyhow::Result<impl Future<Output = ()> + Send + 'static> {
    use tokio::signal::windows;
    let mut ctrl_c = windows::ctrl_c().context("installing the Ctrl-C handler")?;
    let mut ctrl_break = windows::ctrl_break().context("installing the Ctrl-Break handler")?;
    let mut close = windows::ctrl_close().context("installing the console-close handler")?;
    let mut shutdown = windows::ctrl_shutdown().context("installing the shutdown handler")?;
    let mut logoff = windows::ctrl_logoff().context("installing the logoff handler")?;
    Ok(async move {
        // Each logoff event parks one OS console-handler thread inside tokio
        // for the rest of the process (reserved stack only); logoffs reach
        // services, which see few of them, and are cheaper to keep than to
        // let end the service.
        let ignore_logoff = async {
            while logoff.recv().await.is_some() {}
            std::future::pending::<()>().await;
        };
        tokio::select! {
            _ = ctrl_c.recv() => {}
            _ = ctrl_break.recv() => {}
            _ = close.recv() => {}
            _ = shutdown.recv() => {}
            () = ignore_logoff => {}
        }
        tracing::info!("shutting down");
    })
}

/// Configured models whose folder does not exist (setup mode, D-023). Both
/// are checked before any (slow) load, so every missing id is named; a
/// broken manifest or a wrong role still stops startup.
fn missing_models(config: &Config) -> anyhow::Result<Vec<String>> {
    let mut missing = Vec::new();
    for (id, role) in [
        (&config.embedding_model, ModelRole::Embedding),
        (&config.generation_model, ModelRole::Generation),
    ] {
        match find_model(&config.models_dir(), id, role) {
            Ok(_) => {}
            Err(crate::error::OragError::ModelNotInstalled { id }) => missing.push(id),
            Err(err) => return Err(err).with_context(|| format!("model `{id}`")),
        }
    }
    Ok(missing)
}

fn load_models(config: &Config) -> anyhow::Result<(Arc<dyn Embedder>, Arc<dyn Generator>)> {
    let embedder = load_embedder(config)?;
    let generator = load_generator(config)?;
    Ok((embedder, generator))
}

/// Where this process puts its models, decided once so every model goes to
/// the same device (a second model must not pick another GPU because the
/// first one took memory). `orag-vulkan`: `gpu = "off"`, then
/// `ORAG_GPU_DEVICE`, then the automatic choice (v0.6 plan, G-4); other
/// builds place models themselves.
#[cfg(feature = "llama")]
fn gpu_placement(config: &Config) -> anyhow::Result<&'static crate::gpu::Placement> {
    static PLACEMENT: std::sync::OnceLock<crate::gpu::Placement> = std::sync::OnceLock::new();
    if let Some(placement) = PLACEMENT.get() {
        return Ok(placement);
    }
    let placement = decide_gpu_placement(config)?;
    tracing::info!(?placement, "gpu placement");
    Ok(PLACEMENT.get_or_init(|| placement))
}

#[cfg(feature = "vulkan")]
fn decide_gpu_placement(config: &Config) -> anyhow::Result<crate::gpu::Placement> {
    let devices = crate::infer::llama::devices::gpu_devices()?;
    let env_device = std::env::var("ORAG_GPU_DEVICE").ok();
    let allow_software = std::env::var_os("ORAG_VULKAN_ALLOW_SOFTWARE").is_some_and(|v| v == "1");
    crate::gpu::decide_placement(config.gpu, env_device.as_deref(), &devices, allow_software)
        .map_err(anyhow::Error::msg)
}

#[cfg(all(feature = "llama", not(feature = "vulkan")))]
fn decide_gpu_placement(_config: &Config) -> anyhow::Result<crate::gpu::Placement> {
    Ok(crate::gpu::Placement::BuildDefault)
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
        LlamaEmbedder::load_placed(&installed, gpu_placement(config)?)
            .context("loading embedding model")?,
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
        LlamaGenerator::load_placed(&installed, gpu_placement(config)?)
            .context("loading generation model")?,
    ))
}

#[cfg(not(feature = "llama"))]
pub(crate) fn load_embedder(_config: &Config) -> anyhow::Result<Arc<dyn Embedder>> {
    no_backend()
}

/// The installed reranker `id` (v0.4 candidate; eval only for now).
#[cfg(feature = "llama")]
pub(crate) fn load_reranker(
    config: &Config,
    id: &str,
) -> anyhow::Result<Arc<dyn crate::infer::Reranker>> {
    use crate::infer::llama::reranker::LlamaReranker;
    use crate::infer::models::{ModelRole, find_model};
    let installed = find_model(&config.models_dir(), id, ModelRole::Reranking)?;
    tracing::info!(reranker = %installed.manifest.id, "loading reranker");
    Ok(Arc::new(
        LlamaReranker::load_placed(&installed, gpu_placement(config)?)
            .context("loading reranker")?,
    ))
}

#[cfg(not(feature = "llama"))]
pub(crate) fn load_reranker(
    _config: &Config,
    _id: &str,
) -> anyhow::Result<Arc<dyn crate::infer::Reranker>> {
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

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn ui_port(addr: &str, hint: Option<u16>) -> u16 {
        let listener = bind_ui_listener(addr.parse().unwrap(), hint).unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn a_free_port_hint_is_used_for_an_ephemeral_ui_bind() {
        let hint = free_port();
        assert_eq!(ui_port("127.0.0.1:0", Some(hint)), hint);
    }

    #[test]
    fn a_taken_port_hint_falls_back_to_a_free_port() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hint = taken.local_addr().unwrap().port();
        let port = ui_port("127.0.0.1:0", Some(hint));
        assert_ne!(port, hint);
        assert_ne!(port, 0);
    }

    #[test]
    fn a_configured_ui_port_wins_over_the_hint() {
        let configured = free_port();
        let hint = free_port();
        assert_eq!(
            ui_port(&format!("127.0.0.1:{configured}"), Some(hint)),
            configured
        );
        assert_ne!(ui_port("127.0.0.1:0", Some(0)), 0);
        assert_ne!(ui_port("127.0.0.1:0", None), 0);
    }
}
