//! Embeds the git commit as `ORAG_GIT_SHA` ("unknown" outside ORAG's own checkout).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs git in `root`. Returns stdout lines, or git's stderr on failure.
fn git(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|err| format!("cannot run git: {err}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// ORAG's own checkout: `.git` next to the workspace manifest that names
/// this package's repository (`CARGO_PKG_REPOSITORY`, inherited from it). A
/// crate vendored into another repository or unpacked from a package does
/// not, and reports "unknown" without calling git.
fn is_own_checkout(root: &Path) -> bool {
    let repository = std::env::var("CARGO_PKG_REPOSITORY").unwrap_or_default();
    !repository.is_empty()
        && root.join(".git").exists()
        && std::fs::read_to_string(root.join("Cargo.toml")).is_ok_and(|manifest| {
            manifest
                .lines()
                .any(|line| line.trim() == format!("repository = \"{repository}\""))
        })
}

fn main() {
    link_clang_runtime_on_macos();
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest_dir.join("../..");
    // The checkout test reads the workspace manifest, so a change there reruns
    // it. (Any `rerun-if-changed` also turns off Cargo's rerun-on-any-change.)
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    if !is_own_checkout(&root) {
        if root.join(".git").exists() {
            // A checkout whose manifest no longer names this repository (a fork
            // that changed `repository`): say why the commit is not embedded.
            println!(
                "cargo:warning=git_sha is \"unknown\": the workspace Cargo.toml does not name this package's repository"
            );
        }
        println!("cargo:rustc-env=ORAG_GIT_SHA=unknown");
        return;
    }
    match commit_and_watch_paths(&root) {
        Ok((sha, paths)) => {
            println!("cargo:rustc-env=ORAG_GIT_SHA={sha}");
            for path in paths {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        Err(err) => {
            // Inside our own checkout git must work; say why it did not.
            println!("cargo:warning=git_sha is \"unknown\": {err}");
            println!("cargo:rustc-env=ORAG_GIT_SHA=unknown");
        }
    }
}

/// llama.cpp's Metal code uses `@available`, which compiles to calls to
/// `__isPlatformVersionAtLeast` (the deployment target, 14.0, is older than the
/// APIs it checks). Rust links with `-nodefaultlibs`, so clang's runtime is not
/// linked; Rust's own builtins cover the symbol only without LTO, and the
/// release profile's thin LTO drops it ("Undefined symbols ... ___isPlatformVersionAtLeast").
/// Linking clang's runtime archive explicitly fixes both profiles: an archive
/// only provides symbols that are still undefined.
fn link_clang_runtime_on_macos() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || std::env::var_os("CARGO_FEATURE_LLAMA").is_none()
    {
        return;
    }
    // The archive's path changes with the toolchain: rerun when it does.
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=SDKROOT");
    let found = Command::new("xcrun")
        .args(["clang", "-print-file-name=libclang_rt.osx.a"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
        .filter(|path| path.is_absolute() && path.exists());
    match found {
        Some(runtime) => {
            println!("cargo:rerun-if-changed={}", runtime.display());
            println!("cargo:rustc-link-arg={}", runtime.display());
        }
        None => println!(
            "cargo:warning=libclang_rt.osx.a not found via xcrun; a release (LTO) build may fail to link ___isPlatformVersionAtLeast"
        ),
    }
}

/// The short HEAD commit ("unknown" before the first commit) and the files to
/// watch so the build reruns when it changes: HEAD, HEAD's reflog, which every
/// commit, checkout and reset that moves HEAD appends to (files backend), and
/// `reftable/tables.list` (reftable backend). Before the first commit there is
/// no reflog yet, so `refs` is watched until it appears.
fn commit_and_watch_paths(root: &Path) -> Result<(String, Vec<PathBuf>), String> {
    // `--git-path` answers relative to `root`; Cargo resolves relative paths
    // against the package directory, so join them to `root`.
    let paths: Vec<PathBuf> = git(
        root,
        &[
            "rev-parse",
            "--git-path",
            "HEAD",
            "--git-path",
            "logs/HEAD",
            "--git-path",
            "reftable/tables.list",
            "--git-path",
            "refs",
        ],
    )?
    .into_iter()
    .map(|path| root.join(path))
    .collect();
    let [head, reflog, reftable, refs] = <[PathBuf; 4]>::try_from(paths)
        .map_err(|_| "unexpected `git rev-parse --git-path` output".to_string())?;

    let sha = git(root, &["rev-parse", "--short=12", "--verify", "-q", "HEAD"])
        .ok()
        .and_then(|lines| lines.into_iter().next())
        .unwrap_or_else(|| "unknown".to_string());

    // Cargo treats a missing path as always changed, so only existing ones.
    // HEAD is always watched: a branch switch rewrites it even when the reflog
    // is off (`core.logAllRefUpdates=false`).
    let tracked = if reflog.exists() || reftable.exists() {
        vec![head, reflog, reftable]
    } else {
        vec![head, refs]
    };
    let watched: Vec<PathBuf> = tracked.into_iter().filter(|p| p.exists()).collect();
    Ok((sha, watched))
}
