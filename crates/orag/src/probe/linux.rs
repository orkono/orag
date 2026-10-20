//! Linux: `/proc/meminfo`, `/proc/cpuinfo` and the DRM cards under
//! `/sys/class/drm` (driver name, AMD video memory, the NVIDIA driver's
//! model line). Plain file reads; nothing is spawned.

use std::fs;
use std::path::Path;

use super::parse::{best_gpu, cpuinfo, gpu, is_card, meminfo, nvidia_model};
use super::{Gpu, StaticFacts};

/// More cards than this are not read (a probe must stay fast).
const MAX_CARDS: usize = 16;

pub fn static_facts() -> StaticFacts {
    let total = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| meminfo(&text).0);
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .map(|text| cpuinfo(&text))
        .unwrap_or_default();
    StaticFacts {
        total_ram_bytes: total,
        logical_cores: cpu.logical,
        physical_cores: cpu.physical,
        cpu_model: cpu.model,
        gpu: best_gpu(cards()),
    }
}

pub fn available_ram_bytes() -> Option<u64> {
    meminfo(&fs::read_to_string("/proc/meminfo").ok()?).1
}

fn cards() -> Vec<Gpu> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_str().is_some_and(is_card))
        .take(MAX_CARDS)
        .filter_map(|entry| {
            let device = entry.path().join("device");
            let vendor = fs::read_to_string(device.join("vendor")).ok()?;
            let driver = fs::read_link(device.join("driver"))
                .ok()
                .and_then(|link| link.file_name()?.to_str().map(str::to_string));
            let vram = fs::read_to_string(device.join("mem_info_vram_total"))
                .ok()
                .and_then(|text| text.trim().parse().ok());
            let model = (driver.as_deref() == Some("nvidia"))
                .then(|| nvidia_model_at(&device))
                .flatten();
            Some(gpu(&vendor, driver.as_deref(), model, vram))
        })
        .collect()
}

/// The NVIDIA driver's model line for the card at `device`, found by its
/// PCI address (`/proc/driver/nvidia/gpus/0000:01:00.0/information`).
fn nvidia_model_at(device: &Path) -> Option<String> {
    let resolved = fs::canonicalize(device).ok()?;
    let info = Path::new("/proc/driver/nvidia/gpus")
        .join(resolved.file_name()?)
        .join("information");
    nvidia_model(&fs::read_to_string(info).ok()?)
}
