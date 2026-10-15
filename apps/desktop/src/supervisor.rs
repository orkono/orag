//! Runs `orag serve` as the app's sidecar (D-016, D-023 amendment).
//!
//! The supervisor reads the service's two stdout lines (`orag listening on`,
//! `orag ui on`), reports them as [`SidecarEvent::Ready`], restarts the
//! service when it exits (75 after a model import, any other code after a
//! crash) at most [`MAX_RESTARTS`] times within [`RESTART_WINDOW`], and stops
//! it by closing its stdin (`--exit-on-stdin-close`), killing it only after
//! [`SidecarSpec::stop_grace`].

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const LISTENING_PREFIX: &str = "orag listening on ";
pub const UI_PREFIX: &str = "orag ui on ";
/// The service's "restart me" exit code (`orag::server::EXIT_RESTART`).
pub const EXIT_RESTART: i32 = 75;
/// More restarts than this within `RESTART_WINDOW` is a loop (as `orag start`).
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Stderr lines kept for the error page.
pub const STDERR_TAIL: usize = 20;
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct SidecarSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Both lines must arrive within this time after each start.
    pub startup_timeout: Duration,
    /// How long a stop waits after closing stdin before killing.
    pub stop_grace: Duration,
}

impl SidecarSpec {
    /// `orag serve` on `home`, supervised and stopped through stdin.
    pub fn serve(program: PathBuf, home: PathBuf) -> SidecarSpec {
        SidecarSpec {
            program,
            args: vec!["serve".into(), "--exit-on-stdin-close".into()],
            env: vec![
                ("ORAG_HOME".into(), home.into_os_string()),
                ("ORAG_SUPERVISED".into(), "1".into()),
            ],
            // Model loading on a slow disk takes a while; a hang still ends.
            startup_timeout: Duration::from_secs(180),
            stop_grace: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarEvent {
    /// The service is serving: its API and page addresses (`http://…`).
    Ready { api: String, ui: String },
    /// The service exited and is being started again.
    Restarting { reason: String },
    /// The service cannot be run; the supervisor has stopped.
    Failed {
        message: String,
        stderr_tail: Vec<String>,
    },
}

pub struct Supervisor {
    events: Receiver<SidecarEvent>,
    stop: Sender<()>,
    pid: Arc<AtomicU32>,
    thread: Option<JoinHandle<()>>,
}

impl Supervisor {
    pub fn start(spec: SidecarSpec) -> Supervisor {
        let (events_tx, events) = mpsc::channel();
        let (stop, stop_rx) = mpsc::channel();
        let pid = Arc::new(AtomicU32::new(0));
        let thread_pid = pid.clone();
        let thread = std::thread::Builder::new()
            .name("sidecar-supervisor".into())
            .spawn(move || run(&spec, &events_tx, &stop_rx, &thread_pid))
            .ok();
        if thread.is_none() {
            // The receiver is still here, so a failed spawn is reported, not lost.
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(SidecarEvent::Failed {
                message: "could not start the supervisor thread".into(),
                stderr_tail: Vec::new(),
            });
            return Supervisor {
                events: rx,
                stop,
                pid,
                thread: None,
            };
        }
        Supervisor {
            events,
            stop,
            pid,
            thread,
        }
    }

    pub fn events(&self) -> &Receiver<SidecarEvent> {
        &self.events
    }

    /// The running child's process id, if one is running.
    pub fn child_pid(&self) -> Option<u32> {
        Some(self.pid.load(Ordering::SeqCst)).filter(|p| *p != 0)
    }

    /// Stops the service (stdin closed, then a kill after the grace time)
    /// and waits for the supervisor to finish.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// What one run of the service ended with.
enum RunEnd {
    /// Exited by itself with this code (None: killed by a signal).
    Exited(Option<i32>),
    /// The app asked it to stop.
    Stopped,
    /// Could not be used; stop supervising.
    Fatal(String),
}

fn run(spec: &SidecarSpec, events: &Sender<SidecarEvent>, stop: &Receiver<()>, pid: &AtomicU32) {
    let mut restarts: Vec<Instant> = Vec::new();
    loop {
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
        let end = match crate::contain::spawn(&mut command(spec)) {
            Ok(child) => run_once(spec, child, &tail, events, stop, pid),
            Err(err) => RunEnd::Fatal(format!("could not start {}: {err}", spec.program.display())),
        };
        pid.store(0, Ordering::SeqCst);
        let reason = match end {
            RunEnd::Stopped => return,
            RunEnd::Fatal(message) => {
                let _ = events.send(SidecarEvent::Failed {
                    message,
                    stderr_tail: snapshot(&tail),
                });
                return;
            }
            RunEnd::Exited(Some(EXIT_RESTART)) => "loading the new models".to_string(),
            RunEnd::Exited(Some(code)) => format!("the service stopped (exit code {code})"),
            RunEnd::Exited(None) => "the service was ended by the system".to_string(),
        };
        let now = Instant::now();
        restarts.retain(|at| now.duration_since(*at) < RESTART_WINDOW);
        restarts.push(now);
        if restarts.len() > MAX_RESTARTS {
            let _ = events.send(SidecarEvent::Failed {
                message: format!(
                    "the service stopped {} times within {} s ({reason})",
                    restarts.len(),
                    RESTART_WINDOW.as_secs()
                ),
                stderr_tail: snapshot(&tail),
            });
            return;
        }
        if events.send(SidecarEvent::Restarting { reason }).is_err() {
            return;
        }
    }
}

fn command(spec: &SidecarSpec) -> Command {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run_once(
    spec: &SidecarSpec,
    mut child: Child,
    tail: &Arc<Mutex<VecDeque<String>>>,
    events: &Sender<SidecarEvent>,
    stop: &Receiver<()>,
    pid: &AtomicU32,
) -> RunEnd {
    pid.store(child.id(), Ordering::SeqCst);
    let stdin = child.stdin.take();
    let lines = read_lines(child.stdout.take());
    keep_tail(child.stderr.take(), tail.clone());
    let ready = match wait_ready(spec, &mut child, &lines, stop) {
        Ok(ready) => ready,
        Err(end) => return finish(spec, child, stdin, end),
    };
    if events.send(ready).is_err() {
        return finish(spec, child, stdin, RunEnd::Stopped);
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return RunEnd::Exited(status.code()),
            Ok(None) => {}
            Err(err) => {
                return finish(
                    spec,
                    child,
                    stdin,
                    RunEnd::Fatal(format!("waiting for the service: {err}")),
                );
            }
        }
        match stop.recv_timeout(POLL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                return finish(spec, child, stdin, RunEnd::Stopped);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Waits for both lines; an exit, a stop or the timeout ends the wait.
fn wait_ready(
    spec: &SidecarSpec,
    child: &mut Child,
    lines: &Receiver<String>,
    stop: &Receiver<()>,
) -> Result<SidecarEvent, RunEnd> {
    let deadline = Instant::now() + spec.startup_timeout;
    let mut api: Option<String> = None;
    loop {
        match lines.recv_timeout(POLL) {
            Ok(line) => {
                if let Some(url) = line.strip_prefix(LISTENING_PREFIX) {
                    api = Some(url.trim().to_string());
                } else if let (Some(url), Some(api)) = (line.strip_prefix(UI_PREFIX), api.clone()) {
                    return Ok(SidecarEvent::Ready {
                        api,
                        ui: url.trim().to_string(),
                    });
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Stdout closed: the service is exiting; its code decides.
            Err(RecvTimeoutError::Disconnected) => {
                return Err(match child.wait() {
                    Ok(status) => RunEnd::Exited(status.code()),
                    Err(err) => RunEnd::Fatal(format!("waiting for the service: {err}")),
                });
            }
        }
        if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            return Err(RunEnd::Stopped);
        }
        if Instant::now() >= deadline {
            let message = if api.is_some() {
                "the service started without its web page (ui_bind is \"off\" in config.toml)"
                    .to_string()
            } else {
                format!(
                    "the service did not start within {} s",
                    spec.startup_timeout.as_secs()
                )
            };
            return Err(RunEnd::Fatal(message));
        }
    }
}

/// Ends a run: closes stdin, waits up to the grace time, then kills.
fn finish(spec: &SidecarSpec, mut child: Child, stdin: Option<ChildStdin>, end: RunEnd) -> RunEnd {
    drop(stdin);
    let deadline = Instant::now() + spec.stop_grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return end,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return end;
            }
        }
    }
}

fn read_lines(stdout: Option<impl Read + Send + 'static>) -> Receiver<String> {
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

fn keep_tail(stderr: Option<impl Read + Send + 'static>, tail: Arc<Mutex<VecDeque<String>>>) {
    if let Some(stderr) = stderr {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
    }
}

fn snapshot(tail: &Mutex<VecDeque<String>>) -> Vec<String> {
    // The reader may still be draining the last lines of a process that just exited.
    std::thread::sleep(POLL);
    tail.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}
