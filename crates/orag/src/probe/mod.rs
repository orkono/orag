//! What this computer has, for the setup page's generator choice (step 82):
//! RAM, CPU, cores, the x86 instructions the shipped binary needs, Apple
//! Silicon (Metal), free disk in the models folder and, as information only,
//! the GPU. No subprocess and no network; the facts that do not change while
//! ORAG runs are read once per process, available RAM and free disk on every
//! call (each one cheap system call).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", windows, test))]
mod parse;
#[cfg(windows)]
mod windows;
mod x86;

use std::path::Path;
use std::sync::OnceLock;

use serde::{Serialize, Serializer};

pub use x86::X86Features;

/// A graphics adapter, reported and never used to decide anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gpu {
    pub name: Option<String>,
    /// Dedicated video memory; `None` when the platform does not say.
    pub memory_bytes: Option<u64>,
}

/// The x86-64 instructions found, or "not applicable" on other CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuFeatures {
    X86(X86Features),
    NotApplicable,
}

impl Serialize for CpuFeatures {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            CpuFeatures::X86(features) => features.serialize(serializer),
            CpuFeatures::NotApplicable => serializer.serialize_str("not_applicable"),
        }
    }
}

impl CpuFeatures {
    /// The instructions the shipped x86-64 binary is built for
    /// (`.cargo/config.toml`, `scripts/platform-floors.sh`) that this CPU
    /// lacks; empty on other CPUs.
    pub fn missing_required(&self) -> Vec<&'static str> {
        match self {
            CpuFeatures::X86(features) => features.missing(),
            CpuFeatures::NotApplicable => Vec::new(),
        }
    }
}

/// The machine as `GET /v1/models` reports it (`machine`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Machine {
    pub os: &'static str,
    pub arch: &'static str,
    pub total_ram_bytes: Option<u64>,
    pub available_ram_bytes: Option<u64>,
    pub logical_cores: Option<u32>,
    pub physical_cores: Option<u32>,
    pub cpu_model: Option<String>,
    pub cpu_features: CpuFeatures,
    /// An Apple Silicon Mac: llama.cpp runs on its GPU through Metal.
    pub apple_silicon: bool,
    pub metal: bool,
    /// Free space where the models are installed.
    pub free_disk_bytes: Option<u64>,
    pub gpu: Option<Gpu>,
}

/// What does not change while the process runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StaticFacts {
    pub total_ram_bytes: Option<u64>,
    pub logical_cores: Option<u32>,
    pub physical_cores: Option<u32>,
    pub cpu_model: Option<String>,
    pub gpu: Option<Gpu>,
}

const APPLE_SILICON: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

fn static_facts() -> &'static StaticFacts {
    static FACTS: OnceLock<StaticFacts> = OnceLock::new();
    FACTS.get_or_init(|| {
        let mut facts = os::static_facts();
        if facts.cpu_model.is_none() {
            facts.cpu_model = x86::brand();
        }
        if facts.logical_cores.is_none() {
            facts.logical_cores = std::thread::available_parallelism()
                .ok()
                .and_then(|n| u32::try_from(n.get()).ok());
        }
        facts
    })
}

/// Probes this machine; `models_dir` (or its nearest existing parent, on a
/// fresh home) is where free disk is measured.
pub fn probe(models_dir: &Path) -> Machine {
    let facts = static_facts();
    Machine {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        total_ram_bytes: facts.total_ram_bytes,
        available_ram_bytes: os::available_ram_bytes(),
        logical_cores: facts.logical_cores,
        physical_cores: facts.physical_cores,
        cpu_model: facts.cpu_model.clone(),
        cpu_features: x86::features(),
        apple_silicon: APPLE_SILICON,
        metal: APPLE_SILICON,
        free_disk_bytes: free_disk(models_dir),
        gpu: facts.gpu.clone(),
    }
}

fn free_disk(models_dir: &Path) -> Option<u64> {
    let existing = models_dir.ancestors().find(|dir| dir.is_dir())?;
    crate::infer::download::free_bytes(existing)
}

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(windows)]
use windows as os;

/// Platforms without a probe report only what std knows.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    pub fn static_facts() -> super::StaticFacts {
        super::StaticFacts::default()
    }

    pub fn available_ram_bytes() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn this_machine_reports_sane_facts_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let machine = probe(&dir.path().join("not-yet").join("models"));
        let elapsed = started.elapsed();
        assert!(elapsed.as_millis() < 100, "the probe took {elapsed:?}");
        let total = machine.total_ram_bytes.expect("total RAM is known here");
        assert!((GIB..=64 * 1024 * GIB).contains(&total), "{total}");
        let available = machine.available_ram_bytes.expect("available RAM is known");
        assert!(
            available > 0 && available <= total,
            "{available} of {total}"
        );
        let logical = machine.logical_cores.unwrap();
        assert!((1..=4096).contains(&logical), "{logical}");
        if let Some(physical) = machine.physical_cores {
            assert!(physical >= 1 && physical <= logical, "{physical}/{logical}");
        }
        assert!(machine.free_disk_bytes.is_some_and(|free| free > 0));
        if cfg!(target_arch = "x86_64") {
            assert!(matches!(machine.cpu_features, CpuFeatures::X86(_)));
            assert!(machine.cpu_model.is_some());
        } else {
            assert_eq!(machine.cpu_features, CpuFeatures::NotApplicable);
            assert!(machine.cpu_features.missing_required().is_empty());
        }
        assert_eq!(machine.metal, APPLE_SILICON);
        if APPLE_SILICON {
            assert!(machine.cpu_model.as_deref().unwrap().starts_with("Apple"));
        }
        // A second probe reuses the static facts.
        let again = probe(dir.path());
        assert_eq!(again.total_ram_bytes, machine.total_ram_bytes);
        assert_eq!(again.cpu_model, machine.cpu_model);
    }

    #[test]
    fn the_features_serialize_as_an_object_or_not_applicable() {
        let x86 = CpuFeatures::X86(X86Features {
            avx2: true,
            fma: true,
            f16c: false,
            bmi2: true,
        });
        assert_eq!(
            serde_json::to_value(x86).unwrap(),
            serde_json::json!({"avx2": true, "fma": true, "f16c": false, "bmi2": true})
        );
        assert_eq!(x86.missing_required(), ["f16c"]);
        assert_eq!(
            serde_json::to_value(CpuFeatures::NotApplicable).unwrap(),
            serde_json::json!("not_applicable")
        );
    }
}
