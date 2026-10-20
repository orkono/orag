//! The generators the setup page offers by hardware (step 82,
//! `.docs/benchmarks/v0.5-model-tiers.md`): small, default and large, each a
//! catalog entry with the memory it needs and a speed hint. The embedder is
//! not a choice; `qwen3-4b-instruct-2507-q4_k_m` stays in the catalog for a
//! hand-set `generation_model` but is not a tier.

use serde::Serialize;

use crate::infer::download::Catalog;
use crate::infer::models::ModelRole;

const GIB: u64 = 1024 * 1024 * 1024;

/// Tenths of a GiB in bytes.
const fn gib_tenths(tenths: u64) -> u64 {
    tenths * GIB / 10
}

// The measured numbers, in one place (step 81,
// `.docs/benchmarks/v0.5-model-tiers.md`). From the whole `orag` process's
// peak resident memory (generator + embedder + buffers), the larger of the
// Mac (Metal) and Linux (CPU) runs:
// - hard floor = peak + 1 GiB, rounded up to 0.5 GiB: below it the tier is
//   not selectable (`too_big_ram`);
// - recommended = peak + 2 GiB, rounded up to 0.5 GiB: below it the tier is
//   still selectable, with the `below_recommended_ram` warning.
// Peaks: 4B 5.66 GiB (Linux; Mac 4.06), 9B 9.71 GiB (Linux; Mac 6.85).
// A speed hint is the median (p50) answer time in seconds on the dev
// probes: on an Apple M5 Max (Metal), and on a 4-vCPU GitHub-hosted Linux
// runner (CPU), which is a reference machine, not a promise for a PC.

/// The 2B (not offered: it failed the quality rule, abstention recall
/// 0.200 < 0.800; its thresholds serve a hand-set `generation_model`).
const SMALL_MIN_RAM: u64 = gib_tenths(60);
const SMALL_RECOMMENDED_RAM: u64 = gib_tenths(70);
/// The 4B default.
const DEFAULT_MIN_RAM: u64 = gib_tenths(70);
const DEFAULT_RECOMMENDED_RAM: u64 = gib_tenths(80);
/// The 9B large.
const LARGE_MIN_RAM: u64 = gib_tenths(110);
const LARGE_RECOMMENDED_RAM: u64 = gib_tenths(120);
/// Median answer seconds of the default and the large tier.
const DEFAULT_METAL_P50_S: f32 = 0.9;
const LARGE_METAL_P50_S: f32 = 2.5;
const DEFAULT_CPU_P50_S: f32 = 108.0;
const LARGE_CPU_P50_S: f32 = 194.0;

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
    /// Median seconds per answer on the CPU reference (a 4-vCPU GitHub
    /// Linux x86-64 runner).
    pub cpu_p50_answer_s: Option<f32>,
}

/// One generator the page may offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tier {
    /// A catalog id with the generation role.
    pub id: String,
    pub size: TierSize,
    /// Total RAM below which the tier is not selectable.
    pub min_ram_bytes: u64,
    /// Total RAM below which it is selectable with a warning.
    pub recommended_ram_bytes: u64,
    pub speed: SpeedHint,
    /// False when the tier failed the quality rule: it is then never offered.
    pub offered: bool,
}

/// The built-in tiers, smallest first.
pub fn builtin_tiers() -> Vec<Tier> {
    let tier = |id: &str, size, (min, recommended), (metal, cpu), offered| Tier {
        id: id.into(),
        size,
        min_ram_bytes: min,
        recommended_ram_bytes: recommended,
        speed: SpeedHint {
            metal_p50_answer_s: metal,
            cpu_p50_answer_s: cpu,
        },
        offered,
    };
    vec![
        tier(
            "qwen3.5-2b-q4_k_m",
            TierSize::Small,
            (SMALL_MIN_RAM, SMALL_RECOMMENDED_RAM),
            (None, None),
            false,
        ),
        tier(
            "qwen3.5-4b-q4_k_m",
            TierSize::Default,
            (DEFAULT_MIN_RAM, DEFAULT_RECOMMENDED_RAM),
            (Some(DEFAULT_METAL_P50_S), Some(DEFAULT_CPU_P50_S)),
            true,
        ),
        tier(
            "qwen3.5-9b-q4_k_m",
            TierSize::Large,
            (LARGE_MIN_RAM, LARGE_RECOMMENDED_RAM),
            (Some(LARGE_METAL_P50_S), Some(LARGE_CPU_P50_S)),
            true,
        ),
    ]
}

/// The configured generator as the page's extra "current" option, when it
/// is a catalog generator but not an offered tier (a hand-set
/// `qwen3-4b-instruct-2507-q4_k_m`, or the withdrawn 2B). Its RAM thresholds
/// are its own tier's when it has one, else the default tier's: the
/// only such catalog generator today is the same-size 4B instruct model
/// (2.50 GB of weights against the default's 2.74 GB). An id outside the
/// catalog, an offered tier, or a table without a default gives `None`.
pub fn current_tier(configured: &str, tiers: &[Tier], catalog: &Catalog) -> Option<Tier> {
    let own = tiers.iter().find(|tier| tier.id == configured);
    if own.is_some_and(|tier| tier.offered) {
        return None;
    }
    catalog
        .find(configured)
        .filter(|entry| entry.role == ModelRole::Generation)?;
    let default = tiers.iter().find(|tier| tier.size == TierSize::Default)?;
    let thresholds = own.unwrap_or(default);
    Some(Tier {
        id: configured.to_string(),
        size: own.map_or(TierSize::Default, |tier| tier.size),
        min_ram_bytes: thresholds.min_ram_bytes,
        recommended_ram_bytes: thresholds.recommended_ram_bytes,
        speed: own.map_or_else(SpeedHint::default, |tier| tier.speed),
        offered: true,
    })
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
            assert!(pair[0].min_ram_bytes < pair[1].min_ram_bytes);
            let (a, b) = (
                catalog.find(&pair[0].id).unwrap(),
                catalog.find(&pair[1].id).unwrap(),
            );
            assert!(a.size < b.size, "{} vs {}", a.id, b.id);
        }
        for tier in &tiers {
            let entry = catalog.find(&tier.id).expect("a tier is in the catalog");
            assert_eq!(entry.role, ModelRole::Generation, "{}", tier.id);
            // The floor covers at least the weights plus 1 GiB; the
            // recommended RAM is 1 GiB above the floor.
            assert!(tier.min_ram_bytes > entry.size + GIB, "{}", tier.id);
            assert_eq!(
                tier.recommended_ram_bytes,
                tier.min_ram_bytes + GIB,
                "{}",
                tier.id
            );
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
        let floors: Vec<u64> = tiers.iter().map(|t| t.min_ram_bytes).collect();
        assert_eq!(floors, [6 * GIB, 7 * GIB, 11 * GIB]);
        let recommended: Vec<u64> = tiers.iter().map(|t| t.recommended_ram_bytes).collect();
        assert_eq!(recommended, [7 * GIB, 8 * GIB, 12 * GIB]);
        let speeds: Vec<(Option<f32>, Option<f32>)> = tiers
            .iter()
            .map(|t| (t.speed.metal_p50_answer_s, t.speed.cpu_p50_answer_s))
            .collect();
        assert_eq!(
            speeds,
            [
                (None, None),
                (Some(0.9), Some(108.0)),
                (Some(2.5), Some(194.0))
            ]
        );
    }
}
