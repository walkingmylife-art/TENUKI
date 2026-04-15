//! Windows GPU detection and backend candidate ordering for the launcher.

use anyhow::Result;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    Cuda,
    Rocm,
    Vulkan,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Cuda => "cuda",
            BackendKind::Rocm => "rocm",
            BackendKind::Vulkan => "vulkan",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuClass {
    NvidiaAny,
    AmdRx9000OrNewer,
    AmdLegacy,
    Intel,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub description: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub vendor: GpuVendor,
    pub class: GpuClass,
    pub dedicated_video_memory: u64,
}

pub struct BackendDetector;

impl BackendDetector {
    /// Enumerate GPUs.
    pub fn enumerate_gpus() -> Result<Vec<GpuInfo>> {
        #[cfg(feature = "dxgi")]
        {
            return enumerate_gpus_dxgi().context("failed to enumerate GPUs via DXGI");
        }

        #[cfg(not(feature = "dxgi"))]
        {
            log::warn!("DXGI feature not enabled; GPU detection skipped");
            Ok(Vec::new())
        }
    }

    /// Return backend candidates in priority order.
    pub fn build_backend_candidates(gpus: &[GpuInfo]) -> Vec<BackendKind> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        let mut sorted = gpus.to_vec();
        sorted.sort_by_key(|g| std::cmp::Reverse(g.dedicated_video_memory));

        for gpu in &sorted {
            let backends: &[BackendKind] = match gpu.class {
                GpuClass::NvidiaAny => &[BackendKind::Cuda, BackendKind::Vulkan],
                GpuClass::AmdRx9000OrNewer => {
                    if Self::quick_check_rocm() {
                        &[BackendKind::Rocm, BackendKind::Vulkan]
                    } else {
                        &[BackendKind::Vulkan]
                    }
                }
                GpuClass::AmdLegacy => &[BackendKind::Vulkan],
                GpuClass::Intel => &[BackendKind::Vulkan],
                GpuClass::Unknown => &[BackendKind::Vulkan],
            };

            for &backend in backends {
                if seen.insert(backend) {
                    out.push(backend);
                }
            }
        }

        if out.is_empty() {
            out.push(BackendKind::Vulkan);
        }

        out
    }

    pub fn build_backend_candidate_names(gpus: &[GpuInfo]) -> Vec<String> {
        Self::build_backend_candidates(gpus)
            .into_iter()
            .map(|b| b.as_str().to_string())
            .collect()
    }

    pub fn quick_check_rocm() -> bool {
        for cmd in ["hipInfo", "rocminfo"] {
            let ok = std::process::Command::new(cmd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return true;
            }
        }
        false
    }

}

#[cfg(feature = "dxgi")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_DESC1, IDXGIAdapter1, IDXGIFactory1,
};

#[cfg(feature = "dxgi")]
fn enumerate_gpus_dxgi() -> Result<Vec<GpuInfo>> {
    let mut gpus = Vec::new();

    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;

        for i in 0.. {
            let adapter: IDXGIAdapter1 = match factory.EnumAdapters1(i) {
                Ok(a) => a,
                Err(_) => break,
            };

            let mut desc = DXGI_ADAPTER_DESC1::default();
            adapter.GetDesc1(&mut desc)?;

            const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 0x2;
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) != 0 {
                continue;
            }

            let description = utf16_fixed_to_string(&desc.Description);
            let vendor = vendor_from_id(desc.VendorId);
            let class = classify_gpu(vendor, desc.DeviceId, &description);

            gpus.push(GpuInfo {
                description,
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
                vendor,
                class,
                dedicated_video_memory: desc.DedicatedVideoMemory,
            });
        }
    }

    Ok(gpus)
}

#[cfg(feature = "dxgi")]
fn vendor_from_id(vendor_id: u32) -> GpuVendor {
    match vendor_id {
        0x10DE => GpuVendor::Nvidia,
        0x1002 | 0x1022 => GpuVendor::Amd,
        0x8086 => GpuVendor::Intel,
        _ => GpuVendor::Unknown,
    }
}

#[cfg(feature = "dxgi")]
fn classify_gpu(vendor: GpuVendor, device_id: u32, description: &str) -> GpuClass {
    match vendor {
        GpuVendor::Nvidia => GpuClass::NvidiaAny,
        GpuVendor::Amd => {
            if is_amd_rx9000_or_newer(device_id, description) {
                GpuClass::AmdRx9000OrNewer
            } else {
                GpuClass::AmdLegacy
            }
        }
        GpuVendor::Intel => GpuClass::Intel,
        GpuVendor::Unknown => GpuClass::Unknown,
    }
}

#[cfg(feature = "dxgi")]
fn is_amd_rx9000_or_newer(device_id: u32, description: &str) -> bool {
    let desc_lower = description.to_ascii_lowercase();
    if desc_lower.contains("rx 9") || desc_lower.contains("rx9") {
        return true;
    }
    matches!(device_id, 0x7400..=0x74FF)
}

#[cfg(feature = "dxgi")]
fn utf16_fixed_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_names_are_not_empty() {
        let gpus = BackendDetector::enumerate_gpus().unwrap_or_default();
        let candidates = BackendDetector::build_backend_candidate_names(&gpus);
        assert!(!candidates.is_empty());
    }
}
