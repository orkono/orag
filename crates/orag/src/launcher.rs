//! `orag start`: runs `orag serve`, opens the page, and restarts `serve`
//! when it exits with `EXIT_RESTART` (setup finished, D-023).
//!
//! The launcher is for people; sidecars and services run `orag serve` and
//! read its first stdout line (D-016), which the launcher passes through.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::app::{SUPERVISED_ENV, UI_PREFIX};
use crate::config::Config;
use crate::server::EXIT_RESTART;

/// More restarts than this within `RESTART_WINDOW` is a loop, not setup.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(100);

pub fn run(config: &Config) -> anyhow::Result<i32> {
    // Another `serve` owns this home: show its page instead of failing. Only
    // a held lock means that; any other lock error is reported.
    match crate::app::try_instance_lock(&config.home)? {
        Some(lock) => {
            // Released before `serve` starts, which takes it for good.
            lock.unlock().context("releasing the instance lock")?;
        }
        None => {
            match config.ui_bind {
                Some(addr) if addr.port() != 0 => {
                    let url = format!("http://{addr}");
                    eprintln!("orag is already running; opening {url}");
                    open_in_browser(&url);
                }
                Some(_) => eprintln!(
                    "orag is already running on a port chosen at its start (ui_bind uses port 0); open the address it printed"
                ),
                None => eprintln!("orag is already running (its web page is off)"),
            }
            return Ok(0);
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    watch_signals(stop.clone())?;
    let exe = std::env::current_exe().context("locating the orag executable")?;
    let mut spawn = || {
        Command::new(&exe)
            .arg("serve")
            .env(SUPERVISED_ENV, "1")
            .stdout(Stdio::piped())
            .spawn()
    };
    supervise(&mut spawn, &open_in_browser, &stop)
}

/// Runs the child until it exits with something other than `EXIT_RESTART`
/// (or a stop was requested), passing its stdout through and opening the page
/// on its `orag ui on` line: once, and again only if a restart moved it to
/// another address (`ui_bind` with port 0). Returns the child's exit code.
pub(crate) fn supervise(
    spawn: &mut dyn FnMut() -> std::io::Result<Child>,
    open: &(dyn Fn(&str) + Sync),
    stop: &AtomicBool,
) -> anyhow::Result<i32> {
    let opened: Mutex<Option<String>> = Mutex::new(None);
    let mut restarts: Vec<Instant> = Vec::new();
    loop {
        let mut child = spawn().context("starting `orag serve`")?;
        let stdout = child.stdout.take();
        let code = std::thread::scope(|scope| {
            if let Some(stdout) = stdout {
                let opened = &opened;
                scope.spawn(move || pass_through(stdout, open, opened));
            }
            wait(&mut child, stop)
        })?;
        if code != i32::from(EXIT_RESTART) || stop.load(Ordering::SeqCst) {
            return Ok(code);
        }
        let now = Instant::now();
        restarts.retain(|at| now.duration_since(*at) < RESTART_WINDOW);
        restarts.push(now);
        if restarts.len() > MAX_RESTARTS {
            anyhow::bail!(
                "`orag serve` asked to restart {} times within {RESTART_WINDOW:?}; stopping this restart loop (see its log above)",
                restarts.len()
            );
        }
        eprintln!("orag: restarting to load the new models");
    }
}

/// Copies the child's stdout to ours; opens the page on the first UI line.
fn pass_through(
    stdout: std::process::ChildStdout,
    open: &dyn Fn(&str),
    opened: &Mutex<Option<String>>,
) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { return };
        if crate::cli::print_stdout(&line).is_err() {
            return;
        }
        if let Some(url) = line.strip_prefix(UI_PREFIX) {
            let url = url.trim();
            let mut last = opened.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_deref() != Some(url) {
                *last = Some(url.to_string());
                open(url);
            }
        }
    }
}

/// Waits for the child; a stop request is forwarded once (unix: SIGTERM; on
/// Windows the console event already reached the child).
fn wait(child: &mut Child, stop: &AtomicBool) -> anyhow::Result<i32> {
    let mut forwarded = false;
    loop {
        if let Some(status) = child.try_wait().context("waiting for `orag serve`")? {
            return Ok(status.code().unwrap_or(1));
        }
        if !forwarded && stop.load(Ordering::SeqCst) {
            forwarded = true;
            terminate(child);
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(unix)]
fn terminate(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: plain kill(2) on our own unreaped child; no memory involved.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

#[cfg(windows)]
fn terminate(_child: &Child) {}

/// Sets `stop` on Ctrl-C (and SIGTERM, console close) instead of dying, so
/// the child shuts down gracefully first.
fn watch_signals(stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let signal = runtime.block_on(async { crate::app::shutdown_signal() })?;
    std::thread::spawn(move || {
        runtime.block_on(signal);
        stop.store(true, Ordering::SeqCst);
    });
    Ok(())
}

/// Opens `url` in the default browser; failures are only reported.
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    match command.arg(url).stdout(Stdio::null()).spawn() {
        Ok(_) => {}
        Err(err) => eprintln!("orag: open {url} in your browser ({err})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `orag serve`: prints the UI line (port 9, or 10 once restarted
    /// when `port_moves`), then exits 75 until `marker` exists (creating it),
    /// or always 75 when `always_restart`.
    fn fake_serve(
        dir: &std::path::Path,
        always_restart: bool,
        port_moves: bool,
    ) -> impl FnMut() -> std::io::Result<Child> {
        let marker = dir.join("restarted");
        let second_port = if port_moves { 10 } else { 9 };
        #[cfg(unix)]
        let script = {
            let path = dir.join("serve.sh");
            let tail = if always_restart {
                "exit 75".to_string()
            } else {
                format!(
                    "if [ -e '{0}' ]; then exit 0; fi\ntouch '{0}'\nexit 75",
                    marker.display()
                )
            };
            std::fs::write(
                &path,
                format!(
                    "if [ -e '{0}' ]; then echo 'orag ui on http://127.0.0.1:{second_port}'; \
                     else echo 'orag ui on http://127.0.0.1:9'; fi\n{tail}\n",
                    marker.display()
                ),
            )
            .unwrap();
            path
        };
        #[cfg(windows)]
        let script = {
            let path = dir.join("serve.cmd");
            let tail = if always_restart {
                "exit /b 75".to_string()
            } else {
                format!(
                    "if exist \"{0}\" exit /b 0\r\ntype nul > \"{0}\"\r\nexit /b 75",
                    marker.display()
                )
            };
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\nif exist \"{0}\" (echo orag ui on http://127.0.0.1:{second_port}) \
                     else (echo orag ui on http://127.0.0.1:9)\r\n{tail}\r\n",
                    marker.display()
                ),
            )
            .unwrap();
            path
        };
        move || {
            #[cfg(unix)]
            let mut command = Command::new("sh");
            #[cfg(windows)]
            let mut command = {
                let mut command = Command::new("cmd");
                command.arg("/C");
                command
            };
            command.arg(&script).stdout(Stdio::piped()).spawn()
        }
    }

    #[test]
    fn a_restart_request_runs_serve_again_and_opens_the_page_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut spawn = fake_serve(dir.path(), false, false);
        let opened = Mutex::new(Vec::new());
        let open = |url: &str| opened.lock().unwrap().push(url.to_string());
        let code = supervise(&mut spawn, &open, &AtomicBool::new(false)).unwrap();
        assert_eq!(code, 0);
        assert!(dir.path().join("restarted").exists(), "ran twice");
        assert_eq!(*opened.lock().unwrap(), ["http://127.0.0.1:9"]);
    }

    #[test]
    fn a_restart_loop_gives_up_with_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut spawn = fake_serve(dir.path(), true, false);
        let err = supervise(&mut spawn, &|_| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(err.to_string().contains("restart loop"), "{err}");
    }

    #[test]
    fn after_a_stop_request_serve_is_not_restarted() {
        let dir = tempfile::tempdir().unwrap();
        let mut inner = fake_serve(dir.path(), true, false);
        let mut spawns = 0;
        let mut spawn = || {
            spawns += 1;
            inner()
        };
        let code = supervise(&mut spawn, &|_| {}, &AtomicBool::new(true)).unwrap();
        assert_ne!(code, 0);
        assert_eq!(spawns, 1, "a stopped launcher must not run serve again");
    }

    #[test]
    fn a_page_that_moved_after_a_restart_is_opened_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut spawn = fake_serve(dir.path(), false, true);
        let opened = Mutex::new(Vec::new());
        let open = |url: &str| opened.lock().unwrap().push(url.to_string());
        assert_eq!(
            supervise(&mut spawn, &open, &AtomicBool::new(false)).unwrap(),
            0
        );
        assert_eq!(
            *opened.lock().unwrap(),
            ["http://127.0.0.1:9", "http://127.0.0.1:10"]
        );
    }
}
