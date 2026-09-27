//! The Pigment Prose desktop studio (task 11): an eframe/egui shell over
//! the GPU painting renderer. The binary is `src/main.rs`; the modules are
//! a library so the hardware tests can drive the real render worker.
//!
//! - [`app`]: the window (layout, document edits, preview display).
//! - [`worker`]: the render thread on the latest-wins mailbox.
//! - [`preview`]: debounce, preview sizing and stale-result rules.
//! - [`script`]: the scripted responsiveness check (`--script`).
#![allow(clippy::print_stdout, clippy::print_stderr)]

pub mod app;
pub mod preview;
pub mod script;
pub mod worker;
