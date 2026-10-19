//! The downloader against a local HTTP server: no test touches the internet.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::*;
use crate::infer::models::{ModelRole, find_model, verify_model};

/// How the test server answers a path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ranges {
    Honour,
    Ignore,
}

struct Route {
    body: Vec<u8>,
    status: u16,
    /// Send this many bytes, then wait for the gate before the rest.
    gate: Option<(usize, Arc<Mutex<Receiver<()>>>)>,
}

/// `(path, Range header)` of every request.
type RequestLog = Mutex<Vec<(String, Option<String>)>>;

/// A one-thread-per-connection HTTP/1.1 server serving fixed bodies.
struct Server {
    base: String,
    /// `(path, Range header)` of every request.
    requests: Arc<RequestLog>,
}

impl Server {
    fn start(routes: HashMap<String, Route>, ranges: Ranges) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(routes);
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (routes, log) = (routes.clone(), log.clone());
                std::thread::spawn(move || serve(stream, &routes, ranges, &log));
            }
        });
        Server { base, requests }
    }

    fn ranges_asked(&self) -> Vec<Option<String>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }
}

fn serve(stream: TcpStream, routes: &HashMap<String, Route>, ranges: Ranges, log: &RequestLog) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_string());
        }
    }
    log.lock().unwrap().push((path.clone(), range.clone()));
    let mut out = stream;
    let Some(route) = routes.get(&path) else {
        let _ = out
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let start = match (ranges, &range) {
        (Ranges::Honour, Some(r)) => r
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap(),
        _ => 0usize,
    };
    let (status, body) = (
        if start > 0 { 206 } else { route.status },
        &route.body[start..],
    );
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if status == 206 {
        head += &format!(
            "Content-Range: bytes {start}-{}/{}\r\n",
            route.body.len() - 1,
            route.body.len()
        );
    }
    let _ = out.write_all(format!("{head}\r\n").as_bytes());
    match &route.gate {
        Some((first, gate)) => {
            let _ = out.write_all(&body[..*first]);
            let _ = out.flush();
            let _ = gate.lock().unwrap().recv();
            let _ = out.write_all(&body[*first..]);
        }
        None => {
            let _ = out.write_all(body);
        }
    }
}

const WEIGHTS: &str = "/test/emb/resolve/r1/model.gguf";
const LICENSE: &str = "/licenses/emb/LICENSE";

fn payload() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn test_entry(weights: &[u8], license: &[u8]) -> CatalogEntry {
    CatalogEntry {
        id: "emb".into(),
        role: ModelRole::Embedding,
        repo: "test/emb".into(),
        revision: "r1".into(),
        file: "model.gguf".into(),
        sha256: sha(weights),
        size: weights.len() as u64,
        license_url: format!("https://www.example.org{LICENSE}"),
        license_sha256: sha(license),
        license_size: license.len() as u64,
        role_section: "[embedding]\ndimensions = 16\npooling = \"mean\"\nmax_tokens = 128".into(),
    }
}

fn routes(weights: Route) -> HashMap<String, Route> {
    HashMap::from([
        (WEIGHTS.to_string(), weights),
        (
            LICENSE.to_string(),
            Route {
                body: b"license text".to_vec(),
                status: 200,
                gate: None,
            },
        ),
    ])
}

fn plain(body: Vec<u8>) -> Route {
    Route {
        body,
        status: 200,
        gate: None,
    }
}

fn job(models_dir: &Path, entry: CatalogEntry) -> Job {
    let job = Job {
        models_dir: models_dir.to_path_buf(),
        models: vec![entry],
        status: SharedStatus::default(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    job.status.set(job.initial_status());
    job
}

fn part(models_dir: &Path) -> PathBuf {
    models_dir
        .join(DOWNLOAD_DIR)
        .join("emb")
        .join("model.gguf.part")
}

#[test]
fn a_fresh_download_installs_a_model_like_an_import() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    run(&Downloader::with_base_url(&server.base), &job).unwrap();
    let model = find_model(dir.path(), "emb", ModelRole::Embedding).unwrap();
    assert_eq!(model.manifest.source, "https://huggingface.co/test/emb");
    verify_model(dir.path(), "emb").unwrap();
    assert_eq!(
        std::fs::read(model.dir.join("LICENSE")).unwrap(),
        b"license text"
    );
    assert!(
        !dir.path().join(DOWNLOAD_DIR).exists(),
        "the download folder is cleaned up"
    );
    let progress = &job.status.snapshot().models[0];
    assert_eq!(progress.state, ModelState::Installed);
    assert_eq!(
        (progress.received, progress.total),
        (progress.total, 300_012)
    );
    assert_eq!(server.ranges_asked(), [None, None]);
}

#[test]
fn a_part_file_is_resumed_with_a_range_request() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(part(dir.path()).parent().unwrap()).unwrap();
    std::fs::write(part(dir.path()), &payload()[..100_000]).unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    run(&Downloader::with_base_url(&server.base), &job).unwrap();
    verify_model(dir.path(), "emb").unwrap();
    assert_eq!(server.ranges_asked()[0].as_deref(), Some("bytes=100000-"));
}

#[test]
fn a_server_that_ignores_the_range_restarts_the_file() {
    let server = Server::start(routes(plain(payload())), Ranges::Ignore);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(part(dir.path()).parent().unwrap()).unwrap();
    std::fs::write(part(dir.path()), vec![7u8; 100_000]).unwrap(); // not a prefix
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    run(&Downloader::with_base_url(&server.base), &job).unwrap();
    verify_model(dir.path(), "emb").unwrap();
    assert_eq!(server.ranges_asked()[0].as_deref(), Some("bytes=100000-"));
}

#[test]
fn a_checksum_mismatch_removes_the_part_file() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let mut entry = test_entry(&payload(), b"license text");
    entry.sha256 = "0".repeat(64);
    let job = job(dir.path(), entry);
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Checksum, "{err}");
    assert!(
        err.message.contains("model.gguf") && err.message.contains("127.0.0.1"),
        "{err}"
    );
    assert!(!part(dir.path()).exists(), "the bad .part must be removed");
    assert!(!dir.path().join("emb").exists());
    assert_eq!(job.status.snapshot().models[0].state, ModelState::Failed);
}

#[test]
fn an_http_error_names_the_host_and_file() {
    let server = Server::start(HashMap::new(), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Http, "{err}");
    assert!(
        err.message.contains("404") && err.message.contains("model.gguf"),
        "{err}"
    );
}

#[test]
fn an_unreachable_host_is_a_network_error() {
    let unused = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", unused.local_addr().unwrap());
    drop(unused);
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let err = run(&Downloader::with_base_url(base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Network, "{err}");
}

#[test]
fn cancelling_keeps_the_part_file_for_the_next_start() {
    let (open, gate) = channel();
    let weights = Route {
        body: payload(),
        status: 200,
        gate: Some((100_000, Arc::new(Mutex::new(gate)))),
    };
    let server = Server::start(routes(weights), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let (downloader, running) = (Downloader::with_base_url(&server.base), job.clone());
    let thread = std::thread::spawn(move || run(&downloader, &running));
    let deadline = Instant::now() + Duration::from_secs(10);
    while job.status.snapshot().models[0].received < 100_000 {
        assert!(Instant::now() < deadline, "the first bytes never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    job.cancel.store(true, Ordering::SeqCst);
    open.send(()).unwrap();
    let err = thread.join().unwrap().unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
    let kept = std::fs::metadata(part(dir.path())).unwrap().len();
    assert!(kept >= 100_000, "kept {kept} bytes");
    assert_eq!(job.status.snapshot().models[0].state, ModelState::Waiting);
    assert!(!dir.path().join("emb").exists());
}

#[test]
fn the_base_url_replaces_scheme_and_host_only() {
    let downloader = Downloader::with_base_url("http://127.0.0.1:9/");
    assert_eq!(
        downloader.url("https://huggingface.co/a/resolve/r/f.gguf"),
        "http://127.0.0.1:9/a/resolve/r/f.gguf"
    );
    let real = Downloader::production();
    assert_eq!(
        real.url("https://huggingface.co/a"),
        "https://huggingface.co/a"
    );
    assert!(real.http.env_proxy && real.http.https_only);
}

fn license_part(models_dir: &Path) -> PathBuf {
    models_dir
        .join(DOWNLOAD_DIR)
        .join("emb")
        .join("LICENSE.part")
}

#[test]
fn cancelling_a_stalled_download_stops_within_two_seconds() {
    // The sender lives until the end of the test: the peer never sends the rest.
    let (_never, gate) = channel::<()>();
    let weights = Route {
        body: payload(),
        status: 200,
        gate: Some((100_000, Arc::new(Mutex::new(gate)))),
    };
    let server = Server::start(routes(weights), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let (downloader, running) = (Downloader::with_base_url(&server.base), job.clone());
    let thread = std::thread::spawn(move || run(&downloader, &running));
    let deadline = Instant::now() + Duration::from_secs(10);
    while job.status.snapshot().models[0].received < 100_000 {
        assert!(Instant::now() < deadline, "the first bytes never arrived");
        std::thread::sleep(Duration::from_millis(5));
    }
    job.cancel.store(true, Ordering::SeqCst);
    let cancelled_at = Instant::now();
    while !thread.is_finished() {
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(2),
            "the download kept waiting on a stalled peer after cancel"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let err = thread.join().unwrap().unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
    assert_eq!(job.status.snapshot().models[0].state, ModelState::Waiting);
    assert!(std::fs::metadata(part(dir.path())).unwrap().len() >= 100_000);
}

#[test]
fn cancel_with_every_file_already_downloaded_does_not_install() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(part(dir.path()).parent().unwrap()).unwrap();
    std::fs::write(part(dir.path()), payload()).unwrap();
    std::fs::write(license_part(dir.path()), b"license text").unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    job.cancel.store(true, Ordering::SeqCst);
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
    assert!(
        !dir.path().join("emb").exists(),
        "a cancelled run installed"
    );
    assert_eq!(std::fs::read(part(dir.path())).unwrap(), payload());
    assert_eq!(job.status.snapshot().models[0].state, ModelState::Waiting);
    assert!(server.ranges_asked().is_empty(), "nothing needed fetching");
}

#[cfg(unix)]
#[test]
fn a_part_file_linked_outside_the_models_folder_is_refused() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let personal = outside.path().join("personal.txt");
    let original = b"my own notes, not a model".repeat(4);
    std::fs::write(&personal, &original).unwrap();
    std::fs::create_dir_all(part(dir.path()).parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&personal, part(dir.path())).unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Disk, "{err}");
    assert!(err.message.contains("model.gguf.part"), "{err}");
    assert_eq!(std::fs::read(&personal).unwrap(), original);
    assert!(!dir.path().join("emb").exists());
}

#[cfg(unix)]
#[test]
fn a_download_folder_linked_outside_the_models_folder_is_refused() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("emb")).unwrap();
    std::fs::write(outside.path().join("emb").join("keep.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(DOWNLOAD_DIR)).unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Disk, "{err}");
    assert!(err.message.contains(DOWNLOAD_DIR), "{err}");
    assert_eq!(
        std::fs::read(outside.path().join("emb").join("keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read_dir(outside.path().join("emb"))
            .unwrap()
            .count(),
        1
    );
    assert!(!dir.path().join("emb").exists());
}

#[cfg(unix)]
#[test]
fn a_model_download_folder_linked_outside_is_refused() {
    let server = Server::start(routes(plain(payload())), Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
    std::fs::create_dir(dir.path().join(DOWNLOAD_DIR)).unwrap();
    let linked = dir.path().join(DOWNLOAD_DIR).join("emb");
    std::os::unix::fs::symlink(outside.path(), &linked).unwrap();
    let job = job(dir.path(), test_entry(&payload(), b"license text"));
    let err = run(&Downloader::with_base_url(&server.base), &job).unwrap_err();
    assert_eq!(err.code, ErrorCode::Disk, "{err}");
    assert_eq!(
        std::fs::read(outside.path().join("keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
    assert!(!dir.path().join("emb").exists());
}

#[test]
fn two_models_in_one_run_both_install() {
    let mut served = routes(plain(payload()));
    served.insert("/test/emb2/resolve/r1/model.gguf".into(), plain(payload()));
    served.insert(
        "/licenses/emb2/LICENSE".into(),
        plain(b"license text".to_vec()),
    );
    let server = Server::start(served, Ranges::Honour);
    let dir = tempfile::tempdir().unwrap();
    let mut second = test_entry(&payload(), b"license text");
    second.id = "emb2".into();
    second.repo = "test/emb2".into();
    second.license_url = "https://www.example.org/licenses/emb2/LICENSE".into();
    let mut job = job(dir.path(), test_entry(&payload(), b"license text"));
    job.models.push(second);
    job.status.set(job.initial_status());
    run(&Downloader::with_base_url(&server.base), &job).unwrap();
    verify_model(dir.path(), "emb").unwrap();
    verify_model(dir.path(), "emb2").unwrap();
    assert!(!dir.path().join(DOWNLOAD_DIR).exists());
}
