//! File input and output for Pigment Prose.
//!
//! - [`atomic`]: write beside the destination, rename into place on success.
//! - [`png_sink`]: [`PngSink`], the streaming PNG [`TileSink`] for exports.
//! - [`export`]: [`export_png`], size validation and out-of-memory retry.
//!
//! - [`recipe_file`]: read and atomically write recipe files (task 10).
//! - [`document`]: [`Document`], the open painting: recipe, prose, the
//!   keep-source-text choice, path and dirty state (task 10).
//!
//! [`TileSink`]: pigment_core::request::TileSink

pub mod atomic;
pub mod document;
pub mod export;
pub mod png_sink;
pub mod recipe_file;

#[cfg(test)]
mod test_dir;

pub use atomic::AtomicFile;
pub use document::{Document, SaveError};
pub use export::{Attempt, ExportReport, ExportSize, export_png, validate_target};
pub use png_sink::{PngCompression, PngSink};
pub use recipe_file::{FileOp, RecipeFileError, read_recipe, write_recipe};
