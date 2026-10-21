//! The fake sidecar's `eval bench` and `eval retrieval` (bench tests only):
//! a run per invocation from `ORAG_DESKTOP_FAKE_EVAL` (comma-separated,
//! the last repeats): `ok`, `crash:N` (exit N, no report), `not-installed`,
//! `hang` (until stdin closes, then exit 3). `ok` answers every question
//! of `--dataset` (all facts, abstaining on unanswerable ones), 100 ms per
//! answer on the CPU and 10 ms with `ORAG_GPU_DEVICE`, and prints one
//! answer line each. Every call is appended to `eval-calls` in the state
//! folder with the device it was given.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

pub const EVAL_ENV: &str = "ORAG_DESKTOP_FAKE_EVAL";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn next_mode(state: &Path) -> String {
    let file = state.join("eval-starts");
    let run: usize = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let _ = std::fs::write(&file, (run + 1).to_string());
    let modes = std::env::var(EVAL_ENV).unwrap_or_else(|_| "ok".into());
    let modes: Vec<&str> = modes.split(',').collect();
    modes[run.min(modes.len() - 1)].to_string()
}

pub fn eval(state: &Path, args: &[String]) -> i32 {
    let device = std::env::var("ORAG_GPU_DEVICE").ok();
    let mut calls = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("eval-calls"))
        .expect("the state folder");
    let _ = writeln!(
        calls,
        "{} | device={}",
        args.join(" "),
        device.as_deref().unwrap_or("")
    );
    // The pid first: once `eval-starts` counts this call, `eval-pid` is its.
    let _ = std::fs::write(state.join("eval-pid"), std::process::id().to_string());
    let mode = next_mode(state);
    let out = arg(args, "--out").expect("--out");
    let (name, code) = mode.split_once(':').unwrap_or((mode.as_str(), "1"));
    match name {
        "crash" => return code.parse().unwrap_or(1),
        "hang" => {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            return 3;
        }
        "not-installed" => {
            let tier = arg(args, "--tier").unwrap_or_default();
            let report =
                serde_json::json!({"status": "not-installed", "tier": tier, "missing": "x"});
            let _ = std::fs::write(&out, report.to_string());
            return 0;
        }
        _ => {}
    }
    let offload = |layers: u32| {
        device.as_ref().map(|d| {
            serde_json::json!({"layers_on_gpu": layers, "layers_total": layers,
                "cpu_tensor_overrides": 0, "device": format!("Fake GPU {d}"), "reason": null})
        })
    };
    let report = if args.get(1).map(String::as_str) == Some("retrieval") {
        serde_json::json!({"embedding_offload": offload(29),
            "strategies": [{"strategy": "hybrid", "recall_at_10": 0.9}]})
    } else {
        let dataset = std::fs::read_to_string(arg(args, "--dataset").expect("--dataset")).unwrap();
        let mut probes: Vec<serde_json::Value> = dataset
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        if args.iter().any(|a| a == "--index-only") {
            probes.clear();
        } else if args.iter().any(|a| a == "--first-only") {
            probes.truncate(1);
        }
        let ms = if device.is_some() { 10.0 } else { 100.0 };
        let mut stdout = std::io::stdout();
        let results: Vec<serde_json::Value> = probes
            .iter()
            .map(|p| {
                std::thread::sleep(Duration::from_millis(5));
                let expected = p["expect"].as_array().map_or(0, Vec::len);
                let _ = writeln!(stdout, "{{\"event\":\"answer\",\"id\":{},\"ms\":{ms}}}", p["id"]);
                let _ = stdout.flush();
                serde_json::json!({"id": p["id"], "ms": ms, "expected": expected,
                    "found_facts": (0..expected).collect::<Vec<_>>(), "abstained": expected == 0,
                    "valid_citations": expected > 0, "cited_evidence": expected > 0, "answer": "fake"})
            })
            .collect();
        serde_json::json!({"schema": "orag-bench-run/1", "status": "ok",
            "tier": arg(args, "--tier"), "build": if device.is_some() { "vulkan" } else { "cpu" },
            "generator": "fake-gen", "embedder": "fake-emb", "sampler": "dry",
            "index_built": args.iter().any(|a| a == "--build-index"), "index_documents": 12,
            "load_ms": 1.0, "generator_offload": offload(33), "embedder_offload": offload(29),
            "results": results})
    };
    let _ = std::fs::write(&out, report.to_string());
    0
}
