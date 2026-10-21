use std::sync::{Arc, Mutex};

use super::*;

const V: &str = "0.6.0";
const NOW: u64 = 1_800_000_000;
const DAY: u64 = 24 * 60 * 60;

fn marker(version: &str, age: u64) -> Marker {
    Marker {
        version: version.into(),
        reason: "crashed-twice".into(),
        at: NOW - age,
    }
}

fn never() -> ProbeOutcome {
    panic!("the probe ran although the choice was already made")
}

#[test]
fn off_is_the_cpu_without_a_reason_and_no_probe() {
    assert_eq!(
        choose_backend(GpuSetting::Off, None, V, NOW, never),
        (Backend::Cpu, None, false)
    );
}

#[test]
fn a_current_marker_keeps_the_cpu_with_its_reason() {
    let current = marker(V, 6 * DAY);
    assert_eq!(
        choose_backend(GpuSetting::Auto, Some(&current), V, NOW, never),
        (Backend::Cpu, Some("crashed-twice".into()), false)
    );
}

#[test]
fn an_old_or_foreign_marker_probes_again() {
    for stale in [marker(V, 7 * DAY), marker("0.5.0", DAY)] {
        assert_eq!(
            choose_backend(GpuSetting::Auto, Some(&stale), V, NOW, || {
                ProbeOutcome::Chosen(1)
            }),
            (Backend::Vulkan { device: 1 }, None, false),
            "{stale:?}"
        );
    }
}

#[test]
fn a_probe_without_a_device_or_that_fails_is_a_new_reason() {
    assert_eq!(
        choose_backend(GpuSetting::Auto, None, V, NOW, || ProbeOutcome::NoDevice),
        (Backend::Cpu, Some("no-eligible-device".into()), true)
    );
    assert_eq!(
        choose_backend(GpuSetting::Auto, None, V, NOW, || {
            ProbeOutcome::Failed("probe-timeout".into())
        }),
        (Backend::Cpu, Some("probe-timeout".into()), true)
    );
}

#[test]
fn a_missing_loader_is_named_on_windows_and_linux() {
    assert_eq!(exit_reason(Some(0xC000_0135_u32 as i32)), "loader-missing");
    assert_eq!(exit_reason(Some(127)), "loader-missing");
    assert_eq!(exit_reason(Some(3)), "exit-3");
    assert_eq!(exit_reason(None), "killed");
}

#[test]
fn the_setting_is_off_only_when_the_file_says_so() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(read_setting(home.path()), GpuSetting::Auto);
    std::fs::write(home.path().join("config.toml"), "gpu = \"off\"\n").unwrap();
    assert_eq!(read_setting(home.path()), GpuSetting::Off);
    std::fs::write(home.path().join("config.toml"), "gpu = \"auto\"\n").unwrap();
    assert_eq!(read_setting(home.path()), GpuSetting::Auto);
    std::fs::write(home.path().join("config.toml"), "not toml [").unwrap();
    assert_eq!(read_setting(home.path()), GpuSetting::Auto);
}

#[test]
fn the_marker_round_trips_and_is_deleted() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(read_marker(home.path()), None);
    let written = marker(V, 0);
    write_marker(home.path(), &written).unwrap();
    assert_eq!(read_marker(home.path()), Some(written));
    delete_marker(home.path());
    assert_eq!(read_marker(home.path()), None);
}

/// The policy with a scripted probe; counts the probes.
fn policy(home: &Path, outcomes: Vec<ProbeOutcome>) -> (GpuBackends, Arc<Mutex<usize>>) {
    let spec = |name: &str| SidecarSpec::serve(name.into(), home.to_path_buf());
    let probes = Arc::new(Mutex::new(0));
    let counter = probes.clone();
    let backends = GpuBackends::with_probe(
        spec("orag"),
        spec("orag-vulkan"),
        home.to_path_buf(),
        V.into(),
        move |_| {
            let mut n = counter.lock().unwrap();
            let outcome = outcomes[(*n).min(outcomes.len() - 1)].clone();
            *n += 1;
            outcome
        },
    );
    (backends, probes)
}

fn env(spec: &SidecarSpec, key: &str) -> Option<String> {
    spec.env
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.to_string_lossy().into_owned())
}

#[test]
fn a_chosen_device_starts_orag_vulkan_on_it() {
    let home = tempfile::tempdir().unwrap();
    let (mut backends, _) = policy(home.path(), vec![ProbeOutcome::Chosen(2)]);
    let spec = backends.next();
    assert_eq!(spec.program, PathBuf::from("orag-vulkan"));
    assert_eq!(env(&spec, GPU_DEVICE_ENV).as_deref(), Some("2"));
    assert_eq!(env(&spec, GPU_RETRY_ENV).as_deref(), Some("1"));
    assert_eq!(env(&spec, FALLBACK_REASON_ENV), None);
}

#[test]
fn a_gpu_that_never_gets_ready_moves_the_session_to_the_cpu() {
    for (ended, reason) in [
        (
            Ended::NotReady {
                code: Some(127),
                timed_out: false,
                message: String::new(),
            },
            "loader-missing",
        ),
        (
            Ended::NotReady {
                code: None,
                timed_out: true,
                message: String::new(),
            },
            "start-timeout",
        ),
        (
            Ended::NotReady {
                code: None,
                timed_out: false,
                message: String::new(),
            },
            "start-failed",
        ),
    ] {
        let home = tempfile::tempdir().unwrap();
        let (mut backends, probes) = policy(home.path(), vec![ProbeOutcome::Chosen(0)]);
        backends.next();
        assert_eq!(backends.ended(&ended), Verdict::Fallback(reason.into()));
        let cpu = backends.next();
        assert_eq!(cpu.program, PathBuf::from("orag"));
        assert_eq!(env(&cpu, FALLBACK_REASON_ENV).as_deref(), Some(reason));
        assert_eq!(read_marker(home.path()).unwrap().reason, reason);
        // Sticky: later starts in the session do not probe again.
        backends.next();
        assert_eq!(*probes.lock().unwrap(), 1);
    }
}

#[test]
fn one_crash_restarts_the_gpu_and_a_second_one_moves_to_the_cpu() {
    let home = tempfile::tempdir().unwrap();
    let (mut backends, _) = policy(home.path(), vec![ProbeOutcome::Chosen(0)]);
    backends.next();
    assert_eq!(backends.ended(&Ended::Exited(Some(101))), Verdict::Default);
    assert_eq!(backends.next().program, PathBuf::from("orag-vulkan"));
    // A model import's restart is not a crash.
    assert_eq!(backends.ended(&Ended::Exited(Some(75))), Verdict::Default);
    assert_eq!(backends.next().program, PathBuf::from("orag-vulkan"));
    assert_eq!(read_marker(home.path()), None);
    assert_eq!(
        backends.ended(&Ended::Exited(None)),
        Verdict::Fallback("crashed-twice".into())
    );
    assert_eq!(backends.next().program, PathBuf::from("orag"));
    // The CPU sidecar's own crashes are the supervisor's usual business.
    assert_eq!(backends.ended(&Ended::Exited(Some(1))), Verdict::Default);
    assert_eq!(backends.next().program, PathBuf::from("orag"));
}

#[test]
fn the_retry_exit_clears_the_marker_and_the_session() {
    let home = tempfile::tempdir().unwrap();
    let (mut backends, _) = policy(home.path(), vec![ProbeOutcome::Chosen(0)]);
    backends.next();
    backends.ended(&Ended::NotReady {
        code: Some(127),
        timed_out: false,
        message: String::new(),
    });
    assert_eq!(backends.next().program, PathBuf::from("orag"));
    assert_eq!(
        backends.ended(&Ended::Exited(Some(EXIT_GPU_RETRY))),
        Verdict::Fresh
    );
    assert_eq!(read_marker(home.path()), None);
    let spec = backends.next();
    assert_eq!(spec.program, PathBuf::from("orag-vulkan"));
    // The crash count starts again: one crash restarts the GPU.
    assert_eq!(backends.ended(&Ended::Exited(Some(1))), Verdict::Default);
}

#[test]
fn a_marker_that_cannot_be_written_still_keeps_the_session_on_the_cpu() {
    let dir = tempfile::tempdir().unwrap();
    // A home that does not exist: the marker write fails.
    let home = dir.path().join("missing");
    let (mut backends, probes) = policy(&home, vec![ProbeOutcome::Chosen(0)]);
    backends.next();
    assert_eq!(backends.ended(&Ended::Exited(Some(1))), Verdict::Default);
    backends.next();
    assert_eq!(
        backends.ended(&Ended::Exited(Some(1))),
        Verdict::Fallback("crashed-twice".into())
    );
    assert_eq!(read_marker(&home), None);
    let cpu = backends.next();
    assert_eq!(cpu.program, PathBuf::from("orag"));
    assert_eq!(
        env(&cpu, FALLBACK_REASON_ENV).as_deref(),
        Some("crashed-twice")
    );
    assert_eq!(*probes.lock().unwrap(), 2);
}

#[test]
fn gpu_off_and_a_current_marker_skip_the_probe() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.toml"), "gpu = \"off\"\n").unwrap();
    let (mut backends, probes) = policy(home.path(), vec![ProbeOutcome::Chosen(0)]);
    let spec = backends.next();
    assert_eq!(
        (spec.program.clone(), env(&spec, FALLBACK_REASON_ENV)),
        (PathBuf::from("orag"), None)
    );
    std::fs::remove_file(home.path().join("config.toml")).unwrap();
    write_marker(
        home.path(),
        &Marker {
            version: V.into(),
            reason: "loader-missing".into(),
            at: now_secs(),
        },
    )
    .unwrap();
    let (mut backends, _) = policy(home.path(), vec![ProbeOutcome::Chosen(0)]);
    let spec = backends.next();
    assert_eq!(
        env(&spec, FALLBACK_REASON_ENV).as_deref(),
        Some("loader-missing")
    );
    assert_eq!(*probes.lock().unwrap(), 0);
}
