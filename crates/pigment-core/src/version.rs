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
//!   recipe says exactly how its digest was produced. Both are frozen by
//!   `fixtures/seed-vectors.json`; see docs/seeds-and-recipes.md.

/// Recipe file format version written by this build. `2` (task 16) adds
/// `season`; schema 1 files still load, through one explicit migration
/// (`recipe::SCHEMA_V1`, season midsummer).
pub const RECIPE_SCHEMA_VERSION: u32 = 2;

/// Scene generator version. `1` = the scene family approved at the task 08
/// visual gate (2026-09-27); `2` = task 25: a seeded wind on every scene and
/// more complex rocks (faceted, notched, ledged); `3` = review round 8:
/// rock forms vary in height, wear and lean. Any geometry checksum change
/// is a bump.
pub const GENERATOR_VERSION: u32 = 3;

/// Painting renderer version. `1` = the painting approved at the task 08
/// visual gate (2026-09-27, baseline in docs/visual-review/baseline-08/);
/// `2` = task 25: wind and current on water, rock surfaces (baseline in
/// docs/visual-review/baseline-25/); `3` = task 16: the seasons, and (review
/// round 8) mountainsides with their own anatomy (baseline in
/// docs/visual-review/baseline-16/). Any visible change on the same device
/// is a bump.
pub const RENDERER_VERSION: u32 = 3;

/// Text normalization: NFC, then CRLF/CR to LF, as UTF-8 (`text::normalize`).
pub const NORMALIZATION_ID: &str = "nfc-lf-utf8/1";

/// Digest, stream derivation and PRNG (`seed` module): domain-separated
/// SHA-256 and xoshiro256** seeded by SplitMix64.
pub const SEED_ALGORITHM_ID: &str = "pigment-seed/1";

/// Crate version of this build, for diagnostics and render reports.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
