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

/// The same physical adapter on the same backend (two instances enumerate
/// it separately; this matches their reports).
pub fn same_adapter(a: &AdapterReport, b: &AdapterReport) -> bool {
    a.name == b.name
        && a.backend == b.backend
        && a.vendor_id == b.vendor_id
        && a.device_id == b.device_id
}

/// Which adapter shows the window. `candidates` are the display side's
/// adapters, each with whether it claims to present to the window's surface.
/// The best-ranked adapter that presents wins (discrete first), with ties
/// going to the painter's adapter so both can share one device. A software
/// adapter only if allowed. `None` if nothing can present.
///
/// The painter's adapter is deliberately not preferred over a better one:
/// a driver can claim surface support it cannot deliver. On the development
/// machine the Intel iGPU's Vulkan driver reports the Wayland surface as
/// supported, but the compositor, whose outputs are on the NVIDIA card,
/// cannot import its buffers (task "studio on multi-GPU systems").
pub fn choose_display(
    candidates: &[(AdapterReport, bool)],
    painter: &AdapterReport,
    allow_software: bool,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (r, presents))| *presents && (allow_software || !r.software))
        .max_by_key(|(i, (r, _))| (rank(r), same_adapter(r, painter), std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
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

    #[test]
    fn the_display_is_the_best_adapter_that_presents_ties_to_the_painter() {
        let nvidia = AdapterReport {
            device_id: 1,
            ..adapter("NVIDIA", DeviceKind::Discrete, false)
        };
        let intel = AdapterReport {
            device_id: 2,
            ..adapter("Intel", DeviceKind::Integrated, false)
        };
        let llvm = adapter("llvmpipe", DeviceKind::Cpu, true);
        // This desktop: both claim to present; the discrete card that drives
        // the monitors shows the window whichever adapter paints.
        let both = [(intel.clone(), true), (nvidia.clone(), true)];
        assert_eq!(choose_display(&both, &nvidia, false), Some(1));
        assert_eq!(choose_display(&both, &intel, false), Some(1));
        // A laptop whose discrete GPU cannot present: the iGPU shows.
        let laptop = [(nvidia.clone(), false), (intel.clone(), true)];
        assert_eq!(choose_display(&laptop, &nvidia, false), Some(1));
        // Two identical cards: the painter's, so the device is shared.
        let twin = AdapterReport {
            device_id: 3,
            ..nvidia.clone()
        };
        let twins = [(nvidia.clone(), true), (twin.clone(), true)];
        assert_eq!(choose_display(&twins, &twin, false), Some(1));
        assert_eq!(choose_display(&twins, &intel, false), Some(0));
        // Software only if allowed; nothing that presents → None.
        let sw = [(nvidia.clone(), false), (llvm.clone(), true)];
        assert_eq!(choose_display(&sw, &nvidia, false), None);
        assert_eq!(choose_display(&sw, &nvidia, true), Some(1));
        assert_eq!(choose_display(&[], &nvidia, true), None);
        // Identity is name + backend + vendor + device.
        assert!(same_adapter(&nvidia, &nvidia.clone()));
        let other_backend = AdapterReport {
            backend: BackendKind::Gl,
            ..nvidia.clone()
        };
        assert!(!same_adapter(&nvidia, &other_backend));
    }
}
