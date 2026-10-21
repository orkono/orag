//! `orag eval bench` (v0.6 step 88): the one-shot run the desktop's GPU
//! bench starts on `orag` and on `orag-vulkan`, here with fake models.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn orag() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orag"))
}

fn eval_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval")
}

fn bench(home: &Path, index: &Path, out: &Path, extra: &[&str]) -> Output {
    orag()
        .env("ORAG_HOME", home)
        .args(["eval", "bench", "--dev-fake-models", "--corpus"])
        .arg(eval_dir().join("corpus/v02"))
        .arg("--dataset")
        .arg(eval_dir().join("datasets/v06-bench.jsonl"))
        .arg("--index-dir")
        .arg(index)
        .args(["--tier", "default", "--out"])
        .arg(out)
        .args(extra)
        .output()
        .unwrap()
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_index_is_built_once_and_reused_by_later_runs() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let index = work.path().join("index");

    let built = work.path().join("built.json");
    ok(&bench(
        home.path(),
        &index,
        &built,
        &["--build-index", "--index-only"],
    ));
    let built = json(&built);
    assert_eq!(built["status"], "ok");
    assert_eq!(built["index_built"], true);
    assert_eq!(built["results"].as_array().unwrap().len(), 0);
    let documents = built["index_documents"].as_u64().unwrap();
    assert_eq!(documents, 12);

    let full = work.path().join("full.json");
    let run = bench(home.path(), &index, &full, &[]);
    ok(&run);
    let report = json(&full);
    assert_eq!(report["index_built"], false);
    assert_eq!(
        report["index_documents"].as_u64().unwrap(),
        documents,
        "nothing indexed again"
    );
    assert_eq!(report["build"], orag::gpu::Build::current().name());
    assert_eq!(report["schema"], "orag-bench-run/1");
    let results = report["results"].as_array().unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 10);
    assert_eq!(ids[0], "v2-ans-001");
    for result in results {
        assert!(result["ms"].as_f64().unwrap() > 0.0);
        assert!(result["found_facts"].is_array());
    }
    // One progress line per answer on stdout, in dataset order.
    let stdout = String::from_utf8(run.stdout).unwrap();
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 10, "{stdout}");
    assert_eq!(events[0]["event"], "answer");
    assert_eq!(events[0]["id"], "v2-ans-001");

    let first = work.path().join("first.json");
    ok(&bench(home.path(), &index, &first, &["--first-only"]));
    assert_eq!(json(&first)["results"].as_array().unwrap().len(), 1);
}

#[test]
fn bad_runs_are_refused_before_any_work() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let index = work.path().join("index");
    let out = work.path().join("out.json");
    std::fs::write(&out, "{}").unwrap();
    let existing = bench(home.path(), &index, &out, &["--build-index"]);
    assert!(!existing.status.success());
    assert!(String::from_utf8_lossy(&existing.stderr).contains("already exists"));

    // Reusing needs an index; building needs an empty directory.
    let fresh = work.path().join("fresh.json");
    let missing = bench(home.path(), &index, &fresh, &[]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no index to reuse"));
    ok(&bench(
        home.path(),
        &index,
        &work.path().join("a.json"),
        &["--build-index", "--index-only"],
    ));
    let twice = bench(
        home.path(),
        &index,
        &work.path().join("b.json"),
        &["--build-index"],
    );
    assert!(!twice.status.success());
    assert!(String::from_utf8_lossy(&twice.stderr).contains("already exists"));
}

#[test]
fn a_tier_that_is_not_installed_is_reported_not_failed() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = work.path().join("out.json");
    let run = orag()
        .env("ORAG_HOME", home.path())
        .args(["eval", "bench", "--corpus"])
        .arg(eval_dir().join("corpus/v02"))
        .arg("--dataset")
        .arg(eval_dir().join("datasets/v06-bench.jsonl"))
        .arg("--index-dir")
        .arg(work.path().join("index"))
        .args(["--tier", "large", "--build-index", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    ok(&run);
    let report = json(&out);
    assert_eq!(report["status"], "not-installed");
    assert_eq!(report["tier"], "large");
    assert!(!work.path().join("index").join("eval.db").exists());
}

/// The bench's questions are v0.2 dev lines, byte for byte.
#[test]
fn the_bench_questions_are_v02_lines() {
    let v02 = std::fs::read_to_string(eval_dir().join("datasets/v02-answers.jsonl")).unwrap();
    let bench = std::fs::read_to_string(eval_dir().join("datasets/v06-bench.jsonl")).unwrap();
    let lines: Vec<&str> = bench.lines().collect();
    assert_eq!(lines.len(), 10);
    for line in lines {
        assert!(v02.lines().any(|l| l == line), "{line}");
        let probe: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(probe["split"], "dev");
    }
}
