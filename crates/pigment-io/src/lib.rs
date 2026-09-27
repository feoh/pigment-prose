//! File input and output for Pigment Prose.
//!
//! - [`atomic`]: write beside the destination, rename into place on success.
//! - [`png_sink`]: [`PngSink`], the streaming PNG [`TileSink`] for exports.
//! - [`export`]: [`export_png`], size validation and out-of-memory retry.
//!
//! Recipe files and the document model (task 10) will live here too.
//!
//! [`TileSink`]: pigment_core::request::TileSink

pub mod atomic;
pub mod export;
pub mod png_sink;

#[cfg(test)]
mod test_dir;

pub use atomic::AtomicFile;
pub use export::{Attempt, ExportReport, ExportSize, export_png, validate_target};
pub use png_sink::{PngCompression, PngSink};
