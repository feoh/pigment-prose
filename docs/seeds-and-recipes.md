# Seeds and recipes (task 04)

This document specifies how prose becomes a seed and how a painting is recorded as a recipe. It is the reference for `crates/pigment-core/src/{text,seed,recipe}.rs`. The frozen test vectors are in [`fixtures/seed-vectors.json`](../fixtures/seed-vectors.json), and an independent stdlib-only Python implementation is in [`scripts/seed-vectors.py`](../scripts/seed-vectors.py). The Rust code and the Python script must produce identical vectors. Contracts and the surrounding pipeline are in [architecture.md](architecture.md).

Prose is **non-semantic seed material**. Nothing here parses meaning, and no prose is logged, put into an error message or written into an image.

## Text normalization `nfc-lf-utf8/1`

`text::normalize(source) -> Result<NormalizedText, TextError>`:

1. **Input gate** (`text::check_source`). Reject `""` (`TextError::Empty`), text made only of Unicode `White_Space` characters (`WhitespaceOnly`), and text over `MAX_SOURCE_BYTES` = 1 MiB of UTF-8 *before* normalization (`TooLong`). The size check runs first, so an oversized paste never reaches the hasher. The UI shows a prompt and renders nothing.
2. **Unicode NFC** (canonical composition), using the tables of `unicode-normalization` 0.1.25, which are Unicode 17.0.0 (`text::UNICODE_VERSION`). Canonically equivalent text is the same prose. `Cafe` followed by U+0301 equals `Café`, conjoining jamo equal the precomposed Hangul syllable, and U+212A KELVIN SIGN equals `K`.
3. **Line endings.** CRLF becomes LF and a lone CR becomes LF. `\r\r\n` is therefore two line breaks, and so is `\n\r`.
4. **Encoding.** The result is hashed as UTF-8 bytes.

**Kept exactly:** case, punctuation, leading and trailing whitespace, repeated spaces and blank lines, tabs, no-break spaces, U+2028/U+2029, invisible characters (U+200B, a leading U+FEFF byte-order mark) and compatibility variants. Full-width letters and ligatures stay distinct because this is NFC, not NFKC. `Blue dusk.`, `blue dusk.`, `Blue dusk.\n` and `Blue` + U+00A0 + `dusk.` are four different seeds (see the fixture).

**Unicode stability.** Unicode guarantees that NFC never changes for text made of assigned characters. Text containing code points that are *unassigned* in Unicode 17.0 could normalize differently after a future table upgrade. This only affects re-typing such text into a later build. A saved recipe stores the digest and never re-normalizes. A test pins `UNICODE_VERSION`, so a dependency upgrade that changes the tables fails CI and has to be reviewed.

## Seed algorithm `pigment-seed/1`

All hashing is SHA-256 (the `sha2` crate, RustCrypto). `‖` is concatenation, and `u32le(n)` is the 4-byte little-endian encoding.

**Text digest** (`TextDigest::of`, 32 bytes, serialized as 64 lowercase hex digits):

```text
digest = SHA-256( "pigment-prose/text/v1" ‖ 0x00 ‖ normalized_utf8 )
```

**Stream seeds** (`StreamSeed::derive`, `SeedBundle::derive`). Each seed is a `u64`:

```text
msg  = "pigment-prose/stream/v1" ‖ 0x00 ‖ u32le(len(label)) ‖ label ‖ digest
       ‖ u32le(variation)          -- only if the domain uses the variation
seed = u64 from the first 8 bytes of SHA-256(msg), little-endian
```

| Domain | Label | Uses variation | Drives |
| --- | --- | --- | --- |
| `Composition` | `composition` | yes | template, horizon, focal placement, framing |
| `Terrain` | `terrain` | yes | ridge massing, landform detail, shoreline, rock planes |
| `Vegetation` | `vegetation` | yes | woodland placement and silhouettes |
| `PaintDetail` | `paint-detail` | **no** | mark jitter, granulation, paper grain, wash edges |

The labels are ASCII and length-prefixed, and the tag strings end in a NUL, so no two different inputs encode to the same message. Every stream is a function of the digest, its own label and the variation only. No stream is derived from another stream's seed or state.

**Variation.** `Variation(u32)` starts at 0, and "Another Composition" increments it. It changes the three structural streams and leaves paint detail unchanged, so the layout changes while the paper and pigment texture stay familiar. Palette, atmosphere, painting and form settings are not seed inputs at all, so they can never change a seed. `Recipe::seeds()` reads only `seed.digest` and `seed.variation`.

**PRNG** (`StreamSeed::rng() -> Rng`) is xoshiro256\*\* 1.0 (Blackman and Vigna). Its four state words are four successive SplitMix64 outputs starting from the stream seed. The implementation reproduces the reference C code's published sequence for state `{1, 2, 3, 4}`. The fixture's `rng` vectors were checked against the C reference compiled with gcc and against the Python script. Helpers:

| Method | Definition |
| --- | --- |
| `next_u64` | one xoshiro256\*\* step |
| `next_u32` | upper 32 bits of `next_u64` |
| `next_f64` | `(next_u64 >> 11) × 2⁻⁵³`, uniform in `[0, 1)`, exact |
| `range_f64(lo, hi)` | `lo + (hi − lo) × next_f64()` (plain IEEE operations, no fused multiply-add) |
| `below(n)` | rejection sampling: draw until `x ≥ (2⁶⁴ mod n)`, return `x mod n`. Unbiased. Panics on 0. |

Each `rng()` call returns a fresh generator at the start of the stream. Consumers own their generator and never share one across domains. Advancing the paint-detail generator any number of times leaves every terrain value unchanged (tested). GPU passes do not use `Rng`. They hash integer lattice coordinates with the raw `StreamSeed` value (architecture.md, "Texture, brush sizing and halo accounting").

The PRNG is part of `pigment-seed/1`. Changing the digest, the stream derivation or the PRNG requires a new `SEED_ALGORITHM_ID`, never a silent change.

## Reproducibility boundaries

- **Exact everywhere (tier 0):** source text → normalized bytes → digest → stream seeds → `Rng` sequences. The code uses only integer operations, SHA-256 and exact IEEE arithmetic, with no platform math, default hasher or randomized hasher. Evidence: the frozen vectors, two independent implementations, and portable CI on Linux, Windows and macOS.
- **Not guaranteed:** pixels across GPUs, drivers or backends (tier 3 in architecture.md). Scene geometry is exact only within one `GENERATOR_VERSION` (tier 1).
- **No similarity.** A one-character edit (`shore-a` vs `shore-b`) gives an unrelated digest and unrelated streams. No smooth interpolation between texts is implied. The tests compare fixed fixture pairs and do not assume collisions are impossible. Two texts could in principle share a digest; SHA-256 makes that astronomically unlikely, not impossible.
- **Not a privacy mechanism.** Anyone who can guess the prose can confirm the guess against a digest. The digest is not encryption.

## Recipe schema 1

`Recipe::to_canonical_json` writes, and `Recipe::from_json` reads, one JSON object. [`docs/examples/recipe.example.json`](examples/recipe.example.json) is a canonical example (fixture `shore-a`, variation 2). A test requires it to load and to re-serialize byte-identically. The authoritative key table is `recipe::SCHEMA`, and a test checks it against the serde structs.

| Key | JSON type | Constraint |
| --- | --- | --- |
| `schema` | integer | must be `1` |
| `versions.normalization` | string | must be `nfc-lf-utf8/1` |
| `versions.seed_algorithm` | string | must be `pigment-seed/1` |
| `versions.generator`, `versions.renderer` | integer 0…2³²−1 | any; a difference from this build is a notice, not an error |
| `seed.digest` | string | 64 lowercase hex digits |
| `seed.variation` | integer 0…2³²−1 | — |
| `frame.width`, `frame.height` | integer 0…2³²−1 | 64…16384 px each, long/short ≤ 4 |
| `form.{faceting, relief, woodland_density}` | number | range from `settings::CONTROLS` |
| `painting.{edge_looseness, wash_gouache, mark_scale, granulation, paper_grain}` | number | range from `settings::CONTROLS` |
| `palette.id` | string | `lakeshore` or `golden-evening` |
| `palette.intensity`, `atmosphere.haze` | number | range from `settings::CONTROLS` |
| `source_text` | string | **optional**. Present only if the user chose to keep the prose (task 10). It must pass the input gate and reproduce `seed.digest`. |

Every key except `source_text` is required. There are no defaults, so a missing value never turns into a different painting.

## Load policy

`Recipe::from_json` runs these checks in order and returns the first failure as a `RecipeError`. On any error, nothing is applied (task 10 keeps the current document).

| # | Situation | Result |
| --- | --- | --- |
| 1 | more than `MAX_RECIPE_BYTES` (8 MiB) | `TooLarge` before parsing. 8 MiB covers a 1 MiB `source_text` even at JSON's worst-case 6× escaping (tested). |
| 2 | invalid JSON, trailing data, `NaN`/`Infinity`, number literal overflowing `f64` (`1e400`) | `Malformed { kind: Syntax, line, column }` |
| 2 | empty or truncated file | `Malformed { kind: Truncated, … }` |
| 2 | a key repeated in one object, at any depth | `Malformed { kind: DuplicateKey, … }`. A plain JSON value would silently keep the last one. |
| 2 | nesting deeper than 128 | `Malformed` (serde_json's recursion limit, no stack overflow) |
| 3 | top level not an object | `WrongType { path: "", expected: "a JSON object" }` |
| 3 | `schema` missing / not a non-negative integer | `MissingField` / `WrongType` |
| 3 | `schema` ≠ 1 (future or 0) | `UnsupportedSchema { found, supported: 1 }`, checked **before** the shape, so a future file is never reported as a pile of unknown fields |
| 4 | a key not in the schema (typo, newer setting, season) | `UnknownField { path }`. It is never ignored, because ignoring `painting.edge_loosness` would silently repaint. |
| 4 | a required key missing | `MissingField { path }` |
| 4 | wrong JSON type (string for number, fraction or negative for an integer, `null`) | `WrongType { path, expected }` |
| 4 | malformed digest, unknown palette id | `InvalidValue { path, reason }` |
| 5 | unknown normalization or seed algorithm id | `UnsupportedAlgorithm { field, found }`. The seed cannot be reproduced. |
| 5 | a setting outside its range, bad frame size or aspect | `Validation(ValidationError)`. Values are **rejected, never clamped**. |
| 5 | `source_text` empty, blank or oversized | `SourceText(TextError)` |
| 5 | `source_text` does not reproduce `seed.digest` | `SourceTextMismatch`. The digest is authoritative, and showing prose that is not the painting's would be wrong. |
| — | `versions.generator` or `.renderer` differ from this build | **loads**. `Recipe::version_notices()` returns "made with generator v7; this version (v0) may compose differently" for the UI. |

**No file contents in errors.** Paths are built only from schema key names. An unrecognized key or algorithm id is shown only if it is at most 64 characters from `[A-Za-z0-9_./-]`, and otherwise as `<unrecognized>` (`error::sanitized_key`). serde_json's own messages, which can quote values, are never passed through. Only their line, column and category are used. A test plants a marker string in every field position and checks every `Display` and `Debug` output.

## Canonical serialization

`Recipe::to_canonical_json` first runs the same checks as loading (`Recipe::validate`), so the app never writes a file it would refuse to read. For example, a `NaN` setting is an error, not `null`. The output format:

- keys in schema order, which is the table above
- two-space indentation, one key per line, `": "` separators
- numbers in shortest round-trip form (`0.55`, `1.0`), integers without a fraction
- strings with non-ASCII characters kept as UTF-8 and control characters escaped
- no `source_text` key at all when the prose was not kept (never `null` or `""`)
- a single trailing newline

`from_json(to_canonical_json(r)) == r`, and serializing again gives the same bytes (tested, including `f64` values such as ⅓). `.gitattributes` forces LF so the byte-exact example also holds on Windows checkouts.

## Versions and migration limits

- **Schema.** Only schema 1 exists, so there are no migrations. A later schema must come with an explicit, tested migration from each schema it claims to read. A migration may only rename or restructure keys, or add keys whose value reproduces the old painting exactly. If it cannot guarantee that, it must reject the old file with a clear message. It must never guess a default.
- **Normalization and seed algorithm.** These cannot be migrated from the digest alone, because a digest cannot be re-hashed under a new normalization. A build that introduces `pigment-seed/2` must either keep implementing `/1` for old recipes or reject them as `UnsupportedAlgorithm`. Re-deriving from `source_text` is only possible when the user kept it, and it changes the painting.
- **Generator and renderer.** The app ships one of each (architecture.md, "Versioning"). An old recipe opens with a notice and may compose or paint differently. Its recorded versions update only when the user saves.
- **Settings ranges.** Narrowing a range, or removing a palette id, makes old files fail validation. Either avoid that, or bump the schema and migrate.

## Tests and commands

| What | Command |
| --- | --- |
| All portable checks (fmt, clippy, tests, reference vectors) | `scripts/check.sh` |
| Seed, text and recipe tests only | `cargo test --locked -p pigment-core --lib -- text:: seed:: recipe::` |
| Reference implementation agrees with the fixture | `python3 scripts/seed-vectors.py --check` (or `uv run scripts/seed-vectors.py --check`) |
| Regenerate the fixture (**only** with a new algorithm id) | `uv run scripts/seed-vectors.py` |

Coverage by acceptance item:

- **Frozen vectors:** `seed::tests::frozen_vectors_reproduce_exactly` covers 21 texts × variations {0, 1, 2³²−1} × 4 streams, plus xoshiro sequences for four seeds and the rejected-input list. `declared_equivalences_hold` covers CRLF, lone CR, NFD and the Kelvin sign. `rng_matches_the_xoshiro_reference` checks the published sequence.
- **Stream isolation:** `advancing_one_stream_leaves_the_others_alone`, `variation_changes_only_the_structural_streams`, `recipe::tests::palette_and_paint_settings_do_not_touch_seeds`.
- **Recipe round trips:** `canonical_json_round_trips`, `documented_example_is_canonical_and_loads`, `omitted_source_text_still_reproduces_the_seed`, `schema_table_matches_the_structs`.
- **Rejection:** `future_and_unknown_schemas_are_rejected_first`, `unknown_algorithms_are_rejected`, `unknown_and_missing_fields_are_rejected`, `wrong_types_and_bad_values_are_rejected`, `out_of_range_values_are_rejected_not_clamped`, `corrupt_and_oversized_files_are_rejected`, `duplicate_keys_are_rejected_at_any_depth`, `source_text_must_match_the_digest`, `text::tests::normalize_applies_the_input_gate`.
- **Privacy:** `errors_never_contain_file_contents`, `text::tests::debug_and_errors_never_show_text`.
