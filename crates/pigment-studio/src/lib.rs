//! The Pigment Prose desktop studio (tasks 11–13): an eframe/egui shell over
//! the GPU painting renderer. The binary is `src/main.rs`; the modules are
//! a library so the hardware tests can drive the real render worker.
//!
//! - [`app`]: the window (layout, document edits, preview display).
//! - [`controls`]: the artistic controls, generated from `settings::CONTROLS`.
//! - [`files`]: recipe open/save, native dialogs and the unsaved-changes flow.
//! - [`export`]: export sizes, the job snapshot and the export worker.
//! - [`worker`]: the render thread on the latest-wins mailbox.
//! - [`preview`]: debounce, preview sizing and stale-result rules.
//! - [`script`]: the scripted responsiveness check (`--script`).
//! - [`theme`]: the visual system (fonts, colours, spacing).
#![allow(clippy::print_stdout, clippy::print_stderr)]

pub mod app;
pub mod controls;
pub mod export;
pub mod files;
pub mod painter;
pub mod preview;
pub mod script;
pub mod theme;
pub mod worker;

#[cfg(test)]
mod ui_tests;
