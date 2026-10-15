//! `orag serve --exit-on-stdin-close`: the desktop app (D-016) stops its
//! sidecar by closing the sidecar's stdin, on every OS, so a GUI parent
//! without a console can still stop it gracefully.

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::{PROCESS_LIMIT, ephemeral_home, orag, start_server, wait_with_deadline};

fn spawn(home: &std::path::Path, args: &[&str], stdin: Stdio) -> (Child, mpsc::Receiver<String>) {
    let mut child = orag()
        .env("ORAG_HOME", home)
        .arg("serve")
        .arg("--dev-fake-models")
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
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
    (child, rx)
}

/// The instance lock is free again: another server on the same home starts.
fn assert_lock_released(home: &std::path::Path) {
    let (mut next, _addr) = start_server(home);
    let _ = next.child.kill();
}

#[test]
fn closing_stdin_stops_a_serving_sidecar() {
    let home = ephemeral_home();
    let (mut child, lines) = spawn(home.path(), &["--exit-on-stdin-close"], Stdio::piped());
    let line = lines.recv_timeout(PROCESS_LIMIT).expect("listening line");
    assert!(line.starts_with("orag listening on "), "{line}");
    drop(child.stdin.take());
    let status = wait_with_deadline(&mut child, PROCESS_LIMIT).expect("exits after stdin closes");
    assert_eq!(status.code(), Some(0));
    assert_lock_released(home.path());
}

#[test]
fn stdin_closed_during_startup_stops_the_sidecar() {
    let home = ephemeral_home();
    let (mut child, _lines) = spawn(home.path(), &["--exit-on-stdin-close"], Stdio::null());
    let status = wait_with_deadline(&mut child, PROCESS_LIMIT).expect("exits without serving on");
    assert_eq!(status.code(), Some(0));
    assert_lock_released(home.path());
}

#[test]
fn without_the_flag_a_closed_stdin_is_ignored() {
    let home = ephemeral_home();
    let (mut child, lines) = spawn(home.path(), &[], Stdio::null());
    lines.recv_timeout(PROCESS_LIMIT).expect("listening line");
    assert!(
        wait_with_deadline(&mut child, Duration::from_secs(2)).is_none(),
        "a service started from a shell or a service manager keeps running"
    );
    let _ = child.kill();
    let _ = child.wait();
}
