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
            let info = version::version_info(None);
            print_stdout(&serde_json::to_string_pretty(&info)?)
        }
    }
}

/// Writes one line to stdout. A reader that closed the pipe early
/// (`orag version | head -1`) wanted no more output, so that is success;
/// every other write error is reported. (`println!` would panic instead.)
fn print_stdout(text: &str) -> anyhow::Result<()> {
    match writeln!(std::io::stdout().lock(), "{text}") {
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}
