//! Shared helpers for the binary end-to-end tests (`cli.rs`, `isolation.rs`).
#![allow(dead_code)] // each test binary uses a different subset

use std::process::Command;

pub fn orag() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orag"))
}

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Generous for a debug build on a loaded CI runner; a hang still fails.
pub const PROCESS_LIMIT: Duration = Duration::from_secs(30);

/// A running `orag serve`, killed on drop even when a test panics.
pub struct Server {
    pub child: Child,
    pub stdout: mpsc::Receiver<String>,
    pub stderr: PathBuf,
}

impl Server {
    pub fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A home whose config.toml asks for an ephemeral loopback port.
pub fn ephemeral_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.toml"), "bind = \"127.0.0.1:0\"\n").unwrap();
    home
}

/// Starts `orag serve` with stdout lines forwarded to a channel and stderr
/// kept in a file next to the home, so a failure can show the reason.
pub fn spawn_serve(home: &Path, args: &[&str]) -> Server {
    spawn_serve_with(home, args, &[])
}

///  with extra environment variables.
pub fn spawn_serve_with(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Server {
    let stderr = home.with_extension(format!("stderr-{}.log", std::process::id()));
    let mut command = orag();
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .env("ORAG_HOME", home)
        .arg("serve")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Server {
        child,
        stdout: rx,
        stderr,
    }
}

pub fn start_server(home: &Path) -> (Server, String) {
    start_server_with(home, &[])
}

/// Starts `orag serve --dev-fake-models` with extra environment variables.
pub fn start_server_with(home: &Path, env: &[(&str, &str)]) -> (Server, String) {
    let server = spawn_serve_with(home, &["--dev-fake-models"], env);
    let line = server
        .stdout
        .recv_timeout(PROCESS_LIMIT)
        .unwrap_or_else(|_| panic!("no listening line; stderr: {}", server.stderr()));
    let addr = line
        .strip_prefix("orag listening on http://")
        .unwrap_or_else(|| panic!("unexpected: {line}"))
        .to_string();
    (server, addr)
}

/// Waits for the process to exit, or returns `None` after `limit`.
pub fn wait_with_deadline(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

pub fn http_get(addr: &str, path: &str, host: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[cfg(unix)]
pub fn orag_kill(signal: &str, pid: &str) -> ExitStatus {
    std::process::Command::new("kill")
        .args([signal, pid])
        .status()
        .unwrap()
}
