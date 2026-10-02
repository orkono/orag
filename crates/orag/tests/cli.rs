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
