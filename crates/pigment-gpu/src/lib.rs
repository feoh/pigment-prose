//! wgpu backend for Pigment Prose.
//!
//! - [`adapter`]: enumeration, capability reports, selection policy.
//! - [`context`]: the opened device (portable WebGPU limits), device-lost
//!   and error-scope handling. Shared with the egui shell in task 11.
//! - [`smoke`]: the tiled test-card renderer used by `pigment-prose gpu-smoke`
//!   and the hardware test suite.
//!
//! The painting renderer (tasks 06–07) will be `paint.rs` + `paint.wgsl`
//! beside `smoke.rs`, implementing the same `pigment_core::request::Renderer`.

pub mod adapter;
pub mod context;
pub mod smoke;

pub use context::GpuContext;
pub use smoke::SmokeRenderer;
