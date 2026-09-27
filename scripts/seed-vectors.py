#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Independent reference implementation of Pigment Prose seeding.

Regenerates fixtures/seed-vectors.json from fixtures/passages.json and the
extra normalization cases below, using only the Python standard library. The
Rust implementation in crates/pigment-core must reproduce the file exactly;
two implementations agreeing is the evidence that the algorithm is specified
by docs/seeds-and-recipes.md and not by one program's quirks.

    uv run scripts/seed-vectors.py            # rewrite the fixture
    uv run scripts/seed-vectors.py --check    # exit 1 if it would change

Python's unicodedata tables may be an older Unicode version than the Rust
crate's; every fixture character is assigned in both, so NFC agrees.
"""

import hashlib
import json
import sys
import unicodedata
from pathlib import Path

NORMALIZATION_ID = "nfc-lf-utf8/1"
SEED_ALGORITHM_ID = "pigment-seed/1"
TEXT_TAG = b"pigment-prose/text/v1\0"
STREAM_TAG = b"pigment-prose/stream/v1\0"
# (label, uses_variation), in Domain::ALL order.
DOMAINS = [
    ("composition", True),
    ("terrain", True),
    ("vegetation", True),
    ("paint-detail", False),
]
VARIATIONS = [0, 1, 4294967295]
RNG_SEEDS = [0, 1, 0x0123456789ABCDEF, 0xFFFFFFFFFFFFFFFF]
MASK = (1 << 64) - 1

ROOT = Path(__file__).resolve().parent.parent
PASSAGES = ROOT / "fixtures" / "passages.json"
OUT = ROOT / "fixtures" / "seed-vectors.json"

# Normalization cases. `same_as` names the vector whose digest must match.
EXTRA = [
    {"id": "crlf", "text": "The path bends.\r\nThe evening waits.", "same_as": "line-break"},
    {"id": "lone-cr", "text": "The path bends.\rThe evening waits.", "same_as": "line-break"},
    {"id": "decomposed", "text": "Cafe\u0301 lumie\u0300re pre\u0300s du lac.", "same_as": "accent"},
    {"id": "cr-crlf", "text": "Blue\r\r\ndusk."},
    {"id": "two-lf", "text": "Blue\n\ndusk.", "same_as": "cr-crlf"},
    {"id": "lower-case", "text": "blue dusk."},
    {"id": "trailing-newline", "text": "Blue dusk.\n"},
    {"id": "no-break-space", "text": "Blue\u00a0dusk."},
    {"id": "fullwidth", "text": "\uff22lue dusk."},
    {"id": "kelvin", "text": "\u212a", "same_as": "latin-k"},
    {"id": "latin-k", "text": "K"},
]


def normalize(text: str) -> str:
    # The input gate (empty, whitespace-only, oversized) is Rust's
    # text::check_source; the fixture's "rejected" list is checked there.
    text = unicodedata.normalize("NFC", text)
    return text.replace("\r\n", "\n").replace("\r", "\n")


def digest(text: str) -> bytes:
    return hashlib.sha256(TEXT_TAG + normalize(text).encode("utf-8")).digest()


def stream_seed(d: bytes, variation: int, label: str, uses_variation: bool) -> int:
    lab = label.encode("ascii")
    msg = STREAM_TAG + len(lab).to_bytes(4, "little") + lab + d
    if uses_variation:
        msg += variation.to_bytes(4, "little")
    return int.from_bytes(hashlib.sha256(msg).digest()[:8], "little")


def splitmix64(x: int):
    while True:
        x = (x + 0x9E3779B97F4A7C15) & MASK
        z = x
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        yield z ^ (z >> 31)


def rotl(x: int, k: int) -> int:
    return ((x << k) | (x >> (64 - k))) & MASK


def xoshiro(seed: int):
    sm = splitmix64(seed)
    s = [next(sm) for _ in range(4)]
    while True:
        result = (rotl((s[1] * 5) & MASK, 7) * 9) & MASK
        t = (s[1] << 17) & MASK
        s[2] ^= s[0]
        s[3] ^= s[1]
        s[1] ^= s[2]
        s[0] ^= s[3]
        s[2] ^= t
        s[3] = rotl(s[3], 45)
        yield result


def hex64(x: int) -> str:
    return f"{x:016x}"


def build() -> str:
    cases = json.loads(PASSAGES.read_text(encoding="utf-8")) + EXTRA
    vectors = []
    for case in cases:
        d = digest(case["text"])
        vec = {"id": case["id"], "text": case["text"]}
        if "same_as" in case:
            vec["same_as"] = case["same_as"]
        vec["normalized_utf8_bytes"] = len(normalize(case["text"]).encode("utf-8"))
        vec["digest"] = d.hex()
        vec["streams"] = [
            {
                "variation": v,
                **{lab: hex64(stream_seed(d, v, lab, uses)) for lab, uses in DOMAINS},
            }
            for v in VARIATIONS
        ]
        vectors.append(vec)
    by_id = {v["id"]: v for v in vectors}
    for v in vectors:
        if "same_as" in v:
            assert v["digest"] == by_id[v["same_as"]]["digest"], v["id"]
    rng = []
    for seed in RNG_SEEDS:
        gen = xoshiro(seed)
        rng.append({"seed": hex64(seed), "next_u64": [hex64(next(gen)) for _ in range(8)]})
    doc = {
        "normalization": NORMALIZATION_ID,
        "seed_algorithm": SEED_ALGORITHM_ID,
        "rejected": ["", " ", "\r\n", "\t\u3000"],
        "vectors": vectors,
        "rng": rng,
    }
    return json.dumps(doc, ensure_ascii=False, indent=2) + "\n"


def main() -> int:
    text = build()
    if "--check" in sys.argv[1:]:
        if OUT.read_text(encoding="utf-8") != text:
            print(f"{OUT.relative_to(ROOT)} is out of date", file=sys.stderr)
            return 1
        return 0
    OUT.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
