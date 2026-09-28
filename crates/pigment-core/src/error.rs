//! Structured error types shared by every layer.
//!
//! Privacy rule: no error value may contain source prose or recipe contents.
//! Text errors carry sizes only; validation errors name the field and the
//! offending *numeric* value. Display strings are safe to log.

use std::fmt;

use crate::tiles::TilePlanError;

/// A value failed boundary validation (recipe load, UI input, API call).
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    /// Dotted field path, e.g. `painting.edge_looseness` or `frame.width`.
    pub field: &'static str,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    NotFinite,
    OutOfRange {
        value: f64,
        min: f64,
        max: f64,
    },
    TooSmall {
        value: u64,
        min: u64,
    },
    TooLarge {
        value: u64,
        max: u64,
    },
    AspectTooExtreme {
        long_over_short: f64,
        max: f64,
    },
    /// A render size whose reduced aspect ratio differs from the scene's.
    /// Exports never stretch; a new aspect ratio is a new scene.
    AspectMismatch {
        got: (u32, u32),
        scene: (u32, u32),
    },
    /// Buffer arithmetic for this size would overflow the platform.
    Overflow,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = self.field;
        match &self.problem {
            Problem::NotFinite => write!(f, "{field} must be a finite number"),
            Problem::OutOfRange { value, min, max } => {
                write!(f, "{field} = {value} is outside {min}..={max}")
            }
            Problem::TooSmall { value, min } => write!(f, "{field} = {value} is below {min}"),
            Problem::TooLarge { value, max } => write!(f, "{field} = {value} is above {max}"),
            Problem::AspectTooExtreme {
                long_over_short,
                max,
            } => write!(
                f,
                "{field}: aspect ratio {long_over_short:.3}:1 is more extreme than {max}:1"
            ),
            Problem::AspectMismatch { got, scene } => write!(
                f,
                "{field}: aspect ratio {}:{} differs from the scene's {}:{}; \
                 a different aspect ratio recomposes the painting",
                got.0, got.1, scene.0, scene.1
            ),
            Problem::Overflow => write!(f, "{field} is too large for this platform's buffers"),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Source text was rejected before hashing. Carries sizes, never the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    /// `""`: prompt the user to enter prose; do not render or derive a seed.
    Empty,
    /// Only Unicode `White_Space` characters. Same UI treatment as `Empty`.
    WhitespaceOnly,
    /// Larger than [`crate::text::MAX_SOURCE_BYTES`] UTF-8 bytes.
    TooLong { bytes: usize, max: usize },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::Empty => f.write_str("enter some prose to seed a painting"),
            TextError::WhitespaceOnly => {
                f.write_str("the prose contains only whitespace; enter some text")
            }
            TextError::TooLong { bytes, max } => {
                write!(f, "the prose is {bytes} bytes; the limit is {max} bytes")
            }
        }
    }
}

impl std::error::Error for TextError {}

/// A recipe document could not be loaded or saved.
///
/// Paths are dotted schema keys (`painting.edge_looseness`). They are built
/// only from the schema's own key names, plus [`sanitized_key`]'s rendering
/// of an unrecognized key, so no value from the file can reach the message.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeError {
    /// Larger than [`crate::recipe::MAX_RECIPE_BYTES`].
    TooLarge {
        bytes: usize,
        max: usize,
    },
    /// Not well-formed JSON, or a key repeated within one object.
    Malformed {
        line: usize,
        column: usize,
        kind: MalformedKind,
    },
    /// The `schema` number is not one this build reads: `supported` is the
    /// newest (schema 1 is migrated). Newer files are never guessed at.
    UnsupportedSchema {
        found: u64,
        supported: u32,
    },
    /// `versions.normalization` or `versions.seed_algorithm` names an
    /// algorithm this build does not implement, so the seed cannot be
    /// reproduced. `found` is sanitized like a key.
    UnsupportedAlgorithm {
        field: &'static str,
        found: String,
    },
    MissingField {
        path: String,
    },
    UnknownField {
        path: String,
    },
    /// Wrong JSON type, e.g. a string where a number belongs.
    WrongType {
        path: String,
        expected: &'static str,
    },
    /// Right type, unacceptable value (bad digest, unknown palette id).
    InvalidValue {
        path: String,
        reason: &'static str,
    },
    /// A setting or frame dimension failed range validation.
    Validation(ValidationError),
    /// The stored `source_text` is not acceptable prose.
    SourceText(TextError),
    /// The stored `source_text` does not produce `seed.digest`. The digest is
    /// authoritative, so the file is rejected rather than showing prose that
    /// does not belong to the painting.
    SourceTextMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedKind {
    Syntax,
    /// The file ends early (truncated).
    Truncated,
    DuplicateKey,
}

/// Renders a key or identifier from a file for an error message. Short,
/// identifier-like text is shown; anything else becomes a placeholder.
pub fn sanitized_key(key: &str) -> String {
    let ok = !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'));
    if ok {
        key.to_string()
    } else {
        "<unrecognized>".to_string()
    }
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = |p: &str| {
            if p.is_empty() {
                "the recipe".to_string()
            } else {
                format!("`{p}`")
            }
        };
        match self {
            RecipeError::TooLarge { bytes, max } => {
                write!(f, "the recipe is {bytes} bytes; the limit is {max} bytes")
            }
            RecipeError::Malformed { line, column, kind } => {
                let what = match kind {
                    MalformedKind::Syntax => "is not valid JSON",
                    MalformedKind::Truncated => "ends unexpectedly (truncated?)",
                    MalformedKind::DuplicateKey => "repeats a key",
                };
                write!(f, "the recipe {what} at line {line}, column {column}")
            }
            RecipeError::UnsupportedSchema { found, supported } => write!(
                f,
                "recipe schema {found} is not supported; this version reads schemas 1 to {supported}"
            ),
            RecipeError::UnsupportedAlgorithm { field, found } => write!(
                f,
                "`versions.{field}` is {found:?}, which this version does not implement"
            ),
            RecipeError::MissingField { path } => write!(f, "{} is missing", at(path)),
            RecipeError::UnknownField { path } => write!(f, "{} is not a recipe field", at(path)),
            RecipeError::WrongType { path, expected } => {
                write!(f, "{} must be {expected}", at(path))
            }
            RecipeError::InvalidValue { path, reason } => write!(f, "`{path}` {reason}"),
            RecipeError::Validation(e) => e.fmt(f),
            RecipeError::SourceText(e) => write!(f, "`source_text`: {e}"),
            RecipeError::SourceTextMismatch => {
                f.write_str("`source_text` does not match `seed.digest`")
            }
        }
    }
}

impl std::error::Error for RecipeError {}

impl From<ValidationError> for RecipeError {
    fn from(e: ValidationError) -> Self {
        RecipeError::Validation(e)
    }
}

/// A tile sink (preview image, PNG stream) could not accept output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkError {
    pub kind: SinkErrorKind,
    /// Human-readable detail. Must not contain prose; may name a file the
    /// user chose (shown in the UI, never written into image metadata).
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkErrorKind {
    Io,
    DiskFull,
    Encode,
    Other,
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "output failed ({:?}): {}", self.kind, self.detail)
    }
}

impl std::error::Error for SinkError {}

/// Everything that can stop a render. Cancellation is *not* an error: it is
/// [`crate::request::RenderOutcome::Cancelled`].
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// No usable adapter. `help` gives per-OS driver hints.
    NoAdapter {
        help: String,
    },
    /// Only a software rasterizer exists and the policy refuses it.
    SoftwareOnly {
        adapter: String,
        help: String,
    },
    /// `AdapterPolicy::name_filter` matched nothing.
    NoAdapterMatches {
        filter: String,
    },
    /// The adapter refused the device (features/limits/driver).
    DeviceRequest {
        adapter: String,
        detail: String,
    },
    InvalidRequest(ValidationError),
    TilePlan(TilePlanError),
    /// A GPU allocation failed; the caller may retry with a smaller budget.
    OutOfMemory {
        stage: &'static str,
    },
    /// The device was lost (driver reset, TDR, unplug). Recreate the context.
    DeviceLost {
        detail: String,
    },
    /// Any other GPU validation/internal error. Indicates a bug.
    Gpu {
        detail: String,
    },
    Sink(SinkError),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter { help } => write!(f, "no usable GPU adapter.\n{help}"),
            RenderError::SoftwareOnly { adapter, help } => write!(
                f,
                "only a software rasterizer is available ({adapter}); Pigment Prose needs \
                 hardware GPU acceleration.\n{help}"
            ),
            RenderError::NoAdapterMatches { filter } => write!(
                f,
                "no adapter name contains {filter:?}; run `pigment-prose gpu-info` to list adapters"
            ),
            RenderError::DeviceRequest { adapter, detail } => {
                write!(f, "could not open a device on {adapter}: {detail}")
            }
            RenderError::InvalidRequest(e) => write!(f, "invalid render request: {e}"),
            RenderError::TilePlan(e) => write!(f, "cannot plan tiles: {e}"),
            RenderError::OutOfMemory { stage } => write!(f, "GPU out of memory during {stage}"),
            RenderError::DeviceLost { detail } => write!(f, "GPU device lost: {detail}"),
            RenderError::Gpu { detail } => write!(f, "GPU error: {detail}"),
            RenderError::Sink(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<ValidationError> for RenderError {
    fn from(e: ValidationError) -> Self {
        RenderError::InvalidRequest(e)
    }
}

impl From<TilePlanError> for RenderError {
    fn from(e: TilePlanError) -> Self {
        RenderError::TilePlan(e)
    }
}

impl From<SinkError> for RenderError {
    fn from(e: SinkError) -> Self {
        RenderError::Sink(e)
    }
}
