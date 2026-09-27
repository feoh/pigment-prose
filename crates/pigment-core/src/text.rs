//! Source-text boundary and the versioned text normalization.
//!
//! Prose is seed material only. It is never parsed for meaning, logged,
//! written to image metadata or sent anywhere. This module owns the input
//! gate and the normalization that run *before* hashing.
//!
//! Normalization `nfc-lf-utf8/1` (`version::NORMALIZATION_ID`), frozen by
//! the vectors in `fixtures/seed-vectors.json` and documented in
//! `docs/seeds-and-recipes.md`:
//!
//! 1. [`check_source`]: reject empty, whitespace-only and oversized input.
//! 2. Unicode NFC (Unicode 17.0 tables, [`UNICODE_VERSION`]).
//! 3. CRLF and lone CR become LF.
//! 4. The result is hashed as UTF-8.
//!
//! Case, punctuation, all other whitespace (leading, trailing, tabs,
//! no-break spaces, repeated spaces and newlines), invisible characters such
//! as U+200B and U+FEFF, and compatibility variants (full-width letters,
//! ligatures) are kept exactly. Nothing else may construct `NormalizedText`.

use std::fmt;

use unicode_normalization::UnicodeNormalization;

use crate::error::TextError;

/// Unicode version of the NFC tables in use. NFC is stable for assigned
/// characters, so upgrading only matters for text containing code points
/// unassigned in this version. A test pins it so an upgrade is reviewed.
pub const UNICODE_VERSION: (u8, u8, u8) = unicode_normalization::UNICODE_VERSION;

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

/// Applies normalization `nfc-lf-utf8/1` after [`check_source`]. Pure and
/// platform-independent: the same `source` gives the same bytes everywhere.
pub fn normalize(source: &str) -> Result<NormalizedText, TextError> {
    check_source(source)?;
    let mut out = String::with_capacity(source.len());
    let mut chars = source.nfc().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            chars.next_if_eq(&'\n');
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Ok(NormalizedText(out))
}

/// Text after the versioned normalization, ready to hash. Its `Debug` output
/// shows only the length so it cannot leak into logs by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct NormalizedText(String);

impl NormalizedText {
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

    fn norm(s: &str) -> String {
        String::from_utf8(normalize(s).unwrap().as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn line_endings_become_lf() {
        assert_eq!(norm("a\r\nb"), "a\nb");
        assert_eq!(norm("a\rb"), "a\nb");
        assert_eq!(norm("a\r\r\nb"), "a\n\nb", "CR then CRLF is two breaks");
        assert_eq!(norm("a\n\rb"), "a\n\nb", "LF CR is two breaks, not one");
        assert_eq!(norm("a\r"), "a\n");
        assert_eq!(norm("\r\nx\r\n"), "\nx\n", "edges are kept");
    }

    #[test]
    fn composed_and_decomposed_forms_are_equal() {
        assert_eq!(norm("Cafe\u{301}"), "Caf\u{e9}");
        assert_eq!(norm("Caf\u{e9}"), "Caf\u{e9}");
        // Canonical reordering of combining marks (ccc 220 before 230).
        assert_eq!(norm("a\u{301}\u{323}"), norm("a\u{323}\u{301}"));
        // Hangul syllable from conjoining jamo.
        assert_eq!(norm("\u{1100}\u{1161}"), "\u{ac00}");
        // Singleton canonical decomposition: KELVIN SIGN is K.
        assert_eq!(norm("\u{212a}"), "K");
    }

    #[test]
    fn everything_else_is_kept() {
        for s in [
            "  A lantern glows at dawn.  ",
            "Blue dusk.",
            "blue dusk.",
            "tab\there",
            "no\u{a0}break",
            "two  spaces",
            "\u{feff}bom",
            "zero\u{200b}width",
            "\u{ff21}\u{fb01}", // full-width A, "fi" ligature: NFC, not NFKC
            "\n\nparagraphs\n\n",
            "\u{2028}line separator",
        ] {
            assert_eq!(norm(s), s, "{s:?}");
        }
    }

    #[test]
    fn normalize_applies_the_input_gate() {
        assert_eq!(normalize(""), Err(TextError::Empty));
        assert_eq!(normalize("\r\n"), Err(TextError::WhitespaceOnly));
        let over = "a".repeat(MAX_SOURCE_BYTES + 1);
        assert!(matches!(normalize(&over), Err(TextError::TooLong { .. })));
    }

    #[test]
    fn normalization_tables_are_the_reviewed_unicode_version() {
        // Bumping unicode-normalization changes this. Review the Unicode
        // stability notes in docs/seeds-and-recipes.md, then update it.
        assert_eq!(UNICODE_VERSION, (17, 0, 0));
    }

    #[test]
    fn debug_and_errors_never_show_text() {
        let t = normalize("secret marker").unwrap();
        assert_eq!(format!("{t:?}"), "NormalizedText(<13 bytes>)");
        let e = check_source(&"x".repeat(MAX_SOURCE_BYTES + 1)).unwrap_err();
        assert!(!e.to_string().contains('x'));
    }
}
