use std::process::Command;

fn orag() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orag"))
}

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
