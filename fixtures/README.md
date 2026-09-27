# Synthetic seed passages

`passages.json` contains ten short, intentionally non-sensitive, human-written test inputs. They are **not** descriptions of requested image contents: the renderer must treat them only as seed material. No private user writing belongs in this repository.

- `shore-a` and `shore-b` differ by exactly one inserted character (`s`). A one-character edit may produce a completely different scene; no semantic similarity or smooth visual interpolation is guaranteed.
- `accent`, `non-latin`, `emoji` and `punctuation` exercise Unicode; `line-break` exercises LF; `whitespace` checks that significant leading/trailing spaces are preserved for a nonblank passage.
- Text normalization `nfc-lf-utf8/1` (frozen by task 04): Unicode NFC, CRLF and CR to LF, hashed as UTF-8. Case, other whitespace and punctuation are kept. Specification: [docs/seeds-and-recipes.md](../docs/seeds-and-recipes.md).
- `seed-vectors.json` holds the frozen digests, stream seeds and PRNG sequences for every passage above plus extra normalization cases (CRLF, lone CR, decomposed accents, compatibility characters). It is **generated** by `uv run scripts/seed-vectors.py`, an independent stdlib-only implementation, and the Rust tests must reproduce it exactly. Regenerate it only together with a new `NORMALIZATION_ID` or `SEED_ALGORITHM_ID`; a changed vector under the same id is a reproducibility break.
- Empty (`""`) and whitespace-only text are **invalid input**: show a prompt to enter prose and do not render, save a derived seed, or silently generate a default scene. A nonblank passage with extra whitespace remains a different input. The empty cases are policy test cases, **not** additional corpus passages.

When testing, record the passage ID and recipe/renderer versions, never infer image subjects from the text. Do not promise pixel-identical output across GPUs or backends.
