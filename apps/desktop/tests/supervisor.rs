//! The sidecar supervisor against the fake sidecar (`src/fake.rs`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use orag_desktop::fake::{MODES_ENV, STATE_ENV};
use orag_desktop::supervisor::{SidecarEvent, SidecarSpec, Supervisor};

const WAIT: Duration = Duration::from_secs(20);

struct Fake {
    state: tempfile::TempDir,
}

impl Fake {
    fn spec(&self, modes: &str) -> SidecarSpec {
        SidecarSpec {
            program: PathBuf::from(env!("CARGO_BIN_EXE_orag-desktop")),
            args: Vec::new(),
            env: vec![
                (MODES_ENV.into(), modes.into()),
                (STATE_ENV.into(), self.state.path().into()),
            ],
            startup_timeout: Duration::from_secs(3),
            stop_grace: Duration::from_millis(500),
        }
    }

    fn starts(&self) -> usize {
        std::fs::read_to_string(self.state.path().join("starts"))
            .map(|s| s.trim().parse().unwrap())
            .unwrap_or(0)
    }

    fn pid(&self, run: usize) -> u32 {
        std::fs::read_to_string(self.state.path().join(format!("pid-{run}")))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}

fn fake() -> Fake {
    Fake {
        state: tempfile::tempdir().unwrap(),
    }
}

fn next(supervisor: &Supervisor) -> SidecarEvent {
    supervisor.events().recv_timeout(WAIT).expect("an event")
}

fn ready(run: usize) -> SidecarEvent {
    SidecarEvent::Ready {
        api: format!("http://127.0.0.1:{}", 41000 + run),
        ui: format!("http://127.0.0.1:{}", 42000 + run),
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // kill -0 succeeds only for a live (or unreaped) process.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

fn gone_within(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn both_lines_make_it_ready_and_shutdown_stops_it() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("ready"));
    assert_eq!(next(&supervisor), ready(0));
    let pid = f.pid(0);
    assert_eq!(supervisor.child_pid(), Some(pid));
    supervisor.shutdown();
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "stdin close stops it"
    );
    assert_eq!(f.starts(), 1, "a requested stop is not restarted");
}

#[test]
fn restart_code_starts_it_again_on_a_new_address() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("ready-exit:75,ready"));
    assert_eq!(next(&supervisor), ready(0));
    assert!(
        matches!(next(&supervisor), SidecarEvent::Restarting { reason } if reason.contains("new models"))
    );
    assert_eq!(next(&supervisor), ready(1));
    supervisor.shutdown();
}

#[test]
fn a_crash_is_restarted() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("exit:1,ready"));
    assert!(
        matches!(next(&supervisor), SidecarEvent::Restarting { reason } if reason.contains("exit code 1"))
    );
    assert_eq!(next(&supervisor), ready(1));
    supervisor.shutdown();
}

#[test]
fn a_crash_loop_fails_with_the_stderr_tail() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("exit:3"));
    for _ in 0..3 {
        assert!(matches!(next(&supervisor), SidecarEvent::Restarting { .. }));
    }
    match next(&supervisor) {
        SidecarEvent::Failed {
            message,
            stderr_tail,
        } => {
            assert!(message.contains("4 times"), "{message}");
            assert!(
                stderr_tail.iter().any(|l| l.contains("failing on start 3")),
                "{stderr_tail:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.starts(), 4);
}

#[test]
fn a_hang_fails_after_the_startup_timeout_and_is_killed() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("hang"));
    match next(&supervisor) {
        SidecarEvent::Failed { message, .. } => {
            assert!(message.contains("did not start"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(gone_within(f.pid(0), Duration::from_secs(5)));
    assert_eq!(f.starts(), 1, "a hang is not restarted");
}

#[test]
fn a_missing_page_line_says_the_page_is_off() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("api-only"));
    match next(&supervisor) {
        SidecarEvent::Failed { message, .. } => assert!(message.contains("ui_bind"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_sidecar_ignoring_stdin_is_killed_after_the_grace_time() {
    let f = fake();
    let supervisor = Supervisor::start(f.spec("stubborn"));
    assert_eq!(next(&supervisor), ready(0));
    let pid = f.pid(0);
    let started = Instant::now();
    supervisor.shutdown();
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "waited for the grace time"
    );
    assert!(gone_within(pid, Duration::from_secs(5)));
}
