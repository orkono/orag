//! Merges the bench's runs into the results file (`orag-bench/1`) and
//! decides the G-7 gate inputs: warm medians and the speed-up, the per-fact
//! pass rule, the CPU floor against the reference machine, mixed-backend
//! retrieval, and full offload.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bench::plan::{Exe, Index, Job, ROUNDS, TIERS, Tier};

pub const SCHEMA: &str = "orag-bench/1";
/// The warm GPU median must be at least this many times faster (G-7).
pub const MIN_SPEEDUP: f64 = 5.0;
/// A cross-backend retrieval run within this of its same-backend reference.
pub const RETRIEVAL_TOLERANCE: f64 = 0.02;

/// How one job ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Ok,
    NotInstalled,
    Failed(String),
    Skipped(String),
}

impl Status {
    fn text(&self) -> String {
        match self {
            Status::Ok => "ok".into(),
            Status::NotInstalled => "not-installed".into(),
            Status::Failed(reason) => format!("failed:{reason}"),
            Status::Skipped(reason) => format!("skipped:{reason}"),
        }
    }
}

/// One job and, when it wrote one, its report.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub job: Job,
    pub status: Status,
    pub report: Option<serde_json::Value>,
}

/// The reference CPU run's answers per tier (the CPU floor).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Reference {
    pub tiers: BTreeMap<String, Vec<RefAnswer>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefAnswer {
    pub id: String,
    pub found_facts: Vec<usize>,
    pub abstained: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Answer {
    pub id: String,
    pub ms: f64,
    pub expected: usize,
    pub found_facts: Vec<usize>,
    pub abstained: bool,
    pub valid_citations: bool,
    pub cited_evidence: bool,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Offload {
    pub layers_on_gpu: u32,
    pub layers_total: u32,
    #[serde(default)]
    pub cpu_tensor_overrides: usize,
    pub device: Option<String>,
}

impl Offload {
    fn is_full(&self) -> bool {
        self.layers_total > 0
            && self.layers_on_gpu == self.layers_total
            && self.cpu_tensor_overrides == 0
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RunReport {
    generator: Option<String>,
    embedder: Option<String>,
    sampler: Option<String>,
    load_ms: Option<f64>,
    generator_offload: Option<Offload>,
    #[serde(default)]
    results: Vec<Answer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub round: u8,
    pub status: String,
    pub load_ms: Option<f64>,
    pub generator_offload: Option<Offload>,
    pub answers: Vec<Answer>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Side {
    pub runs: Vec<Run>,
    /// Question 1's answer time over the cold starts.
    pub cold_answer_ms: Vec<f64>,
    pub cold_answer_p50_ms: Option<f64>,
    /// Questions 2.. of round 1.
    pub warm_ms: Vec<f64>,
    pub warm_p50_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub answerable: bool,
    pub pass: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TierResult {
    pub tier: &'static str,
    /// `measured`, `not-installed` or `cpu-round-1-failed`.
    pub status: String,
    pub generator: Option<String>,
    pub embedder: Option<String>,
    pub sampler: Option<String>,
    pub cpu: Side,
    pub vulkan: Side,
    pub vulkan_full_offload: bool,
    pub speedup_warm_p50: Option<f64>,
    pub checks: Vec<Check>,
    pub all_pass: bool,
    /// `ok`, `failed` or `no-reference`.
    pub cpu_floor: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalRun {
    pub exe: Exe,
    pub index: Index,
    pub status: String,
    pub recall_at_10: Option<f64>,
    pub embedding_offload: Option<Offload>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Retrieval {
    pub runs: Vec<RetrievalRun>,
    /// Vulkan on index A minus CPU on A.
    pub cross_a_delta: Option<f64>,
    /// CPU on index B minus Vulkan on B.
    pub cross_b_delta: Option<f64>,
    pub within_tolerance: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gate {
    pub pass: bool,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Merged {
    pub schema: &'static str,
    pub complete: bool,
    pub order: Vec<Job>,
    pub tiers: Vec<TierResult>,
    pub retrieval: Retrieval,
    pub gate: Gate,
}

/// Merges `outcomes` (in run order); `complete` is false for a cancelled or
/// cut bench, which never passes.
pub fn merge(outcomes: &[Outcome], reference: Option<&Reference>, complete: bool) -> Merged {
    let tiers: Vec<TierResult> = TIERS
        .iter()
        .map(|&tier| tier_result(tier, outcomes, reference))
        .collect();
    let retrieval = retrieval(outcomes);
    let gate = gate(complete, &tiers, &retrieval);
    Merged {
        schema: SCHEMA,
        complete,
        order: outcomes.iter().map(|o| o.job).collect(),
        tiers,
        retrieval,
        gate,
    }
}

fn find(outcomes: &[Outcome], job: Job) -> Option<&Outcome> {
    outcomes.iter().find(|o| o.job == job)
}

fn report(outcome: &Outcome) -> RunReport {
    match (&outcome.status, &outcome.report) {
        (Status::Ok, Some(value)) => serde_json::from_value(value.clone()).unwrap_or_default(),
        _ => RunReport::default(),
    }
}

fn side(tier: Tier, exe: Exe, outcomes: &[Outcome]) -> (Side, Option<RunReport>) {
    let mut side = Side::default();
    let mut first = None;
    for round in 1..=ROUNDS {
        let Some(outcome) = find(outcomes, Job::Answers { exe, tier, round }) else {
            continue;
        };
        let report = report(outcome);
        if outcome.status == Status::Ok
            && let Some(q1) = report.results.first()
        {
            side.cold_answer_ms.push(q1.ms);
        }
        if round == 1 && outcome.status == Status::Ok {
            side.warm_ms = report.results.iter().skip(1).map(|a| a.ms).collect();
        }
        side.runs.push(Run {
            round,
            status: outcome.status.text(),
            load_ms: report.load_ms,
            generator_offload: report.generator_offload.clone(),
            answers: report.results.clone(),
        });
        if round == 1 {
            first = Some(report);
        }
    }
    side.cold_answer_p50_ms = median(&side.cold_answer_ms);
    side.warm_p50_ms = median(&side.warm_ms);
    (side, first)
}

fn tier_result(tier: Tier, outcomes: &[Outcome], reference: Option<&Reference>) -> TierResult {
    let cpu_first = find(
        outcomes,
        Job::Answers {
            exe: Exe::Cpu,
            tier,
            round: 1,
        },
    );
    let (cpu, cpu_report) = side(tier, Exe::Cpu, outcomes);
    let (vulkan, vulkan_report) = side(tier, Exe::Vulkan, outcomes);
    let status = match cpu_first.map(|o| &o.status) {
        Some(Status::Ok) => "measured",
        Some(Status::NotInstalled) => "not-installed",
        _ => "cpu-round-1-failed",
    };
    let cpu_answers = cpu_report
        .as_ref()
        .map(|r| r.results.as_slice())
        .unwrap_or(&[]);
    let vulkan_answers = vulkan_report
        .as_ref()
        .map(|r| r.results.as_slice())
        .unwrap_or(&[]);
    let checks: Vec<Check> = cpu_answers
        .iter()
        .map(|c| {
            let v = vulkan_answers.iter().find(|v| v.id == c.id);
            Check {
                id: c.id.clone(),
                answerable: c.expected > 0,
                pass: v.is_some_and(|v| passes(c, v)),
            }
        })
        .collect();
    let all_pass = !checks.is_empty() && checks.iter().all(|c| c.pass);
    let vulkan_full_offload = vulkan.runs.iter().filter(|r| r.status == "ok").count() > 0
        && vulkan
            .runs
            .iter()
            .filter(|r| r.status == "ok")
            .all(|r| r.generator_offload.as_ref().is_some_and(Offload::is_full));
    let speedup_warm_p50 = match (cpu.warm_p50_ms, vulkan.warm_p50_ms) {
        (Some(c), Some(v)) if v > 0.0 => Some(round3(c / v)),
        _ => None,
    };
    let cpu_floor = match reference.and_then(|r| r.tiers.get(tier.name())) {
        None => "no-reference",
        Some(answers) if cpu_floor_holds(answers, cpu_answers) => "ok",
        Some(_) => "failed",
    };
    TierResult {
        tier: tier.name(),
        status: status.into(),
        generator: cpu_report.as_ref().and_then(|r| r.generator.clone()),
        embedder: cpu_report.as_ref().and_then(|r| r.embedder.clone()),
        sampler: cpu_report.as_ref().and_then(|r| r.sampler.clone()),
        cpu,
        vulkan,
        vulkan_full_offload,
        speedup_warm_p50,
        checks,
        all_pass,
        cpu_floor,
    }
}

/// The per-fact rule: an answerable GPU answer keeps every fact the CPU
/// answer found and its valid citations and cited evidence; an unanswerable
/// one abstains, unless the CPU did not abstain either.
fn passes(cpu: &Answer, gpu: &Answer) -> bool {
    if cpu.expected == 0 {
        return gpu.abstained || !cpu.abstained;
    }
    cpu.found_facts.iter().all(|f| gpu.found_facts.contains(f))
        && (!cpu.valid_citations || gpu.valid_citations)
        && (!cpu.cited_evidence || gpu.cited_evidence)
}

/// At least the reference's facts minus one in total, and an abstention
/// wherever the reference abstained.
fn cpu_floor_holds(reference: &[RefAnswer], cpu: &[Answer]) -> bool {
    let wanted: usize = reference.iter().map(|r| r.found_facts.len()).sum();
    let found: usize = cpu.iter().map(|a| a.found_facts.len()).sum();
    let abstains = reference
        .iter()
        .filter(|r| r.abstained)
        .all(|r| cpu.iter().any(|a| a.id == r.id && a.abstained));
    found + 1 >= wanted && abstains && cpu.len() == reference.len()
}

fn retrieval(outcomes: &[Outcome]) -> Retrieval {
    let runs: Vec<RetrievalRun> = [
        (Exe::Cpu, Index::A),
        (Exe::Vulkan, Index::A),
        (Exe::Vulkan, Index::B),
        (Exe::Cpu, Index::B),
    ]
    .into_iter()
    .map(|(exe, index)| {
        let outcome = find(outcomes, Job::Retrieval { exe, index });
        let value = outcome
            .filter(|o| o.status == Status::Ok)
            .and_then(|o| o.report.as_ref());
        RetrievalRun {
            exe,
            index,
            status: outcome.map_or("missing".into(), |o| o.status.text()),
            recall_at_10: value.and_then(|v| {
                v["strategies"]
                    .as_array()?
                    .iter()
                    .find(|s| s["strategy"] == "hybrid")?["recall_at_10"]
                    .as_f64()
            }),
            embedding_offload: value
                .and_then(|v| serde_json::from_value(v["embedding_offload"].clone()).ok()),
        }
    })
    .collect();
    let recall = |i: usize| runs[i].recall_at_10;
    let delta = |cross: usize, same: usize| Some(round3(recall(cross)? - recall(same)?));
    let cross_a_delta = delta(1, 0);
    let cross_b_delta = delta(3, 2);
    let gpu_used = [1, 2].iter().all(|&i| {
        runs[i]
            .embedding_offload
            .as_ref()
            .is_some_and(|o| o.layers_on_gpu > 0)
    });
    let within_tolerance = gpu_used
        && [cross_a_delta, cross_b_delta]
            .iter()
            .all(|d| d.is_some_and(|d| d.abs() <= RETRIEVAL_TOLERANCE + 1e-9));
    Retrieval {
        runs,
        cross_a_delta,
        cross_b_delta,
        within_tolerance,
    }
}

fn gate(complete: bool, tiers: &[TierResult], retrieval: &Retrieval) -> Gate {
    let mut failed = Vec::new();
    if !complete {
        failed.push("the bench did not complete".to_string());
    }
    let measured: Vec<&TierResult> = tiers.iter().filter(|t| t.status == "measured").collect();
    if measured.is_empty() {
        failed.push("no tier was measured".into());
    }
    for tier in measured {
        let name = tier.tier;
        let times = [&tier.cpu, &tier.vulkan].iter().all(|side| {
            side.runs.first().is_some_and(|r| {
                r.status == "ok" && !r.answers.is_empty() && r.answers.iter().all(|a| a.ms > 0.0)
            })
        });
        if !times {
            failed.push(format!("{name}: a question has no time on one backend"));
        }
        if !tier.vulkan_full_offload {
            failed.push(format!("{name}: the answer model was not fully on the GPU"));
        }
        if tier.cpu_floor != "ok" {
            failed.push(format!("{name}: CPU floor {}", tier.cpu_floor));
        }
        if !tier.all_pass {
            failed.push(format!("{name}: a GPU answer failed its checks"));
        }
        if !tier.speedup_warm_p50.is_some_and(|s| s >= MIN_SPEEDUP) {
            failed.push(format!(
                "{name}: warm speed-up {:?} below {MIN_SPEEDUP}",
                tier.speedup_warm_p50
            ));
        }
    }
    if !retrieval.within_tolerance {
        failed.push("mixed-backend retrieval is not within tolerance".into());
    }
    Gate {
        pass: failed.is_empty(),
        failed,
    }
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests;
