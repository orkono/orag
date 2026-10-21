//! GPU devices and the rule that picks one (v0.6 plan, G-3 and Task 2).
//! The rule is pure and lives in every build; only `orag-vulkan` has devices
//! to give it (`crate::infer::llama::devices`).

use serde::Serialize;

/// Free memory a device needs before it is chosen.
pub const MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Names of software rasterisers: Vulkan devices that run on the CPU.
const SOFTWARE_DEVICE_NAMES: [&str; 3] = ["llvmpipe", "lavapipe", "swiftshader"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuDevice {
    /// llama.cpp's device index, as `LlamaModelParams::with_devices` takes it.
    pub index: usize,
    /// What the driver calls it, e.g. "NVIDIA GeForce RTX 3060".
    pub name: String,
    pub kind: DeviceKind,
    pub memory_total: u64,
    pub memory_free: u64,
}

impl GpuDevice {
    fn is_software(&self) -> bool {
        let name = self.name.to_lowercase();
        SOFTWARE_DEVICE_NAMES.iter().any(|soft| name.contains(soft))
    }
}

/// The device ORAG uses: the discrete GPU with the most free memory, at
/// least [`MIN_FREE_BYTES`] free. Integrated GPUs and other device types are
/// never chosen; neither are software rasterisers unless `allow_software`
/// (tests only: `ORAG_VULKAN_ALLOW_SOFTWARE=1`), which also lets a software
/// device stand in for a discrete one whatever type it reports.
pub fn choose_device(devices: &[GpuDevice], allow_software: bool) -> Option<&GpuDevice> {
    devices
        .iter()
        .filter(|device| device.memory_free >= MIN_FREE_BYTES)
        .filter(|device| {
            if device.is_software() {
                allow_software
            } else {
                device.kind == DeviceKind::Discrete
            }
        })
        .max_by_key(|device| (device.memory_free, std::cmp::Reverse(device.index)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn device(index: usize, name: &str, kind: DeviceKind, free_gib: u64) -> GpuDevice {
        GpuDevice {
            index,
            name: name.into(),
            kind,
            memory_total: free_gib * GIB + GIB,
            memory_free: free_gib * GIB,
        }
    }

    #[test]
    fn the_discrete_gpu_with_the_most_free_memory_is_chosen() {
        let devices = [
            device(0, "Intel(R) UHD Graphics 630", DeviceKind::Integrated, 12),
            device(1, "NVIDIA GeForce GTX 1650", DeviceKind::Discrete, 3),
            device(2, "NVIDIA GeForce RTX 3060", DeviceKind::Discrete, 11),
        ];
        assert_eq!(choose_device(&devices, false).map(|d| d.index), Some(2));
    }

    #[test]
    fn integrated_other_and_small_devices_are_never_chosen() {
        let devices = [
            device(0, "AMD Radeon(TM) Graphics", DeviceKind::Integrated, 8),
            device(1, "Some accelerator", DeviceKind::Other, 16),
            device(2, "NVIDIA GeForce GT 1030", DeviceKind::Discrete, 1),
        ];
        assert_eq!(choose_device(&devices, false), None);
        assert_eq!(choose_device(&[], false), None);
    }

    #[test]
    fn exactly_two_gib_free_is_enough() {
        let mut gpu = device(0, "AMD Radeon RX 6500 XT", DeviceKind::Discrete, 2);
        assert_eq!(
            choose_device(std::slice::from_ref(&gpu), false).map(|d| d.index),
            Some(0)
        );
        gpu.memory_free -= 1;
        assert_eq!(choose_device(&[gpu], false), None);
    }

    #[test]
    fn a_software_rasteriser_is_chosen_only_when_tests_allow_it() {
        let devices = [
            device(
                0,
                "llvmpipe (LLVM 15.0.7, 256 bits)",
                DeviceKind::Discrete,
                8,
            ),
            device(1, "SwiftShader Device (Subzero)", DeviceKind::Discrete, 8),
        ];
        assert_eq!(choose_device(&devices, false), None);
        let lavapipe = [device(
            0,
            "llvmpipe (LLVM 15.0.7, 256 bits)",
            DeviceKind::Other,
            8,
        )];
        assert_eq!(choose_device(&lavapipe, true).map(|d| d.index), Some(0));
    }

    #[test]
    fn equal_free_memory_prefers_the_lower_index() {
        let devices = [
            device(0, "AMD Radeon RX 7600", DeviceKind::Discrete, 7),
            device(1, "AMD Radeon RX 7600", DeviceKind::Discrete, 7),
        ];
        assert_eq!(choose_device(&devices, false).map(|d| d.index), Some(0));
    }

    #[test]
    fn devices_serialise_as_the_probe_prints_them() {
        let json = serde_json::to_value(device(
            3,
            "NVIDIA GeForce RTX 4070",
            DeviceKind::Discrete,
            10,
        ))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "index": 3, "name": "NVIDIA GeForce RTX 4070", "kind": "discrete",
                "memory_total": 11 * GIB, "memory_free": 10 * GIB
            })
        );
    }
}
