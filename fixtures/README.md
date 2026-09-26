# Synthetic seed passages

`passages.json` contains ten short, intentionally non-sensitive, human-written test inputs. They are **not** descriptions of requested image contents: the renderer must treat them only as seed material. No private user writing belongs in this repository.

- `shore-a` and `shore-b` differ by exactly one inserted character (`s`). A one-character edit may produce a completely different scene; no semantic similarity or smooth visual interpolation is guaranteed.
- `accent`, `non-latin`, `emoji` and `punctuation` exercise Unicode; `line-break` exercises LF; `whitespace` checks that significant leading/trailing spaces are preserved for a nonblank passage.
- Text normalization proposed for version 1: normalize Unicode to NFC; convert CRLF and CR to LF; encode as UTF-8 for a versioned hash. Preserve case, other whitespace and punctuation. Test canonically equivalent composed/decomposed text and CRLF/LF equivalence when implementing task 04. The algorithm is a proposal until frozen by tests, not a claim that a seed engine exists.
- Empty (`""`) and whitespace-only text are **invalid input**: show a prompt to enter prose and do not render, save a derived seed, or silently generate a default scene. A nonblank passage with extra whitespace remains a different input. The empty cases are policy test cases, **not** additional corpus passages.

When testing, record the passage ID and recipe/renderer versions, never infer image subjects from the text. Do not promise pixel-identical output across GPUs or backends.
