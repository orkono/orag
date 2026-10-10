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

#[cfg(unix)]
#[path = "isolate/unix.rs"]
mod sys;
#[cfg(windows)]
#[path = "isolate/windows.rs"]
mod sys;

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
/// Memory cap for the child (decompression bombs): the address space on
/// Linux, committed memory on Windows (its job object). Generous, because
/// address space includes reservations; malloc arenas are limited too.
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
/// Debug builds only: written to stderr when the delay starts, after the
/// child's limits and signal dispositions are installed, so a test can act
/// on an event instead of guessing how long process startup takes.
pub const TEST_DELAY_READY: &str = "orag-test: parse delay started";
/// Debug builds only: input that starts with this prefix and an outcome
/// makes the child end as a test asks: `abort` (a crash), `exit3` (an error
/// exit), `hang` (never ends). In the input, not the environment, so only
/// that one parse is affected.
pub const TEST_OUTCOME_PREFIX: &[u8] = b"ORAG-TEST-OUTCOME:";

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
    let (Some(mut stdin), Some(stdout), Some(stderr)) = child.take_pipes() else {
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
        return Err(sys::classify(status));
    }
    decode_result(&output)
}

/// The error for a parse that crashed or ran out of memory: the file's fault.
fn file_fault() -> OragError {
    OragError::InvalidInput(
        "the file could not be parsed (the parser crashed or ran out of memory); it was not indexed".into(),
    )
}

/// Starts `orag __parse <format>` with piped stdio, contained (`sys::spawn`).
fn spawn_parser(executable: &Path, format: SourceFormat) -> Result<sys::Contained> {
    let mut command = Command::new(executable);
    command
        .args([PARSE_SUBCOMMAND, format.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    sys::spawn(&mut command).map_err(|e| {
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
    child: &mut sys::Contained,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        match child.exited() {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(err) => break Err(err.into()),
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
    // On every path: the whole process group (job on Windows), while the
    // child is still unreaped (so its pid, and with it the group id, cannot
    // be reused), so nothing a parser started keeps the pipes and the reader
    // threads open.
    child.kill_tree();
    let status = child.wait();
    outcome?;
    Ok(status?)
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
    sys::ignore_stop_signals();
    if let Err(err) = sys::apply_child_limits() {
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
        eprintln!("{TEST_DELAY_READY}");
        std::thread::sleep(Duration::from_millis(ms));
    }
    let format = SourceFormat::from_name(format_name)?;
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    #[cfg(debug_assertions)]
    if let Some(outcome) = bytes.strip_prefix(TEST_OUTCOME_PREFIX) {
        match outcome {
            b"abort" => std::process::abort(),
            b"exit3" => std::process::exit(3),
            b"hang" => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            _ => {}
        }
    }
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
    let orphaned = sys::orphan_check();
    let started = Instant::now();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if orphaned() || started.elapsed() > limit {
                // The parser may hold locks that exit-time destructors need.
                crate::exit::exit_without_native_teardown(3);
            }
        }
    });
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
        let mut child = sys::spawn(Command::new("sleep").arg("30")).unwrap();
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
            sys::apply_child_limits().unwrap();
            let (soft, hard) = sys::address_space_limit().unwrap();
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
