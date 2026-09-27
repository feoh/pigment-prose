//! Recipe files on disk (task 10). The format and its validation belong to
//! `pigment_core::recipe` (docs/seeds-and-recipes.md); this module reads
//! and writes files safely.
//!
//! - **Reading** checks the size before reading (a huge file is never
//!   loaded into memory), requires UTF-8, then runs the strict loader.
//!   Nothing is returned unless the whole recipe is valid.
//! - **Writing** serializes canonically (which validates first), writes a
//!   temporary file beside the destination and renames it into place
//!   ([`AtomicFile`]), so an existing valid recipe survives a failed or
//!   interrupted save.
//! - **Errors** carry an operation, an I/O kind and byte counts. They never
//!   include file contents, prose or the path (the caller knows which file
//!   it asked for), so they are safe to log.
//! - Everything stays local: no network, telemetry or sync.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use pigment_core::error::RecipeError;
use pigment_core::recipe::{MAX_RECIPE_BYTES, Recipe};

use crate::atomic::AtomicFile;

/// Suggested file name suffix for recipes. Nothing depends on it.
pub const RECIPE_EXTENSION: &str = "recipe.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Open,
    Read,
    /// Creating or writing the temporary file.
    Write,
    /// Moving the finished file over the destination.
    Replace,
}

impl fmt::Display for FileOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FileOp::Open => "open",
            FileOp::Read => "read",
            FileOp::Write => "write",
            FileOp::Replace => "replace",
        })
    }
}

/// A recipe file could not be read or written. Safe to log: no contents,
/// no prose, no path.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeFileError {
    Io {
        op: FileOp,
        kind: io::ErrorKind,
        /// The operating system's message (for example "Permission denied
        /// (os error 13)"), which never quotes file contents.
        message: String,
    },
    /// The path names a directory or other non-file.
    NotAFile,
    /// The file is not UTF-8 text; `valid_up_to` bytes were.
    NotUtf8 { valid_up_to: usize },
    /// The contents are not a valid recipe (see `RecipeError`).
    Recipe(RecipeError),
}

impl RecipeFileError {
    fn io(op: FileOp, e: &io::Error) -> RecipeFileError {
        RecipeFileError::Io {
            op,
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

impl fmt::Display for RecipeFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecipeFileError::Io { op, kind, message } => match kind {
                io::ErrorKind::NotFound => write!(f, "cannot {op} the recipe: it does not exist"),
                io::ErrorKind::PermissionDenied => {
                    write!(f, "cannot {op} the recipe: permission denied")
                }
                io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                    write!(f, "cannot {op} the recipe: the disk is full")
                }
                _ => write!(f, "cannot {op} the recipe: {message}"),
            },
            RecipeFileError::NotAFile => f.write_str("the recipe path is not a file"),
            RecipeFileError::NotUtf8 { valid_up_to } => write!(
                f,
                "the recipe is not UTF-8 text (invalid bytes after byte {valid_up_to})"
            ),
            RecipeFileError::Recipe(e) => write!(f, "invalid recipe: {e}"),
        }
    }
}

impl std::error::Error for RecipeFileError {}

impl From<RecipeError> for RecipeFileError {
    fn from(e: RecipeError) -> Self {
        RecipeFileError::Recipe(e)
    }
}

/// Reads and fully validates a recipe file. On any error nothing is
/// returned, so the caller's current document is untouched.
pub fn read_recipe(path: &Path) -> Result<Recipe, RecipeFileError> {
    let file = File::open(path).map_err(|e| RecipeFileError::io(FileOp::Open, &e))?;
    let meta = file
        .metadata()
        .map_err(|e| RecipeFileError::io(FileOp::Open, &e))?;
    if !meta.is_file() {
        return Err(RecipeFileError::NotAFile);
    }
    let too_large = |bytes: u64| {
        RecipeFileError::Recipe(RecipeError::TooLarge {
            bytes: usize::try_from(bytes).unwrap_or(usize::MAX),
            max: MAX_RECIPE_BYTES,
        })
    };
    if meta.len() > MAX_RECIPE_BYTES as u64 {
        return Err(too_large(meta.len()));
    }
    // Read at most one byte past the limit, in case the file grew since
    // the size check.
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_RECIPE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| RecipeFileError::io(FileOp::Read, &e))?;
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err(too_large(bytes.len() as u64));
    }
    let text = String::from_utf8(bytes).map_err(|e| RecipeFileError::NotUtf8 {
        valid_up_to: e.utf8_error().valid_up_to(),
    })?;
    Ok(Recipe::from_json(&text)?)
}

/// Validates, serializes canonically and atomically replaces `path`.
/// Returns the number of bytes written. `recipe.source_text` is written
/// exactly as given: callers decide whether to keep the prose (see
/// `Document::save_as`), and `None` means the file has no `source_text` key.
pub fn write_recipe(path: &Path, recipe: &Recipe) -> Result<u64, RecipeFileError> {
    let json = recipe.to_canonical_json()?;
    let mut file = AtomicFile::create(path).map_err(|e| {
        if e.kind() == io::ErrorKind::IsADirectory {
            RecipeFileError::NotAFile
        } else {
            RecipeFileError::io(FileOp::Write, &e)
        }
    })?;
    file.file()
        .expect("a new AtomicFile is open")
        .write_all(json.as_bytes())
        .map_err(|e| RecipeFileError::io(FileOp::Write, &e))?;
    file.commit()
        .map_err(|e| RecipeFileError::io(FileOp::Replace, &e))?;
    Ok(json.len() as u64)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pigment_core::error::MalformedKind;
    use pigment_core::frame::UHD_4K;
    use pigment_core::seed::TextDigest;

    use super::*;
    use crate::test_dir::TestDir;

    const MARKER: &str = "zebra lantern";

    fn sample() -> Recipe {
        Recipe::new(TextDigest::from_source("Blue dusk.").unwrap(), UHD_4K)
    }

    #[test]
    fn round_trip_is_canonical_and_byte_stable() {
        let dir = TestDir::new("recipe-rt");
        let path = dir.path().join("a.recipe.json");
        let r = sample();
        let n = write_recipe(&path, &r).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(n, bytes.len() as u64);
        assert_eq!(read_recipe(&path).unwrap(), r);
        write_recipe(&path, &read_recipe(&path).unwrap()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes, "stable bytes");
        assert_eq!(dir.entries(), vec!["a.recipe.json".to_string()]);
    }

    #[test]
    fn invalid_recipes_are_never_written() {
        let dir = TestDir::new("recipe-invalid");
        let path = dir.path().join("a.recipe.json");
        write_recipe(&path, &sample()).unwrap();
        let before = fs::read(&path).unwrap();
        let mut bad = sample();
        bad.form.relief = 7.0;
        assert!(matches!(
            write_recipe(&path, &bad),
            Err(RecipeFileError::Recipe(RecipeError::Validation(_)))
        ));
        let mut wrong = sample();
        wrong.source_text = Some("not the prose of this digest".into());
        assert!(matches!(
            write_recipe(&path, &wrong),
            Err(RecipeFileError::Recipe(RecipeError::SourceTextMismatch))
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(dir.entries(), vec!["a.recipe.json".to_string()]);
    }

    #[test]
    fn unicode_names_and_prose_round_trip() {
        let dir = TestDir::new("recipe-unicode");
        let path = dir.path().join("湖畔 — ünïcødé 🌲.recipe.json");
        let prose = "Café au lait\r\nunder 山 and 🌲\u{00a0}— «ok»\u{200b}";
        let mut r = Recipe::new(TextDigest::from_source(prose).unwrap(), UHD_4K);
        r.source_text = Some(prose.into());
        write_recipe(&path, &r).unwrap();
        assert_eq!(read_recipe(&path).unwrap(), r);
    }

    #[test]
    fn truncated_files_are_rejected_at_every_length() {
        let dir = TestDir::new("recipe-trunc");
        let path = dir.path().join("t.recipe.json");
        let full = sample().to_canonical_json().unwrap();
        for cut in (0..full.len() - 1).step_by(7) {
            fs::write(&path, &full.as_bytes()[..cut]).unwrap();
            let e = read_recipe(&path).unwrap_err();
            assert!(
                matches!(
                    e,
                    RecipeFileError::Recipe(RecipeError::Malformed { .. })
                        | RecipeFileError::Recipe(RecipeError::WrongType { .. })
                ),
                "cut {cut}: {e}"
            );
        }
        fs::write(&path, "").unwrap();
        assert!(matches!(
            read_recipe(&path),
            Err(RecipeFileError::Recipe(RecipeError::Malformed {
                kind: MalformedKind::Truncated,
                ..
            }))
        ));
    }

    #[test]
    fn future_schemas_are_rejected() {
        let dir = TestDir::new("recipe-future");
        let path = dir.path().join("f.recipe.json");
        let json = sample()
            .to_canonical_json()
            .unwrap()
            .replace("\"schema\": 1", "\"schema\": 2")
            .replace(
                "\"atmosphere\": {",
                "\"season\": \"autumn\",\n  \"atmosphere\": {",
            );
        fs::write(&path, json).unwrap();
        assert!(matches!(
            read_recipe(&path),
            Err(RecipeFileError::Recipe(RecipeError::UnsupportedSchema {
                found: 2,
                supported: 1
            }))
        ));
    }

    #[test]
    fn oversized_binary_and_missing_files_are_rejected_without_reading() {
        let dir = TestDir::new("recipe-size");
        let big = dir.path().join("big.recipe.json");
        let f = File::create(&big).unwrap();
        f.set_len(MAX_RECIPE_BYTES as u64 + 1).unwrap(); // sparse: nothing read
        assert!(matches!(
            read_recipe(&big),
            Err(RecipeFileError::Recipe(RecipeError::TooLarge { .. }))
        ));
        let bin = dir.path().join("bin.recipe.json");
        fs::write(&bin, b"{\"schema\": 1, \xff\xfe}").unwrap();
        assert_eq!(
            read_recipe(&bin),
            Err(RecipeFileError::NotUtf8 { valid_up_to: 14 })
        );
        let e = read_recipe(&dir.path().join("missing.recipe.json")).unwrap_err();
        assert!(matches!(
            e,
            RecipeFileError::Io {
                op: FileOp::Open,
                kind: io::ErrorKind::NotFound,
                ..
            }
        ));
        assert!(matches!(
            read_recipe(dir.path()),
            Err(RecipeFileError::NotAFile) | Err(RecipeFileError::Io { .. })
        ));
        assert_eq!(
            write_recipe(dir.path(), &sample()),
            Err(RecipeFileError::NotAFile)
        );
    }

    #[test]
    fn an_interrupted_save_keeps_the_previous_file() {
        let dir = TestDir::new("recipe-crash");
        let path = dir.path().join("keep.recipe.json");
        let old = sample();
        write_recipe(&path, &old).unwrap();
        // A save that dies after writing half the new file: the process is
        // gone, so no destructor runs (mem::forget stands in for the crash).
        let mut new = sample();
        new.form.relief = 0.9;
        let json = new.to_canonical_json().unwrap();
        let mut f = AtomicFile::create(&path).unwrap();
        f.file()
            .unwrap()
            .write_all(&json.as_bytes()[..json.len() / 2])
            .unwrap();
        let orphan = f.temp_path().to_path_buf();
        std::mem::forget(f);
        assert_eq!(read_recipe(&path).unwrap(), old, "the old recipe survives");
        assert!(orphan.exists(), "a crash can leave a hidden temporary file");
        // The next successful save still works and never picks it up.
        write_recipe(&path, &new).unwrap();
        assert_eq!(read_recipe(&path).unwrap(), new);
    }

    #[cfg(unix)]
    #[test]
    fn permission_errors_are_reported_and_keep_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("recipe-perm");
        let sub = dir.path().join("locked");
        fs::create_dir(&sub).unwrap();
        let path = sub.join("p.recipe.json");
        write_recipe(&path, &sample()).unwrap();
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let root = fs::read(&path).is_ok(); // root ignores permissions
        if !root {
            let e = read_recipe(&path).unwrap_err();
            assert!(
                matches!(
                    e,
                    RecipeFileError::Io {
                        kind: io::ErrorKind::PermissionDenied,
                        ..
                    }
                ),
                "{e}"
            );
            assert_eq!(e.to_string(), "cannot open the recipe: permission denied");
            let mut changed = sample();
            changed.form.relief = 0.1;
            let e = write_recipe(&path, &changed).unwrap_err();
            assert!(
                matches!(
                    e,
                    RecipeFileError::Io {
                        op: FileOp::Write,
                        kind: io::ErrorKind::PermissionDenied,
                        ..
                    }
                ),
                "{e}"
            );
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn errors_are_redacted() {
        let dir = TestDir::new("recipe-redact");
        let secret_dir = dir.path().join(MARKER);
        fs::create_dir(&secret_dir).unwrap();
        let path = secret_dir.join(format!("{MARKER}.recipe.json"));
        let mut kept = sample();
        kept.source_text = Some(MARKER.into());
        kept.seed.digest = TextDigest::from_source(MARKER).unwrap();
        let json = kept.to_canonical_json().unwrap();
        let cases: Vec<Vec<u8>> = vec![
            json.replace("0.55", &format!("\"{MARKER}\"")).into_bytes(),
            json.replace("\"lakeshore\"", MARKER).into_bytes(),
            json.replace(&format!("\"{MARKER}\""), "\"zebra lanterns\"")
                .into_bytes(),
            json.as_bytes()[..json.len() - 9].to_vec(),
            [json.as_bytes(), b"\xff zebra"].concat(),
        ];
        let mut errors: Vec<RecipeFileError> = cases
            .iter()
            .map(|bytes| {
                fs::write(&path, bytes).unwrap();
                read_recipe(&path).unwrap_err()
            })
            .collect();
        errors.push(read_recipe(&secret_dir.join("missing")).unwrap_err());
        errors.push(write_recipe(&secret_dir.join("no/such"), &kept).unwrap_err());
        for e in errors {
            for shown in [e.to_string(), format!("{e:?}")] {
                assert!(!shown.contains("zebra"), "{shown}");
            }
        }
    }
}

#[cfg(test)]
mod documented_messages {
    use std::fs;

    use pigment_core::frame::UHD_4K;
    use pigment_core::seed::TextDigest;

    use super::*;

    /// The sample diagnostics quoted in docs/recipe-files.md, exactly.
    #[test]
    fn sample_messages_are_as_documented() {
        let dir = crate::test_dir::TestDir::new("samples");
        let p = dir.path().join("r.recipe.json");
        let mut r = Recipe::new(TextDigest::from_source("Private words.").unwrap(), UHD_4K);
        r.source_text = Some("Private words.".into());
        let json = r.to_canonical_json().unwrap();
        let cases = [
            (
                json[..json.len() / 2].to_string(),
                "invalid recipe: the recipe ends unexpectedly (truncated?) at line 18, column 8",
            ),
            (
                json.replace("\"schema\": 1", "\"schema\": 2"),
                "invalid recipe: recipe schema 2 is not supported; this version reads schema 1",
            ),
            (
                json.replace("0.55", "1.7"),
                "invalid recipe: form.faceting = 1.7 is outside 0..=1",
            ),
            (
                json.replace("\"faceting\"", "\"my private diary entry about the lake\""),
                "invalid recipe: `form.<unrecognized>` is not a recipe field",
            ),
            (
                json.replace("\"edge_looseness\"", "\"edge_loosness\""),
                "invalid recipe: `painting.edge_loosness` is not a recipe field",
            ),
            (
                json.replace("Private words.", "Other words."),
                "invalid recipe: `source_text` does not match `seed.digest`",
            ),
            (
                json.replace("\"lakeshore\"", "\"sunset\""),
                "invalid recipe: `palette.id` is not a known palette",
            ),
        ];
        for (contents, message) in cases {
            fs::write(&p, contents).unwrap();
            assert_eq!(read_recipe(&p).unwrap_err().to_string(), message);
        }
        assert_eq!(
            read_recipe(&dir.path().join("x")).unwrap_err().to_string(),
            "cannot open the recipe: it does not exist"
        );
        let full = io::Error::from(io::ErrorKind::StorageFull);
        assert_eq!(
            RecipeFileError::io(FileOp::Write, &full).to_string(),
            "cannot write the recipe: the disk is full"
        );
    }
}
