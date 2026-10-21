//! Command-line entry point.

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::version;

#[derive(Debug, Parser)]
#[command(name = "orag", version, about = "Local-first, offline RAG engine")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version information as JSON.
    Version,
    /// Manage offline model packs.
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    /// Start ORAG and open its web page; restarts it after a model import.
    Start,
    /// Run the HTTP service.
    Serve {
        /// Serve with deterministic fake models (development and UI work only).
        #[arg(long, hide = true)]
        dev_fake_models: bool,
        /// Stop gracefully when stdin reaches end of file (the desktop app's
        /// stop signal for its sidecar, D-016).
        #[arg(long, hide = true)]
        exit_on_stdin_close: bool,
        /// Try this port first when `ui_bind` uses port 0 (the desktop app
        /// keeps its page origin, and so the page's stored settings).
        #[arg(long, hide = true)]
        ui_port_hint: Option<u16>,
    },
    /// Write a consistent copy of the database (safe while the server runs).
    Backup { dest: std::path::PathBuf },
    /// Internal: parse one document from stdin in an isolated child process.
    #[command(name = "__parse", hide = true)]
    Parse { format: String },
    /// Internal: GPU checks for the desktop app and CI (v0.6). Prints the
    /// GPU devices and the one ORAG would use as one JSON line. `--fit <gguf>`
    /// instead runs llama.cpp's memory fit once; `--load <gguf>` loads the
    /// file as a generator would (placement as `serve` decides it, without
    /// a config file) and prints what went to the GPU. Only `orag-vulkan`
    /// has devices; the CPU build reports none.
    #[command(name = "gpu-probe", hide = true)]
    GpuProbe {
        #[arg(long, conflicts_with = "load")]
        fit: Option<std::path::PathBuf>,
        #[arg(long)]
        load: Option<std::path::PathBuf>,
    },
    /// Evaluation tools.
    Eval {
        #[command(subcommand)]
        command: EvalCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    /// Compare lexical, dense and hybrid retrieval on a labeled dataset.
    Retrieval {
        #[arg(long)]
        corpus: std::path::PathBuf,
        #[arg(long)]
        dataset: std::path::PathBuf,
        /// Also write the report as JSON.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Exit non-zero (after printing) when hybrid recall@10 is below this (0-1).
        #[arg(long)]
        min_recall_at_10: Option<f64>,
        /// Query ids (comma-separated) whose evidence must reach the answer
        /// context with hybrid retrieval; any miss exits non-zero after printing.
        #[arg(long, value_delimiter = ',')]
        require_in_context: Vec<String>,
        /// Lexical query mode to evaluate instead of the served one
        /// (`exact` or `prefix:<n>`, optionally `+stem:<m>`; experiments).
        #[arg(long, hide = true)]
        lexical_query: Option<String>,
        /// Report each label's rank down to this depth (10-50; diagnosis,
        /// scores are unchanged, latency covers the deeper retrieval).
        #[arg(long, hide = true)]
        rank_depth: Option<usize>,
        /// Score the answer context as the first 8 candidates, without loading
        /// the answer model to apply its token budget (faster; as before
        /// 0.3.0-alpha.8).
        #[arg(long)]
        no_context_budget: bool,
        /// Rerank hybrid candidates with this installed reranker (v0.4
        /// experiment).
        #[arg(long, hide = true)]
        rerank_model: Option<String>,
        /// Fused candidates the reranker scores (8-50).
        #[arg(long, hide = true, default_value_t = 20)]
        rerank_depth: usize,
        /// Keep each list's promoted leaders in front of the reranked ones.
        #[arg(long, hide = true)]
        rerank_pin_leaders: bool,
        /// Rerank only the sentences around the query's terms, at most this
        /// many characters per chunk (100-4000).
        #[arg(long, hide = true)]
        rerank_window: Option<usize>,
        /// Keep the index in this directory, or query the index a run
        /// with the same directory left there (v0.6: one backend's index,
        /// another's queries).
        #[arg(long, hide = true)]
        index_dir: Option<std::path::PathBuf>,
        /// Exit when stdin closes (the desktop's GPU bench, v0.6).
        #[arg(long, hide = true)]
        exit_on_stdin_close: bool,
        /// Place the embedder by itself, setting `gpu = "off"` aside (the
        /// desktop's GPU bench, v0.6; `serve` keeps the setting).
        #[arg(long, hide = true)]
        gpu_auto: bool,
        #[arg(long, hide = true)]
        dev_fake_models: bool,
    },
    /// Measure dense-search latency on synthetic vectors (D-003 gate).
    VectorScale {
        #[arg(long, default_value_t = 100_000)]
        chunks: usize,
        #[arg(long, default_value_t = 1024)]
        dimensions: usize,
        #[arg(long, default_value_t = 50)]
        queries: usize,
        /// Directory for the throwaway database (default: the system temp
        /// dir). Use a directory on the disk ORAG_HOME lives on: /tmp can be
        /// RAM-backed (tmpfs) on Linux.
        #[arg(long)]
        work_dir: Option<std::path::PathBuf>,
        /// The p95 gate in milliseconds (tests only).
        #[arg(long, hide = true, default_value_t = crate::eval::vector_scale::TARGET_P95_MS)]
        target_p95_ms: f64,
    },
    /// Answer labeled questions end to end and score what users see
    /// (expected facts, refusals, loops, cut answers) per sampler.
    Answers {
        #[arg(long)]
        corpus: std::path::PathBuf,
        #[arg(long)]
        dataset: std::path::PathBuf,
        /// Also write the report, with every answer, as JSON.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Sampler profiles to compare (comma-separated): greedy, dry,
        /// presence, qwen, qwen:<seed>. Default: the one answers are served with.
        #[arg(long, value_delimiter = ',')]
        sampler: Vec<String>,
        /// Experimental: ask the generator whether each answer only states
        /// what its cited sources say. Reported, never used by any gate.
        #[arg(long)]
        judge: bool,
        #[arg(long, hide = true)]
        dev_fake_models: bool,
    },
    /// One run of the desktop's GPU bench (v0.6): the fixed questions on
    /// this executable's backend, over an index built once and reused.
    #[command(hide = true)]
    Bench(crate::eval::bench::BenchArgs),
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// Verify and install a model pack directory.
    Import { dir: std::path::PathBuf },
    /// List installed models.
    List,
    /// Re-check an installed model's SHA-256.
    Verify { id: String },
}

/// Ends the process with `code` when it is not 0 (`serve` asking `start` to
/// restart it, or `start` passing on the code of `serve`). Everything the
/// command owned has been dropped by then.
fn exit_with(code: i32) -> anyhow::Result<()> {
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

pub fn run() -> ExitCode {
    match execute(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Version => {
            let info =
                version::version_info(Some(crate::store::migrations::SUPPORTED_SCHEMA_VERSION));
            print_stdout(&serde_json::to_string_pretty(&info)?)
        }
        Command::Models { command } => run_models(command),
        Command::Start => exit_with(crate::launcher::run(&load_config()?)?),
        Command::Serve {
            dev_fake_models,
            exit_on_stdin_close,
            ui_port_hint,
        } => {
            let code = crate::app::run_server(
                load_config()?,
                crate::app::ServeOptions {
                    dev_fake_models,
                    exit_on_stdin_close,
                    ui_port_hint,
                },
            )?;
            exit_with(i32::from(code))
        }
        Command::Eval {
            command:
                EvalCommand::Retrieval {
                    corpus,
                    dataset,
                    out,
                    min_recall_at_10,
                    require_in_context,
                    lexical_query,
                    rank_depth,
                    no_context_budget,
                    rerank_model,
                    rerank_depth,
                    rerank_pin_leaders,
                    rerank_window,
                    index_dir,
                    exit_on_stdin_close,
                    gpu_auto,
                    dev_fake_models,
                },
        } => {
            if exit_on_stdin_close {
                crate::eval::bench::exit_when_stdin_closes();
            }
            run_eval_retrieval(
                &corpus,
                &dataset,
                out.as_deref(),
                EvalGates {
                    min_recall_at_10,
                    require_in_context,
                },
                EvalRetrievalOptions {
                    lexical_query: lexical_query.as_deref(),
                    rank_depth,
                    no_context_budget,
                    rerank_model: rerank_model.as_deref(),
                    rerank_depth,
                    rerank_pin_leaders,
                    rerank_window,
                    index_dir: index_dir.as_deref(),
                    gpu_auto,
                    dev_fake_models,
                },
            )
        }
        Command::Eval {
            command:
                EvalCommand::VectorScale {
                    chunks,
                    dimensions,
                    queries,
                    work_dir,
                    target_p95_ms,
                },
        } => {
            let cfg = crate::eval::vector_scale::VectorScaleConfig {
                chunks,
                dimensions,
                queries,
                k: 50,
                seed: 0x5eed,
                target_p95_ms,
            };
            run_eval_vector_scale(&cfg, work_dir.as_deref())
        }
        Command::Eval {
            command:
                EvalCommand::Answers {
                    corpus,
                    dataset,
                    out,
                    sampler,
                    judge,
                    dev_fake_models,
                },
        } => run_eval_answers(
            &corpus,
            &dataset,
            out.as_deref(),
            &sampler,
            judge,
            dev_fake_models,
        ),
        Command::Eval {
            command: EvalCommand::Bench(args),
        } => crate::eval::bench::run_cli(&args),
        Command::GpuProbe {
            fit: Some(gguf), ..
        } => gpu_probe_fit(&gguf),
        Command::GpuProbe {
            load: Some(gguf), ..
        } => gpu_probe_load(&gguf),
        Command::GpuProbe { .. } => gpu_probe_devices(),
        Command::Parse { format } => Ok(crate::ingest::isolate::run_child(
            &format,
            crate::ingest::parse::parse_in_process,
        )?),
        Command::Backup { dest } => {
            // Read-only: no config is loaded, so a mistyped ORAG_HOME is not
            // created, and the live database is never migrated by this binary.
            let home = crate::config::resolve_home(&|key| std::env::var_os(key))?;
            crate::store::backup_database(&home.join(crate::config::DB_FILE), &dest)?;
            print_stdout(&format!("backup written to {}", dest.display()))
        }
    }
}

/// Reads `$ORAG_HOME/config.toml` (D-019); `ORAG_HOME` is the only environment variable.
fn load_config() -> anyhow::Result<crate::config::Config> {
    let home = crate::config::resolve_home(&|key| std::env::var_os(key))?;
    Ok(crate::config::Config::load(&home)?)
}

#[cfg(feature = "vulkan")]
fn gpu_probe_fit(gguf: &std::path::Path) -> anyhow::Result<()> {
    // The context size only shapes the estimate; the probe proves the link.
    let outcome = crate::infer::llama::fit::fit_once(gguf, 512)?;
    println!(
        "{}",
        serde_json::json!({"backend": "vulkan", "fit": outcome.label()})
    );
    Ok(())
}

#[cfg(not(feature = "vulkan"))]
fn gpu_probe_fit(_gguf: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("this is the CPU build; GPU checks need orag-vulkan")
}

#[cfg(feature = "vulkan")]
fn gpu_probe_load(gguf: &std::path::Path) -> anyhow::Result<()> {
    // llama.cpp's load and context logs on stderr: CI reads its buffer lines.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .try_init();
    let devices = crate::infer::llama::devices::gpu_devices()?;
    let env_device = std::env::var("ORAG_GPU_DEVICE").ok();
    let allow_software = std::env::var_os("ORAG_VULKAN_ALLOW_SOFTWARE").is_some_and(|v| v == "1");
    let placement = crate::gpu::decide_placement(
        crate::gpu::GpuSetting::Auto,
        env_device.as_deref(),
        &devices,
        allow_software,
    )
    .map_err(anyhow::Error::msg)?;
    let offload = crate::infer::llama::offload::probe_load(gguf, &placement)?;
    println!(
        "{}",
        serde_json::json!({"backend": "vulkan", "offload": offload})
    );
    Ok(())
}

#[cfg(not(feature = "vulkan"))]
fn gpu_probe_load(_gguf: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("this is the CPU build; GPU checks need orag-vulkan")
}

fn gpu_probe_devices() -> anyhow::Result<()> {
    #[cfg(feature = "vulkan")]
    let (backend, devices) = ("vulkan", crate::infer::llama::devices::gpu_devices()?);
    #[cfg(not(feature = "vulkan"))]
    let (backend, devices) = ("cpu", Vec::new());
    // Tests only: lets a software rasteriser (lavapipe in CI) be chosen.
    let allow_software = std::env::var_os("ORAG_VULKAN_ALLOW_SOFTWARE").is_some_and(|v| v == "1");
    let chosen = crate::gpu::choose_device(&devices, allow_software).map(|device| device.index);
    println!(
        "{}",
        serde_json::json!({"backend": backend, "devices": devices, "chosen": chosen})
    );
    Ok(())
}

fn run_models(command: ModelsCommand) -> anyhow::Result<()> {
    use crate::infer::models;
    let config = load_config()?;
    match command {
        ModelsCommand::Import { dir } => {
            let manifest = models::import_pack(&dir, &config.models_dir())?;
            print_stdout(&format!(
                "installed {} ({}, {})",
                manifest.id,
                manifest.role.as_str(),
                manifest.license
            ))
        }
        ModelsCommand::List => {
            let listing = models::list_models(&config.models_dir())?;
            for model in &listing.installed {
                let m = &model.manifest;
                print_stdout(&format!(
                    "{}\t{}\t{}\t{}",
                    m.id,
                    m.role.as_str(),
                    m.file,
                    m.license
                ))?;
            }
            for broken in &listing.broken {
                eprintln!("warning: {}: {}", broken.dir.display(), broken.error);
            }
            Ok(())
        }
        ModelsCommand::Verify { id } => {
            models::verify_model(&config.models_dir(), &id)?;
            print_stdout(&format!("{id}: ok"))
        }
    }
}

/// Writes one line to stdout. A reader that closed the pipe early
/// (`orag version | head -1`) wanted no more output, so that is success;
/// every other write error is reported. (`println!` would panic instead.)
pub(crate) fn print_stdout(text: &str) -> anyhow::Result<()> {
    match writeln!(std::io::stdout().lock(), "{text}") {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

/// Cheap checks first (dataset, `--out`), then the model, then the run; the
/// report is never written over an existing file.
fn run_eval_retrieval(
    corpus: &std::path::Path,
    dataset: &std::path::Path,
    out: Option<&std::path::Path>,
    gates: EvalGates,
    options: EvalRetrievalOptions<'_>,
) -> anyhow::Result<()> {
    let EvalRetrievalOptions {
        lexical_query,
        rank_depth,
        no_context_budget,
        rerank_model,
        rerank_depth,
        rerank_pin_leaders,
        rerank_window,
        index_dir,
        gpu_auto,
        dev_fake_models,
    } = options;
    if !(8..=50).contains(&rerank_depth) {
        anyhow::bail!("--rerank-depth must be between 8 and 50, got {rerank_depth}");
    }
    if let Some(n) = rerank_window
        && !(100..=4000).contains(&n)
    {
        anyhow::bail!("--rerank-window must be between 100 and 4000, got {n}");
    }
    use anyhow::Context;
    let mut config = crate::retrieval::hybrid::RetrievalConfig::default();
    if let Some(mode) = lexical_query {
        config.lexical_query = crate::domain::lexical_query::LexicalQuery::parse(mode)
            .with_context(|| format!("unknown lexical query {mode:?} (exact or prefix:<n>, optionally +stem:<m> with m >= 3)"))?;
    }
    let rank_depth = rank_depth.unwrap_or(10);
    let max_depth = crate::eval::retrieval::MAX_RANK_DEPTH;
    if !(10..=max_depth).contains(&rank_depth) {
        anyhow::bail!("--rank-depth must be between 10 and {max_depth}, got {rank_depth}");
    }
    if let Some(floor) = gates.min_recall_at_10
        && !(0.0..=1.0).contains(&floor)
    {
        anyhow::bail!("--min-recall-at-10 must be between 0 and 1, got {floor}");
    }
    let queries = crate::eval::dataset::load_dataset(dataset)
        .with_context(|| format!("dataset {}", dataset.display()))?;
    for id in &gates.require_in_context {
        if !queries.iter().any(|q| q.id == *id && q.answerable) {
            anyhow::bail!("--require-in-context: {id} is not an answerable query in the dataset");
        }
    }
    if let Some(path) = out
        && path.exists()
    {
        anyhow::bail!("{} already exists; choose a new --out file", path.display());
    }
    let (embedder, generator) = load_eval_models(dev_fake_models, no_context_budget, gpu_auto)?;
    // Exclusively created and removed on drop; never touches pre-existing
    // paths. With --index-dir the index is kept there instead.
    let work = tempfile::Builder::new().prefix("orag-eval-").tempdir()?;
    let (work_dir, reuse_index) = match index_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("--index-dir {}", dir.display()))?;
            let reuse = dir.join("eval.db").exists();
            if reuse {
                eprintln!("reusing the index in {}", dir.display());
            }
            (dir, reuse)
        }
        None => (work.path(), false),
    };
    let rerank = match rerank_model {
        None => None,
        Some(id) => {
            let reranker: std::sync::Arc<dyn crate::infer::Reranker> = if dev_fake_models {
                std::sync::Arc::new(crate::infer::fake::FakeReranker)
            } else {
                crate::app::load_reranker(&load_config()?, id)?
            };
            Some(crate::retrieval::rerank::Rerank {
                reranker,
                depth: rerank_depth,
                pin_leaders: rerank_pin_leaders,
                // An eval measures the reranker; it never falls back on time.
                timeout: std::time::Duration::from_secs(600),
                window_chars: rerank_window,
            })
        }
    };
    let report = crate::eval::retrieval::run_retrieval_eval_run(
        embedder,
        corpus,
        &queries,
        work_dir,
        config,
        crate::eval::retrieval::EvalRun {
            rank_depth,
            generator,
            rerank,
            reuse_index,
        },
    )?;
    print_stdout(report.to_markdown().trim_end())?;
    if let Some(path) = out {
        write_new_json(path, &report)?;
    }
    gates.check(&report)
}

/// Experiment and diagnosis switches of `orag eval retrieval`.
struct EvalRetrievalOptions<'a> {
    lexical_query: Option<&'a str>,
    rank_depth: Option<usize>,
    no_context_budget: bool,
    rerank_model: Option<&'a str>,
    rerank_depth: usize,
    rerank_pin_leaders: bool,
    rerank_window: Option<usize>,
    index_dir: Option<&'a std::path::Path>,
    gpu_auto: bool,
    dev_fake_models: bool,
}

/// Pass/fail conditions on the hybrid strategy, checked after the report is out.
struct EvalGates {
    min_recall_at_10: Option<f64>,
    require_in_context: Vec<String>,
}

impl EvalGates {
    fn check(&self, report: &crate::eval::retrieval::RetrievalReport) -> anyhow::Result<()> {
        use anyhow::Context;
        let hybrid = report
            .strategies
            .iter()
            .find(|s| s.strategy == "hybrid")
            .context("the report has no hybrid strategy")?;
        // A mean of per-query fractions can land a hair under an exact floor.
        if let Some(floor) = self.min_recall_at_10
            && hybrid.recall_at_10 < floor - 1e-9
        {
            anyhow::bail!(
                "hybrid recall@10 {:.3} is below {floor}",
                hybrid.recall_at_10
            );
        }
        let missed: Vec<&str> = self
            .require_in_context
            .iter()
            .filter(|id| hybrid.missed_in_context.contains(id))
            .map(String::as_str)
            .collect();
        if !missed.is_empty() {
            anyhow::bail!(
                "evidence for {} is not within the top {} hybrid candidates (the answer context)",
                missed.join(", "),
                report.context_chunks
            );
        }
        Ok(())
    }
}

fn run_eval_answers(
    corpus: &std::path::Path,
    dataset: &std::path::Path,
    out: Option<&std::path::Path>,
    samplers: &[String],
    judge: bool,
    dev_fake_models: bool,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let profiles = parse_samplers(samplers)?;
    let probes = crate::eval::answers::load_probes(dataset)
        .with_context(|| format!("dataset {}", dataset.display()))?;
    if let Some(path) = out
        && path.exists()
    {
        anyhow::bail!("{} already exists; choose a new --out file", path.display());
    }
    let (embedder, generator) = load_answer_models(dev_fake_models)?;
    let work = tempfile::Builder::new().prefix("orag-eval-").tempdir()?;
    let report = crate::eval::answers::run_answer_eval(
        embedder,
        generator,
        corpus,
        &probes,
        &profiles,
        work.path(),
        judge,
    )?;
    print_stdout(report.to_markdown().trim_end())?;
    if let Some(path) = out {
        write_new_json(path, &report)?;
    }
    Ok(())
}

/// The named profiles, or the served one when none is named.
fn parse_samplers(names: &[String]) -> anyhow::Result<Vec<crate::infer::SamplerProfile>> {
    use anyhow::Context;
    if names.is_empty() {
        return Ok(vec![crate::retrieval::answer::ANSWER_SAMPLER]);
    }
    names
        .iter()
        .map(|name| {
            crate::infer::SamplerProfile::parse(name).with_context(|| {
                format!("unknown sampler {name:?} (greedy, dry, presence, qwen, qwen:<seed>)")
            })
        })
        .collect()
}

type AnswerModels = (
    std::sync::Arc<dyn crate::infer::Embedder>,
    std::sync::Arc<dyn crate::infer::Generator>,
);

/// The installed models, or fakes that need no config and no ORAG_HOME.
fn load_answer_models(dev_fake_models: bool) -> anyhow::Result<AnswerModels> {
    if dev_fake_models {
        return Ok((
            std::sync::Arc::new(crate::infer::fake::FakeEmbedder::new()),
            std::sync::Arc::new(crate::infer::fake::FakeGenerator::new("fake answer")),
        ));
    }
    let config = load_config()?;
    Ok((
        crate::app::load_embedder(&config)?,
        crate::app::load_generator(&config)?,
    ))
}

type EvalModels = (
    std::sync::Arc<dyn crate::infer::Embedder>,
    Option<std::sync::Arc<dyn crate::infer::Generator>>,
);

/// The embedder, and the answer model when the context is budgeted (D-018). The
/// answer model only counts tokens here; when it is not installed the context
/// is scored unbudgeted and the report says so (`context budget: none`).
fn load_eval_models(
    dev_fake_models: bool,
    no_context_budget: bool,
    gpu_auto: bool,
) -> anyhow::Result<EvalModels> {
    if dev_fake_models {
        let (embedder, generator) = load_answer_models(true)?;
        return Ok((embedder, (!no_context_budget).then_some(generator)));
    }
    let mut config = load_config()?;
    if gpu_auto {
        config.gpu = crate::gpu::GpuSetting::Auto;
    }
    let embedder = crate::app::load_embedder(&config)?;
    if no_context_budget {
        return Ok((embedder, None));
    }
    match crate::app::load_generator(&config) {
        Ok(generator) => Ok((embedder, Some(generator))),
        Err(err)
            if matches!(
                err.downcast_ref::<crate::error::OragError>(),
                Some(crate::error::OragError::ModelNotInstalled { .. })
            ) =>
        {
            eprintln!(
                "note: the answer model is not installed, so the answer context is scored without its token budget ({err})"
            );
            Ok((embedder, None))
        }
        Err(err) => Err(err),
    }
}

/// Writes `report` as pretty JSON to a file that must not exist yet.
fn write_new_json(path: &std::path::Path, report: &impl serde::Serialize) -> anyhow::Result<()> {
    use anyhow::Context;
    let json = serde_json::to_string_pretty(report)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Runs the D-003 benchmark in a fresh directory (removed afterwards) and
/// exits non-zero when the gate fails.
fn run_eval_vector_scale(
    cfg: &crate::eval::vector_scale::VectorScaleConfig,
    work_dir: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let builder = {
        let mut builder = tempfile::Builder::new();
        builder.prefix("orag-vector-scale-");
        builder
    };
    let work = match work_dir {
        Some(dir) => builder
            .tempdir_in(dir)
            .with_context(|| format!("work dir {}", dir.display()))?,
        None => builder.tempdir()?,
    };
    let report = crate::eval::vector_scale::run_vector_scale(cfg, work.path())?;
    print_stdout(report.to_markdown().trim_end())?;
    if !report.passed {
        anyhow::bail!(
            "p95 {:.1} ms exceeds the {:.0} ms target",
            report.p95_ms,
            report.target_p95_ms
        );
    }
    Ok(())
}
