//! The pure parts of the per-OS probes (unit-tested on every platform):
//! the text of Linux's `/proc` and `/sys` files, and the adapters DXGI
//! lists on Windows.
// Each OS build uses only its own half.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;

use super::Gpu;

/// `DXGI_ADAPTER_FLAG_SOFTWARE`: the Basic Render Driver (WARP).
pub const DXGI_SOFTWARE_ADAPTER: u32 = 2;

/// A DXGI adapter from its description (UTF-16, NUL-padded), dedicated
/// video memory and flags; `None` for the software rasterizer.
pub fn dxgi_adapter(description: &[u16], dedicated_video_memory: u64, flags: u32) -> Option<Gpu> {
    if flags & DXGI_SOFTWARE_ADAPTER != 0 {
        return None;
    }
    let end = description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(description.len());
    let name = String::from_utf16_lossy(&description[..end])
        .trim()
        .to_string();
    Some(Gpu {
        name: (!name.is_empty()).then_some(name),
        memory_bytes: (dedicated_video_memory > 0).then_some(dedicated_video_memory),
    })
}

/// `MemTotal` and `MemAvailable` of `/proc/meminfo`, in bytes.
pub fn meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let mut parts = rest.split_whitespace();
            let value: u64 = parts.next()?.parse().ok()?;
            match parts.next() {
                Some("kB") => value.checked_mul(1024),
                None => Some(value),
                Some(_) => None,
            }
        })
    };
    (field("MemTotal"), field("MemAvailable"))
}

/// What `/proc/cpuinfo` says about the CPU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuInfo {
    pub model: Option<String>,
    pub logical: Option<u32>,
    /// Distinct (`physical id`, `core id`) pairs; `None` when the file has
    /// no core ids (most ARM kernels, some virtual machines).
    pub physical: Option<u32>,
}

pub fn cpuinfo(text: &str) -> CpuInfo {
    let mut model = None;
    let mut logical = 0u32;
    let mut cores = HashSet::new();
    let (mut package, mut core) = (None, None);
    let mut flush = |package: &mut Option<String>, core: &mut Option<String>| {
        if let Some(core) = core.take() {
            cores.insert((package.take().unwrap_or_default(), core));
        }
        *package = None;
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut package, &mut core);
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" => logical += 1,
            "model name" if model.is_none() && !value.is_empty() => model = Some(value.to_string()),
            "physical id" => package = Some(value.to_string()),
            "core id" => core = Some(value.to_string()),
            _ => {}
        }
    }
    flush(&mut package, &mut core);
    let count = |n: usize| u32::try_from(n).ok().filter(|&n| n > 0);
    CpuInfo {
        model,
        logical: count(logical as usize),
        physical: count(cores.len()),
    }
}

/// A `/sys/class/drm/cardN` entry: `cardN` itself, not a connector such as
/// `card0-HDMI-A-1` or a render node.
pub fn is_card(name: &str) -> bool {
    name.strip_prefix("card")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The `Model:` line of `/proc/driver/nvidia/gpus/<bus>/information`.
pub fn nvidia_model(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix("Model:")?.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// One DRM card from its PCI vendor id (`0x10de`), its kernel driver and,
/// when known, its model name and video memory.
pub fn gpu(vendor: &str, driver: Option<&str>, model: Option<String>, vram: Option<u64>) -> Gpu {
    let vendor_name = match vendor.trim().to_ascii_lowercase().as_str() {
        "0x10de" => Some("NVIDIA"),
        "0x1002" => Some("AMD"),
        "0x8086" => Some("Intel"),
        _ => None,
    };
    let name = model.or_else(|| match (vendor_name, driver) {
        (Some(vendor), Some(driver)) => Some(format!("{vendor} GPU ({driver})")),
        (Some(vendor), None) => Some(format!("{vendor} GPU")),
        (None, Some(driver)) => Some(format!("GPU ({driver})")),
        (None, None) => None,
    });
    Gpu {
        name,
        memory_bytes: vram.filter(|&bytes| bytes > 0),
    }
}

/// The card to report: the one with the most video memory, a named card
/// before an unnamed one.
pub fn best_gpu(cards: Vec<Gpu>) -> Option<Gpu> {
    cards
        .into_iter()
        .max_by_key(|gpu| (gpu.memory_bytes.unwrap_or(0), gpu.name.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dxgi_adapter_is_named_and_the_software_one_skipped() {
        let mut description = [0u16; 128];
        for (slot, c) in description
            .iter_mut()
            .zip("NVIDIA GeForce RTX 4060 ".encode_utf16())
        {
            *slot = c;
        }
        assert_eq!(
            dxgi_adapter(&description, 8 << 30, 0),
            Some(Gpu {
                name: Some("NVIDIA GeForce RTX 4060".into()),
                memory_bytes: Some(8 << 30)
            })
        );
        assert_eq!(dxgi_adapter(&description, 0, DXGI_SOFTWARE_ADAPTER), None);
        let unnamed = dxgi_adapter(&[0; 128], 0, 0).unwrap();
        assert_eq!((unnamed.name, unnamed.memory_bytes), (None, None));
    }

    #[test]
    fn meminfo_reads_total_and_available_in_bytes() {
        let text =
            "MemTotal:       16303428 kB\nMemFree:         1234 kB\nMemAvailable:   9876543 kB\n";
        assert_eq!(
            meminfo(text),
            (Some(16_303_428 * 1024), Some(9_876_543 * 1024))
        );
        // An old kernel without MemAvailable, and garbage.
        assert_eq!(meminfo("MemTotal: 1024 kB\n"), (Some(1024 * 1024), None));
        assert_eq!(meminfo("MemTotal: lots\nMemTotalX: 5 kB"), (None, None));
        assert_eq!(meminfo(""), (None, None));
    }

    #[test]
    fn cpuinfo_counts_threads_and_distinct_cores() {
        let block = |n: u32, package: u32, core: u32| {
            format!(
                "processor\t: {n}\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Core(TM) i7-4770 CPU @ 3.40GHz\nphysical id\t: {package}\ncore id\t\t: {core}\n\n"
            )
        };
        // 4 cores with 2 threads each.
        let text: String = (0..8).map(|n| block(n, 0, n % 4)).collect();
        let info = cpuinfo(&text);
        assert_eq!(
            info.model.as_deref(),
            Some("Intel(R) Core(TM) i7-4770 CPU @ 3.40GHz")
        );
        assert_eq!((info.logical, info.physical), (Some(8), Some(4)));
        // Two sockets with the same core ids are distinct cores.
        let text: String = (0..4).map(|n| block(n, n / 2, n % 2)).collect();
        assert_eq!(cpuinfo(&text).physical, Some(4));
    }

    #[test]
    fn cpuinfo_without_core_ids_leaves_physical_unknown() {
        let text = "processor\t: 0\nBogoMIPS\t: 50.00\n\nprocessor\t: 1\nBogoMIPS\t: 50.00\n";
        assert_eq!(
            cpuinfo(text),
            CpuInfo {
                model: None,
                logical: Some(2),
                physical: None
            }
        );
    }

    #[test]
    fn only_cards_count_and_names_come_from_the_driver() {
        assert!(is_card("card0") && is_card("card12"));
        assert!(!is_card("card0-HDMI-A-1") && !is_card("renderD128") && !is_card("card"));
        let info = "Model: \t\t NVIDIA GeForce RTX 3080\nIRQ:   140\n";
        assert_eq!(
            nvidia_model(info).as_deref(),
            Some("NVIDIA GeForce RTX 3080")
        );
        let nvidia = gpu("0x10de", Some("nvidia"), nvidia_model(info), None);
        assert_eq!(nvidia.name.as_deref(), Some("NVIDIA GeForce RTX 3080"));
        let amd = gpu("0x1002\n", Some("amdgpu"), None, Some(8 << 30));
        assert_eq!(
            (amd.name.as_deref(), amd.memory_bytes),
            (Some("AMD GPU (amdgpu)"), Some(8 << 30))
        );
        let intel = gpu("0x8086", Some("i915"), None, Some(0));
        assert_eq!(
            (intel.name.as_deref(), intel.memory_bytes),
            (Some("Intel GPU (i915)"), None)
        );
        assert_eq!(gpu("0x1234", None, None, None).name, None);
        assert_eq!(best_gpu(vec![intel.clone(), amd.clone()]), Some(amd));
        assert_eq!(best_gpu(vec![]), None);
    }
}
