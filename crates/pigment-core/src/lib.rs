//! Portable contracts for Pigment Prose.
//!
//! This crate has no GPU, windowing or filesystem dependency, so every test
//! here runs in portable CI on Linux, Windows and macOS. The data flow is:
//!
//! ```text
//! prose ──text──▶ SeedBundle ──┐
//!                               ├─ SceneGenerator ─▶ Arc<Scene> ─┐
//! FormSettings + Frame aspect ──┘                                 ├─▶ RenderRequest ─▶ Renderer ─▶ RenderReport
//! Appearance (painting, palette, atmosphere) + RenderTarget ─────┘
//! ```
//!
//! See `docs/architecture.md` for the full contract, the cross-reference
//! table and which task owns each unimplemented piece.

pub mod capability;
pub mod composite;
pub mod error;
pub mod frame;
pub mod invalidate;
pub mod job;
pub mod palette;
pub mod recipe;
pub mod request;
pub mod scene;
pub mod season;
pub mod seed;
pub mod settings;
pub mod text;
pub mod tiles;
pub mod version;
