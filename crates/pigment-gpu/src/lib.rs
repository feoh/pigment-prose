//! wgpu backend for Pigment Prose.
//!
//! - [`adapter`]: enumeration, capability reports, selection policy.
//! - [`context`]: the opened device (portable WebGPU limits), device-lost
//!   and error-scope handling. Shared with the egui shell in task 11.
//! - [`smoke`]: the tiled test-card renderer used by `pigment-prose gpu-smoke`
//!   and the hardware test suite.
//! - [`debug`]: flat-value and region-overlay views of a scene (task 05),
//!   used by `pigment-prose contact-sheet`.
//! - `tiled` (private): the tile loop every renderer shares.
//!
//! The painting renderer (tasks 06–07) will be `paint.rs` + `paint.wgsl`
//! beside them, implementing the same `pigment_core::request::Renderer`.

pub mod adapter;
pub mod context;
pub mod debug;
pub mod smoke;
mod tiled;

pub use context::GpuContext;
pub use debug::{DebugRenderer, DebugView};
pub use smoke::SmokeRenderer;
