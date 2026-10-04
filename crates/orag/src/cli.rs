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
                    dev_fake_models,
                },
        } => run_eval_retrieval(&corpus, &dataset, out.as_deref(), dev_fake_models),
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
    dev_fake_models: bool,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let queries = crate::eval::dataset::load_dataset(dataset)
        .with_context(|| format!("dataset {}", dataset.display()))?;
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
    let report =
        crate::eval::retrieval::run_retrieval_eval(embedder, corpus, &queries, work.path())?;
    print_stdout(report.to_markdown().trim_end())?;
    if let Some(path) = out {
        let json = serde_json::to_string_pretty(&report)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        file.write_all(json.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}
