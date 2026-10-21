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

/// Which inference backend this executable was built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// Apple Silicon: every layer on the GPU through Metal.
    Metal,
    /// `orag-vulkan`.
    Vulkan,
    /// The CPU `orag` on Windows and Linux.
    Cpu,
}

impl Build {
    /// `metal`, `vulkan` or `cpu`.
    pub fn name(self) -> &'static str {
        match self {
            Build::Metal => "metal",
            Build::Vulkan => "vulkan",
            Build::Cpu => "cpu",
        }
    }

    pub fn current() -> Build {
        if cfg!(feature = "vulkan") {
            Build::Vulkan
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Build::Metal
        } else {
            Build::Cpu
        }
    }
}

/// The env var the desktop app passes a CPU sidecar after a GPU fallback.
pub const FALLBACK_REASON_ENV: &str = "ORAG_GPU_FALLBACK_REASON";
/// Set only by the desktop supervisor, which restarts on `EXIT_GPU_RETRY`.
pub const GPU_RETRY_ENV: &str = "ORAG_GPU_RETRY";

/// What the page shows about the GPU (`GET /v1/version`, `GET /v1/models`;
/// v0.6 plan, G-5). Computed once at start from the generator's recorded
/// offload; the desktop's fallback marker only supplies `reason`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuStatus {
    /// `metal`, `vulkan` or `cpu`: what this executable can use.
    pub backend: &'static str,
    /// The answer model has layers on a GPU.
    pub in_use: bool,
    /// ... all of them, and no tensor kept in host memory.
    pub full: bool,
    pub layers_on_gpu: Option<u32>,
    pub layers_total: Option<u32>,
    pub device: Option<String>,
    /// Why the answer model is not on a GPU; `None` when that is a choice
    /// (`gpu = "off"`, the CPU `orag` run directly) or it is on one.
    pub reason: Option<String>,
    /// The page may offer "try the graphics card again" (`POST
    /// /v1/system/gpu-retry`): a desktop fallback, under the desktop app.
    pub retry: bool,
}

impl GpuStatus {
    pub fn of(
        build: Build,
        generator: Option<&Offload>,
        fallback_reason: Option<String>,
        retry_supported: bool,
    ) -> GpuStatus {
        let (in_use, full) = match (build, generator) {
            (Build::Metal, _) => (true, true),
            (_, Some(offload)) => (offload.layers_on_gpu > 0, offload.is_full()),
            (_, None) => (false, false),
        };
        // A model on a GPU needs no reason (Metal never falls back).
        let reason = if in_use {
            None
        } else {
            fallback_reason.filter(|r| !r.is_empty()).or_else(|| {
                generator
                    .and_then(|offload| offload.reason)
                    .filter(|reason| *reason != "gpu_off")
                    .map(String::from)
            })
        };
        let retry = retry_supported && build == Build::Cpu && reason.is_some();
        GpuStatus {
            backend: build.name(),
            in_use,
            full,
            layers_on_gpu: generator.map(|o| o.layers_on_gpu),
            layers_total: generator.map(|o| o.layers_total),
            device: generator.and_then(|o| o.device.clone()),
            reason,
            retry,
        }
    }

    /// The setup page's service, with no answer model loaded: nothing is on
    /// a GPU, Metal included (`of` takes a Metal build's model as on it). A
    /// desktop fallback keeps its reason and retry.
    pub fn in_setup(self) -> GpuStatus {
        GpuStatus {
            in_use: false,
            full: false,
            ..self
        }
    }

    /// From this process's environment.
    pub fn from_env(generator: Option<&Offload>) -> GpuStatus {
        GpuStatus::of(
            Build::current(),
            generator,
            std::env::var(FALLBACK_REASON_ENV).ok(),
            std::env::var_os(GPU_RETRY_ENV).is_some_and(|v| v == "1"),
        )
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

    fn gen_offload(on: u32, reason: Option<&'static str>) -> Offload {
        Offload {
            layers_on_gpu: on,
            layers_total: 33,
            cpu_tensor_overrides: 0,
            device: (on > 0).then(|| "NVIDIA GeForce RTX 3060".into()),
            reason,
        }
    }

    #[test]
    fn the_status_comes_from_the_generator_offload_never_the_marker() {
        let full = GpuStatus::of(Build::Vulkan, Some(&gen_offload(33, None)), None, true);
        assert_eq!(
            (
                full.in_use,
                full.full,
                full.device.as_deref(),
                full.reason,
                full.retry
            ),
            (true, true, Some("NVIDIA GeForce RTX 3060"), None, false)
        );
        let partial = GpuStatus::of(Build::Vulkan, Some(&gen_offload(20, None)), None, true);
        assert_eq!((partial.in_use, partial.full), (true, false));
        assert_eq!(
            (partial.layers_on_gpu, partial.layers_total),
            (Some(20), Some(33))
        );
    }

    #[test]
    fn a_desktop_fallback_reason_comes_from_the_environment_and_offers_a_retry() {
        for reason in ["crashed-twice", "no-eligible-device", "loader-missing"] {
            let cpu = GpuStatus::of(Build::Cpu, None, Some(reason.into()), true);
            assert_eq!(
                (cpu.in_use, cpu.reason.as_deref(), cpu.retry),
                (false, Some(reason), true)
            );
        }
        // Without the desktop supervisor's retry, no button.
        let cli = GpuStatus::of(Build::Cpu, None, Some("crashed-twice".into()), false);
        assert!(!cli.retry);
    }

    #[test]
    fn the_cpu_by_choice_has_no_reason() {
        let direct = GpuStatus::of(Build::Cpu, None, None, false);
        assert_eq!(
            (direct.in_use, direct.reason.clone(), direct.retry),
            (false, None, false)
        );
        let off = GpuStatus::of(
            Build::Vulkan,
            Some(&gen_offload(0, Some("gpu_off"))),
            None,
            true,
        );
        assert_eq!(
            (off.in_use, off.reason.clone(), off.retry),
            (false, None, false)
        );
        let metal = GpuStatus::of(Build::Metal, None, Some("crashed-twice".into()), true);
        assert_eq!(
            (metal.backend, metal.in_use, metal.reason, metal.retry),
            ("metal", true, None, false)
        );
    }

    #[test]
    fn setup_mode_has_no_model_on_any_gpu() {
        for build in [Build::Metal, Build::Vulkan, Build::Cpu] {
            let setup = GpuStatus::of(build, None, None, false).in_setup();
            assert_eq!(
                (setup.in_use, setup.full, setup.reason.clone(), setup.retry),
                (false, false, None, false),
                "{build:?}"
            );
        }
        let fallback =
            GpuStatus::of(Build::Cpu, None, Some("crashed-twice".into()), true).in_setup();
        assert_eq!(
            (fallback.reason.as_deref(), fallback.retry),
            (Some("crashed-twice"), true)
        );
    }

    #[test]
    fn a_cpu_load_inside_orag_vulkan_says_why() {
        let failed = GpuStatus::of(
            Build::Vulkan,
            Some(&gen_offload(0, Some("fit_failed"))),
            None,
            true,
        );
        assert_eq!(
            (failed.in_use, failed.reason.as_deref(), failed.retry),
            (false, Some("fit_failed"), false)
        );
    }

    #[test]
    fn the_setting_reads_auto_and_off_only() {
        assert_eq!(GpuSetting::parse(None).unwrap(), GpuSetting::Auto);
        assert_eq!(GpuSetting::parse(Some("auto")).unwrap(), GpuSetting::Auto);
        assert_eq!(GpuSetting::parse(Some("off")).unwrap(), GpuSetting::Off);
        assert!(GpuSetting::parse(Some("on")).unwrap_err().contains("gpu"));
    }
}
