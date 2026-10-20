//! The generators the setup page offers by hardware (step 82,
//! `.docs/benchmarks/v0.5-model-tiers.md`): small, default and large, each a
//! catalog entry with the memory it needs and a speed hint. The embedder is
//! not a choice; `qwen3-4b-instruct-2507-q4_k_m` stays in the catalog for a
//! hand-set `generation_model` but is not a tier.

use serde::Serialize;

const GIB: u64 = 1024 * 1024 * 1024;

/// Tenths of a GiB in bytes.
const fn gib_tenths(tenths: u64) -> u64 {
    tenths * GIB / 10
}

// The measured numbers, in one place (step 81). A need is the whole `orag`
// process's peak resident memory (generator + embedder + buffers) plus 2 GiB
// for the system, rounded up to 0.5 GiB; the final need is the larger of the
// Mac (Metal) and Linux (CPU) numbers. A speed hint is the typical (p50)
// answer time in seconds on the dev probes.

/// PROVISIONAL: replaced by the step 81 measurement (not offered: failed
/// the quality rule, abstention recall 0.200 < 0.800).
const SMALL_NEED: u64 = gib_tenths(50);
/// PROVISIONAL: replaced by the step 81 measurement (Mac max RSS 4.06 GiB
/// + 2 GiB; the Linux CPU run may raise it).
const DEFAULT_NEED: u64 = gib_tenths(65);
/// PROVISIONAL: replaced by the step 81 measurement (Mac max RSS 6.85 GiB
/// + 2 GiB; the Linux CPU run may raise it).
const LARGE_NEED: u64 = gib_tenths(90);
/// Mac (Metal) p50 answer seconds of the default and the large tier.
const DEFAULT_METAL_P50_S: f32 = 0.9;
const LARGE_METAL_P50_S: f32 = 2.5;

/// Where a tier sits in the list, smallest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TierSize {
    Small,
    Default,
    Large,
}

impl TierSize {
    /// The page's dictionary key of the tier's name.
    pub fn label_key(self) -> &'static str {
        match self {
            TierSize::Small => "setup.tier.small",
            TierSize::Default => "setup.tier.default",
            TierSize::Large => "setup.tier.large",
        }
    }
}

/// Typical answer time, shown as a labelled estimate; `None` until measured.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub struct SpeedHint {
    /// Median seconds per answer on Apple Silicon (Metal).
    pub metal_p50_answer_s: Option<f32>,
    /// Median seconds per answer on the CPU reference (Linux x86-64).
    pub cpu_p50_answer_s: Option<f32>,
}

/// One generator the page may offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tier {
    /// A catalog id with the generation role.
    pub id: String,
    pub size: TierSize,
    /// Total RAM a machine needs for this tier.
    pub need_ram_bytes: u64,
    pub speed: SpeedHint,
    /// False when the tier failed the quality rule: it is then never offered.
    pub offered: bool,
}

/// The built-in tiers, smallest first.
pub fn builtin_tiers() -> Vec<Tier> {
    let tier = |id: &str, size, need_ram_bytes, metal, offered| Tier {
        id: id.into(),
        size,
        need_ram_bytes,
        speed: SpeedHint {
            metal_p50_answer_s: metal,
            // PROVISIONAL: the Linux CPU measurement is still running.
            cpu_p50_answer_s: None,
        },
        offered,
    };
    vec![
        tier(
            "qwen3.5-2b-q4_k_m",
            TierSize::Small,
            SMALL_NEED,
            None,
            false,
        ),
        tier(
            "qwen3.5-4b-q4_k_m",
            TierSize::Default,
            DEFAULT_NEED,
            Some(DEFAULT_METAL_P50_S),
            true,
        ),
        tier(
            "qwen3.5-9b-q4_k_m",
            TierSize::Large,
            LARGE_NEED,
            Some(LARGE_METAL_P50_S),
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::download::Catalog;
    use crate::infer::models::ModelRole;

    #[test]
    fn every_tier_is_a_catalog_generator_and_they_grow() {
        let catalog = Catalog::builtin();
        let tiers = builtin_tiers();
        let sizes: Vec<TierSize> = tiers.iter().map(|t| t.size).collect();
        assert_eq!(sizes, [TierSize::Small, TierSize::Default, TierSize::Large]);
        for pair in tiers.windows(2) {
            assert!(pair[0].need_ram_bytes < pair[1].need_ram_bytes);
            let (a, b) = (
                catalog.find(&pair[0].id).unwrap(),
                catalog.find(&pair[1].id).unwrap(),
            );
            assert!(a.size < b.size, "{} vs {}", a.id, b.id);
        }
        for tier in &tiers {
            let entry = catalog.find(&tier.id).expect("a tier is in the catalog");
            assert_eq!(entry.role, ModelRole::Generation, "{}", tier.id);
            // The need covers at least the weights plus the 2 GiB headroom.
            assert!(tier.need_ram_bytes > entry.size + 2 * GIB, "{}", tier.id);
        }
        assert_eq!(tiers[1].id, crate::config::DEFAULT_GENERATION_MODEL);
        assert!(
            !tiers
                .iter()
                .any(|t| t.id == "qwen3-4b-instruct-2507-q4_k_m"),
            "same size as the default: not a choice by hardware"
        );
    }

    #[test]
    fn the_step_81_results_decide_what_is_offered() {
        let tiers = builtin_tiers();
        let offered: Vec<(&str, bool)> = tiers.iter().map(|t| (t.id.as_str(), t.offered)).collect();
        assert_eq!(
            offered,
            [
                ("qwen3.5-2b-q4_k_m", false),
                ("qwen3.5-4b-q4_k_m", true),
                ("qwen3.5-9b-q4_k_m", true)
            ]
        );
        let needs: Vec<u64> = tiers.iter().map(|t| t.need_ram_bytes).collect();
        assert_eq!(needs, [5 * GIB, 13 * GIB / 2, 9 * GIB]);
        assert_eq!(tiers[1].speed.metal_p50_answer_s, Some(0.9));
        assert_eq!(tiers[2].speed.cpu_p50_answer_s, None);
    }
}
