//! No orphan sidecar (D-016): the app dies hard, the real `orag serve` it
//! started stops too. The "app" is `orag-desktop` in its debug-only fake
//! parent mode (`src/fake.rs`); the sidecar is the staged `orag` binary
//! (`scripts/stage-sidecar.sh`). With `ORAG_REQUIRE_SIDECAR=1` (CI) a missing
//! binary fails the test instead of skipping it.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use orag_desktop::fake::{PARENT_ENV, STATE_ENV, WHEN_ENV};

fn staged_orag() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    let found = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("orag-"))
        });
    if found.is_none() && std::env::var("ORAG_REQUIRE_SIDECAR").is_ok_and(|v| v == "1") {
        panic!(
            "no staged sidecar in {}; run scripts/stage-sidecar.sh",
            dir.display()
        );
    }
    found
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

/// Starts the fake app, reads its sidecar's pid, kills the app hard and
/// checks that the sidecar is gone within five seconds.
fn app_killed_hard_leaves_no_sidecar(when: &str) {
    let Some(orag) = staged_orag() else {
        eprintln!("skipped: no staged sidecar (scripts/stage-sidecar.sh)");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "bind = \"127.0.0.1:0\"\nui_bind = \"127.0.0.1:0\"\n",
    )
    .unwrap();
    let mut app = Command::new(env!("CARGO_BIN_EXE_orag-desktop"))
        .env(PARENT_ENV, &orag)
        .env(STATE_ENV, home.path())
        .env(WHEN_ENV, when)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(app.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid: u32 = line
        .strip_prefix("child ")
        .unwrap_or_else(|| panic!("unexpected: {line:?}"))
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid), "the sidecar runs while the app does");
    app.kill().unwrap(); // SIGKILL / TerminateProcess: no cleanup code runs
    app.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(Instant::now() < deadline, "sidecar {pid} outlived the app");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn killing_a_ready_app_stops_its_sidecar() {
    app_killed_hard_leaves_no_sidecar("ready");
}

#[test]
fn killing_the_app_right_after_spawn_stops_its_sidecar() {
    app_killed_hard_leaves_no_sidecar("spawned");
}
