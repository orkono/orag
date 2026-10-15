//! A stand-in for `orag serve`, for the supervisor tests only. Debug builds
//! of `orag-desktop` act as it when `ORAG_DESKTOP_FAKE_SIDECAR` is set;
//! release builds never do.
//!
//! The variable lists one behaviour per start, comma-separated (the last one
//! repeats); `ORAG_DESKTOP_FAKE_STATE` is a folder that counts the starts.
//! Behaviours: `ready` (both lines, then serve until stdin closes),
//! `ready-exit:N` (both lines, then exit N), `exit:N` (a stderr line, exit N),
//! `api-only` (only the listening line), `hang` (no output, never exits),
//! `stubborn` (both lines, ignores stdin closing).

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub const MODES_ENV: &str = "ORAG_DESKTOP_FAKE_SIDECAR";
pub const STATE_ENV: &str = "ORAG_DESKTOP_FAKE_STATE";
/// A stand-in for the app instead: supervises the real `orag` at this path
/// (home `ORAG_DESKTOP_FAKE_STATE`, fake models), prints `child <pid>` as soon
/// as the service is spawned (`ORAG_DESKTOP_FAKE_WHEN=spawned`) or ready, and
/// waits to be killed. The containment tests kill it hard.
pub const PARENT_ENV: &str = "ORAG_DESKTOP_FAKE_PARENT";
pub const WHEN_ENV: &str = "ORAG_DESKTOP_FAKE_WHEN";

/// Runs the fake sidecar if requested and returns its exit code.
pub fn run_if_requested() -> Option<i32> {
    if !cfg!(debug_assertions) {
        return None;
    }
    if let Ok(orag) = std::env::var(PARENT_ENV) {
        return Some(parent(&orag));
    }
    let modes = std::env::var(MODES_ENV).ok()?;
    let state = std::env::var(STATE_ENV).ok()?;
    let run = next_run(Path::new(&state));
    let modes: Vec<&str> = modes.split(',').collect();
    let mode = modes[run.min(modes.len() - 1)];
    Some(behave(mode, run))
}

fn next_run(state: &Path) -> usize {
    let file = state.join("starts");
    let run = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0usize);
    let _ = std::fs::write(&file, (run + 1).to_string());
    let _ = std::fs::write(
        state.join(format!("pid-{run}")),
        std::process::id().to_string(),
    );
    run
}

fn lines(run: usize, ui: bool) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "orag listening on http://127.0.0.1:{}", 41000 + run);
    if ui {
        let _ = writeln!(out, "orag ui on http://127.0.0.1:{}", 42000 + run);
    }
    let _ = out.flush();
}

fn until_stdin_closes() {
    let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
}

fn behave(mode: &str, run: usize) -> i32 {
    let (name, arg) = mode.split_once(':').unwrap_or((mode, ""));
    let code: i32 = arg.parse().unwrap_or(0);
    match name {
        "ready" => {
            lines(run, true);
            until_stdin_closes();
            0
        }
        "ready-exit" => {
            lines(run, true);
            std::thread::sleep(Duration::from_millis(100));
            code
        }
        "exit" => {
            eprintln!("fake sidecar: failing on start {run}");
            code
        }
        "api-only" => {
            lines(run, false);
            until_stdin_closes();
            0
        }
        "stubborn" => {
            lines(run, true);
            let mut buf = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut buf);
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        _ => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
    }
}

fn parent(orag: &str) -> i32 {
    use crate::supervisor::{SidecarEvent, SidecarSpec, Supervisor};
    let Ok(home) = std::env::var(STATE_ENV) else {
        return 2;
    };
    let mut spec = SidecarSpec::serve(orag.into(), home.into());
    spec.args.push("--dev-fake-models".into());
    let supervisor = Supervisor::start(spec);
    if std::env::var(WHEN_ENV).is_ok_and(|w| w == "spawned") {
        while supervisor.child_pid().is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
    } else if !matches!(
        supervisor.events().recv_timeout(Duration::from_secs(60)),
        Ok(SidecarEvent::Ready { .. })
    ) {
        return 3;
    }
    if let Some(pid) = supervisor.child_pid() {
        println!("child {pid}");
        let _ = std::io::stdout().flush();
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
