//! The GPU policy (`src/gpu.rs`) under the real supervisor, with the fake
//! sidecar standing in for both `orag-vulkan` and `orag` (`src/fake.rs`):
//! each has its own state folder, so their starts are counted apart.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use orag_desktop::fake::{MODES_ENV, PROBE_ENV, STATE_ENV};
use orag_desktop::gpu::{GpuBackends, read_marker, run_probe};
use orag_desktop::supervisor::{SidecarEvent, SidecarSpec, Supervisor};

const WAIT: Duration = Duration::from_secs(20);

struct Setup {
    home: tempfile::TempDir,
    cpu: tempfile::TempDir,
    vulkan: tempfile::TempDir,
}

fn spec(state: &Path, modes: &str, probe: &str) -> SidecarSpec {
    SidecarSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_orag-desktop")),
        args: Vec::new(),
        env: vec![
            (MODES_ENV.into(), modes.into()),
            (STATE_ENV.into(), state.into()),
            (PROBE_ENV.into(), probe.into()),
        ],
        startup_timeout: Duration::from_secs(2),
        stop_grace: Duration::from_millis(500),
    }
}

impl Setup {
    fn new() -> Setup {
        Setup {
            home: tempfile::tempdir().unwrap(),
            cpu: tempfile::tempdir().unwrap(),
            vulkan: tempfile::tempdir().unwrap(),
        }
    }

    /// The supervisor on `orag-vulkan` (`vulkan` modes, probe answer
    /// `probe`) and `orag` (`cpu` modes); the probe is killed after 1 s.
    fn start(&self, vulkan: &str, probe: &str, cpu: &str) -> Supervisor {
        let backends = GpuBackends::with_probe(
            spec(self.cpu.path(), cpu, "none"),
            spec(self.vulkan.path(), vulkan, probe),
            self.home.path().to_path_buf(),
            "0.6.0-test".into(),
            |spec, cancelled| run_probe(spec, Duration::from_secs(1), cancelled),
        );
        Supervisor::start_with(backends)
    }

    fn starts(state: &Path) -> usize {
        std::fs::read_to_string(state.join("starts"))
            .map(|s| s.trim().parse().unwrap())
            .unwrap_or(0)
    }

    fn env(state: &Path, run: usize) -> String {
        std::fs::read_to_string(state.join(format!("env-{run}"))).unwrap()
    }

    fn marker_reason(&self) -> Option<String> {
        read_marker(self.home.path()).map(|m| m.reason)
    }
}

fn next(supervisor: &Supervisor) -> SidecarEvent {
    supervisor.events().recv_timeout(WAIT).expect("an event")
}

fn is_ready(event: &SidecarEvent) -> bool {
    matches!(event, SidecarEvent::Ready { .. })
}

fn fallback(event: &SidecarEvent, reason: &str) -> bool {
    matches!(event, SidecarEvent::Restarting { reason: r } if r.contains("using the CPU") && r.contains(reason))
}

#[test]
fn a_gpu_sidecar_that_cannot_load_its_libraries_runs_the_cpu() {
    let s = Setup::new();
    let supervisor = s.start("exit:127", "chosen:0", "ready");
    let event = next(&supervisor);
    assert!(fallback(&event, "loader-missing"), "{event:?}");
    assert!(is_ready(&next(&supervisor)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (1, 1)
    );
    assert!(Setup::env(s.vulkan.path(), 0).contains("device=0"));
    assert!(Setup::env(s.cpu.path(), 0).contains("reason=loader-missing"));
    assert!(Setup::env(s.cpu.path(), 0).contains("retry=1"));
    assert_eq!(s.marker_reason().as_deref(), Some("loader-missing"));
    supervisor.shutdown();
}

#[test]
fn a_hung_gpu_sidecar_is_killed_at_the_timeout_and_the_cpu_runs() {
    let s = Setup::new();
    let supervisor = s.start("hang", "chosen:0", "ready");
    let event = next(&supervisor);
    assert!(fallback(&event, "start-timeout"), "{event:?}");
    assert!(is_ready(&next(&supervisor)));
    let pid: u32 = std::fs::read_to_string(s.vulkan.path().join("pid-0"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "the hung sidecar was reaped"
    );
    supervisor.shutdown();
}

#[test]
fn a_second_gpu_crash_moves_the_session_to_the_cpu() {
    let s = Setup::new();
    let supervisor = s.start("ready-exit:1", "chosen:0", "ready");
    assert!(is_ready(&next(&supervisor)));
    assert!(
        matches!(next(&supervisor), SidecarEvent::Restarting { reason } if reason.contains("exit code 1"))
    );
    assert!(is_ready(&next(&supervisor)));
    let event = next(&supervisor);
    assert!(fallback(&event, "crashed-twice"), "{event:?}");
    assert!(is_ready(&next(&supervisor)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (2, 1)
    );
    assert_eq!(s.marker_reason().as_deref(), Some("crashed-twice"));
    supervisor.shutdown();
}

#[test]
fn a_model_import_restarts_the_gpu_sidecar_without_a_marker() {
    let s = Setup::new();
    let supervisor = s.start("ready-exit:75,ready", "chosen:0", "ready");
    assert!(is_ready(&next(&supervisor)));
    assert!(
        matches!(next(&supervisor), SidecarEvent::Restarting { reason } if reason.contains("new models"))
    );
    assert!(is_ready(&next(&supervisor)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (2, 0)
    );
    assert_eq!(s.marker_reason(), None);
    supervisor.shutdown();
}

#[test]
fn an_import_restart_before_ready_is_not_a_gpu_failure() {
    let s = Setup::new();
    let supervisor = s.start("exit:75,ready", "chosen:0", "ready");
    let event = next(&supervisor);
    assert!(
        matches!(&event, SidecarEvent::Restarting { reason } if reason.contains("new models")),
        "{event:?}"
    );
    assert!(is_ready(&next(&supervisor)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (2, 0)
    );
    assert_eq!(s.marker_reason(), None);
    supervisor.shutdown();
}

#[test]
fn closing_the_app_during_a_hung_probe_stops_at_once_and_starts_nothing() {
    let s = Setup::new();
    let backends = GpuBackends::with_probe(
        spec(s.cpu.path(), "ready", "none"),
        spec(s.vulkan.path(), "ready", "hang"),
        s.home.path().to_path_buf(),
        "0.6.0-test".into(),
        |spec, cancelled| run_probe(spec, Duration::from_secs(30), cancelled),
    );
    let supervisor = Supervisor::start_with(backends);
    let probe = s.vulkan.path().join("probe-pid");
    let deadline = Instant::now() + WAIT;
    while !probe.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: u32 = std::fs::read_to_string(&probe)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let started = Instant::now();
    supervisor.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "shutdown waited {:?}",
        started.elapsed()
    );
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "the cancelled probe was reaped"
    );
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (0, 0)
    );
    assert_eq!(s.marker_reason(), None, "a cancelled probe is no reason");
}

#[test]
fn a_hung_probe_is_killed_and_the_cpu_runs() {
    let s = Setup::new();
    let started = Instant::now();
    let supervisor = s.start("ready", "hang", "ready");
    assert!(is_ready(&next(&supervisor)));
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid: u32 = std::fs::read_to_string(s.vulkan.path().join("probe-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "the hung probe was reaped"
    );
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (0, 1)
    );
    assert!(Setup::env(s.cpu.path(), 0).contains("reason=probe-timeout"));
    assert_eq!(s.marker_reason().as_deref(), Some("probe-timeout"));
    supervisor.shutdown();
}

#[test]
fn the_gpu_retry_exit_goes_back_to_the_gpu() {
    let s = Setup::new();
    let supervisor = s.start("exit:127,ready", "chosen:0", "ready-exit:76");
    assert!(fallback(&next(&supervisor), "loader-missing"));
    assert!(is_ready(&next(&supervisor)));
    let event = next(&supervisor);
    assert!(
        matches!(&event, SidecarEvent::Restarting { reason } if reason.contains("GPU again")),
        "{event:?}"
    );
    assert!(is_ready(&next(&supervisor)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (2, 1)
    );
    assert_eq!(s.marker_reason(), None);
    supervisor.shutdown();
}

#[test]
fn a_current_marker_starts_the_cpu_without_probing() {
    let s = Setup::new();
    let supervisor = s.start("exit:127", "chosen:0", "ready");
    assert!(fallback(&next(&supervisor), "loader-missing"));
    assert!(is_ready(&next(&supervisor)));
    supervisor.shutdown();
    // The next app start: the marker holds, no probe, no GPU start.
    std::fs::remove_file(s.vulkan.path().join("probe-pid")).unwrap();
    let supervisor = s.start("exit:127", "chosen:0", "ready");
    assert!(is_ready(&next(&supervisor)));
    assert!(!s.vulkan.path().join("probe-pid").exists());
    assert_eq!(Setup::starts(s.vulkan.path()), 1);
    assert!(Setup::env(s.cpu.path(), 1).contains("reason=loader-missing"));
    supervisor.shutdown();
}

/// Windows: every sidecar starts with system error dialogs off (inherited
/// from the app, which sets them before its first spawn).
#[cfg(windows)]
#[test]
fn sidecars_inherit_quiet_error_dialogs() {
    let s = Setup::new();
    let supervisor = s.start("exit:127", "chosen:0", "ready");
    assert!(fallback(&next(&supervisor), "loader-missing"));
    assert!(is_ready(&next(&supervisor)));
    for state in [s.vulkan.path(), s.cpu.path()] {
        let mode: u32 = std::fs::read_to_string(state.join("error-mode-0"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX
        assert_eq!(mode & 0x8003, 0x8003, "error mode {mode:#x}");
    }
    supervisor.shutdown();
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
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

/// v0.6 step 88: the GPU bench pauses the supervisor and resumes it with
/// the same policy, so a session that fell back to the CPU stays there even
/// when its marker could not be written.
#[test]
fn a_paused_supervisor_resumes_with_the_session_fallback() {
    let s = Setup::new();
    let supervisor = s.start("exit:127,ready", "chosen:0", "ready");
    assert!(fallback(&next(&supervisor), "loader-missing"));
    assert!(is_ready(&next(&supervisor)));
    // As if the marker could not be written: only the session knows.
    std::fs::remove_file(s.home.path().join("gpu-fallback.json")).unwrap();
    let backends = supervisor.pause().expect("the policy comes back");
    let resumed = Supervisor::resume(backends);
    assert!(is_ready(&next(&resumed)));
    assert_eq!(
        (Setup::starts(s.vulkan.path()), Setup::starts(s.cpu.path())),
        (1, 2),
        "the CPU again, no GPU start"
    );
    assert!(Setup::env(s.cpu.path(), 1).contains("reason=loader-missing"));
    resumed.shutdown();
}
