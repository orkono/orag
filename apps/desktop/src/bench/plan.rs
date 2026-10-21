//! The bench's runs, in order (v0.6 plan, Task 6 design note): one index
//! built on the CPU (A), per tier the paired answer rounds, an index built
//! on the GPU (B), then retrieval across both indexes in both directions.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Exe {
    Cpu,
    Vulkan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Index {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Default,
    Large,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Default => "default",
            Tier::Large => "large",
        }
    }
}

pub const TIERS: [Tier; 2] = [Tier::Default, Tier::Large];
/// Cold starts per backend and tier: round 1 answers every question,
/// rounds 2 and 3 only the first.
pub const ROUNDS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "job", rename_all = "lowercase")]
pub enum Job {
    /// `eval bench --build-index --index-only`.
    Index { exe: Exe, index: Index },
    /// `eval bench` on index A; round 1 answers all, later rounds the first.
    Answers { exe: Exe, tier: Tier, round: u8 },
    /// `eval retrieval --index-dir` on an index built by either backend.
    Retrieval { exe: Exe, index: Index },
}

impl Job {
    pub fn exe(self) -> Exe {
        match self {
            Job::Index { exe, .. } | Job::Answers { exe, .. } | Job::Retrieval { exe, .. } => exe,
        }
    }

    /// The answers this job gives, for the progress count.
    pub fn answers(self, questions: usize) -> usize {
        match self {
            Job::Answers { round: 1, .. } => questions,
            Job::Answers { .. } => 1,
            _ => 0,
        }
    }
}

/// Every job, in the order they run. CPU goes first in each pair; the
/// order is recorded, its effect is not assumed.
pub fn jobs() -> Vec<Job> {
    let mut jobs = vec![Job::Index {
        exe: Exe::Cpu,
        index: Index::A,
    }];
    for tier in TIERS {
        for round in 1..=ROUNDS {
            for exe in [Exe::Cpu, Exe::Vulkan] {
                jobs.push(Job::Answers { exe, tier, round });
            }
        }
    }
    jobs.push(Job::Index {
        exe: Exe::Vulkan,
        index: Index::B,
    });
    for (exe, index) in [
        (Exe::Cpu, Index::A),
        (Exe::Vulkan, Index::A),
        (Exe::Vulkan, Index::B),
        (Exe::Cpu, Index::B),
    ] {
        jobs.push(Job::Retrieval { exe, index });
    }
    jobs
}

/// How a finished job went, as far as later jobs care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    Ok,
    /// `eval bench` said the tier's models are not installed.
    NotInstalled,
    Failed,
    Skipped,
}

/// Why `job` does not run, given the jobs done so far; None: it runs.
/// No GPU (`gpu`: the probe's failure) skips every Vulkan job; a tier that
/// is not installed, or whose CPU round 1 failed, skips the tier; index B
/// missing skips the runs on it; a failed index A ends the bench.
pub fn skip_reason(job: Job, gpu: Option<&str>, done: &[(Job, Done)]) -> Option<String> {
    let outcome = |wanted: Job| {
        done.iter()
            .find(|(j, _)| *j == wanted)
            .map(|(_, d)| d.clone())
    };
    let index_a = Job::Index {
        exe: Exe::Cpu,
        index: Index::A,
    };
    if job != index_a && outcome(index_a) != Some(Done::Ok) {
        return Some("index-a-failed".into());
    }
    if job.exe() == Exe::Vulkan
        && let Some(reason) = gpu
    {
        return Some(reason.to_string());
    }
    match job {
        Job::Answers { exe, tier, round } if !(exe == Exe::Cpu && round == 1) => {
            match outcome(Job::Answers {
                exe: Exe::Cpu,
                tier,
                round: 1,
            }) {
                Some(Done::Ok) => None,
                Some(Done::NotInstalled) => Some("not-installed".into()),
                _ => Some("cpu-round-1-failed".into()),
            }
        }
        Job::Retrieval {
            index: Index::B, ..
        } => match outcome(Job::Index {
            exe: Exe::Vulkan,
            index: Index::B,
        }) {
            Some(Done::Ok) => None,
            _ => Some("index-b-failed".into()),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_is_index_rounds_index_retrieval() {
        let jobs = jobs();
        assert_eq!(jobs.len(), 1 + 2 * 3 * 2 + 1 + 4);
        assert_eq!(
            jobs[0],
            Job::Index {
                exe: Exe::Cpu,
                index: Index::A
            }
        );
        assert_eq!(
            &jobs[1..3],
            &[
                Job::Answers {
                    exe: Exe::Cpu,
                    tier: Tier::Default,
                    round: 1
                },
                Job::Answers {
                    exe: Exe::Vulkan,
                    tier: Tier::Default,
                    round: 1
                }
            ]
        );
        assert_eq!(
            jobs[13],
            Job::Index {
                exe: Exe::Vulkan,
                index: Index::B
            }
        );
        let answers: usize = jobs.iter().map(|j| j.answers(10)).sum();
        assert_eq!(answers, 2 * 2 * (10 + 1 + 1));
    }

    fn cpu1(tier: Tier) -> Job {
        Job::Answers {
            exe: Exe::Cpu,
            tier,
            round: 1,
        }
    }

    const A: Job = Job::Index {
        exe: Exe::Cpu,
        index: Index::A,
    };

    #[test]
    fn skips_follow_what_failed() {
        let vulkan1 = Job::Answers {
            exe: Exe::Vulkan,
            tier: Tier::Default,
            round: 1,
        };
        assert_eq!(skip_reason(A, None, &[]), None);
        assert_eq!(
            skip_reason(cpu1(Tier::Default), None, &[(A, Done::Failed)]).as_deref(),
            Some("index-a-failed")
        );
        let ok = [(A, Done::Ok), (cpu1(Tier::Default), Done::Ok)];
        assert_eq!(skip_reason(vulkan1, None, &ok), None);
        assert_eq!(
            skip_reason(vulkan1, Some("loader-missing"), &ok).as_deref(),
            Some("loader-missing")
        );
        let missing = [(A, Done::Ok), (cpu1(Tier::Large), Done::NotInstalled)];
        let large2 = Job::Answers {
            exe: Exe::Cpu,
            tier: Tier::Large,
            round: 2,
        };
        assert_eq!(
            skip_reason(large2, None, &missing).as_deref(),
            Some("not-installed")
        );
        let failed = [(A, Done::Ok), (cpu1(Tier::Large), Done::Failed)];
        assert_eq!(
            skip_reason(large2, None, &failed).as_deref(),
            Some("cpu-round-1-failed")
        );
        let on_b = Job::Retrieval {
            exe: Exe::Cpu,
            index: Index::B,
        };
        let b_failed = [
            (A, Done::Ok),
            (
                Job::Index {
                    exe: Exe::Vulkan,
                    index: Index::B,
                },
                Done::Failed,
            ),
        ];
        assert_eq!(
            skip_reason(on_b, None, &b_failed).as_deref(),
            Some("index-b-failed")
        );
        let on_a = Job::Retrieval {
            exe: Exe::Cpu,
            index: Index::A,
        };
        assert_eq!(
            skip_reason(on_a, Some("no-eligible-device"), &[(A, Done::Ok)]),
            None
        );
    }
}
