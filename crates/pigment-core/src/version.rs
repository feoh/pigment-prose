//! Version identifiers recorded in every recipe and render report.
//!
//! Compatibility policy (docs/architecture.md, "Versioning"):
//! - `RECIPE_SCHEMA_VERSION` changes when the recipe *format* changes. A newer
//!   schema than this build understands is rejected, never guessed at.
//! - `GENERATOR_VERSION` changes whenever the same seed, form settings and
//!   aspect ratio would produce a different `Scene` (any geometry checksum
//!   change). `0` means "pre-approval, unstable": fixtures may change freely
//!   until the task 08 visual gate, after which it becomes `1` and every
//!   further change is a bump.
//! - `RENDERER_VERSION` changes whenever the same scene and appearance would
//!   paint visibly differently on the same device. Same `0` rule.
//! - Text normalization and seed derivation have their own identifiers so a
//!   recipe says exactly how its digest was produced (frozen by task 04).

/// Recipe file format version understood by this build.
pub const RECIPE_SCHEMA_VERSION: u32 = 1;

/// Scene generator version. `0` = pre-approval, no cross-build promise.
pub const GENERATOR_VERSION: u32 = 0;

/// Painting renderer version. `0` = pre-approval, no cross-build promise.
pub const RENDERER_VERSION: u32 = 0;

/// Identifier of the text normalization algorithm (task 04 freezes it).
pub const NORMALIZATION_ID: &str = "nfc-lf-utf8/1";

/// Identifier of the digest and stream-derivation algorithm (task 04 freezes it).
pub const SEED_ALGORITHM_ID: &str = "pigment-seed/1";

/// Crate version of this build, for diagnostics and render reports.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
