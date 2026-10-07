mod common;

use std::path::Path;

use common::*;

#[test]
fn version_flag_prints_crate_version() {
    let out = orag().arg("--version").output().expect("run orag");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim(), format!("orag {}", orag::version::VERSION));
}

#[test]
fn version_command_prints_json() {
    let out = orag().arg("version").output().expect("run orag");
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["version"], orag::version::VERSION);
    assert_eq!(json["api"], "v1");
    assert_eq!(
        json["schema_version"],
        orag::store::migrations::SUPPORTED_SCHEMA_VERSION
    );
    let sha = json["git_sha"].as_str().expect("git_sha");
    let is_commit = sha.len() >= 12 && sha.chars().all(|c| c.is_ascii_hexdigit());
    assert!(
        sha == "unknown" || is_commit,
        "git_sha {sha:?} is neither \"unknown\" nor an abbreviated commit"
    );
    // CI builds from ORAG's own checkout, so the commit must be embedded there:
    // "unknown" in CI means build.rs stopped recognising the repository.
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        assert!(is_commit, "CI build embedded git_sha {sha:?}");
    }
}

#[test]
fn models_import_then_list() {
    let home = tempfile::tempdir().unwrap();
    let pack = home.path().join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(pack.join("m.gguf"), b"w").unwrap();
    std::fs::write(pack.join("LICENSE"), b"l").unwrap();
    let sha = "50e721e49c013f00c62cf59f2163542a9d8df02464efeb615d31051b0fddc326"; // sha256("w")
    std::fs::write(
        pack.join("orag-model.toml"),
        format!(
            "id = \"tiny\"\nrole = \"generation\"\nfile = \"m.gguf\"\nsha256 = \"{sha}\"\nlicense = \"MIT\"\n\
             license_file = \"LICENSE\"\nsource = \"s\"\nrevision = \"r\"\n\n[generation]\ncontext_tokens = 2048\n\
             max_output_tokens = 128\n"
        ),
    )
    .unwrap();
    let import = orag()
        .env("ORAG_HOME", home.path())
        .args(["models", "import"])
        .arg(&pack)
        .output()
        .unwrap();
    assert!(
        import.status.success(),
        "{}",
        String::from_utf8_lossy(&import.stderr)
    );
    let list = orag()
        .env("ORAG_HOME", home.path())
        .args(["models", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(
        stdout.contains("tiny") && stdout.contains("generation"),
        "{stdout}"
    );
}

#[test]
fn serve_starts_on_ephemeral_port_without_credentials() {
    let home = ephemeral_home();
    let (_server, addr) = start_server(home.path());
    assert!(http_get(&addr, "/v1/health", &addr).starts_with("HTTP/1.1 200"));
    let version = http_get(&addr, "/v1/version", &addr);
    assert!(version.starts_with("HTTP/1.1 200"), "{version}");
    assert!(version.contains("\"max_document_mb\":5"), "{version}");
    assert!(
        version.contains(&format!("\"bind\":\"{addr}\"")),
        "reports the real port: {version}"
    );
    assert!(http_get(&addr, "/v1/version", "evil.example").starts_with("HTTP/1.1 403"));
    assert!(
        !home.path().join("api-token").exists(),
        "no credentials are created"
    );
}

#[cfg(unix)]
#[test]
fn a_signal_shuts_down_cleanly_after_exactly_one_stdout_line() {
    for signal in ["-INT", "-TERM"] {
        let home = ephemeral_home();
        let (mut server, _addr) = start_server(home.path());
        let pid = server.child.id().to_string();
        assert!(orag_kill(signal, &pid).success());
        let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
            .unwrap_or_else(|| panic!("{signal}: still running; stderr: {}", server.stderr()));
        assert!(
            status.success(),
            "{signal}: {status}; stderr: {}",
            server.stderr()
        );
        let extra: Vec<String> = server.stdout.iter().collect();
        assert!(
            extra.is_empty(),
            "{signal}: more stdout after the listening line: {extra:?}"
        );
        assert!(
            server.stderr().contains("shutting down"),
            "{}",
            server.stderr()
        );
    }
}

#[test]
fn config_is_read_only_at_startup() {
    let home = ephemeral_home();
    let (_server, addr) = start_server(home.path());
    std::fs::write(
        home.path().join("config.toml"),
        "bind = \"127.0.0.1:0\"\nmax_document_mb = 10\n",
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let version = http_get(&addr, "/v1/version", &addr);
    assert!(
        version.contains("\"max_document_mb\":5"),
        "a running server must not reload config: {version}"
    );
}

#[test]
fn invalid_config_stops_startup_with_the_reason() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.toml"), "max_document_mb = 50\n").unwrap();
    let out = orag()
        .env("ORAG_HOME", home.path())
        .args(["serve", "--dev-fake-models"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("between 1 and 10"));
}

#[test]
fn a_busy_port_is_reported_before_models_load() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let home = tempfile::tempdir().unwrap();
    let bind = taken.local_addr().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        format!("bind = \"{bind}\"\n"),
    )
    .unwrap();
    // No models are installed: binding must fail first, not the model load.
    let out = orag()
        .env("ORAG_HOME", home.path())
        .arg("serve")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&format!("binding {bind}")), "{stderr}");
}

/// Starts a server with the web UI on; returns it with the API and UI addresses.
fn start_server_with_ui() -> (tempfile::TempDir, Server, String, String) {
    let home = ephemeral_home_with_ui();
    let (server, addr) = start_server(home.path());
    let line = server
        .stdout
        .recv_timeout(PROCESS_LIMIT)
        .unwrap_or_else(|_| panic!("no ui line; stderr: {}", server.stderr()));
    let ui = line
        .strip_prefix("orag ui on http://")
        .unwrap_or_else(|| panic!("unexpected second line: {line}"))
        .to_string();
    (home, server, addr, ui)
}

fn query_with_origin(addr: &str, origin: &str) -> String {
    let body = r#"{"query":"iade"}"#;
    http_send(
        addr,
        &format!(
            "POST /v1/collections/1/query HTTP/1.1\r\nHost: {addr}\r\nOrigin: {origin}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[test]
fn serve_prints_the_ui_line_after_the_api_line_and_serves_the_page() {
    let (_home, _server, addr, ui) = start_server_with_ui();
    assert_ne!(addr, ui);
    let page = http_get(&ui, "/", &ui);
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.contains("content-security-policy: default-src 'self'"),
        "{page}"
    );
    assert!(page.contains("<!doctype html>"), "{page}");
    assert!(http_get(&ui, "/app.js", &ui).starts_with("HTTP/1.1 200"));
    assert!(http_get(&ui, "/", "evil.example").starts_with("HTTP/1.1 403"));
    let version = http_get(&addr, "/v1/version", &addr);
    assert!(
        version.contains(&format!("\"ui_bind\":\"{ui}\"")),
        "reports the real UI port: {version}"
    );
}

#[test]
fn the_page_may_call_the_api_and_other_origins_may_not() {
    let (_home, _server, _addr, ui) = start_server_with_ui();
    let port = ui.rsplit(':').next().unwrap();
    for origin in [format!("http://{ui}"), format!("http://localhost:{port}")] {
        let reply = query_with_origin(&ui, &origin);
        assert!(reply.starts_with("HTTP/1.1 200"), "{origin}: {reply}");
    }
    for origin in ["https://evil.example", "http://localhost:7613", "null"] {
        let reply = query_with_origin(&ui, origin);
        assert!(reply.starts_with("HTTP/1.1 403"), "{origin}: {reply}");
        assert!(reply.contains("forbidden_origin"), "{origin}: {reply}");
    }
}

#[test]
fn ui_off_prints_only_the_api_line() {
    let home = ephemeral_home();
    let (server, addr) = start_server(home.path());
    assert!(http_get(&addr, "/v1/health", &addr).starts_with("HTTP/1.1 200"));
    assert!(
        server
            .stdout
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "no ui line when ui_bind = \"off\""
    );
    assert!(http_get(&addr, "/v1/version", &addr).contains("\"ui_bind\":\"off\""));
}

#[cfg(unix)]
#[test]
fn a_signal_stops_both_listeners() {
    let (_home, mut server, _addr, ui) = start_server_with_ui();
    // An idle keep-alive connection on the UI port must not hold up the exit.
    let mut idle = std::net::TcpStream::connect(&ui).unwrap();
    std::io::Write::write_all(
        &mut idle,
        format!("GET /app.css HTTP/1.1\r\nHost: {ui}\r\n\r\n").as_bytes(),
    )
    .unwrap();
    assert!(orag_kill("-TERM", &server.child.id().to_string()).success());
    let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
        .unwrap_or_else(|| panic!("still running; stderr: {}", server.stderr()));
    assert!(status.success(), "{status}; stderr: {}", server.stderr());
    let extra: Vec<String> = server.stdout.iter().collect();
    assert!(extra.is_empty(), "more stdout after the ui line: {extra:?}");
}

#[test]
fn a_busy_ui_port_is_reported_before_models_load() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ui = taken.local_addr().unwrap();
    let home = home_with_config(&format!("bind = \"127.0.0.1:0\"\nui_bind = \"{ui}\"\n"));
    // No models are installed: binding must fail first, not the model load.
    let out = orag()
        .env("ORAG_HOME", home.path())
        .arg("serve")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("binding the web UI to {ui}")),
        "{stderr}"
    );
    assert!(stderr.contains("ui_bind"), "says how to fix it: {stderr}");
}

#[test]
fn second_instance_on_the_same_home_is_refused() {
    let home = ephemeral_home();
    let (_server, _addr) = start_server(home.path());
    let mut second = spawn_serve(home.path(), &["--dev-fake-models"]);
    let status = wait_with_deadline(&mut second.child, PROCESS_LIMIT)
        .expect("the second instance must exit, not serve");
    assert!(!status.success());
    assert!(
        second.stderr().contains("another orag instance"),
        "{}",
        second.stderr()
    );
}

fn backup(home: &Path, dest: &Path) -> std::process::Output {
    orag()
        .env("ORAG_HOME", home)
        .arg("backup")
        .arg(dest)
        .output()
        .unwrap()
}

fn collection_names(db: &Path) -> Vec<String> {
    let store = orag::store::Store::open(db).unwrap();
    store
        .list_collections()
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

#[test]
fn backup_writes_a_consistent_copy_and_refuses_overwrite() {
    let home = tempfile::tempdir().unwrap();
    let store = orag::store::Store::open(&home.path().join("orag.db")).unwrap();
    store.create_collection("arsiv").unwrap();
    drop(store);
    let dest = home.path().join("backup.db");
    let first = backup(home.path(), &dest);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let written = std::fs::read(&dest).unwrap();
    let second = backup(home.path(), &dest);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        written,
        "an existing backup is never touched"
    );
    assert!(collection_names(&dest).contains(&"arsiv".to_string()));
}

#[test]
fn backup_of_a_missing_home_fails_and_creates_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let home = parent.path().join("yanlis-yol");
    let dest = parent.path().join("backup.db");
    let out = backup(&home, &dest);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no database"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.exists(), "backup must not create a home");
    assert!(!dest.exists(), "no empty backup may be written");
}

#[test]
fn backup_while_serving_is_a_usable_copy() {
    let home = ephemeral_home();
    let (_server, _addr) = start_server(home.path());
    let dest = home.path().join("live.db");
    let out = backup(home.path(), &dest);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(collection_names(&dest).contains(&"default".to_string()));
}

#[cfg(feature = "llama")]
#[test]
fn serve_without_installed_models_explains_how_to_fix() {
    let home = ephemeral_home();
    let out = orag()
        .env("ORAG_HOME", home.path())
        .arg("serve")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("orag models import"), "{stderr}");
}

#[test]
fn eval_retrieval_runs_on_seed_set() {
    let home = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
    let out = orag()
        .env("ORAG_HOME", home.path())
        .args(["eval", "retrieval", "--dev-fake-models", "--corpus"])
        .arg(root.join("corpus/seed"))
        .arg("--dataset")
        .arg(root.join("datasets/seed.jsonl"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("| hybrid |")
    );
}

#[test]
fn eval_answers_compares_samplers_and_rejects_unknown_ones() {
    let home = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
    let run = |sampler: &str| {
        orag()
            .env("ORAG_HOME", home.path())
            .args(["eval", "answers", "--dev-fake-models", "--corpus"])
            .arg(root.join("corpus/ceza"))
            .arg("--dataset")
            .arg(root.join("datasets/answers-ceza-tr.jsonl"))
            .args(["--sampler", sampler])
            .output()
            .unwrap()
    };
    let out = run("greedy,qwen:7");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("| greedy |") && stdout.contains("| qwen:7 |"),
        "{stdout}"
    );
    let default = orag()
        .env("ORAG_HOME", home.path())
        .args(["eval", "answers", "--dev-fake-models", "--corpus"])
        .arg(root.join("corpus/ceza"))
        .arg("--dataset")
        .arg(root.join("datasets/answers-ceza-tr.jsonl"))
        .output()
        .unwrap();
    let stdout = String::from_utf8(default.stdout).unwrap();
    assert!(
        stdout.contains("| dry |"),
        "defaults to the served sampler: {stdout}"
    );
    let bad = run("beam");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown sampler"));
    assert!(
        !home.path().join("orag.db").exists(),
        "eval must not touch ORAG_HOME"
    );
}

fn eval_seed(home: &Path, extra: &[&std::ffi::OsStr]) -> std::process::Output {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
    orag()
        .env("ORAG_HOME", home)
        .args(["eval", "retrieval", "--dev-fake-models", "--corpus"])
        .arg(root.join("corpus/seed"))
        .arg("--dataset")
        .arg(root.join("datasets/seed.jsonl"))
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn eval_with_fake_models_writes_json_and_leaves_orag_home_alone() {
    let parent = tempfile::tempdir().unwrap();
    let home = parent.path().join("home");
    let report = parent.path().join("report.json");
    let out = eval_seed(&home, &["--out".as_ref(), report.as_os_str()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(json["strategies"].as_array().unwrap().len(), 3);
    assert_eq!(json["answerable"], 14);
    assert!(!home.exists(), "eval must not create ORAG_HOME");
}

#[test]
fn eval_refuses_to_overwrite_out_before_running() {
    let home = tempfile::tempdir().unwrap();
    let report = home.path().join("taken.json");
    std::fs::write(&report, "keep").unwrap();
    let out = eval_seed(home.path(), &["--out".as_ref(), report.as_os_str()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("taken.json"));
    assert!(out.stdout.is_empty(), "nothing may run first");
    assert_eq!(std::fs::read_to_string(&report).unwrap(), "keep");
}

#[test]
fn eval_errors_name_the_missing_path() {
    let home = tempfile::tempdir().unwrap();
    for (flag, missing) in [("--corpus", "/nope-corpus"), ("--dataset", "/nope.jsonl")] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval");
        let corpus = root.join("corpus/seed");
        let dataset = root.join("datasets/seed.jsonl");
        let mut args: Vec<std::ffi::OsString> = vec![
            "eval".into(),
            "retrieval".into(),
            "--dev-fake-models".into(),
        ];
        for (name, value) in [
            ("--corpus", corpus.as_os_str()),
            ("--dataset", dataset.as_os_str()),
        ] {
            args.push(name.into());
            args.push(
                if name == flag {
                    missing.as_ref()
                } else {
                    value
                }
                .into(),
            );
        }
        let out = orag()
            .env("ORAG_HOME", home.path())
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(missing), "{flag}: {stderr}");
    }
}

#[test]
fn eval_vector_scale_small_run_prints_a_result_and_rejects_bad_sizes() {
    let parent = tempfile::tempdir().unwrap();
    let home = parent.path().join("home");
    let work = parent.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let run = |args: &[&str]| {
        orag()
            .env("ORAG_HOME", &home)
            .args(["eval", "vector-scale", "--dimensions", "16"])
            .args(args)
            .output()
            .unwrap()
    };
    let work_arg = work.to_str().unwrap();
    let out = run(&["--chunks", "500", "--queries", "20", "--work-dir", work_arg]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("| 500 | 16 | 50 | 20 |") && stdout.contains("| PASS |"),
        "{stdout}"
    );
    assert!(!home.exists(), "the benchmark must not create ORAG_HOME");
    assert_eq!(
        std::fs::read_dir(&work).unwrap().count(),
        0,
        "the work dir is removed"
    );
    // A failed gate is a non-zero exit with the reason, after the table.
    let failed = run(&["--chunks", "500", "--target-p95-ms", "0"]);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stdout).contains("| FAIL |"));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("exceeds the 0 ms target"));
    for bad in [["--chunks", "0"], ["--queries", "3"]] {
        let out = run(&bad);
        assert!(!out.status.success(), "{bad:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("must be"), "{bad:?}: {stderr}");
    }
}

#[test]
fn eval_retrieval_fails_below_the_recall_floor_after_printing() {
    let home = tempfile::tempdir().unwrap();
    let pass = eval_seed(
        home.path(),
        &["--min-recall-at-10".as_ref(), "0.5".as_ref()],
    );
    assert!(
        pass.status.success(),
        "{}",
        String::from_utf8_lossy(&pass.stderr)
    );
    let fail = eval_seed(
        home.path(),
        &["--min-recall-at-10".as_ref(), "1.0".as_ref()],
    );
    assert!(!fail.status.success());
    assert!(String::from_utf8_lossy(&fail.stdout).contains("| hybrid |"));
    let stderr = String::from_utf8_lossy(&fail.stderr);
    assert!(
        stderr.contains("hybrid recall@10") && stderr.contains("below 1"),
        "{stderr}"
    );
    let bad = eval_seed(
        home.path(),
        &["--min-recall-at-10".as_ref(), "1.5".as_ref()],
    );
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("between 0 and 1"));
}

#[test]
fn eval_retrieval_requires_named_queries_in_the_context() {
    let home = tempfile::tempdir().unwrap();
    let report = home.path().join("r.json");
    let first = eval_seed(home.path(), &["--out".as_ref(), report.as_os_str()]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let hybrid = &json["strategies"][2];
    assert_eq!(hybrid["strategy"], "hybrid");
    let missed: Vec<String> = serde_json::from_value(hybrid["missed_in_context"].clone()).unwrap();
    let all_ids = [
        "tr-001", "tr-002", "tr-003", "tr-004", "tr-005", "tr-006", "en-001", "en-002", "en-003",
        "en-004", "en-005", "x-001", "x-002", "x-003",
    ];
    let found = all_ids
        .iter()
        .find(|id| !missed.iter().any(|m| m == *id))
        .unwrap();
    let ok = eval_seed(
        home.path(),
        &["--require-in-context".as_ref(), found.as_ref()],
    );
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    // The failing path must always be exercised: if the toy embedder ever
    // stops missing a seed question, pick another failing case here.
    let miss = missed
        .first()
        .expect("the fake hybrid run must miss a seed question for this test");
    let fail = eval_seed(
        home.path(),
        &["--require-in-context".as_ref(), miss.as_ref()],
    );
    assert!(!fail.status.success());
    assert!(String::from_utf8_lossy(&fail.stderr).contains(miss.as_str()));
    // Unknown and unanswerable ids are mistakes in the gate, refused before the run.
    for bad in ["nope", "u-001"] {
        let out = eval_seed(
            home.path(),
            &["--require-in-context".as_ref(), bad.as_ref()],
        );
        assert!(!out.status.success(), "{bad}");
        assert!(out.stdout.is_empty(), "{bad}: refused before running");
        assert!(String::from_utf8_lossy(&out.stderr).contains(bad), "{bad}");
    }
}
