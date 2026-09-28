//! The opened device, shared by every renderer. The studio either shares
//! egui's device (when the window shows on the painting adapter) or opens
//! its own on another adapter ([`GpuContext::from_existing`]).

use std::sync::{Arc, Mutex};

use pigment_core::capability::{AdapterPolicy, GpuCapabilities};
use pigment_core::error::RenderError;

use crate::adapter;

/// Resolved `wgpu` version, recorded in every capability report.
pub const WGPU_VERSION: &str = env!("PIGMENT_WGPU_VERSION");

#[derive(Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub capabilities: GpuCapabilities,
    lost: Arc<Mutex<Option<String>>>,
    uncaptured: Arc<Mutex<Option<String>>>,
}

/// The device every renderer needs: **WebGPU's portable default limits** and
/// no optional features. The window's device is requested with the same
/// descriptor, so the painter can share it.
pub fn device_descriptor() -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("pigment-prose"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: Default::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: Default::default(),
    }
}

impl GpuContext {
    /// Select an adapter and open a device with **WebGPU's portable default
    /// limits** and no optional features, so nothing silently depends on one
    /// vendor's larger limits. Tiling makes output size independent of
    /// `max_texture_dimension_2d` (task 02).
    pub fn new(policy: &AdapterPolicy) -> Result<GpuContext, RenderError> {
        let instance = adapter::instance(policy.include_gl);
        let (adapter, report) = adapter::select(&instance, policy)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&device_descriptor()))
            .map_err(|e| RenderError::DeviceRequest {
                adapter: report.name.clone(),
                detail: e.to_string(),
            })?;
        Ok(GpuContext::wrap(instance, adapter, report, device, queue))
    }

    /// Wraps a device someone else opened (the window's), which must have
    /// been requested with [`device_descriptor`]. Installs the device-lost
    /// and error hooks and reports the adapter's capabilities.
    pub fn from_existing(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> GpuContext {
        let report = adapter::report(&adapter);
        GpuContext::wrap(instance, adapter, report, device, queue)
    }

    fn wrap(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        report: pigment_core::capability::AdapterReport,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> GpuContext {
        let limits = wgpu::Limits::default();
        let lost = Arc::new(Mutex::new(None));
        {
            let lost = lost.clone();
            device.set_device_lost_callback(move |reason, msg| {
                *lost.lock().expect("lost flag poisoned") = Some(format!("{reason:?}: {msg}"));
            });
        }
        let uncaptured = Arc::new(Mutex::new(None));
        {
            let uncaptured = uncaptured.clone();
            device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
                uncaptured
                    .lock()
                    .expect("error slot poisoned")
                    .get_or_insert_with(|| e.to_string());
            }));
        }

        GpuContext {
            instance,
            capabilities: GpuCapabilities {
                adapter: report,
                device_limits: adapter::limits_report(&limits),
                wgpu_version: WGPU_VERSION,
            },
            adapter,
            device,
            queue,
            lost,
            uncaptured,
        }
    }

    /// `Err(DeviceLost)` once the driver has reported device loss. The
    /// context is then unusable; the caller recreates it.
    pub fn check_alive(&self) -> Result<(), RenderError> {
        if let Some(detail) = self.lost.lock().expect("lost flag poisoned").clone() {
            return Err(RenderError::DeviceLost { detail });
        }
        if let Some(detail) = self.uncaptured.lock().expect("error slot poisoned").take() {
            return Err(RenderError::Gpu { detail });
        }
        Ok(())
    }

    /// Run `f` inside out-of-memory and validation error scopes.
    pub fn scoped<T>(&self, stage: &'static str, f: impl FnOnce() -> T) -> Result<T, RenderError> {
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let out = f();
        if pollster::block_on(oom.pop()).is_some() {
            drop(validation);
            return Err(RenderError::OutOfMemory { stage });
        }
        if let Some(e) = pollster::block_on(validation.pop()) {
            return Err(RenderError::Gpu {
                detail: format!("{stage}: {e}"),
            });
        }
        Ok(out)
    }
}
