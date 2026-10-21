//! The GPU devices llama.cpp's Vulkan backend sees (`orag-vulkan` only;
//! v0.6 plan, Task 2). CPU and accelerator entries are left out: they are
//! not GPUs and ORAG never picks them.

use llama_cpp_2::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};

use crate::error::Result;
use crate::gpu::{DeviceKind, GpuDevice};
use crate::infer::llama::backend;

/// Every GPU-type device, in llama.cpp's index order.
pub fn gpu_devices() -> Result<Vec<GpuDevice>> {
    backend()?;
    Ok(list_llama_ggml_backend_devices()
        .into_iter()
        .filter_map(|device| {
            let kind = match device.device_type {
                LlamaBackendDeviceType::Gpu => DeviceKind::Discrete,
                LlamaBackendDeviceType::IntegratedGpu => DeviceKind::Integrated,
                LlamaBackendDeviceType::Unknown => DeviceKind::Other,
                LlamaBackendDeviceType::Cpu | LlamaBackendDeviceType::Accelerator => return None,
            };
            // The description is the driver's name ("NVIDIA GeForce RTX
            // 3060"); the name is only ggml's ("Vulkan0").
            let name = if device.description.is_empty() {
                device.name
            } else {
                device.description
            };
            Some(GpuDevice {
                index: device.index,
                name,
                kind,
                memory_total: device.memory_total as u64,
                memory_free: device.memory_free as u64,
            })
        })
        .collect())
}
