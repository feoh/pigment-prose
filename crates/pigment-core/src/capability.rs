//! GPU capability reporting as plain data, so the UI, diagnostics and tests
//! do not depend on wgpu types. `pigment-gpu` fills these in.

use crate::tiles::DeviceTileLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Vulkan,
    Dx12,
    Metal,
    Gl,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Virtual,
    /// Software rasterizer (llvmpipe, lavapipe, WARP, SwiftShader).
    Cpu,
    Other,
}

/// One adapter as enumerated, before a device is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterReport {
    pub name: String,
    pub backend: BackendKind,
    pub kind: DeviceKind,
    /// True for software rasterizers, including ones that misreport their
    /// type. Never presented as hardware acceleration.
    pub software: bool,
    pub vendor_id: u32,
    pub device_id: u32,
    pub driver: String,
    pub driver_info: String,
    /// The adapter's own maxima (not what we request).
    pub adapter_limits: LimitsReport,
}

/// The limits the renderer depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitsReport {
    pub max_texture_dimension_2d: u32,
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroup_size_x: u32,
    pub max_compute_workgroup_size_y: u32,
    pub max_compute_invocations_per_workgroup: u32,
    pub max_storage_textures_per_shader_stage: u32,
}

impl LimitsReport {
    pub fn tile_limits(&self) -> DeviceTileLimits {
        DeviceTileLimits {
            max_texture_dimension_2d: self.max_texture_dimension_2d,
            max_buffer_size: self.max_buffer_size,
        }
    }
}

/// The opened device: which adapter was chosen and what we requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCapabilities {
    pub adapter: AdapterReport,
    /// Limits the device was created with (WebGPU defaults by policy).
    pub device_limits: LimitsReport,
    pub wgpu_version: &'static str,
}

impl GpuCapabilities {
    /// Short label for reports and the diagnostics panel.
    pub fn label(&self) -> String {
        let sw = if self.adapter.software {
            ", SOFTWARE"
        } else {
            ""
        };
        format!("{} ({:?}{sw})", self.adapter.name, self.adapter.backend)
    }
}

/// How to choose an adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdapterPolicy {
    /// Case-insensitive substring of the adapter name.
    pub name_filter: Option<String>,
    /// Accept a software rasterizer (labelled; diagnostics only).
    pub allow_software: bool,
    /// Also enumerate the GL backend (off by default: compute on GL is
    /// untested, task 02).
    pub include_gl: bool,
}

/// Selection order: discrete > integrated > virtual > other > software.
pub fn rank(a: &AdapterReport) -> u32 {
    if a.software {
        return 0;
    }
    match a.kind {
        DeviceKind::Discrete => 4,
        DeviceKind::Integrated => 3,
        DeviceKind::Virtual => 2,
        DeviceKind::Other => 1,
        DeviceKind::Cpu => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, kind: DeviceKind, software: bool) -> AdapterReport {
        AdapterReport {
            name: name.into(),
            backend: BackendKind::Vulkan,
            kind,
            software,
            vendor_id: 0,
            device_id: 0,
            driver: String::new(),
            driver_info: String::new(),
            adapter_limits: LimitsReport {
                max_texture_dimension_2d: 8192,
                max_buffer_size: 1 << 28,
                max_storage_buffer_binding_size: 1 << 27,
                max_compute_workgroup_size_x: 256,
                max_compute_workgroup_size_y: 256,
                max_compute_invocations_per_workgroup: 256,
                max_storage_textures_per_shader_stage: 4,
            },
        }
    }

    #[test]
    fn software_always_ranks_last() {
        let sw = adapter("llvmpipe", DeviceKind::Other, true);
        let igpu = adapter("Intel", DeviceKind::Integrated, false);
        let dgpu = adapter("NVIDIA", DeviceKind::Discrete, false);
        assert!(rank(&dgpu) > rank(&igpu));
        assert!(rank(&igpu) > rank(&sw));
        assert_eq!(rank(&adapter("x", DeviceKind::Discrete, true)), 0);
    }
}
