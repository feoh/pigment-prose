//! Source-text boundary.
//!
//! Prose is seed material only. It is never parsed for meaning, logged,
//! written to image metadata or sent anywhere. This module owns the input
//! gate that runs *before* normalization and hashing.
//!
//! Task 04 adds, in this file:
//!
//! ```ignore
//! /// Proposed v1 (`version::NORMALIZATION_ID`): check_source, then Unicode
//! /// NFC, then CRLF and lone CR to LF. Case and all other whitespace are kept.
//! pub fn normalize(source: &str) -> Result<NormalizedText, TextError>;
//! ```
//!
//! and freezes its test vectors. Nothing else may construct `NormalizedText`.

use std::fmt;

use crate::error::TextError;

/// Upper bound on accepted source prose, in UTF-8 bytes *before*
/// normalization (1 MiB). Checked before any other work so oversized pastes
/// fail fast and never reach the hasher.
pub const MAX_SOURCE_BYTES: usize = 1 << 20;

/// Input gate. Rejects empty, whitespace-only and oversized text. A nonblank
/// passage is accepted unchanged: leading/trailing whitespace stays
/// significant and is never trimmed.
///
/// "Whitespace" is Unicode `White_Space` (`char::is_whitespace`), so a
/// passage of only U+3000 IDEOGRAPHIC SPACE is also rejected.
pub fn check_source(source: &str) -> Result<(), TextError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(TextError::TooLong {
            bytes: source.len(),
            max: MAX_SOURCE_BYTES,
        });
    }
    if source.is_empty() {
        return Err(TextError::Empty);
    }
    if source.chars().all(char::is_whitespace) {
        return Err(TextError::WhitespaceOnly);
    }
    Ok(())
}

/// Text after the versioned normalization, ready to hash. Its `Debug` output
/// shows only the length so it cannot leak into logs by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct NormalizedText(String);

impl NormalizedText {
    /// Only the task 04 `normalize` function may call this.
    #[allow(dead_code)]
    pub(crate) fn from_normalized(s: String) -> Self {
        NormalizedText(s)
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for NormalizedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NormalizedText(<{} bytes>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_blank_are_rejected() {
        assert_eq!(check_source(""), Err(TextError::Empty));
        for blank in [" ", "\n", "\r\n", "\t \t", "\u{3000}", "\u{a0}\u{2029}"] {
            assert_eq!(
                check_source(blank),
                Err(TextError::WhitespaceOnly),
                "{blank:?}"
            );
        }
    }

    #[test]
    fn nonblank_passages_are_accepted_unchanged() {
        // Fixture `whitespace`: surrounding spaces are significant.
        assert_eq!(check_source("  A lantern glows at dawn.  "), Ok(()));
        assert_eq!(check_source("風が湖を渡る。"), Ok(()));
        assert_eq!(check_source("\u{200b}"), Ok(()), "ZWSP is not White_Space");
    }

    #[test]
    fn size_bound_is_in_bytes_and_inclusive() {
        let at_limit = "a".repeat(MAX_SOURCE_BYTES);
        assert_eq!(check_source(&at_limit), Ok(()));
        let over = "é".repeat(MAX_SOURCE_BYTES / 2 + 1);
        assert_eq!(
            check_source(&over),
            Err(TextError::TooLong {
                bytes: MAX_SOURCE_BYTES + 2,
                max: MAX_SOURCE_BYTES
            })
        );
    }

    #[test]
    fn debug_and_errors_never_show_text() {
        let t = NormalizedText::from_normalized("secret marker".into());
        assert_eq!(format!("{t:?}"), "NormalizedText(<13 bytes>)");
        let e = check_source(&"x".repeat(MAX_SOURCE_BYTES + 1)).unwrap_err();
        assert!(!e.to_string().contains('x'));
    }
}
