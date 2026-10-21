//! Runs one bench job as a contained child (`contain::spawn`, stdin kept
//! open with `--exit-on-stdin-close`, like the serve sidecar), counts its
//! answer lines for progress, and reads its report.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::bench::merge::{Outcome, Status};
use crate::bench::plan::{Exe, Index, Job};

/// One job's process may take at most this long.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// What the jobs need: both executables, the service home, the files
/// written out by `data::extract`, and a folder for indexes and reports.
#[derive(Debug, Clone)]
pub struct Setup {
    pub cpu: PathBuf,
    pub vulkan: PathBuf,
    pub home: PathBuf,
    pub corpus: PathBuf,
    pub questions: PathBuf,
    pub retrieval: PathBuf,
    pub work: PathBuf,
    /// The device `orag-vulkan` runs on (the bench's own probe).
    pub device: Option<usize>,
    /// Extra arguments for every job (tests: `--dev-fake-models`).
    pub extra_args: Vec<OsString>,
}

fn index_dir(work: &Path, index: Index) -> PathBuf {
    work.join(match index {
        Index::A => "index-a",
        Index::B => "index-b",
    })
}

/// The job's report file name, unique per job.
pub fn report_name(job: Job) -> String {
    let exe = |e: Exe| match e {
        Exe::Cpu => "cpu",
        Exe::Vulkan => "vulkan",
    };
    let index = |i: Index| match i {
        Index::A => "a",
        Index::B => "b",
    };
    match job {
        Job::Index { exe: e, index: i } => format!("index-{}-{}.json", index(i), exe(e)),
        Job::Answers {
            exe: e,
            tier,
            round,
        } => format!("answers-{}-{}-{round}.json", tier.name(), exe(e)),
        Job::Retrieval { exe: e, index: i } => format!("retrieval-{}-on-{}.json", exe(e), index(i)),
    }
}

/// The child's command line and environment.
pub fn command(job: Job, setup: &Setup) -> Command {
    let program = match job.exe() {
        Exe::Cpu => &setup.cpu,
        Exe::Vulkan => &setup.vulkan,
    };
    let mut command = Command::new(program);
    let out = setup.work.join(report_name(job));
    match job {
        Job::Index { index, .. } => {
            command
                .args([
                    "eval",
                    "bench",
                    "--build-index",
                    "--index-only",
                    "--tier",
                    "default",
                ])
                .arg("--index-dir")
                .arg(index_dir(&setup.work, index));
        }
        Job::Answers { tier, round, .. } => {
            command
                .args(["eval", "bench", "--tier", tier.name()])
                .arg("--index-dir")
                .arg(index_dir(&setup.work, Index::A));
            if round > 1 {
                command.arg("--first-only");
            }
        }
        Job::Retrieval { index, .. } => {
            command
                .args(["eval", "retrieval", "--no-context-budget", "--gpu-auto"])
                .arg("--index-dir")
                .arg(index_dir(&setup.work, index));
        }
    }
    let dataset = match job {
        Job::Retrieval { .. } => &setup.retrieval,
        _ => &setup.questions,
    };
    command
        .arg("--corpus")
        .arg(&setup.corpus)
        .arg("--dataset")
        .arg(dataset)
        .arg("--out")
        .arg(out)
        .arg("--exit-on-stdin-close")
        .args(&setup.extra_args)
        .env("ORAG_HOME", &setup.home)
        .env_remove("ORAG_GPU_DEVICE")
        .env_remove("ORAG_VULKAN_ALLOW_SOFTWARE");
    if job.exe() == Exe::Vulkan
        && let Some(device) = setup.device
    {
        command.env("ORAG_GPU_DEVICE", device.to_string());
    }
    command
}

/// Runs `job`; `on_answer` is called per answer line, `cancelled` is
/// polled, and the child is killed and reaped on cancel or `timeout`.
pub fn run_job(
    job: Job,
    setup: &Setup,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
    on_answer: &mut dyn FnMut(),
) -> Outcome {
    let out = setup.work.join(report_name(job));
    let stderr = std::fs::File::create(out.with_extension("stderr.txt"))
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    let mut command = command(job, setup);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr);
    let failed = |reason: &str| Outcome {
        job,
        status: Status::Failed(reason.into()),
        report: None,
        first_answer_ms: None,
    };
    let started = Instant::now();
    let mut first_answer_ms = None;
    let mut child = match crate::contain::spawn(&mut command) {
        Ok(child) => child,
        Err(_) => return failed("not-started"),
    };
    // Held until the end: closing it is how the child is told to stop.
    let stdin = child.stdin.take();
    let lines = read_lines(child.stdout.take());
    let deadline = Instant::now() + timeout;
    let mut answer_line = |line: &str| {
        if line.starts_with("{\"event\":\"answer\"") {
            first_answer_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
            on_answer();
        }
    };
    let status = loop {
        while let Ok(line) = lines.try_recv() {
            answer_line(&line);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if cancelled() => break Err("cancelled"),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => break Err("timeout"),
            Err(_) => break Err("wait-failed"),
        }
    };
    drop(stdin);
    let status = match status {
        Ok(status) => status,
        Err(reason) => {
            let _ = child.kill();
            let _ = child.wait();
            return failed(reason);
        }
    };
    // The reader may still hold the last lines; it ends with the child.
    for line in lines.iter() {
        answer_line(&line);
    }
    if !status.success() {
        return failed(&crate::gpu::exit_reason(status.code()));
    }
    let report = std::fs::read(&out)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    match report {
        Some(report) if report["status"] == "not-installed" => Outcome {
            job,
            status: Status::NotInstalled,
            report: Some(report),
            first_answer_ms,
        },
        Some(report) => Outcome {
            job,
            status: Status::Ok,
            report: Some(report),
            first_answer_ms,
        },
        None => failed("no-report"),
    }
}

fn read_lines(stdout: Option<impl std::io::Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    rx
}
