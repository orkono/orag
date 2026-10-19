//! The limits of a download connection before and around its TLS session:
//! the trickled-record checks through a CONNECT proxy (the proxy connection
//! runs the connector chain again, nesting a second watch), and a TLS
//! handshake that a peer never completes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::infer::download::status::ErrorCode;
use crate::infer::download::tls_tests::{
    Route, assert_cancel_ends_it, assert_idle_limit_ends_it, join_within, server_identity,
    spawn_fetch, start_fetch, trickle_server,
};

/// A CONNECT proxy for one tunnel: answers `200 Connection established` and
/// pipes bytes both ways. Returns its `http://host:port`.
fn connect_proxy() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((client, _)) = listener.accept() {
            let _ = tunnel(client);
        }
    });
    address
}

fn tunnel(client: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line
        .strip_prefix("CONNECT ")
        .and_then(|rest| rest.split(' ').next())
        .ok_or_else(|| std::io::Error::other(format!("not a CONNECT: {line}")))?
        .to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
    }
    let upstream = TcpStream::connect(target)?;
    let mut answer = client.try_clone()?;
    answer.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;
    let (mut up_write, mut up_read) = (upstream.try_clone()?, upstream);
    let mut down_write = client;
    // Copying from `reader` first passes on what it has already buffered.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut up_write);
        let _ = up_write.shutdown(Shutdown::Write);
    });
    std::io::copy(&mut up_read, &mut down_write)?;
    Ok(())
}

#[test]
fn through_a_connect_proxy_cancel_stops_a_trickled_tls_read() {
    let server = trickle_server();
    let proxy = connect_proxy();
    let dir = tempfile::tempdir().unwrap();
    let running = start_fetch(&server, Duration::from_secs(60), Some(&proxy), dir.path());
    assert_cancel_ends_it(running);
}

#[test]
fn through_a_connect_proxy_a_trickled_tls_record_hits_the_idle_limit() {
    let server = trickle_server();
    let proxy = connect_proxy();
    let dir = tempfile::tempdir().unwrap();
    let running = start_fetch(&server, Duration::from_secs(1), Some(&proxy), dir.path());
    assert_idle_limit_ends_it(running);
}

/// Accepts TCP, reads the ClientHello, then sends the header of a 16 KiB
/// handshake record and its body a byte every 20 ms: the handshake never
/// completes, yet a byte arrives far more often than any limit.
struct StalledHandshake {
    url: String,
    stop: Arc<AtomicBool>,
}

impl Drop for StalledHandshake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn stalled_handshake() -> StalledHandshake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://{}/model.gguf", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut hello = [0u8; 4096];
            let _ = sock.read(&mut hello);
            let _ = sock.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
            while !stopping.load(Ordering::SeqCst) && sock.write_all(&[0]).is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    });
    StalledHandshake { url, stop }
}

fn handshake_route<'a>(
    server: &'a StalledHandshake,
    root: &'a ureq::tls::Certificate<'static>,
    connect_timeout: Duration,
) -> Route<'a> {
    Route {
        url: &server.url,
        root,
        proxy: None,
        idle_timeout: Duration::from_secs(60),
        connect_timeout,
    }
}

/// A CONNECT proxy that reads the request, then sends `200 Connection
/// established` and an endless header line a byte every 20 ms: the CONNECT
/// answer never completes, yet bytes keep arriving. Stops with the guard.
struct TricklingProxy {
    address: String,
    stop: Arc<AtomicBool>,
}

impl Drop for TricklingProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn trickling_proxy() -> TricklingProxy {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut request = [0u8; 4096];
            let _ = sock.read(&mut request);
            let answer = b"HTTP/1.1 200 Connection established\r\nX-Pad: ";
            let endless = answer.iter().copied().chain(std::iter::repeat(b'a'));
            for byte in endless {
                if stopping.load(Ordering::SeqCst) || sock.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    });
    TricklingProxy { address, stop }
}

fn proxied_route<'a>(
    proxy: &'a TricklingProxy,
    root: &'a ureq::tls::Certificate<'static>,
    connect_timeout: Duration,
) -> Route<'a> {
    Route {
        // Never reached: the proxy never finishes its answer.
        url: "https://127.0.0.1:9/model.gguf",
        root,
        proxy: Some(&proxy.address),
        idle_timeout: Duration::from_secs(60),
        connect_timeout,
    }
}

#[test]
fn a_trickled_connect_answer_ends_at_the_connect_limit() {
    let proxy = trickling_proxy();
    let root = some_root();
    let dir = tempfile::tempdir().unwrap();
    let route = proxied_route(&proxy, &root, Duration::from_secs(1));
    let err = join_within(
        spawn_fetch(&route, dir.path()).thread,
        Duration::from_secs(4),
        "a trickled CONNECT answer kept the connection alive past the connect limit",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network, "{err}");
    assert!(err.message.contains("timed out"), "{err}");
}

#[test]
fn cancel_during_a_trickled_connect_answer_ends_cancelled() {
    let proxy = trickling_proxy();
    let root = some_root();
    let dir = tempfile::tempdir().unwrap();
    let route = proxied_route(&proxy, &root, Duration::from_secs(30));
    let running = spawn_fetch(&route, dir.path());
    std::thread::sleep(Duration::from_millis(300));
    running.cancel.store(true, Ordering::SeqCst);
    let err = join_within(
        running.thread,
        Duration::from_secs(2),
        "cancel was not seen during the CONNECT answer",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
}

fn some_root() -> ureq::tls::Certificate<'static> {
    ureq::tls::Certificate::from_der(&server_identity().0).to_owned()
}

#[test]
fn a_handshake_that_never_completes_ends_at_the_connect_limit() {
    let server = stalled_handshake();
    let root = some_root();
    let dir = tempfile::tempdir().unwrap();
    let route = handshake_route(&server, &root, Duration::from_secs(1));
    let err = join_within(
        spawn_fetch(&route, dir.path()).thread,
        Duration::from_secs(4),
        "a trickled handshake kept the connection alive past the connect limit",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network, "{err}");
    assert!(err.message.contains("timed out"), "{err}");
}

#[test]
fn cancel_during_a_stalled_handshake_ends_cancelled() {
    let server = stalled_handshake();
    let root = some_root();
    let dir = tempfile::tempdir().unwrap();
    let route = handshake_route(&server, &root, Duration::from_secs(30));
    let running = spawn_fetch(&route, dir.path());
    std::thread::sleep(Duration::from_millis(300));
    running.cancel.store(true, Ordering::SeqCst);
    let err = join_within(
        running.thread,
        Duration::from_secs(2),
        "cancel was not seen during the handshake",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
}
