use serde_json::json;

use super::*;
use crate::bench::plan::{Exe, Index, Job, Tier};

/// An `eval bench` answer: `facts` found of `expected`.
fn answer(
    id: &str,
    ms: f64,
    expected: usize,
    facts: &[usize],
    abstained: bool,
) -> serde_json::Value {
    json!({
        "id": id, "ms": ms, "expected": expected, "found_facts": facts,
        "abstained": abstained, "valid_citations": !abstained && expected > 0,
        "cited_evidence": !abstained && expected > 0, "answer": "a"
    })
}

fn offload(on: u32, total: u32) -> serde_json::Value {
    json!({"layers_on_gpu": on, "layers_total": total, "cpu_tensor_overrides": 0, "device": "RTX", "reason": null})
}

/// Round 1 has three questions: two answerable (2 facts, 1 fact) and one
/// unanswerable; later rounds only the first.
fn run(exe: Exe, round: u8, ms: f64) -> serde_json::Value {
    let all = vec![
        answer("q1", ms, 2, &[0, 1], false),
        answer("q2", ms * 2.0, 1, &[0], false),
        answer("q3", ms * 3.0, 0, &[], true),
    ];
    let results = if round == 1 { all } else { all[..1].to_vec() };
    json!({
        "schema": "orag-bench-run/1", "status": "ok", "tier": "default",
        "build": if exe == Exe::Cpu { "cpu" } else { "vulkan" },
        "generator": "qwen3.5-4b-q4_k_m", "embedder": "qwen3-embedding-0.6b-q8_0",
        "sampler": "dry", "index_built": false, "index_documents": 12, "load_ms": 900.0,
        "generator_offload": if exe == Exe::Cpu { serde_json::Value::Null } else { offload(33, 33) },
        "embedder_offload": serde_json::Value::Null,
        "results": results
    })
}

fn retrieval(exe: Exe, recall: f64) -> serde_json::Value {
    json!({
        "embedding_offload": if exe == Exe::Cpu { serde_json::Value::Null } else { offload(29, 29) },
        "strategies": [{"strategy": "hybrid", "recall_at_10": recall}]
    })
}

/// The first answer comes 900 ms after the process starts.
fn ok(job: Job, report: serde_json::Value) -> Outcome {
    let first = report["results"][0]["ms"].as_f64().map(|ms| ms + 900.0);
    Outcome {
        job,
        status: Status::Ok,
        report: Some(report),
        first_answer_ms: first,
    }
}

fn reference() -> Reference {
    Reference {
        sampler: Some("dry".into()),
        tiers: [(
            "default".to_string(),
            vec![
                RefAnswer {
                    id: "q1".into(),
                    found_facts: vec![0, 1],
                    abstained: false,
                },
                RefAnswer {
                    id: "q2".into(),
                    found_facts: vec![0],
                    abstained: false,
                },
                RefAnswer {
                    id: "q3".into(),
                    found_facts: vec![],
                    abstained: true,
                },
            ],
        )]
        .into_iter()
        .collect(),
    }
}

/// A full default-tier bench: Vulkan 10x faster, large not installed.
fn full_bench(cpu_ms: f64, vulkan_ms: f64) -> Vec<Outcome> {
    let mut outcomes = vec![ok(
        Job::Index {
            exe: Exe::Cpu,
            index: Index::A,
        },
        json!({"status": "ok", "embedder_offload": null}),
    )];
    for round in 1..=3 {
        for (exe, ms) in [(Exe::Cpu, cpu_ms), (Exe::Vulkan, vulkan_ms)] {
            outcomes.push(ok(
                Job::Answers {
                    exe,
                    tier: Tier::Default,
                    round,
                },
                run(exe, round, ms),
            ));
        }
    }
    outcomes.push(Outcome {
        job: Job::Answers {
            exe: Exe::Cpu,
            tier: Tier::Large,
            round: 1,
        },
        status: Status::NotInstalled,
        report: None,
        first_answer_ms: None,
    });
    outcomes.push(ok(
        Job::Index {
            exe: Exe::Vulkan,
            index: Index::B,
        },
        json!({"status": "ok", "embedder_offload": offload(29, 29)}),
    ));
    for (exe, index, recall) in [
        (Exe::Cpu, Index::A, 0.95),
        (Exe::Vulkan, Index::A, 0.94),
        (Exe::Vulkan, Index::B, 0.95),
        (Exe::Cpu, Index::B, 0.96),
    ] {
        outcomes.push(ok(Job::Retrieval { exe, index }, retrieval(exe, recall)));
    }
    outcomes
}

#[test]
fn a_full_fast_bench_passes_the_gate() {
    let file = merge(&full_bench(10_000.0, 1_000.0), Some(&reference()), true);
    let tier = &file.tiers[0];
    assert_eq!(tier.tier, "default");
    assert_eq!(tier.cpu.cold_ms, vec![10_900.0; 3]);
    assert_eq!(tier.cpu.warm_ms, vec![20_000.0, 30_000.0]);
    assert_eq!(tier.cpu.warm_p50_ms, Some(25_000.0));
    assert_eq!(tier.speedup_warm_p50, Some(10.0));
    assert!(tier.checks.iter().all(|c| c.pass), "{:?}", tier.checks);
    assert_eq!(tier.cpu_floor, "ok");
    assert!(tier.vulkan_full_offload);
    assert_eq!(file.tiers[1].status, "not-installed");
    assert!(file.retrieval.within_tolerance);
    assert!(file.gate.pass, "{:?}", file.gate.failed);
}

#[test]
fn a_slow_or_partial_or_incomplete_bench_fails_the_gate() {
    let slow = merge(&full_bench(10_000.0, 4_000.0), Some(&reference()), true);
    assert!(
        slow.gate.failed.iter().any(|f| f.contains("speed")),
        "{:?}",
        slow.gate.failed
    );

    let mut partial = full_bench(10_000.0, 1_000.0);
    partial[2].report.as_mut().unwrap()["generator_offload"] = offload(20, 33);
    let partial = merge(&partial, Some(&reference()), true);
    assert!(!partial.tiers[0].vulkan_full_offload);
    assert!(!partial.gate.pass);

    let cut = merge(
        &full_bench(10_000.0, 1_000.0)[..5],
        Some(&reference()),
        false,
    );
    assert!(!cut.complete);
    assert!(cut.gate.failed.iter().any(|f| f.contains("complete")));

    let unreferenced = merge(&full_bench(10_000.0, 1_000.0), None, true);
    assert_eq!(unreferenced.tiers[0].cpu_floor, "no-reference");
    assert!(!unreferenced.gate.pass);
}

#[test]
fn a_gpu_answer_must_keep_every_fact_the_cpu_found() {
    let mut outcomes = full_bench(10_000.0, 1_000.0);
    // Vulkan round 1 finds the second fact of q1 only.
    outcomes[2].report.as_mut().unwrap()["results"][0]["found_facts"] = json!([1]);
    let file = merge(&outcomes, Some(&reference()), true);
    let q1 = file.tiers[0].checks.iter().find(|c| c.id == "q1").unwrap();
    assert!(!q1.pass);
    assert!(!file.gate.pass);
}

#[test]
fn an_unanswerable_question_passes_when_the_gpu_abstains_or_the_cpu_did_not() {
    let mut cpu_answered = full_bench(10_000.0, 1_000.0);
    // The CPU answered the unanswerable q3; the GPU did too: no worse.
    cpu_answered[1].report.as_mut().unwrap()["results"][2]["abstained"] = json!(false);
    cpu_answered[2].report.as_mut().unwrap()["results"][2]["abstained"] = json!(false);
    let file = merge(&cpu_answered, Some(&reference()), true);
    assert!(
        file.tiers[0]
            .checks
            .iter()
            .find(|c| c.id == "q3")
            .unwrap()
            .pass
    );
    // But the CPU then misses the reference's abstention: the floor fails.
    assert_eq!(file.tiers[0].cpu_floor, "failed");

    let mut gpu_answered = full_bench(10_000.0, 1_000.0);
    gpu_answered[2].report.as_mut().unwrap()["results"][2]["abstained"] = json!(false);
    let file = merge(&gpu_answered, Some(&reference()), true);
    assert!(
        !file.tiers[0]
            .checks
            .iter()
            .find(|c| c.id == "q3")
            .unwrap()
            .pass
    );
}

#[test]
fn the_cpu_floor_allows_one_missing_fact() {
    let mut one_less = full_bench(10_000.0, 1_000.0);
    one_less[1].report.as_mut().unwrap()["results"][0]["found_facts"] = json!([0]);
    one_less[2].report.as_mut().unwrap()["results"][0]["found_facts"] = json!([0]);
    assert_eq!(
        merge(&one_less, Some(&reference()), true).tiers[0].cpu_floor,
        "ok"
    );
    one_less[1].report.as_mut().unwrap()["results"][1]["found_facts"] = json!([]);
    assert_eq!(
        merge(&one_less, Some(&reference()), true).tiers[0].cpu_floor,
        "failed"
    );
}

#[test]
fn mixed_retrieval_must_stay_within_tolerance_of_its_reference() {
    let mut outcomes = full_bench(10_000.0, 1_000.0);
    let last = outcomes.len() - 1;
    // CPU on the GPU-built index B: 0.92 against Vulkan on B's 0.95.
    outcomes[last].report = Some(retrieval(Exe::Cpu, 0.92));
    let file = merge(&outcomes, Some(&reference()), true);
    assert_eq!(file.retrieval.cross_b_delta, Some(-0.03));
    assert!(!file.retrieval.within_tolerance);
    assert!(!file.gate.pass);
}

#[test]
fn no_gpu_is_recorded_and_fails_the_gate() {
    let outcomes: Vec<Outcome> = full_bench(10_000.0, 1_000.0)
        .into_iter()
        .map(|o| {
            if o.job.exe() == Exe::Vulkan {
                Outcome {
                    job: o.job,
                    status: Status::Skipped("no-eligible-device".into()),
                    report: None,
                    first_answer_ms: None,
                }
            } else {
                o
            }
        })
        .collect();
    let file = merge(&outcomes, Some(&reference()), true);
    assert_eq!(
        file.tiers[0].vulkan.runs[0].status,
        "skipped:no-eligible-device"
    );
    assert_eq!(file.tiers[0].speedup_warm_p50, None);
    assert!(!file.gate.pass);
}

#[test]
fn a_tier_that_failed_to_run_fails_the_gate() {
    let mut outcomes = full_bench(10_000.0, 1_000.0);
    let large = outcomes
        .iter_mut()
        .find(|o| {
            o.job
                == Job::Answers {
                    exe: Exe::Cpu,
                    tier: Tier::Large,
                    round: 1,
                }
        })
        .unwrap();
    large.status = Status::Failed("exit-9".into());
    let file = merge(&outcomes, Some(&reference()), true);
    assert_eq!(file.tiers[1].status, "cpu-round-1-failed");
    assert!(
        file.gate.failed.iter().any(|f| f.starts_with("large")),
        "{:?}",
        file.gate.failed
    );
}

#[test]
fn a_failed_later_round_fails_the_gate() {
    let mut outcomes = full_bench(10_000.0, 1_000.0);
    outcomes[4].status = Status::Failed("exit-9".into());
    outcomes[4].report = None;
    let file = merge(&outcomes, Some(&reference()), true);
    assert!(!file.gate.pass);
}

#[test]
fn index_b_must_have_been_built_on_the_gpu() {
    let mut outcomes = full_bench(10_000.0, 1_000.0);
    let b = outcomes
        .iter_mut()
        .find(|o| {
            o.job
                == Job::Index {
                    exe: Exe::Vulkan,
                    index: Index::B,
                }
        })
        .unwrap();
    b.report = Some(json!({"status": "ok", "embedder_offload": null}));
    let file = merge(&outcomes, Some(&reference()), true);
    assert!(!file.retrieval.within_tolerance);
    assert_eq!(file.indexes.len(), 2);
}

#[test]
fn the_gate_compares_the_unrounded_speed_up() {
    let file = merge(&full_bench(4_999.6, 1_000.0), Some(&reference()), true);
    assert!(
        file.gate.failed.iter().any(|f| f.contains("speed")),
        "{:?}",
        file.gate.failed
    );
}

#[test]
fn a_run_with_another_sampler_is_not_compared_with_the_reference() {
    let mut other = reference();
    other.sampler = Some("greedy".into());
    let file = merge(&full_bench(10_000.0, 1_000.0), Some(&other), true);
    assert_eq!(file.tiers[0].cpu_floor, "sampler-mismatch");
    assert!(!file.gate.pass);
}
