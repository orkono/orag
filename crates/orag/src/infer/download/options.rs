//! Which generator tiers fit this machine (step 82): the one place that
//! decides the setup page's list and the eligibility `POST
//! /v1/models/download` checks a chosen generator against.
//!
//! RAM: total RAM against the tier's need (available RAM only warns). Disk:
//! the embedder (if not installed), the generator (if not installed) and
//! 512 MiB against the free space of the models folder. Unknown facts never
//! block. The default tier is recommended whenever it fits; the large one
//! is never recommended (it is slower and uses more memory, step 82). The
//! configured generator, when it is a catalog generator but not a tier, is
//! offered as an extra "current" option with its own verdict.

use serde::Serialize;

use super::tiers::{SpeedHint, Tier, TierSize};
use crate::probe::Machine;

/// Free space kept on top of the downloaded files.
pub const DISK_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Recommended,
    Fits,
    TooBigRam,
    TooBigDisk,
}

impl Verdict {
    /// The tier may be downloaded on this machine.
    pub fn eligible(self) -> bool {
        matches!(self, Verdict::Recommended | Verdict::Fits)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportStatus {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportCode {
    /// An x86-64 CPU without the instructions the binary is built for.
    CpuUnsupported,
    /// No offered tier fits the RAM and disk.
    NoTierFits,
}

/// Whether this machine can run ORAG with any offered generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Support {
    pub status: SupportStatus,
    pub code: Option<SupportCode>,
    /// The required x86-64 instructions the CPU lacks.
    pub missing_cpu_features: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    /// Only one tier fits: it is offered without a choice (owner decision).
    SingleOption,
    /// The tier fits the total RAM, but less is free right now.
    LowAvailableMemory,
    /// An x86-64 CPU without AVX2/FMA/F16C/BMI2 outside Windows: it runs,
    /// on slower code paths (only the Windows binary is built for that floor).
    SlowCpu,
}

/// An offered tier with what this machine needs to install it.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub tier: &'a Tier,
    /// Bytes of the tier's pack (weights and license).
    pub bytes: u64,
    pub installed: bool,
    /// The configured generator, offered although it is not a tier.
    pub current: bool,
}

/// One tier as `download.options` lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TierOption {
    pub id: String,
    /// `None` for the current (configured, not a tier) option.
    pub size: Option<TierSize>,
    pub current: bool,
    pub label_key: &'static str,
    pub bytes: u64,
    pub installed: bool,
    pub need_ram_bytes: u64,
    pub speed: SpeedHint,
    pub verdict: Verdict,
    pub warnings: Vec<Warning>,
}

impl TierOption {
    /// Bytes a download of this tier fetches.
    pub fn download_bytes(&self) -> u64 {
        if self.installed { 0 } else { self.bytes }
    }
}

/// The choice offered to this machine.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Options {
    pub support: Support,
    /// Machine-level warnings (`single_option`).
    pub warnings: Vec<Warning>,
    pub options: Vec<TierOption>,
    /// The option the page starts with: the configured generator when it is
    /// an option that fits, else the recommended one; `None` when unsupported.
    pub selected: Option<String>,
    /// Bytes of the embedder still to download (0 when installed).
    pub embedding_bytes: u64,
}

/// Why a chosen generator may not be downloaded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ineligible {
    /// Not an offered tier.
    Unknown,
    /// An offered tier this machine cannot take, and why (`cpu_unsupported`,
    /// `no_tier_fits`, `too_big_ram` or `too_big_disk`).
    NotEligible(&'static str),
}

impl Options {
    /// The offered tier `id` if this machine can take it now.
    pub fn eligible(&self, id: &str) -> Result<&TierOption, Ineligible> {
        let option = self
            .options
            .iter()
            .find(|option| option.id == id)
            .ok_or(Ineligible::Unknown)?;
        let reason = match (self.support.code, option.verdict) {
            (Some(SupportCode::CpuUnsupported), _) => "cpu_unsupported",
            (Some(SupportCode::NoTierFits), _) => "no_tier_fits",
            (None, Verdict::TooBigRam) => "too_big_ram",
            (None, Verdict::TooBigDisk) => "too_big_disk",
            (None, Verdict::Recommended | Verdict::Fits) => return Ok(option),
        };
        Err(Ineligible::NotEligible(reason))
    }
}

/// The options for `machine`. `candidates` are the tiers smallest first
/// (tiers not offered are left out here), `embedding_bytes` what the
/// embedder still needs, `configured` the configured generation model.
pub fn evaluate(
    machine: &Machine,
    candidates: &[Candidate],
    embedding_bytes: u64,
    configured: &str,
) -> Options {
    let missing_cpu_features = machine.cpu_features.missing_required();
    // Only the Windows binary is built with the x86-64 floor; elsewhere the
    // same CPU runs llama.cpp's slower paths.
    let cpu_blocks = machine.os == "windows" && !missing_cpu_features.is_empty();
    let slow_cpu = !cpu_blocks && !missing_cpu_features.is_empty();
    let mut options: Vec<TierOption> = candidates
        .iter()
        .filter(|candidate| candidate.tier.offered || candidate.current)
        .map(|candidate| option(machine, candidate, embedding_bytes))
        .collect();
    let fitting = options.iter().filter(|o| o.verdict.eligible()).count();
    let code = if cpu_blocks {
        Some(SupportCode::CpuUnsupported)
    } else if fitting == 0 {
        Some(SupportCode::NoTierFits)
    } else {
        None
    };
    if code.is_none()
        && let Some(default) = options
            .iter_mut()
            .find(|o| !o.current && o.size == Some(TierSize::Default) && o.verdict.eligible())
    {
        default.verdict = Verdict::Recommended;
    }
    let selected = code
        .is_none()
        .then(|| preselect(&options, configured))
        .flatten();
    Options {
        support: Support {
            status: match code {
                None => SupportStatus::Supported,
                Some(_) => SupportStatus::Unsupported,
            },
            code,
            missing_cpu_features,
        },
        warnings: [
            (code.is_none() && fitting == 1, Warning::SingleOption),
            (slow_cpu, Warning::SlowCpu),
        ]
        .into_iter()
        .filter_map(|(on, warning)| on.then_some(warning))
        .collect(),
        options,
        selected,
        embedding_bytes,
    }
}

/// One candidate's verdict before the recommendation: `fits` or why not.
fn option(machine: &Machine, candidate: &Candidate, embedding_bytes: u64) -> TierOption {
    let tier = candidate.tier;
    let download = if candidate.installed {
        0
    } else {
        candidate.bytes
    };
    let needed_disk = embedding_bytes
        .saturating_add(download)
        .saturating_add(DISK_HEADROOM_BYTES);
    let below = |have: Option<u64>, need: u64| have.is_some_and(|have| have < need);
    let verdict = if below(machine.total_ram_bytes, tier.need_ram_bytes) {
        Verdict::TooBigRam
    } else if below(machine.free_disk_bytes, needed_disk) {
        Verdict::TooBigDisk
    } else {
        Verdict::Fits
    };
    let low_memory = verdict.eligible() && below(machine.available_ram_bytes, tier.need_ram_bytes);
    TierOption {
        id: tier.id.clone(),
        size: (!candidate.current).then_some(tier.size),
        current: candidate.current,
        label_key: if candidate.current {
            "setup.tier.current"
        } else {
            tier.size.label_key()
        },
        bytes: candidate.bytes,
        installed: candidate.installed,
        need_ram_bytes: tier.need_ram_bytes,
        speed: tier.speed,
        verdict,
        warnings: if low_memory {
            vec![Warning::LowAvailableMemory]
        } else {
            Vec::new()
        },
    }
}

/// The configured generator when it is an option that fits, else the
/// recommended one, else the first that fits.
fn preselect(options: &[TierOption], configured: &str) -> Option<String> {
    options
        .iter()
        .find(|o| o.id == configured && o.verdict.eligible())
        .or_else(|| options.iter().find(|o| o.verdict == Verdict::Recommended))
        .or_else(|| options.iter().find(|o| o.verdict.eligible()))
        .map(|o| o.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::download::tiers::{builtin_tiers, current_tier};
    use crate::probe::{CpuFeatures, X86Features};

    const GIB: u64 = 1024 * 1024 * 1024;
    const EMBEDDER: u64 = 639_161_950;
    /// The pack sizes of the built-in tiers (weights + license).
    const PACKS: [u64; 3] = [1_280_847_384, 2_740_949_432, 5_680_534_008];

    fn machine(total_gib: u64, available_gib: u64, free_disk_gib: u64) -> Machine {
        Machine {
            os: "test",
            arch: "aarch64",
            total_ram_bytes: Some(total_gib * GIB),
            available_ram_bytes: Some(available_gib * GIB),
            logical_cores: Some(8),
            physical_cores: Some(8),
            cpu_model: Some("Test CPU".into()),
            cpu_features: CpuFeatures::NotApplicable,
            apple_silicon: false,
            metal: false,
            free_disk_bytes: Some(free_disk_gib * GIB),
            gpu: None,
        }
    }

    fn options_for(machine: &Machine, configured: &str) -> Options {
        let tiers = builtin_tiers();
        let catalog = crate::infer::download::Catalog::builtin();
        let current = current_tier(configured, &tiers, &catalog);
        let mut candidates: Vec<Candidate> = current
            .iter()
            .map(|tier| Candidate {
                tier,
                bytes: catalog.find(&tier.id).unwrap().total_size(),
                installed: false,
                current: true,
            })
            .collect();
        candidates.extend(tiers.iter().zip(PACKS).map(|(tier, bytes)| Candidate {
            tier,
            bytes,
            installed: false,
            current: false,
        }));
        evaluate(machine, &candidates, EMBEDDER, configured)
    }

    fn verdicts(options: &Options) -> Vec<Verdict> {
        options.options.iter().map(|o| o.verdict).collect()
    }

    use Verdict::{Fits, Recommended, TooBigDisk, TooBigRam};

    const DEFAULT: &str = "qwen3.5-4b-q4_k_m";
    const LARGE: &str = "qwen3.5-9b-q4_k_m";
    const SMALL: &str = "qwen3.5-2b-q4_k_m";
    const INSTRUCT: &str = "qwen3-4b-instruct-2507-q4_k_m";

    fn low_memory(options: &Options) -> Vec<bool> {
        options
            .options
            .iter()
            .map(|o| o.warnings.contains(&Warning::LowAvailableMemory))
            .collect()
    }

    fn ids(options: &Options) -> Vec<&str> {
        options.options.iter().map(|o| o.id.as_str()).collect()
    }

    fn x86(os: &'static str, avx2: bool) -> Machine {
        let mut m = machine(32, 32, 500);
        m.os = os;
        m.arch = "x86_64";
        m.cpu_features = CpuFeatures::X86(X86Features {
            avx2,
            fma: true,
            f16c: true,
            bmi2: true,
        });
        m
    }

    #[test]
    fn the_default_tier_is_recommended_whenever_it_fits() {
        // The offered tiers are the default (6.5 GiB) and the large (9 GiB).
        let table = [
            (4, [TooBigRam, TooBigRam], None, false),
            (6, [TooBigRam, TooBigRam], None, false),
            (8, [Recommended, TooBigRam], Some(DEFAULT), true),
            (16, [Recommended, Fits], Some(DEFAULT), false),
            (32, [Recommended, Fits], Some(DEFAULT), false),
        ];
        for (gib, expected, selected, single) in table {
            let options = options_for(&machine(gib, gib, 500), "");
            assert_eq!(verdicts(&options), expected, "{gib} GB");
            assert_eq!(options.selected.as_deref(), selected, "{gib} GB");
            assert_eq!(
                options.warnings.contains(&Warning::SingleOption),
                single,
                "{gib} GB"
            );
            assert_eq!(options.embedding_bytes, EMBEDDER);
        }
    }

    #[test]
    fn the_large_tier_is_never_recommended_but_a_configured_one_is_preselected() {
        let options = options_for(&machine(32, 32, 500), LARGE);
        assert_eq!(verdicts(&options), [Recommended, Fits]);
        assert_eq!(options.selected.as_deref(), Some(LARGE));
        assert!(options.eligible(LARGE).is_ok());
    }

    #[test]
    fn a_machine_below_the_default_need_is_unsupported() {
        let options = options_for(&machine(4, 4, 500), DEFAULT);
        assert_eq!(
            options.support,
            Support {
                status: SupportStatus::Unsupported,
                code: Some(SupportCode::NoTierFits),
                missing_cpu_features: vec![],
            }
        );
        assert_eq!(options.selected, None);
        assert!(options.warnings.is_empty());
        assert_eq!(
            options.eligible(DEFAULT),
            Err(Ineligible::NotEligible("no_tier_fits"))
        );
    }

    #[test]
    fn only_one_tier_that_fits_is_offered_with_a_warning() {
        let options = options_for(&machine(8, 8, 500), LARGE);
        assert_eq!(options.support.status, SupportStatus::Supported);
        assert_eq!(options.warnings, [Warning::SingleOption]);
        assert_eq!(options.selected.as_deref(), Some(DEFAULT));
        assert!(options.eligible(DEFAULT).is_ok());
        assert_eq!(
            options.eligible(LARGE),
            Err(Ineligible::NotEligible("too_big_ram"))
        );
    }

    #[test]
    fn low_disk_rules_out_the_tiers_that_do_not_fit_it() {
        // 5 GiB free: the 4B (2.74 GB + embedder + 512 MiB) fits, the 9B not.
        let options = options_for(&machine(16, 16, 5), "");
        assert_eq!(verdicts(&options), [Recommended, TooBigDisk]);
        assert_eq!(options.warnings, [Warning::SingleOption]);
        // RAM is checked first: a tier too big for both says RAM.
        let options = options_for(&machine(8, 8, 3), "");
        assert_eq!(verdicts(&options), [TooBigDisk, TooBigRam]);
        assert_eq!(options.support.code, Some(SupportCode::NoTierFits));
        // Installed files need no space: an installed 4B fits in 1 GiB.
        let tiers = builtin_tiers();
        let candidates: Vec<Candidate> = tiers
            .iter()
            .zip(PACKS)
            .map(|(tier, bytes)| Candidate {
                tier,
                bytes,
                installed: tier.id == DEFAULT,
                current: false,
            })
            .collect();
        let options = evaluate(&machine(16, 16, 1), &candidates, 0, "");
        assert_eq!(verdicts(&options), [Recommended, TooBigDisk]);
        assert_eq!(options.options[0].download_bytes(), 0);
        assert_eq!(options.options[1].download_bytes(), PACKS[2]);
    }

    #[test]
    fn unknown_ram_or_disk_never_blocks() {
        let mut unknown = machine(0, 0, 0);
        unknown.total_ram_bytes = None;
        unknown.available_ram_bytes = None;
        unknown.free_disk_bytes = None;
        let options = options_for(&unknown, "");
        assert_eq!(verdicts(&options), [Recommended, Fits]);
        assert!(options.options.iter().all(|o| o.warnings.is_empty()));
    }

    #[test]
    fn on_windows_a_cpu_without_avx2_is_unsupported() {
        let options = options_for(&x86("windows", false), "");
        assert_eq!(
            options.support,
            Support {
                status: SupportStatus::Unsupported,
                code: Some(SupportCode::CpuUnsupported),
                missing_cpu_features: vec!["avx2"],
            }
        );
        assert_eq!(options.selected, None);
        assert_eq!(verdicts(&options), [Fits, Fits], "nothing is recommended");
        assert_eq!(
            options.eligible(LARGE),
            Err(Ineligible::NotEligible("cpu_unsupported"))
        );
        // A CPU with the whole floor is supported.
        let ok = options_for(&x86("windows", true), "");
        assert_eq!(ok.support.status, SupportStatus::Supported);
        assert!(ok.warnings.is_empty());
    }

    #[test]
    fn on_linux_a_cpu_without_avx2_only_warns_that_it_is_slow() {
        let options = options_for(&x86("linux", false), "");
        assert_eq!(
            options.support,
            Support {
                status: SupportStatus::Supported,
                code: None,
                missing_cpu_features: vec!["avx2"],
            }
        );
        assert_eq!(options.warnings, [Warning::SlowCpu]);
        assert_eq!(verdicts(&options), [Recommended, Fits]);
        assert_eq!(options.selected.as_deref(), Some(DEFAULT));
        assert!(options.eligible(LARGE).is_ok());
        let fast = options_for(&x86("linux", true), "");
        assert!(fast.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(&options.warnings).unwrap(),
            serde_json::json!(["slow_cpu"])
        );
    }

    #[test]
    fn little_free_memory_warns_but_does_not_block() {
        let options = options_for(&machine(16, 6, 500), "");
        assert_eq!(verdicts(&options), [Recommended, Fits]);
        assert_eq!(low_memory(&options), [true, true], "6 GiB free");
        let options = options_for(&machine(16, 8, 500), "");
        assert_eq!(low_memory(&options), [false, true], "8 GiB free");
        assert!(options.eligible(LARGE).is_ok());
    }

    #[test]
    fn a_configured_catalog_generator_is_the_current_option() {
        let options = options_for(&machine(32, 32, 500), INSTRUCT);
        assert_eq!(ids(&options), [INSTRUCT, DEFAULT, LARGE]);
        let current = &options.options[0];
        assert!(current.current);
        assert_eq!(current.label_key, "setup.tier.current");
        assert_eq!(current.size, None);
        assert_eq!(
            current.need_ram_bytes,
            13 * GIB / 2,
            "the default tier's need"
        );
        assert_eq!(verdicts(&options), [Fits, Recommended, Fits]);
        assert_eq!(options.selected.as_deref(), Some(INSTRUCT));
        assert!(options.eligible(INSTRUCT).is_ok());
        // Too big here: it is listed with its verdict, the default selected.
        let options = options_for(&machine(32, 32, 3), INSTRUCT);
        assert_eq!(verdicts(&options), [TooBigDisk, TooBigDisk, TooBigDisk]);
        assert_eq!(options.support.code, Some(SupportCode::NoTierFits));
        let options = options_for(&machine(4, 4, 500), INSTRUCT);
        assert_eq!(verdicts(&options), [TooBigRam, TooBigRam, TooBigRam]);
    }

    #[test]
    fn a_configured_small_tier_is_offered_only_as_the_current_option() {
        let options = options_for(&machine(6, 6, 500), SMALL);
        assert_eq!(ids(&options), [SMALL, DEFAULT, LARGE]);
        assert_eq!(options.options[0].need_ram_bytes, 5 * GIB, "its own need");
        assert_eq!(verdicts(&options), [Fits, TooBigRam, TooBigRam]);
        assert_eq!(options.support.status, SupportStatus::Supported);
        assert_eq!(options.warnings, [Warning::SingleOption]);
        assert_eq!(options.selected.as_deref(), Some(SMALL));
        // Not configured: never offered.
        let options = options_for(&machine(32, 32, 500), DEFAULT);
        assert_eq!(ids(&options), [DEFAULT, LARGE]);
        assert_eq!(options.eligible(SMALL), Err(Ineligible::Unknown));
    }

    #[test]
    fn a_configured_id_outside_the_catalog_is_not_offered() {
        let options = options_for(&machine(32, 32, 500), "my-own-pack");
        assert_eq!(ids(&options), [DEFAULT, LARGE]);
        assert_eq!(options.selected.as_deref(), Some(DEFAULT));
        assert_eq!(options.eligible("my-own-pack"), Err(Ineligible::Unknown));
        // An embedder id is not a generator either.
        let options = options_for(&machine(32, 32, 500), "qwen3-embedding-0.6b-q8_0");
        assert_eq!(ids(&options), [DEFAULT, LARGE]);
    }

    #[test]
    fn a_withdrawn_tier_is_neither_listed_nor_eligible() {
        let mut tiers = builtin_tiers();
        tiers[2].offered = false;
        let candidates: Vec<Candidate> = tiers
            .iter()
            .zip(PACKS)
            .map(|(tier, bytes)| Candidate {
                tier,
                bytes,
                installed: false,
                current: false,
            })
            .collect();
        let options = evaluate(&machine(32, 32, 500), &candidates, EMBEDDER, "");
        assert_eq!(verdicts(&options), [Recommended]);
        assert_eq!(options.warnings, [Warning::SingleOption]);
        assert_eq!(options.eligible(LARGE), Err(Ineligible::Unknown));
        assert_eq!(options.eligible("nope"), Err(Ineligible::Unknown));
    }

    #[test]
    fn an_option_carries_what_the_page_shows() {
        let options = options_for(&machine(16, 16, 500), "");
        let default = &options.options[0];
        assert_eq!(default.id, DEFAULT);
        assert_eq!(default.label_key, "setup.tier.default");
        assert_eq!(options.options[1].label_key, "setup.tier.large");
        assert_eq!(default.bytes, PACKS[1]);
        assert_eq!(default.need_ram_bytes, 13 * GIB / 2);
        let json = serde_json::to_value(default).unwrap();
        assert_eq!(json["verdict"], "recommended");
        assert_eq!(json["size"], "default");
        assert_eq!(json["current"], false);
        assert_eq!(json["installed"], false);
        assert!((json["speed"]["metal_p50_answer_s"].as_f64().unwrap() - 0.9).abs() < 1e-6);
        assert_eq!(json["speed"]["cpu_p50_answer_s"], serde_json::Value::Null);
        let json = serde_json::to_value(options_for(&machine(4, 4, 500), "")).unwrap();
        assert_eq!(json["support"]["status"], "unsupported");
        assert_eq!(json["support"]["code"], "no_tier_fits");
        assert_eq!(json["options"][0]["verdict"], "too_big_ram");
        assert_eq!(json["selected"], serde_json::Value::Null);
    }
}
