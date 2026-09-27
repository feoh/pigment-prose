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
    OutOfRange { value: f64, min: f64, max: f64 },
    TooSmall { value: u64, min: u64 },
    TooLarge { value: u64, max: u64 },
    AspectTooExtreme { long_over_short: f64, max: f64 },
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
