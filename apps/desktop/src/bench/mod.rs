//! The one-button GPU bench (v0.6 plan, Task 6, step 88): the desktop
//! stops its service and runs `eval bench` and `eval retrieval` on `orag`
//! and `orag-vulkan` in turn, then writes one results file.

pub mod data;
pub mod merge;
pub mod plan;
pub mod run;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::bench::merge::{Merged, Outcome, Reference, Status, merge};
use crate::bench::plan::{Done, Job, jobs, skip_reason};
use crate::gpu::ProbeOutcome;

pub const RESULTS_SCHEMA: &str = "orag-bench/1";

/// What the page shows while the bench runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub answers_done: usize,
    pub answers_total: usize,
    pub jobs_done: usize,
    pub jobs_total: usize,
}

/// The bench's inputs beyond the files it writes out itself.
pub struct Bench {
    pub cpu: PathBuf,
    pub vulkan: PathBuf,
    pub home: PathBuf,
    /// The app data folder: `bench/<stamp>/` and the results file go here.
    pub data: PathBuf,
    pub app_version: String,
    pub reference: Option<Reference>,
    pub timeout: Duration,
    pub extra_args: Vec<std::ffi::OsString>,
}

#[derive(Debug, Serialize)]
struct Results<'a> {
    schema: &'static str,
    app_version: &'a str,
    os: &'static str,
    stamp: &'a str,
    dataset: Dataset,
    probe: ProbeRecord,
    #[serde(flatten)]
    merged: Merged,
}

#[derive(Debug, Serialize)]
struct Dataset {
    id: &'static str,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct ProbeRecord {
    chosen: Option<usize>,
    failed: Option<String>,
}

impl Bench {
    /// Runs every job (`probe` chooses the device), calling `progress`
    /// after each answer and job; `cancelled` ends it early with
    /// `complete: false`. Returns the results file's path; it is written
    /// whole (temp file, then rename) even when jobs failed.
    pub fn run(
        &self,
        stamp: &str,
        probe: impl FnOnce() -> ProbeOutcome,
        cancelled: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(&Progress),
    ) -> std::io::Result<PathBuf> {
        let work = self.data.join("bench").join(stamp);
        let files = data::extract(&work)?;
        let (device, gpu_failed) = match probe() {
            ProbeOutcome::Chosen(index) => (Some(index), None),
            ProbeOutcome::NoDevice => (None, Some("no-eligible-device".to_string())),
            ProbeOutcome::Failed(reason) => (None, Some(reason)),
        };
        let setup = run::Setup {
            cpu: self.cpu.clone(),
            vulkan: self.vulkan.clone(),
            home: self.home.clone(),
            corpus: files.corpus,
            questions: files.questions,
            retrieval: files.retrieval,
            work: work.clone(),
            device,
            extra_args: self.extra_args.clone(),
        };
        let questions = data::QUESTIONS
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .count();
        let all = jobs();
        let mut state = Progress {
            answers_done: 0,
            answers_total: all.iter().map(|j| j.answers(questions)).sum(),
            jobs_done: 0,
            jobs_total: all.len(),
        };
        progress(&state);
        let mut outcomes: Vec<Outcome> = Vec::new();
        let mut done: Vec<(Job, Done)> = Vec::new();
        let mut complete = true;
        for job in all {
            if cancelled() {
                complete = false;
                break;
            }
            let outcome = match skip_reason(job, gpu_failed.as_deref(), &done) {
                Some(reason) => {
                    state.answers_done += job.answers(questions);
                    Outcome {
                        job,
                        status: Status::Skipped(reason),
                        report: None,
                        first_answer_ms: None,
                    }
                }
                None => {
                    let mut answered = 0;
                    let outcome = run::run_job(job, &setup, self.timeout, cancelled, &mut || {
                        answered += 1;
                        state.answers_done += 1;
                        progress(&state);
                    });
                    // Skipped or partly answered jobs still count as done.
                    state.answers_done += job.answers(questions).saturating_sub(answered);
                    outcome
                }
            };
            // A timeout goes on, but the file can no longer pass.
            if outcome.status == Status::Failed("timeout".into()) {
                complete = false;
            }
            let stop = outcome.status == Status::Failed("cancelled".into()) || cancelled();
            done.push((job, done_of(&outcome.status)));
            outcomes.push(outcome);
            state.jobs_done += 1;
            progress(&state);
            if stop {
                complete = false;
                break;
            }
        }
        let results = Results {
            schema: RESULTS_SCHEMA,
            app_version: &self.app_version,
            os: std::env::consts::OS,
            stamp,
            dataset: Dataset {
                id: "v06-bench",
                sha256: data::questions_sha256(),
            },
            probe: ProbeRecord {
                chosen: device,
                failed: gpu_failed,
            },
            merged: merge(&outcomes, self.reference.as_ref(), complete),
        };
        let path = self.data.join(format!("orag-bench-{stamp}.json"));
        write_whole(&path, &serde_json::to_vec_pretty(&results)?)?;
        Ok(path)
    }
}

fn done_of(status: &Status) -> Done {
    match status {
        Status::Ok => Done::Ok,
        Status::NotInstalled => Done::NotInstalled,
        Status::Failed(_) => Done::Failed,
        Status::Skipped(_) => Done::Skipped,
    }
}

/// Writes `path` through a temp file and a rename, never over an existing file.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        ));
    }
    let temp = path.with_extension("json.part");
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)
}
