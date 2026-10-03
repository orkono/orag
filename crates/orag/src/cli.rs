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
