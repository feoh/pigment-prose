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
//! - [`paint`]: the painting renderer (tasks 06–07), with its
//!   [`coverage`] index.

pub mod adapter;
pub mod context;
pub mod coverage;
pub mod debug;
pub mod paint;
mod relief;
pub mod smoke;
mod tiled;

pub use context::GpuContext;
pub use debug::{DebugRenderer, DebugView};
pub use paint::{CompositeCase, CompositeOp, PaintRenderer};
pub use smoke::SmokeRenderer;
