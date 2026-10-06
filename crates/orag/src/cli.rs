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
    /// Run the HTTP service.
    Serve {
        /// Serve with deterministic fake models (development and UI work only).
        #[arg(long, hide = true)]
        dev_fake_models: bool,
    },
    /// Write a consistent copy of the database (safe while the server runs).
    Backup { dest: std::path::PathBuf },
    /// Internal: parse one document from stdin in an isolated child process.
    #[command(name = "__parse", hide = true)]
    Parse { format: String },
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
        /// (`exact` or `prefix:<n>`; experiments).
        #[arg(long, hide = true)]
        lexical_query: Option<String>,
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
        #[arg(long, hide = true)]
        dev_fake_models: bool,
    },
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
        Command::Serve { dev_fake_models } => {
            crate::app::run_server(load_config()?, crate::app::ServeOptions { dev_fake_models })
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
                    dev_fake_models,
                },
        } => run_eval_retrieval(
            &corpus,
            &dataset,
            out.as_deref(),
            EvalGates {
                min_recall_at_10,
                require_in_context,
            },
            lexical_query.as_deref(),
            dev_fake_models,
        ),
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
                    dev_fake_models,
                },
        } => run_eval_answers(&corpus, &dataset, out.as_deref(), &sampler, dev_fake_models),
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
    lexical_query: Option<&str>,
    dev_fake_models: bool,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut config = crate::retrieval::hybrid::RetrievalConfig::default();
    if let Some(mode) = lexical_query {
        config.lexical_query = crate::domain::lexical_query::LexicalQuery::parse(mode)
            .with_context(|| format!("unknown lexical query {mode:?} (exact or prefix:<n>)"))?;
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
    let embedder: std::sync::Arc<dyn crate::infer::Embedder> = if dev_fake_models {
        // No config is loaded: fake models need none, and ORAG_HOME is not created.
        std::sync::Arc::new(crate::infer::fake::FakeEmbedder::new())
    } else {
        crate::app::load_embedder(&load_config()?)?
    };
    // Exclusively created and removed on drop; never touches pre-existing paths.
    let work = tempfile::Builder::new().prefix("orag-eval-").tempdir()?;
    let report = crate::eval::retrieval::run_retrieval_eval_with(
        embedder,
        corpus,
        &queries,
        work.path(),
        config,
    )?;
    print_stdout(report.to_markdown().trim_end())?;
    if let Some(path) = out {
        write_new_json(path, &report)?;
    }
    gates.check(&report)
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
