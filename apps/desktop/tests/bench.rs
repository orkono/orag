//! The GPU bench (`src/bench`) with the fake sidecar standing in for both
//! `orag` and `orag-vulkan` (`src/fake_eval.rs`), each with its own state
//! folder.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use orag_desktop::bench::{Bench, Progress};
use orag_desktop::fake::{MODES_ENV, STATE_ENV};
use orag_desktop::fake_eval::EVAL_ENV;
use orag_desktop::gpu::ProbeOutcome;

struct Setup {
    data: tempfile::TempDir,
    home: tempfile::TempDir,
    cpu: tempfile::TempDir,
    vulkan: tempfile::TempDir,
}

/// A wrapper script per backend, so each has its own state and modes.
fn wrapper(dir: &Path, name: &str, state: &Path, modes: &str) -> PathBuf {
    let exe = env!("CARGO_BIN_EXE_orag-desktop");
    #[cfg(windows)]
    {
        let path = dir.join(format!("{name}.cmd"));
        std::fs::write(
            &path,
            format!(
                "@echo off\r\nset {MODES_ENV}=ready\r\nset {STATE_ENV}={}\r\nset {EVAL_ENV}={modes}\r\n\"{exe}\" %*\r\n",
                state.display()
            ),
        )
        .unwrap();
        path
    }
    #[cfg(unix)]
    {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nexport {MODES_ENV}=ready {STATE_ENV}='{}' {EVAL_ENV}='{modes}'\nexec '{exe}' \"$@\"\n",
                state.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

impl Setup {
    fn new() -> Setup {
        Setup {
            data: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
            cpu: tempfile::tempdir().unwrap(),
            vulkan: tempfile::tempdir().unwrap(),
        }
    }

    fn bench(&self, cpu_modes: &str, vulkan_modes: &str) -> Bench {
        Bench {
            cpu: wrapper(self.cpu.path(), "orag", self.cpu.path(), cpu_modes),
            vulkan: wrapper(
                self.vulkan.path(),
                "orag-vulkan",
                self.vulkan.path(),
                vulkan_modes,
            ),
            home: self.home.path().to_path_buf(),
            data: self.data.path().to_path_buf(),
            app_version: "0.6.0-test".into(),
            reference: None,
            timeout: Duration::from_secs(20),
            extra_args: Vec::new(),
        }
    }

    fn calls(state: &Path) -> Vec<String> {
        std::fs::read_to_string(state.join("eval-calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_full_bench_runs_every_job_in_order_and_writes_the_file() {
    let s = Setup::new();
    let mut seen: Vec<Progress> = Vec::new();
    let path = s
        .bench("ok", "ok")
        .run("t1", || ProbeOutcome::Chosen(1), &|| false, &mut |p| {
            seen.push(p.clone())
        })
        .unwrap();
    assert_eq!(path, s.data.path().join("orag-bench-t1.json"));
    let file = read(&path);
    assert_eq!(file["schema"], "orag-bench/1");
    assert_eq!(file["complete"], true);
    assert_eq!(file["probe"]["chosen"], 1);
    assert_eq!(file["dataset"]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(file["order"].as_array().unwrap().len(), 18);
    let tier = &file["tiers"][0];
    assert_eq!(tier["status"], "measured");
    assert_eq!(tier["speedup_warm_p50"], 10.0);
    assert_eq!(tier["all_pass"], true);
    assert_eq!(tier["cpu_floor"], "no-reference");
    assert_eq!(file["retrieval"]["within_tolerance"], true);

    let cpu = Setup::calls(s.cpu.path());
    let vulkan = Setup::calls(s.vulkan.path());
    // Index A, 2 tiers x 3 rounds, retrieval on A and B.
    assert_eq!((cpu.len(), vulkan.len()), (1 + 6 + 2, 6 + 1 + 2));
    assert!(cpu[0].contains("--build-index") && cpu[0].contains("--index-only"));
    assert!(cpu.iter().all(|c| c.ends_with("device=")), "{cpu:?}");
    assert!(vulkan.iter().all(|c| c.ends_with("device=1")), "{vulkan:?}");
    assert!(
        cpu.iter()
            .chain(&vulkan)
            .all(|c| c.contains("--exit-on-stdin-close"))
    );
    // Retrieval sets `gpu = "off"` aside like the answer runs do.
    assert!(
        vulkan
            .iter()
            .filter(|c| c.contains("retrieval"))
            .all(|c| c.contains("--gpu-auto"))
    );
    let cold = tier["vulkan"]["cold_ms"].as_array().unwrap();
    assert_eq!(cold.len(), 3);
    assert!(
        cold.iter().all(|ms| ms.as_f64().unwrap() > 0.0),
        "process start to first answer"
    );
    let last = seen.last().unwrap();
    assert_eq!(
        (last.answers_done, last.jobs_done),
        (last.answers_total, 18)
    );
    assert_eq!(last.answers_total, 2 * 2 * (10 + 1 + 1));
}

#[test]
fn no_gpu_skips_every_vulkan_job_and_still_writes_the_file() {
    let s = Setup::new();
    let mut last: Option<Progress> = None;
    let path = s
        .bench("ok", "ok")
        .run(
            "t2",
            || ProbeOutcome::Failed("loader-missing".into()),
            &|| false,
            &mut |p| last = Some(p.clone()),
        )
        .unwrap();
    let file = read(&path);
    assert_eq!(file["probe"]["failed"], "loader-missing");
    assert_eq!(
        file["tiers"][0]["vulkan"]["runs"][0]["status"],
        "skipped:loader-missing"
    );
    assert_eq!(file["gate"]["pass"], false);
    assert!(Setup::calls(s.vulkan.path()).is_empty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("\"schema\"").count(), 1, "one schema key");
    let last = last.unwrap();
    assert_eq!(
        last.answers_done, last.answers_total,
        "skipped answers count as done"
    );
}

#[test]
fn a_vulkan_crash_is_recorded_and_the_bench_goes_on() {
    let s = Setup::new();
    let path = s
        .bench("ok", "crash:9,ok")
        .run("t3", || ProbeOutcome::Chosen(0), &|| false, &mut |_| {})
        .unwrap();
    let file = read(&path);
    assert_eq!(file["complete"], true);
    assert_eq!(
        file["tiers"][0]["vulkan"]["runs"][0]["status"],
        "failed:exit-9"
    );
    assert_eq!(file["tiers"][0]["vulkan"]["runs"][1]["status"], "ok");
    assert_eq!(file["gate"]["pass"], false);
}

#[test]
fn a_tier_that_is_not_installed_is_skipped() {
    let s = Setup::new();
    // CPU calls: index A, default rounds 1-3, then large round 1.
    let path = s
        .bench("ok,ok,ok,ok,not-installed,ok", "ok")
        .run("t4", || ProbeOutcome::Chosen(0), &|| false, &mut |_| {})
        .unwrap();
    let file = read(&path);
    assert_eq!(file["tiers"][1]["status"], "not-installed");
    assert_eq!(
        file["tiers"][1]["vulkan"]["runs"][0]["status"],
        "skipped:not-installed"
    );
    assert_eq!(file["tiers"][0]["status"], "measured");
}

#[test]
fn cancelling_kills_the_running_job_and_writes_an_incomplete_file() {
    let s = Setup::new();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let hung = s.cpu.path().join("eval-starts");
    let flag = cancel.clone();
    // Cancel once the second CPU job (round 1, hanging) has started.
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if std::fs::read_to_string(&hung).is_ok_and(|n| n.trim() == "2") {
                std::thread::sleep(Duration::from_millis(300));
                flag.store(true, Ordering::SeqCst);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let started = Instant::now();
    let path = s
        .bench("ok,hang", "ok")
        .run(
            "t5",
            || ProbeOutcome::Chosen(0),
            &|| cancel.load(Ordering::SeqCst),
            &mut |_| {},
        )
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(15));
    let file = read(&path);
    assert_eq!(file["complete"], false);
    assert_eq!(file["gate"]["pass"], false);
    assert_eq!(
        file["tiers"][0]["cpu"]["runs"][0]["status"],
        "failed:cancelled"
    );
    assert_eq!(
        file["order"].as_array().unwrap().len(),
        2,
        "nothing ran after the cancel"
    );
}

#[test]
fn an_existing_results_file_is_never_replaced() {
    let s = Setup::new();
    std::fs::write(s.data.path().join("orag-bench-t6.json"), "keep").unwrap();
    let err = s
        .bench("ok", "ok")
        .run("t6", || ProbeOutcome::Chosen(0), &|| false, &mut |_| {});
    assert!(err.is_err());
    assert_eq!(
        std::fs::read_to_string(s.data.path().join("orag-bench-t6.json")).unwrap(),
        "keep"
    );
}

#[test]
fn a_timed_out_job_makes_the_bench_incomplete_but_it_goes_on() {
    let s = Setup::new();
    let mut bench = s.bench("ok", "ok,ok,hang,ok");
    bench.timeout = Duration::from_secs(6);
    let path = bench
        .run("t7", || ProbeOutcome::Chosen(0), &|| false, &mut |_| {})
        .unwrap();
    let file = read(&path);
    assert_eq!(file["complete"], false);
    assert_eq!(
        file["tiers"][0]["vulkan"]["runs"][2]["status"],
        "failed:timeout"
    );
    assert_eq!(
        file["order"].as_array().unwrap().len(),
        18,
        "the bench went on"
    );
    assert_eq!(file["gate"]["pass"], false);
}
