//! The downloader over real TLS against a peer that trickles one TLS record
//! a byte at a time: rustls keeps reading the socket until the record is
//! whole, so cancel and the idle limit must hold beneath it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::infer::download::catalog::RemoteFile;
use crate::infer::download::fetch::{Fetch, fetch};
use crate::infer::download::http::{HttpSettings, agent_trusting};
use crate::infer::download::status::{DownloadError, ErrorCode};

/// Body bytes sent at full speed before the trickle starts.
const FIRST: usize = 1000;
/// One TLS record's worth of body, sent a byte every `TRICKLE` (minutes).
const RECORD: usize = 16_000;
const TRICKLE: Duration = Duration::from_millis(20);

/// A TLS server for 127.0.0.1 that answers one request: headers and `FIRST`
/// bytes at once, then a record of `RECORD` bytes trickled until `stop`.
struct TrickleServer {
    url: String,
    root: ureq::tls::Certificate<'static>,
    stop: Arc<AtomicBool>,
}

impl Drop for TrickleServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn trickle_server() -> TrickleServer {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = made.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(made.signing_key.serialize_der()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert.clone()], key)
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("https://{}/model.gguf", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        if let Ok((sock, _)) = listener.accept() {
            let _ = serve(sock, Arc::new(config), &stopping);
        }
    });
    TrickleServer {
        url,
        root: ureq::tls::Certificate::from_der(&cert).to_owned(),
        stop,
    }
}

fn serve(
    mut sock: TcpStream,
    config: Arc<rustls::ServerConfig>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut conn = rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = rustls::Stream::new(&mut conn, &mut sock).read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        FIRST + RECORD
    );
    let mut stream = rustls::Stream::new(&mut conn, &mut sock);
    stream.write_all(head.as_bytes())?;
    stream.write_all(&[1u8; FIRST])?;
    stream.flush()?;
    conn.writer().write_all(&[2u8; RECORD])?;
    let mut record = Vec::new();
    while conn.wants_write() {
        conn.write_tls(&mut record)?;
    }
    for byte in record {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        sock.write_all(&[byte])?;
        std::thread::sleep(TRICKLE);
    }
    Ok(())
}

/// Fetches from `server` on a thread; returns the thread, the bytes on disk
/// so far, and the cancel flag.
fn start_fetch(
    server: &TrickleServer,
    idle_timeout: Duration,
    dir: &std::path::Path,
) -> (
    JoinHandle<Result<(), DownloadError>>,
    Arc<AtomicU64>,
    Arc<AtomicBool>,
) {
    let cancel = Arc::new(AtomicBool::new(false));
    let have = Arc::new(AtomicU64::new(0));
    let settings = HttpSettings {
        env_proxy: false,
        https_only: true,
        idle_timeout,
    };
    let agent = agent_trusting(&settings, cancel.clone(), server.root.clone());
    let (url, part) = (server.url.clone(), dir.join("model.gguf.part"));
    let (flag, seen) = (cancel.clone(), have.clone());
    let thread = std::thread::spawn(move || {
        let file = RemoteFile {
            url: url.clone(),
            name: "model.gguf".into(),
            sha256: String::new(),
            size: (FIRST + RECORD) as u64,
        };
        let progress = |bytes: u64| seen.store(bytes, Ordering::SeqCst);
        fetch(&Fetch {
            agent: &agent,
            url: &url,
            file: &file,
            part: &part,
            cancel: &flag,
            progress: &progress,
        })
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while have.load(Ordering::SeqCst) < FIRST as u64 {
        assert!(Instant::now() < deadline, "the first bytes never arrived");
        assert!(!thread.is_finished(), "the fetch ended early");
        std::thread::sleep(Duration::from_millis(5));
    }
    (thread, have, cancel)
}

/// Joins `thread` if it ends within `limit`; panics with `what` otherwise.
fn join_within(
    thread: JoinHandle<Result<(), DownloadError>>,
    limit: Duration,
    what: &str,
) -> Result<(), DownloadError> {
    let started = Instant::now();
    while !thread.is_finished() {
        assert!(started.elapsed() < limit, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
    thread.join().unwrap()
}

#[test]
fn cancel_stops_a_tls_read_that_a_peer_trickles() {
    let server = trickle_server();
    let dir = tempfile::tempdir().unwrap();
    let (thread, _, cancel) = start_fetch(&server, Duration::from_secs(60), dir.path());
    // Let the trickle start, so the client sits inside a TLS record.
    std::thread::sleep(Duration::from_millis(300));
    cancel.store(true, Ordering::SeqCst);
    let err = join_within(
        thread,
        Duration::from_secs(2),
        "cancel was not seen inside a trickled TLS record",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
}

#[test]
fn a_trickled_tls_record_does_not_reset_the_idle_limit() {
    let server = trickle_server();
    let dir = tempfile::tempdir().unwrap();
    let (thread, have, _) = start_fetch(&server, Duration::from_secs(1), dir.path());
    let err = join_within(
        thread,
        Duration::from_secs(4),
        "a trickled TLS record kept the read alive past the idle limit",
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Network, "{err}");
    assert!(err.message.contains("timed out"), "{err}");
    assert_eq!(have.load(Ordering::SeqCst), FIRST as u64);
}
