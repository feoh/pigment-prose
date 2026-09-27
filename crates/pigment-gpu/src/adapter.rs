//! Adapter enumeration, capability reports and selection.

use pigment_core::capability::{
    AdapterPolicy, AdapterReport, BackendKind, DeviceKind, LimitsReport, rank,
};
use pigment_core::error::RenderError;

fn backends(include_gl: bool) -> wgpu::Backends {
    if include_gl {
        wgpu::Backends::all()
    } else {
        // Vulkan, Metal, DX12 (and BROWSER_WEBGPU, unused natively).
        wgpu::Backends::PRIMARY
    }
}

pub fn instance(include_gl: bool) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends(include_gl);
    wgpu::Instance::new(desc)
}

/// Software rasterizers. Some GL drivers report llvmpipe as `Other`, so the
/// name is checked too (task 02).
pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    let name = info.name.to_lowercase();
    info.device_type == wgpu::DeviceType::Cpu
        || ["llvmpipe", "lavapipe", "swiftshader", "warp"]
            .iter()
            .any(|s| name.contains(s))
}

pub(crate) fn limits_report(l: &wgpu::Limits) -> LimitsReport {
    LimitsReport {
        max_texture_dimension_2d: l.max_texture_dimension_2d,
        max_buffer_size: l.max_buffer_size,
        max_storage_buffer_binding_size: l.max_storage_buffer_binding_size,
        max_compute_workgroup_size_x: l.max_compute_workgroup_size_x,
        max_compute_workgroup_size_y: l.max_compute_workgroup_size_y,
        max_compute_invocations_per_workgroup: l.max_compute_invocations_per_workgroup,
        max_storage_textures_per_shader_stage: l.max_storage_textures_per_shader_stage,
    }
}

pub fn report(adapter: &wgpu::Adapter) -> AdapterReport {
    let i = adapter.get_info();
    AdapterReport {
        name: i.name.clone(),
        backend: match i.backend {
            wgpu::Backend::Vulkan => BackendKind::Vulkan,
            wgpu::Backend::Dx12 => BackendKind::Dx12,
            wgpu::Backend::Metal => BackendKind::Metal,
            wgpu::Backend::Gl => BackendKind::Gl,
            _ => BackendKind::Other,
        },
        kind: match i.device_type {
            wgpu::DeviceType::DiscreteGpu => DeviceKind::Discrete,
            wgpu::DeviceType::IntegratedGpu => DeviceKind::Integrated,
            wgpu::DeviceType::VirtualGpu => DeviceKind::Virtual,
            wgpu::DeviceType::Cpu => DeviceKind::Cpu,
            _ => DeviceKind::Other,
        },
        software: is_software(&i),
        vendor_id: i.vendor,
        device_id: i.device,
        driver: i.driver.clone(),
        driver_info: i.driver_info.clone(),
        adapter_limits: limits_report(&adapter.limits()),
    }
}

/// Every adapter the policy's backends expose, unfiltered.
pub fn enumerate(
    instance: &wgpu::Instance,
    policy: &AdapterPolicy,
) -> Vec<(wgpu::Adapter, AdapterReport)> {
    pollster::block_on(instance.enumerate_adapters(backends(policy.include_gl)))
        .into_iter()
        .map(|a| {
            let r = report(&a);
            (a, r)
        })
        .collect()
}

/// Best adapter under `policy`, or an actionable error.
pub fn select(
    instance: &wgpu::Instance,
    policy: &AdapterPolicy,
) -> Result<(wgpu::Adapter, AdapterReport), RenderError> {
    let mut all = enumerate(instance, policy);
    if all.is_empty() {
        return Err(RenderError::NoAdapter {
            help: driver_help(),
        });
    }
    if let Some(f) = &policy.name_filter {
        let f = f.to_lowercase();
        all.retain(|(_, r)| r.name.to_lowercase().contains(&f));
        if all.is_empty() {
            return Err(RenderError::NoAdapterMatches {
                filter: f.to_string(),
            });
        }
    }
    all.sort_by_key(|(_, r)| std::cmp::Reverse(rank(r)));
    let best = all.remove(0);
    if best.1.software && !policy.allow_software {
        return Err(RenderError::SoftwareOnly {
            adapter: best.1.name.clone(),
            help: driver_help(),
        });
    }
    Ok(best)
}

pub fn driver_help() -> String {
    "Check that a hardware GPU driver is installed and visible:\n  \
     Linux: install the vendor Vulkan driver (nvidia-utils, or mesa-vulkan-drivers) and check `vulkaninfo --summary`\n  \
     Windows: update the GPU driver (Direct3D 12 or Vulkan)\n  \
     macOS: Metal is built in; the Mac must support Metal\n\
     WGPU_BACKEND=vulkan|dx12|metal forces a backend when diagnosing."
        .to_string()
}
