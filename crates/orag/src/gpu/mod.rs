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

/// `gpu` in `config.toml`: `"auto"` (the default) lets `orag-vulkan` use a
/// GPU, `"off"` keeps every model on the CPU. The CPU `orag` and the macOS
/// build ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuSetting {
    #[default]
    Auto,
    Off,
}

impl GpuSetting {
    pub fn parse(value: Option<&str>) -> std::result::Result<GpuSetting, String> {
        match value {
            None | Some("auto") => Ok(GpuSetting::Auto),
            Some("off") => Ok(GpuSetting::Off),
            Some(other) => Err(format!("gpu = {other:?}; use \"auto\" or \"off\"")),
        }
    }
}

/// Where a model's layers go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// What the build does on its own: every layer on Metal on Apple
    /// Silicon, the CPU elsewhere. No fit, no offload record.
    BuildDefault,
    /// Fitted to this device's free memory (`orag-vulkan`).
    Gpu { index: usize, name: String },
    /// No GPU device at all, for a stated reason.
    CpuOnly { reason: &'static str },
}

/// `orag-vulkan`'s placement, by precedence: `gpu = "off"`, then the
/// device the desktop passed in `ORAG_GPU_DEVICE` (any listed device; an
/// index that is not listed is an error), then [`choose_device`].
pub fn decide_placement(
    setting: GpuSetting,
    env_device: Option<&str>,
    devices: &[GpuDevice],
    allow_software: bool,
) -> std::result::Result<Placement, String> {
    if setting == GpuSetting::Off {
        return Ok(Placement::CpuOnly { reason: "gpu_off" });
    }
    let chosen = match env_device {
        Some(text) => {
            let listed = text
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|index| devices.iter().find(|device| device.index == index));
            Some(listed.ok_or_else(|| {
                let known: Vec<String> = devices
                    .iter()
                    .map(|d| format!("{} ({})", d.index, d.name))
                    .collect();
                format!(
                    "ORAG_GPU_DEVICE={text:?} is not a GPU device here; devices: [{}]",
                    known.join(", ")
                )
            })?)
        }
        None => choose_device(devices, allow_software),
    };
    Ok(match chosen {
        Some(device) => Placement::Gpu {
            index: device.index,
            name: device.name.clone(),
        },
        None => Placement::CpuOnly {
            reason: "no_eligible_device",
        },
    })
}

/// What one model's load did with the GPU (`orag-vulkan`; v0.6 plan G-4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offload {
    pub layers_on_gpu: u32,
    pub layers_total: u32,
    /// Tensors the fit kept in host memory although their layer is on the
    /// GPU (a mixture-of-experts model's experts).
    pub cpu_tensor_overrides: usize,
    /// The device the layers went to; `None` for a CPU-only load.
    pub device: Option<String>,
    /// Why nothing went to the GPU: `gpu_off`, `no_eligible_device`,
    /// `fit_failed`, `fit_zero_layers`, `alloc_failed`.
    pub reason: Option<&'static str>,
}

impl Offload {
    /// Every layer and every tensor on the GPU: only then is the model "on
    /// the GPU" (G-5); anything less is a partial offload.
    pub fn is_full(&self) -> bool {
        self.layers_total > 0
            && self.layers_on_gpu == self.layers_total
            && self.cpu_tensor_overrides == 0
    }
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

    fn devices() -> Vec<GpuDevice> {
        vec![
            device(0, "Intel(R) UHD Graphics 770", DeviceKind::Integrated, 8),
            device(1, "NVIDIA GeForce RTX 3060", DeviceKind::Discrete, 10),
        ]
    }

    #[test]
    fn gpu_off_wins_over_everything() {
        let placement = decide_placement(GpuSetting::Off, Some("1"), &devices(), false).unwrap();
        assert_eq!(placement, Placement::CpuOnly { reason: "gpu_off" });
    }

    #[test]
    fn an_explicit_device_wins_over_the_automatic_choice() {
        let placement = decide_placement(GpuSetting::Auto, Some("0"), &devices(), false).unwrap();
        assert_eq!(
            placement,
            Placement::Gpu {
                index: 0,
                name: "Intel(R) UHD Graphics 770".into()
            }
        );
        let placement = decide_placement(GpuSetting::Auto, None, &devices(), false).unwrap();
        assert_eq!(
            placement,
            Placement::Gpu {
                index: 1,
                name: "NVIDIA GeForce RTX 3060".into()
            }
        );
    }

    #[test]
    fn an_invalid_explicit_device_is_an_error_that_names_it() {
        for bad in ["7", "x", "-1", ""] {
            let err = decide_placement(GpuSetting::Auto, Some(bad), &devices(), false).unwrap_err();
            assert!(err.contains("ORAG_GPU_DEVICE"), "{err}");
        }
    }

    #[test]
    fn no_eligible_device_is_a_cpu_only_load_with_its_reason() {
        let only_integrated = &devices()[..1];
        let placement = decide_placement(GpuSetting::Auto, None, only_integrated, false).unwrap();
        assert_eq!(
            placement,
            Placement::CpuOnly {
                reason: "no_eligible_device"
            }
        );
    }

    #[test]
    fn a_model_is_on_the_gpu_only_when_nothing_stayed_behind() {
        let offload = |layers_on_gpu, cpu_tensor_overrides| Offload {
            layers_on_gpu,
            layers_total: 33,
            cpu_tensor_overrides,
            device: Some("NVIDIA GeForce RTX 3060".into()),
            reason: None,
        };
        assert!(offload(33, 0).is_full());
        assert!(!offload(20, 0).is_full(), "some layers on the CPU");
        assert!(!offload(33, 2).is_full(), "experts kept in host memory");
        assert!(!offload(0, 0).is_full());
    }

    #[test]
    fn the_setting_reads_auto_and_off_only() {
        assert_eq!(GpuSetting::parse(None).unwrap(), GpuSetting::Auto);
        assert_eq!(GpuSetting::parse(Some("auto")).unwrap(), GpuSetting::Auto);
        assert_eq!(GpuSetting::parse(Some("off")).unwrap(), GpuSetting::Off);
        assert!(GpuSetting::parse(Some("on")).unwrap_err().contains("gpu"));
    }
}
