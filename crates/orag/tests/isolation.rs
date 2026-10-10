//! DOCX/PDF parsing in an isolated child process, end to end (D-010/D-011).

mod common;

// Used only by the end-to-end tests that drive a server with unix signals.
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::net::TcpStream;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use common::*;
use orag::error::OragError;
use orag::ingest::format::SourceFormat;
use orag::ingest::isolate::{TEST_OUTCOME_PREFIX, parse_isolated_until};

/// The real `orag __parse` child, on every platform: a crash is the file's
/// fault (unix signal, Windows exception status), an error exit the host's,
/// and a stop request interrupts instead of failing. Debug builds only: the
/// outcome hook is compiled out of release binaries.
#[test]
#[cfg_attr(not(debug_assertions), ignore = "needs the debug-only parse hooks")]
fn the_isolated_child_is_classified_the_same_on_every_platform() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_orag"));
    let run = |outcome: &str, stop: &dyn Fn() -> bool| {
        let input = [TEST_OUTCOME_PREFIX, outcome.as_bytes()].concat();
        parse_isolated_until(
            exe,
            SourceFormat::Pdf,
            &input,
            Duration::from_secs(60),
            stop,
        )
    };
    let crash = run("abort", &|| false).unwrap_err();
    assert!(
        matches!(crash, OragError::InvalidInput(_)),
        "abort: {crash:?}"
    );
    let host = run("exit3", &|| false).unwrap_err();
    assert!(matches!(host, OragError::Internal(_)), "exit 3: {host:?}");
    let started = std::time::Instant::now();
    let stopped = run("hang", &|| started.elapsed() > Duration::from_millis(500)).unwrap_err();
    assert!(
        matches!(stopped, OragError::Interrupted),
        "hang: {stopped:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the stop did not kill the child"
    );
}

#[test]
fn hidden_parse_subcommand_round_trips_a_pdf() {
    use std::io::Write;
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-tr.pdf");
    let mut child = orag()
        .args(["__parse", "pdf"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixture).unwrap())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let json = parse_child_stdout(&out.stdout);
    assert!(
        json["Parsed"]["blocks"]
            .to_string()
            .contains("İade süresi 14 gündür"),
        "{json}"
    );
}

/// systemd's control-group kill and `pkill orag` signal the parser child
/// together with `orag serve`; it must finish (or be stopped by its parent),
/// never die of the signal and look like a crash.
#[cfg(unix)]
#[test]
#[cfg_attr(
    not(debug_assertions),
    ignore = "needs the debug-only parse delay hook"
)]
fn the_parser_child_ignores_stop_signals() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-tr.pdf");
    let mut child = orag()
        .args(["__parse", "pdf"])
        .env("ORAG_TEST_PARSE_DELAY_MS", "1500")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixture).unwrap())
        .unwrap();
    // Signal only once the child says its dispositions are installed: the
    // first start of a freshly built binary can take seconds (macOS scans it).
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    std::io::BufRead::read_line(&mut stderr, &mut line).unwrap();
    assert_eq!(line.trim_end(), orag::ingest::isolate::TEST_DELAY_READY);
    let pid = child.id().to_string();
    for signal in ["-TERM", "-INT", "-HUP"] {
        assert!(orag_kill(signal, &pid).success());
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", out.status);
    assert!(parse_child_stdout(&out.stdout)["Parsed"].is_object());
}

#[test]
fn hidden_parse_subcommand_rejects_garbage_with_a_message() {
    use std::io::Write;
    let mut child = orag()
        .args(["__parse", "pdf"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"%PDF-1.7 not really")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let json = parse_child_stdout(&out.stdout);
    assert!(json["Rejected"].is_string(), "{json}");
}

/// Isolation end to end (D-010/D-011): a PDF parsed in a child process is
/// interrupted by shutdown, not failed, and is published exactly once after restart.
#[cfg(unix)]
#[test]
// The parse-delay hook exists only in debug builds; a release build parses too fast to interrupt.
#[cfg_attr(
    not(debug_assertions),
    ignore = "needs the debug-only parse delay hook"
)]
fn interrupted_isolated_parse_resumes_after_restart_exactly_once() {
    let home = ephemeral_home();
    let pdf = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample-tr.pdf"),
    )
    .unwrap();
    // The delay hook exists only in debug builds, which is what `cargo test` runs.
    // 30 s leaves ample room on slow CI; shutdown kills the sleeping child anyway.
    let (server, addr) = start_server_with(home.path(), &[("ORAG_TEST_PARSE_DELAY_MS", "30000")]);
    let created = http_upload(&addr, "politika.pdf", "application/pdf", &pdf);
    let job: serde_json::Value =
        serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    // Wait until the worker has claimed the job; the child then sleeps inside the parse.
    let job_uri = format!("/v1/jobs/{}", job["job_id"]);
    let running = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        http_get(&addr, &job_uri, &addr).contains("\"status\":\"running\"")
    });
    assert!(running, "the worker never claimed the job");
    let (took, stderr) = stop_gracefully(server);
    // The parser child is stopped by the shutdown itself, not left to the
    // service's 5 s task limit and its `_exit` fallback.
    assert!(
        took < Duration::from_secs(4),
        "shutdown took {took:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("indexing interrupted by shutdown"),
        "{stderr}"
    );
    let store = orag::store::Store::open(&home.path().join("orag.db")).unwrap();
    let status = store
        .get_job(job["job_id"].as_i64().unwrap())
        .unwrap()
        .status;
    assert_eq!(
        status,
        orag::store::jobs::JobStatus::Running,
        "shutdown must not fail the job"
    );
    drop(store);
    let (_server, addr) = start_server(home.path());
    let uri = format!("/v1/collections/1/documents/{}", job["document_id"]);
    let ready = (0..100).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        http_get(&addr, &uri, &addr).contains("\"status\":\"ready\"")
    });
    assert!(ready, "document was not indexed after restart");
    let db = rusqlite::Connection::open(home.path().join("orag.db")).unwrap();
    let (chunks, first_id): (i64, i64) = db
        .query_row("SELECT COUNT(*), MIN(id) FROM chunks", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let recorded: i64 = db
        .query_row("SELECT chunk_count FROM documents", [], |r| r.get(0))
        .unwrap();
    assert!(
        chunks > 0 && chunks == recorded,
        "chunks {chunks}, recorded {recorded}"
    );
    // AUTOINCREMENT never reuses ids: in this fresh database a second publish
    // (delete + insert) would leave MIN(id) > 1, so this proves exactly one.
    assert_eq!(first_id, 1, "the document was published more than once");
}

/// SIGTERM and wait (bounded), so graceful shutdown runs (unlike `Drop`,
/// which kills). Returns how long the shutdown took and the server's stderr.
#[cfg(unix)]
fn stop_gracefully(mut server: Server) -> (Duration, String) {
    let pid = server.child.id().to_string();
    let started = Instant::now();
    assert!(orag_kill("-TERM", &pid).success());
    let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
        .unwrap_or_else(|| panic!("still running after SIGTERM; stderr: {}", server.stderr()));
    let took = started.elapsed();
    assert!(status.success(), "{status}; stderr: {}", server.stderr());
    (took, server.stderr())
}

#[cfg(unix)]
fn http_upload(addr: &str, filename: &str, mime: &str, bytes: &[u8]) -> String {
    let boundary = "orag-cli-test";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "POST /v1/collections/1/documents HTTP/1.1\r\nHost: {addr}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// JSON after the last result marker (anything before it is parser noise).
fn parse_child_stdout(stdout: &[u8]) -> serde_json::Value {
    let text = String::from_utf8_lossy(stdout);
    let json = text.rsplit("@@ORAG-PARSE-RESULT@@\n").next().unwrap();
    serde_json::from_str(json).unwrap()
}
