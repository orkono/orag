//! Crash and hang isolation for the binary-format parsers (DOCX, PDF).
//!
//! A hostile file can overflow the stack or loop forever inside a third-party
//! parser; `catch_unwind` cannot stop either. When isolation is enabled (by
//! `orag serve`), each DOCX/PDF is parsed by a child `orag __parse <format>`
//! process with a deadline, so a bad file fails its own job instead of killing
//! or blocking the service. Tests and `orag eval` parse in-process.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::domain::document::ParsedDocument;
use crate::error::{OragError, Result};
use crate::ingest::format::SourceFormat;

/// Hidden CLI subcommand that runs one parse in a child process.
pub const PARSE_SUBCOMMAND: &str = "__parse";
pub const PARSE_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

/// Routes all later DOCX/PDF parsing in this process through `executable`.
pub fn enable(executable: PathBuf) {
    let _ = EXECUTABLE.set(executable);
}

/// The running binary for parser children. On Linux `/proc/self/exe`, which
/// stays this exact image even after an upgrade replaces the file on disk (a
/// newer binary could speak another result format). Elsewhere the path at
/// startup: after an in-place upgrade, restart `orag serve`.
pub fn self_executable() -> std::io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let proc_exe = PathBuf::from("/proc/self/exe");
        if proc_exe.exists() {
            return Ok(proc_exe);
        }
    }
    std::env::current_exe()
}

pub fn isolated_executable() -> Option<&'static Path> {
    EXECUTABLE.get().map(PathBuf::as_path)
}

/// What the child writes to stdout.
#[derive(Debug, Serialize, Deserialize)]
pub enum ChildOutput {
    Parsed(ParsedDocument),
    /// A user-facing error (bad or unsupported file).
    Rejected(String),
    /// The parser environment is broken (not the file's fault).
    Failed(String),
}

/// Separates the child's result from anything a third-party parser prints to stdout.
pub const RESULT_MARKER: &[u8] = b"\n@@ORAG-PARSE-RESULT@@\n";
/// Address-space cap for the child on Linux (decompression bombs). Generous,
/// because address space includes reservations; malloc arenas are limited too.
pub const CHILD_MEMORY_LIMIT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// The child exits on its own this long after the parent's deadline, or as soon
/// as its parent disappears, so it can never outlive a killed `orag serve`.
const CHILD_GRACE: Duration = Duration::from_secs(10);
/// Diagnostics kept from the child's stderr (and logged); the rest is drained.
const STDERR_LOG_BYTES: usize = 2048;
/// Upper bound on the child's result. A parse that produces more is rejected,
/// so a hostile file cannot grow the model-holding parent's memory.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Debug builds only: integration tests set this (milliseconds) to slow the child down.
pub const TEST_DELAY_ENV: &str = "ORAG_TEST_PARSE_DELAY_MS";

/// Parses `bytes` in a child process; kills it after `timeout`, or stops it
/// and returns `Interrupted` once `stop` turns true (service shutdown): the
/// job is requeued, not failed.
pub fn parse_isolated_until(
    executable: &Path,
    format: SourceFormat,
    bytes: &[u8],
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Result<ParsedDocument> {
    run_isolated(executable, format, bytes, timeout, MAX_OUTPUT_BYTES, stop)
}

fn run_isolated(
    executable: &Path,
    format: SourceFormat,
    bytes: &[u8],
    timeout: Duration,
    max_output: usize,
    stop: &dyn Fn() -> bool,
) -> Result<ParsedDocument> {
    let mut child = spawn_parser(executable, format)?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(OragError::Internal("child pipes unavailable".into()));
    };
    // Feed and drain on threads so a child that stops reading or writing cannot
    // block us. Scoped threads borrow `bytes`: no second copy of the document.
    let (status, output, diagnostics) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(bytes));
        let out_reader = scope.spawn(move || read_bounded(stdout, max_output, false));
        let err_reader = scope.spawn(move || read_bounded(stderr, STDERR_LOG_BYTES, true));
        let status = wait_with_deadline(&mut child, timeout, stop);
        let _ = writer.join();
        let output = out_reader.join();
        let diagnostics = err_reader.join();
        (status, output, diagnostics)
    });
    let (output, output_overflowed) =
        output.map_err(|_| OragError::Internal("stdout reader panicked".into()))??;
    let (diagnostics, _) =
        diagnostics.map_err(|_| OragError::Internal("stderr reader panicked".into()))??;
    log_diagnostics(format, &diagnostics);
    let status = status?;
    if output_overflowed {
        return Err(OragError::InvalidInput(format!(
            "the parser produced more than {} KiB of output; the file was not indexed",
            max_output / 1024
        )));
    }
    if !status.success() {
        return Err(failed_exit(status));
    }
    decode_result(&output)
}

/// Why a child that ended without success failed:
/// - a crash signal (stack overflow, abort, the memory cap): the file;
/// - SIGKILL (OOM killer, an operator, a cgroup limit) or a stop signal the
///   child ignores (SIGTERM/INT/HUP) still killing it: something outside orag;
/// - an exit code: `run_child` reports a bad file (also a parser panic) as a
///   `Rejected` result, so an error exit is a host problem.
fn failed_exit(status: std::process::ExitStatus) -> OragError {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let outside = [libc::SIGKILL, libc::SIGTERM, libc::SIGINT, libc::SIGHUP];
            if outside.contains(&signal) {
                return OragError::Internal(format!(
                    "the parser process was killed from outside orag (signal {signal}; \
                     the host may be out of memory)"
                ));
            }
            return OragError::InvalidInput(
                "the file could not be parsed (the parser crashed or ran out of memory); it was not indexed".into(),
            );
        }
    }
    OragError::Internal(format!("the parser process failed ({status})"))
}

/// Starts `orag __parse <format>` with piped stdio in its own process group.
fn spawn_parser(executable: &Path, format: SourceFormat) -> Result<std::process::Child> {
    let mut command = Command::new(executable);
    command
        .args([PARSE_SUBCOMMAND, format.as_str()])
        // Few malloc arenas keep address-space use (RLIMIT_AS) proportional to real use.
        .env("MALLOC_ARENA_MAX", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own process group: a terminal Ctrl-C reaches `orag serve`, which then
    // stops the child itself and reports `Interrupted` instead of a crash.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().map_err(|e| {
        OragError::Internal(format!(
            "cannot start the parser process {}: {e}",
            executable.display()
        ))
    })
}

fn log_diagnostics(format: SourceFormat, diagnostics: &[u8]) {
    if !diagnostics.is_empty() {
        let shown = String::from_utf8_lossy(diagnostics).into_owned();
        tracing::warn!(format = format.as_str(), stderr = %shown, "document parser wrote diagnostics");
    }
}

/// Keeps at most `limit` bytes; returns `true` if the source had more.
/// `drain_excess = false` closes the pipe at the limit, so an oversized result
/// is cut off (the parse is rejected anyway). `drain_excess = true` reads and
/// discards the rest, so a chatty child never hits EPIPE on stderr.
fn read_bounded(
    mut source: impl Read,
    limit: usize,
    drain_excess: bool,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    // Grow by hand so capacity never exceeds `limit`; Vec's doubling
    // could otherwise reserve ~2x the cap in the model-holding parent.
    let overflowed = loop {
        let n = match source.read(&mut chunk) {
            Ok(0) => break false,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let keep = n.min(limit - out.len());
        if out.capacity() < out.len() + keep {
            let target = (out.capacity() * 2).max(out.len() + keep).min(limit);
            out.reserve_exact(target - out.len());
        }
        out.extend_from_slice(&chunk[..keep]);
        if keep < n {
            break true;
        }
    };
    if overflowed && drain_excess {
        std::io::copy(&mut source, &mut std::io::sink())?;
    }
    Ok((out, overflowed))
}

fn wait_with_deadline(
    child: &mut std::process::Child,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        match exited_unreaped(child) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(err) => break Err(err),
        }
        if stop() {
            break Err(OragError::Interrupted);
        }
        if Instant::now() >= deadline {
            break Err(OragError::InvalidInput(format!(
                "parsing took longer than {} s and was stopped; the file was not indexed",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    // On every path: the whole process group, while the child is still
    // unreaped (so its pid, and with it the group id, cannot be reused), so
    // nothing a parser started keeps the pipes and the reader threads open.
    kill_group(child);
    let status = child.wait();
    outcome?;
    Ok(status?)
}

/// True once the child has exited, without reaping it (`WNOWAIT`).
#[cfg(unix)]
fn exited_unreaped(child: &std::process::Child) -> Result<bool> {
    let pid = libc::id_t::from(child.id());
    // SAFETY: `info` is a zeroed, writable siginfo_t; waitid only fills it in.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    // SAFETY: valid id type and pointer; WNOWAIT leaves the child waitable.
    if unsafe { libc::waitid(libc::P_PID, pid, &mut info, flags) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: after a successful waitid, si_pid is set (0 if nothing changed).
    Ok(unsafe { info.si_pid() } != 0)
}

#[cfg(not(unix))]
fn exited_unreaped(child: &mut std::process::Child) -> Result<bool> {
    Ok(child.try_wait()?.is_some())
}

/// SIGKILL to the child's process group (it leads its own, see
/// `spawn_parser`); the child alone if it leads none.
#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    let group_killed = libc::pid_t::try_from(child.id()).is_ok_and(|pid| {
        // SAFETY: killpg only sends a signal; the group id is our unreaped
        // child's own pid, so no other process group can be affected.
        unsafe { libc::killpg(pid, libc::SIGKILL) == 0 }
    });
    if !group_killed {
        let _ = child.kill();
    }
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// The result is the JSON after the last `RESULT_MARKER`; earlier bytes are parser noise.
fn decode_result(output: &[u8]) -> Result<ParsedDocument> {
    let start = output
        .windows(RESULT_MARKER.len())
        .rposition(|w| w == RESULT_MARKER)
        .map(|i| i + RESULT_MARKER.len())
        .ok_or_else(|| OragError::Internal("the parser process produced no result".into()))?;
    match serde_json::from_slice::<ChildOutput>(&output[start..])
        .map_err(|e| OragError::Internal(format!("unreadable parser output: {e}")))?
    {
        ChildOutput::Parsed(doc) => Ok(doc),
        ChildOutput::Rejected(message) => Err(OragError::InvalidInput(message)),
        ChildOutput::Failed(message) => Err(OragError::Internal(message)),
    }
}

/// Body of `orag __parse <format>`: stdin bytes → stdout `RESULT_MARKER` + `ChildOutput` JSON.
pub fn run_child(
    format_name: &str,
    parse: impl Fn(SourceFormat, &[u8]) -> Result<ParsedDocument>,
) -> Result<()> {
    ignore_stop_signals();
    if let Err(err) = apply_child_limits() {
        // A denied setrlimit is a host problem; report it as internal, not as a bad file.
        return emit(&ChildOutput::Failed(format!(
            "cannot limit parser memory: {err}"
        )));
    }
    spawn_watchdog(PARSE_TIMEOUT + CHILD_GRACE);
    // Test hook for interrupting a parse in progress. Debug builds only: a
    // release binary ignores it, so ORAG_HOME stays the only variable users see (D-019).
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var(TEST_DELAY_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        std::thread::sleep(Duration::from_millis(ms));
    }
    let format = SourceFormat::from_name(format_name)?;
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    emit(&child_output(format, &bytes, &parse))
}

/// The child's verdict on one file. A panicking parser means the file is
/// one it cannot handle (a `Rejected` result), never an error exit, which
/// the parent reads as a host problem.
fn child_output(
    format: SourceFormat,
    bytes: &[u8],
    parse: &dyn Fn(SourceFormat, &[u8]) -> Result<ParsedDocument>,
) -> ChildOutput {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(format, bytes)));
    match parsed {
        Ok(Ok(doc)) => ChildOutput::Parsed(doc),
        Ok(Err(OragError::InvalidInput(message) | OragError::UnsupportedFormat(message))) => {
            ChildOutput::Rejected(message)
        }
        Ok(Err(err)) => ChildOutput::Failed(err.to_string()),
        Err(_) => ChildOutput::Rejected(
            "the file could not be parsed (the parser failed on it); it was not indexed".into(),
        ),
    }
}

/// The parent stops the child itself (SIGKILL to its process group) and the
/// watchdog ends an orphan, so stop signals sent to every process at once
/// (systemd's control-group kill, `pkill orag`) must not end a parse early:
/// that would look like a crash. The parent then reports `Interrupted`.
fn ignore_stop_signals() {
    #[cfg(unix)]
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: setting a standard signal's disposition to SIG_IGN has no
        // preconditions and installs no handler code.
        unsafe {
            libc::signal(signal, libc::SIG_IGN);
        }
    }
}

fn emit(output: &ChildOutput) -> Result<()> {
    let json = serde_json::to_vec(output).map_err(|e| OragError::Internal(e.to_string()))?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(RESULT_MARKER)?;
    stdout.write_all(&json)?;
    stdout.flush()?;
    Ok(())
}

/// Exits the child if its parent goes away or the absolute deadline passes.
fn spawn_watchdog(limit: Duration) {
    #[cfg(unix)]
    let parent = std::os::unix::process::parent_id();
    let started = Instant::now();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            #[cfg(unix)]
            let orphaned = std::os::unix::process::parent_id() != parent;
            #[cfg(not(unix))]
            let orphaned = false;
            if orphaned || started.elapsed() > limit {
                // The parser may hold locks that exit-time destructors need.
                crate::exit::exit_without_native_teardown(3);
            }
        }
    });
}

/// Linux: cap the child's address space and make it the OOM killer's first
/// choice, so a decompression bomb cannot take down `orag serve` (which holds
/// the models). Other platforms rely on the parse deadline.
/// Fails (and the parse is refused) if the memory cap cannot be installed; a
/// stricter inherited limit is kept.
#[cfg(target_os = "linux")]
fn apply_child_limits() -> Result<()> {
    // Best effort: some containers forbid it. The address-space cap below is
    // the real guard, and a warning here would be logged on every parse.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    let (current_soft, current_hard) = address_space_limit()?;
    let cap = CHILD_MEMORY_LIMIT_BYTES as libc::rlim_t;
    let hard = current_hard.min(cap);
    let limit = libc::rlimit {
        rlim_cur: current_soft.min(hard),
        rlim_max: hard,
    };
    // SAFETY: setrlimit only reads `limit` and changes the limits of this (child) process alone.
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } != 0 {
        return Err(OragError::Internal(format!(
            "cannot install the parser memory limit: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Current (soft, hard) RLIMIT_AS of this process.
#[cfg(target_os = "linux")]
fn address_space_limit() -> Result<(libc::rlim_t, libc::rlim_t)> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes into the valid `limit` struct we pass.
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) } != 0 {
        return Err(OragError::Internal(format!(
            "cannot read the address-space limit: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok((limit.rlim_cur, limit.rlim_max))
}

#[cfg(not(target_os = "linux"))]
fn apply_child_limits() -> Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// One isolated PDF parse of `b"x"` that is never stopped.
    fn parse(exe: &Path, timeout: Duration) -> Result<ParsedDocument> {
        parse_isolated_until(exe, SourceFormat::Pdf, b"x", timeout, &|| false)
    }

    /// A stand-in "orag" that runs `script` instead of parsing.
    fn fake_executable(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("fake-orag");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn parsed_output_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"Parsed":{"title":null,"blocks":[{"Paragraph":"merhaba"}],"warnings":[]}}"#;
        let exe = fake_executable(
            dir.path(),
            &format!(
                "cat > /dev/null; printf 'noise from a parser\\n@@ORAG-PARSE-RESULT@@\\n%s' '{json}'"
            ),
        );
        let doc = parse_isolated_until(
            &exe,
            SourceFormat::Pdf,
            b"%PDF-",
            Duration::from_secs(5),
            &|| false,
        )
        .unwrap();
        assert_eq!(doc.blocks.len(), 1);
    }

    #[test]
    fn rejection_message_reaches_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(
            dir.path(),
            r#"cat > /dev/null; printf '\n@@ORAG-PARSE-RESULT@@\n%s' '{"Rejected":"cannot read PDF"}'"#,
        );
        let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
        assert!(
            matches!(err, OragError::InvalidInput(ref m) if m == "cannot read PDF"),
            "{err}"
        );
    }

    #[test]
    fn output_without_result_marker_is_an_internal_error() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(dir.path(), r#"cat > /dev/null; printf '{"Rejected":"x"}'"#);
        let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, OragError::Internal(_)), "{err}");
    }

    #[test]
    fn missing_executable_is_reported_with_its_path() {
        let err = parse(Path::new("/nonexistent/orag"), Duration::from_secs(5))
            .unwrap_err()
            .to_string();
        assert!(err.contains("/nonexistent/orag"), "{err}");
    }

    #[test]
    fn crashing_child_fails_the_document_not_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(dir.path(), "kill -SEGV $$");
        let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
        assert!(err.to_string().contains("parser crashed"), "{err}");
    }

    #[test]
    fn an_error_exit_is_the_parser_environment_not_the_file() {
        // `run_child` exits 1 only for host problems (stdin, a bad format name);
        // a bad file is a `Rejected` result, a crash a signal.
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(dir.path(), "cat > /dev/null; exit 1");
        let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, OragError::Internal(_)), "{err}");
    }

    #[test]
    fn shutdown_interrupts_instead_of_failing_the_document() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let err = wait_with_deadline(&mut child, Duration::from_secs(30), &|| true).unwrap_err();
        assert!(matches!(err, OragError::Interrupted), "{err}");
    }

    #[test]
    fn an_outside_kill_is_a_host_problem_not_the_file() {
        // SIGKILL comes from the OOM killer, an operator or a cgroup limit, never
        // from the file itself; SIGTERM/INT/HUP the real child ignores (see
        // `ignore_stop_signals`), so either one means something outside orag.
        let dir = tempfile::tempdir().unwrap();
        for signal in ["KILL", "TERM"] {
            let exe = fake_executable(dir.path(), &format!("cat > /dev/null; kill -{signal} $$"));
            let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
            assert!(matches!(err, OragError::Internal(_)), "{signal}: {err}");
        }
    }

    #[test]
    fn a_panicking_parser_is_a_rejected_file() {
        let output = child_output(SourceFormat::Docx, b"x", &|_, _| panic!("zip broke"));
        assert!(
            matches!(output, ChildOutput::Rejected(ref m) if m.contains("could not be parsed")),
            "{output:?}"
        );
        let output = child_output(SourceFormat::Docx, b"x", &|_, _| {
            Err(OragError::Internal("disk".into()))
        });
        assert!(matches!(output, ChildOutput::Failed(_)), "{output:?}");
    }

    #[test]
    fn a_grandchild_holding_the_pipes_cannot_outlive_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        // No `exec`: `sleep` is a grandchild that inherits stdout and stderr.
        let exe = fake_executable(dir.path(), "sleep 30");
        let started = Instant::now();
        let err = parse(&exe, Duration::from_millis(300)).unwrap_err();
        assert!(err.to_string().contains("took longer"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn oversized_child_output_is_rejected_without_buffering_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(dir.path(), "cat > /dev/null; head -c 5000000 /dev/zero");
        let err = run_isolated(
            &exe,
            SourceFormat::Pdf,
            b"x",
            Duration::from_secs(10),
            1024,
            &|| false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("KiB of output"), "{err}");
    }

    #[test]
    fn read_bounded_never_reserves_more_than_the_limit() {
        let source = std::io::repeat(7).take(10_000_000);
        let (out, overflowed) = read_bounded(source, 3_000_000, true).unwrap();
        assert!(overflowed);
        assert_eq!(out.len(), 3_000_000);
        assert!(out.capacity() <= 3_000_000, "capacity {}", out.capacity());
        let (small, overflowed) = read_bounded(&b"abc"[..], 10, false).unwrap();
        assert_eq!((small.as_slice(), overflowed), (&b"abc"[..], false));
    }

    #[test]
    fn environment_failure_is_internal_not_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(
            dir.path(),
            r#"cat > /dev/null; printf '\n@@ORAG-PARSE-RESULT@@\n%s' '{"Failed":"cannot limit parser memory"}'"#,
        );
        let err = parse(&exe, Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, OragError::Internal(_)), "{err}");
    }

    #[test]
    fn chatty_stderr_is_drained_not_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        // 1 MB of diagnostics (well past the 2 KiB cap and the 64 KiB pipe buffer) followed by a valid result.
        // If the parent closed stderr at the cap, `head` would fail with EPIPE
        // and `|| exit 3` would turn the parse into a crash.
        let exe = fake_executable(
            dir.path(),
            r#"cat > /dev/null; head -c 1000000 /dev/zero >&2 || exit 3; printf '\n@@ORAG-PARSE-RESULT@@\n%s' '{"Rejected":"chatty"}'"#,
        );
        let err = run_isolated(
            &exe,
            SourceFormat::Pdf,
            b"x",
            Duration::from_secs(10),
            1024,
            &|| false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("chatty"), "{err}");
    }

    /// Linux CI: the real limit is installed in a re-executed copy of this test binary.
    #[cfg(target_os = "linux")]
    #[test]
    fn child_memory_limit_is_installed_on_linux() {
        const PROBE_SENTINEL: &str = "ORAG_LIMIT_PROBE_OK";
        if std::env::var_os("ORAG_LIMIT_PROBE").is_some() {
            apply_child_limits().unwrap();
            let (soft, hard) = address_space_limit().unwrap();
            assert!(
                soft <= CHILD_MEMORY_LIMIT_BYTES as libc::rlim_t
                    && hard <= CHILD_MEMORY_LIMIT_BYTES as libc::rlim_t
            );
            println!("{PROBE_SENTINEL}");
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ingest::isolate::tests::child_memory_limit_is_installed_on_linux",
                "--nocapture",
            ])
            .env("ORAG_LIMIT_PROBE", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        // The sentinel proves the probe really ran; a renamed test matching
        // nothing would also exit 0.
        assert!(
            out.status.success() && stdout.contains(PROBE_SENTINEL),
            "{stdout}"
        );
    }

    #[test]
    fn hanging_child_is_killed_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_executable(dir.path(), "exec sleep 30");
        let started = Instant::now();
        let err = parse_isolated_until(
            &exe,
            SourceFormat::Docx,
            b"x",
            Duration::from_millis(300),
            &|| false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("took longer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
