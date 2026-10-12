//! `orag serve` without installed models: setup mode, import from the page,
//! and the restart exit code `orag start` relies on (D-023).

mod common;

#[cfg(feature = "llama")]
mod setup_mode {
    use std::path::Path;

    use crate::common::{
        PROCESS_LIMIT, ephemeral_home, http_get, http_send, spawn_serve, spawn_serve_with,
        wait_with_deadline,
    };

    const EMBEDDING: &str = "qwen3-embedding-0.6b-q8_0";
    const GENERATION: &str = "qwen3.5-4b-q4_k_m";

    fn listening_addr(server: &crate::common::Server) -> String {
        let line = server
            .stdout
            .recv_timeout(PROCESS_LIMIT)
            .unwrap_or_else(|_| panic!("no listening line; stderr: {}", server.stderr()));
        line.strip_prefix("orag listening on http://")
            .unwrap_or_else(|| panic!("unexpected: {line}"))
            .to_string()
    }

    /// A valid pack for `id` at `dir` (tiny fake weights, matching SHA-256).
    fn write_pack(dir: &Path, role: &str, id: &str) {
        use sha2::Digest;
        std::fs::create_dir_all(dir).unwrap();
        let payload = format!("weights of {id}");
        std::fs::write(dir.join("model.gguf"), &payload).unwrap();
        std::fs::write(dir.join("LICENSE"), "Apache-2.0 text").unwrap();
        let sha = hex::encode(sha2::Sha256::digest(payload.as_bytes()));
        let section = if role == "embedding" {
            "[embedding]\ndimensions = 16\npooling = \"mean\"\nmax_tokens = 128\n"
        } else {
            "[generation]\ncontext_tokens = 2048\nmax_output_tokens = 256\n"
        };
        std::fs::write(
            dir.join("orag-model.toml"),
            format!(
                "id = \"{id}\"\nrole = \"{role}\"\nfile = \"model.gguf\"\nsha256 = \"{sha}\"\n\
                 license = \"Apache-2.0\"\nlicense_file = \"LICENSE\"\nsource = \"test\"\nrevision = \"r1\"\n\n{section}"
            ),
        )
        .unwrap();
    }

    fn import(addr: &str, folder: &Path) -> String {
        let body = serde_json::json!({ "path": folder }).to_string();
        http_send(
            addr,
            &format!(
                "POST /v1/models/import HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    #[test]
    fn serve_without_models_starts_in_setup_mode() {
        let home = ephemeral_home();
        let server = spawn_serve(home.path(), &[]);
        let addr = listening_addr(&server);
        let models = http_get(&addr, "/v1/models", &addr);
        assert!(models.contains("\"mode\":\"setup\""), "{models}");
        assert!(
            models.contains(EMBEDDING) && models.contains(GENERATION),
            "{models}"
        );
        let query = http_send(
            &addr,
            &format!(
                "POST /v1/collections/1/query HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
                 Content-Length: 13\r\nConnection: close\r\n\r\n{{\"query\":\"x\"}}"
            ),
        );
        assert!(query.starts_with("HTTP/1.1 503"), "{query}");
        assert!(query.contains("models_missing"), "{query}");
        // Without models `serve` no longer exits (D-023); the log still says why
        // and how to fix it from the command line.
        let log = server.stderr();
        assert!(log.contains("setup mode"), "the log says why: {log}");
        assert!(
            log.contains("orag models import"),
            "and how to fix it: {log}"
        );
    }

    #[test]
    fn a_broken_installed_model_still_stops_startup() {
        // Only a missing model folder means setup mode; a corrupt manifest must
        // not be hidden behind the setup page.
        let home = ephemeral_home();
        let broken = home.path().join("models").join(EMBEDDING);
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("orag-model.toml"), "id = ").unwrap();
        let mut server = spawn_serve(home.path(), &[]);
        let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
            .unwrap_or_else(|| panic!("still running; stderr: {}", server.stderr()));
        assert!(!status.success());
        assert!(server.stderr().contains(EMBEDDING), "{}", server.stderr());
    }

    #[test]
    fn a_supervised_import_that_ends_setup_exits_with_the_restart_code() {
        let packs = tempfile::tempdir().unwrap();
        write_pack(&packs.path().join("emb"), "embedding", EMBEDDING);
        write_pack(&packs.path().join("gen"), "generation", GENERATION);

        let home = ephemeral_home();
        let mut server = spawn_serve_with(home.path(), &[], &[("ORAG_SUPERVISED", "1")]);
        let addr = listening_addr(&server);
        let response = import(&addr, packs.path());
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("\"restart\":\"automatic\""), "{response}");
        let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
            .unwrap_or_else(|| panic!("did not exit; stderr: {}", server.stderr()));
        assert_eq!(status.code(), Some(75), "stderr: {}", server.stderr());
    }

    #[test]
    fn an_unsupervised_import_asks_for_a_manual_restart_and_keeps_serving() {
        let packs = tempfile::tempdir().unwrap();
        write_pack(&packs.path().join("emb"), "embedding", EMBEDDING);
        write_pack(&packs.path().join("gen"), "generation", GENERATION);

        let home = ephemeral_home();
        let server = spawn_serve(home.path(), &[]);
        let addr = listening_addr(&server);
        let response = import(&addr, packs.path());
        assert!(response.contains("\"restart\":\"manual\""), "{response}");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(http_get(&addr, "/v1/health", &addr).starts_with("HTTP/1.1 200"));
    }
}

#[cfg(not(feature = "llama"))]
#[test]
fn a_build_without_a_backend_does_not_offer_setup() {
    use common::{PROCESS_LIMIT, ephemeral_home, spawn_serve, wait_with_deadline};
    // Importing packs would not help a build that cannot run them.
    let home = ephemeral_home();
    let mut server = spawn_serve(home.path(), &[]);
    let status = wait_with_deadline(&mut server.child, PROCESS_LIMIT)
        .unwrap_or_else(|| panic!("still running; stderr: {}", server.stderr()));
    assert!(!status.success());
    assert!(
        server.stderr().contains("no inference backend"),
        "{}",
        server.stderr()
    );
}
