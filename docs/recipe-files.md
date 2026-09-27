# Recipe files and the source-text choice (task 10)

This covers saving and reopening editable paintings, and where the privacy boundary between a recipe and an exported image sits. The recipe **format** (schema 1, canonical JSON, the strict loader) is specified in [seeds-and-recipes.md](seeds-and-recipes.md). This document covers files on disk and the document model:

- `crates/pigment-io/src/recipe_file.rs`: `read_recipe`, `write_recipe`, `RecipeFileError`
- `crates/pigment-io/src/document.rs`: `Document`, `SaveError`

## What a recipe file holds

A recipe holds everything needed to repaint without the prose: the text digest and variation (`seed`), the frame, every form, painting, palette and atmosphere setting, and the algorithm and component versions. Every key is required, so nothing is filled in with a default. The UI shows the painting from these fields alone.

- **Where:** wherever the user saves it. The app keeps no gallery, database, auto-save history or hidden copies, makes no network, telemetry or sync calls, and writes nothing outside the chosen file (plus its temporary sibling while saving). The suggested suffix is `.recipe.json` (`RECIPE_EXTENSION`). Nothing depends on it.
- **Reproduction after restart:** a saved recipe reopened by a new process rebuilds the same seeds, settings and scene (`a_saved_recipe_reproduces_after_restart` runs the save in a child process). On the same device, driver and backend it also repaints the same pixels (`approved_recipes_repaint_identically_after_save_and_load`). Across devices, see the reproducibility tiers in [architecture.md](architecture.md#reproducibility-tiers).

## The source-text choice

`Document::keep_source_text` is an explicit, per-document choice.

| State | Saved file | Reopened document |
| --- | --- | --- |
| **Off (default)** | no `source_text` key at all: not `null`, not `""`, not an encoding of the text | `prose()` is `None`. The painting reproduces from the digest, but the words cannot be recovered from the file. |
| **On**, prose known | `"source_text": "<the exact prose, not normalized>"`, which must reproduce the digest | `prose()` returns it; the choice stays on |
| On, but no prose known (opened from a source-free recipe) | no `source_text` (`saves_source_text()` is false) | as before |

- A new document from prose starts with the choice **off**. Opening a recipe that kept its prose turns it **on**, and opening one without keeps it off.
- Changing the choice marks the document dirty only when it changes what would be saved.
- **Exports never contain source text, whatever the choice.** A render request carries a scene, seeds and appearance only, and the PNG writer adds no text chunks ([export.md](export.md#png-encoding)). `kept_prose_never_reaches_the_exported_png` saves a recipe that keeps the prose, reopens it, exports it, and searches the PNG for the words.
- **A digest is not encryption.** The `seed.digest` in every recipe is SHA-256 of the normalized prose. It cannot be reversed, but anyone who can guess the text (a famous quotation, a short phrase, a name) can confirm the guess by hashing it. A source-free recipe therefore hides the prose only as well as the prose is hard to guess. It cannot guarantee secrecy for short or well-known text. There is no password encryption (a non-goal).

## Document model

`Document` is the UI's open painting: the effective recipe (always held without source text), the prose in the editor (in memory only), the keep choice, the current path and a dirty flag. It lives in `pigment-io` and has no renderer state, and task 11/12 hold one per window.

| Operation | Behaviour |
| --- | --- |
| `Document::from_prose(prose, frame)` | New unsaved document with default settings. Starts dirty; prose rejected by the input gate is an error |
| `set_prose`, `set_variation`, `next_variation`, `set_frame`, `set_form`, `set_appearance`, `set_keep_source_text` | Validate first. An invalid value changes nothing. The document is marked dirty only if the saved file would differ; the same digest (for example CRLF vs LF) is not a change unless the prose is kept |
| `Document::open(path)` | Reads and validates the file into a **new** document, plus version notices for the UI. It never touches the current document |
| `replace_from_file(&mut self, path)` | Replaces the document only if the file loads. On any error the current document, including unsaved edits, is unchanged (`failed_open_leaves_the_current_document_unchanged`) |
| `save_as(path)`, `save()` | Write through `write_recipe`. On success: the path is set, dirty is cleared, and the recorded generator and renderer versions become this build's (the rule in [architecture.md](architecture.md#versioning-and-compatibility)). On failure nothing changes. `save()` without a path is `SaveError::NoPath` |
| `to_saved_recipe()` | Exactly what a save would write |

`Debug` for `Document` and for `Recipe` shows the prose only as a byte count (`<13 bytes>`), so neither can leak it into a log or a test failure.

## Reading and writing safely

- **Validate before use.** `read_recipe` checks the file's size before reading it, reading at most 8 MiB + 1 byte in case it grew after the check. It then requires UTF-8 and runs `Recipe::from_json`, the strict loader: future schemas, unknown and missing keys, wrong types and out-of-range values are all rejected, never clamped or defaulted. Nothing is returned unless the whole file is valid.
- **Never write an invalid file.** `write_recipe` serializes canonically, which validates first, so the app cannot save a file it would refuse to open. That includes a `source_text` that does not match the digest.
- **Atomic replacement** (`AtomicFile`, shared with PNG export). The new file is written to a hidden temporary file in the same directory (`.NAME.<pid>-<n>.pigment-tmp`) and flushed to disk (`fsync`). It is then renamed over the destination (POSIX `rename`; `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on Windows), and on Unix the directory is synced too. A failed write, a permission error or a crash leaves the previous valid file untouched. A crash can leave the hidden temporary file behind. It is never read and can be deleted.
- **Limits:** the replaced file's permissions are not copied (the new file gets the process defaults). On Windows, saving over a file that another program holds open fails and keeps the old file.

## Errors and redacted diagnostics

`RecipeFileError` is `Io { op, kind, message }`, `NotAFile`, `NotUtf8 { valid_up_to }` or `Recipe(RecipeError)`. No variant contains file contents, prose or the path: the UI knows which file it asked for and shows it itself. `message` is the operating system's text, which never quotes file contents. Recipe errors name schema keys only; an unrecognized key is shown only if it looks like an identifier. These are the exact messages the code produces (`sample_messages_are_as_documented`):

```text
cannot open the recipe: it does not exist
cannot open the recipe: permission denied
cannot write the recipe: the disk is full
the recipe is not UTF-8 text (invalid bytes after byte 14)
invalid recipe: the recipe ends unexpectedly (truncated?) at line 18, column 8
invalid recipe: recipe schema 2 is not supported; this version reads schema 1
invalid recipe: form.faceting = 1.7 is outside 0..=1
invalid recipe: `painting.edge_loosness` is not a recipe field
invalid recipe: `form.<unrecognized>` is not a recipe field
invalid recipe: `palette.id` is not a known palette
invalid recipe: `source_text` does not match `seed.digest`
```

The `<unrecognized>` line comes from a file whose key was a sentence of prose. Pigment Prose has no logging framework. Whatever the UI adds must log these values, never the prose or the file.

## Tests

| Test | Where | Covers |
| --- | --- | --- |
| `approved_recipes_round_trip_and_reproduce_their_scenes` | `crates/pigment-io/tests/recipes.rs` (portable) | All 47 task 08 baseline recipes: open, save byte-identically, reload equal, and regenerate the frozen geometry checksum from the sheet notes |
| `a_saved_recipe_reproduces_after_restart` | same | Saved in a child process, reopened in the parent: same seeds, settings, checksum; no prose |
| `approved_recipes_repaint_identically_after_save_and_load` | `crates/pigment-io/tests/gpu_export.rs` (hardware) | Reopened copies of the 47 recipes paint the same pixels as the originals (single tile vs 128 px tiles); on the baseline device they equal the approved image hashes |
| `kept_prose_never_reaches_the_exported_png` | same | Kept prose is in the recipe, never in the PNG |
| `source_free_recipes_truly_omit_the_prose`, `kept_prose_round_trips_and_can_be_dropped`, `keep_choice_without_prose_saves_no_source` | `crates/pigment-io/src/document.rs` | The choice. A source-free file has exactly the eight schema keys and contains no word or hex encoding of the prose; kept Unicode prose (CRLF, emoji, CJK, NBSP) round-trips exactly |
| `dirty_state_follows_real_changes_only`, `invalid_edits_change_nothing`, `failed_open_leaves_the_current_document_unchanged`, `failed_save_keeps_path_dirty_state_and_file`, `saving_records_current_versions_and_notices_old_ones`, `debug_never_shows_the_prose` | same | The document model |
| `round_trip_is_canonical_and_byte_stable`, `invalid_recipes_are_never_written`, `unicode_names_and_prose_round_trip`, `truncated_files_are_rejected_at_every_length`, `future_schemas_are_rejected`, `oversized_binary_and_missing_files_are_rejected_without_reading`, `an_interrupted_save_keeps_the_previous_file`, `permission_errors_are_reported_and_keep_the_file` (Unix), `errors_are_redacted`, `sample_messages_are_as_documented` | `crates/pigment-io/src/recipe_file.rs` | Files: Unicode file names, truncation every 7 bytes, schema 2, a sparse 8 MiB + 1 file (rejected before reading), invalid UTF-8, a save killed halfway (`mem::forget` stands in for the crash), read-only directory and unreadable file, and a marker planted in the file contents and path |
| `recipe::tests::debug_shows_the_length_of_kept_source_text_only` | `pigment-core` | Redacted `Debug` |

Run: `cargo test --locked -p pigment-io` (portable) and `scripts/gpu-tests.sh` (hardware). Permission tests are Unix-only, and their assertions are skipped when running as root (root ignores permissions). Windows and macOS run the portable tests in CI; their atomic-replace behaviour is only as verified as that CI.
