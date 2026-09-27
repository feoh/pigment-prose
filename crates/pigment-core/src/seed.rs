//! Seed contract: one text digest plus a variation index fan out into
//! independent, domain-separated streams.
//!
//! Algorithm `pigment-seed/1` (`version::SEED_ALGORITHM_ID`), frozen by
//! `fixtures/seed-vectors.json` and specified byte-for-byte in
//! `docs/seeds-and-recipes.md`:
//!
//! - **Digest:** `SHA-256("pigment-prose/text/v1\0" ‖ normalized UTF-8)`.
//! - **Stream seed:** the first 8 bytes, little-endian, of
//!   `SHA-256("pigment-prose/stream/v1\0" ‖ u32le(len(label)) ‖ label ‖
//!   digest ‖ u32le(variation))`, where the variation is appended only if
//!   [`Domain::uses_variation`]. Each stream depends only on the digest, its
//!   own label and the variation, never on another stream's state.
//! - **PRNG:** xoshiro256\*\* 1.0, its state filled by four SplitMix64
//!   outputs from the stream seed ([`Rng`]).
//!
//! No language default or randomized hasher is involved, so results match on
//! every platform and build (tier 0 in docs/architecture.md). Paint detail
//! ignores the variation, so "Another Composition" keeps the paper and
//! pigment texture while the layout changes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::error::TextError;
use crate::text::{self, NormalizedText};

/// Domain-separation prefix of the text digest (`pigment-seed/1`).
const TEXT_TAG: &[u8] = b"pigment-prose/text/v1\0";
/// Domain-separation prefix of every stream seed (`pigment-seed/1`).
const STREAM_TAG: &[u8] = b"pigment-prose/stream/v1\0";

/// 256-bit digest of the normalized text. Serialized as 64 lowercase hex
/// digits. A digest of guessable text can be brute-forced: it is not
/// encryption and not a privacy guarantee (task 10 documents this).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextDigest(pub [u8; 32]);

impl TextDigest {
    /// Digest of already-normalized text.
    pub fn of(text: &NormalizedText) -> TextDigest {
        let mut h = Sha256::new();
        h.update(TEXT_TAG);
        h.update(text.as_bytes());
        TextDigest(h.finalize().into())
    }

    /// Gate, normalize and digest source prose in one step.
    pub fn from_source(source: &str) -> Result<TextDigest, TextError> {
        Ok(TextDigest::of(&text::normalize(source)?))
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<TextDigest> {
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 32];
        for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            out[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Some(TextDigest(out))
    }
}

impl fmt::Debug for TextDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TextDigest({})", self.to_hex())
    }
}

impl Serialize for TextDigest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for TextDigest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        TextDigest::from_hex(&s)
            .ok_or_else(|| serde::de::Error::custom("digest must be 64 lowercase hex digits"))
    }
}

/// Composition variation index. `0` is the first composition for a text;
/// "Another Composition" increments it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Variation(pub u32);

/// Independent random domains. The label strings are part of the seed
/// algorithm and must never change for a given `SEED_ALGORITHM_ID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    /// Template choice, horizon, focal placement, framing.
    Composition,
    /// Ridge massing, fractal landform detail, shoreline, rock planes.
    Terrain,
    /// Woodland cluster placement and silhouettes.
    Vegetation,
    /// Mark jitter, granulation, paper grain, wash-edge irregularity.
    PaintDetail,
}

impl Domain {
    pub const ALL: [Domain; 4] = [
        Domain::Composition,
        Domain::Terrain,
        Domain::Vegetation,
        Domain::PaintDetail,
    ];

    /// Stable domain-separation label.
    pub fn label(self) -> &'static str {
        match self {
            Domain::Composition => "composition",
            Domain::Terrain => "terrain",
            Domain::Vegetation => "vegetation",
            Domain::PaintDetail => "paint-detail",
        }
    }

    /// Whether the composition variation index feeds this stream.
    pub fn uses_variation(self) -> bool {
        !matches!(self, Domain::PaintDetail)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Seed of one stream. CPU consumers build their own PRNG with
/// [`StreamSeed::rng`] and never share mutable RNG state across domains; GPU
/// passes hash lattice coordinates together with the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamSeed(pub u64);

impl StreamSeed {
    /// Derives the seed of `domain` (`pigment-seed/1`).
    pub fn derive(digest: TextDigest, variation: Variation, domain: Domain) -> StreamSeed {
        let label = domain.label().as_bytes();
        let mut h = Sha256::new();
        h.update(STREAM_TAG);
        h.update((label.len() as u32).to_le_bytes());
        h.update(label);
        h.update(digest.0);
        if domain.uses_variation() {
            h.update(variation.0.to_le_bytes());
        }
        let out: [u8; 32] = h.finalize().into();
        StreamSeed(u64::from_le_bytes(out[..8].try_into().expect("8 bytes")))
    }

    /// A fresh generator at the start of this stream. Every call returns an
    /// identical, independent generator.
    pub fn rng(self) -> Rng {
        Rng::new(self)
    }
}

/// xoshiro256\*\* 1.0 (Blackman and Vigna), seeded by SplitMix64. Integer
/// and exact IEEE operations only, so the sequence is identical on every
/// platform. Not cryptographic; it only has to be stable and well mixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    fn new(seed: StreamSeed) -> Rng {
        let mut x = seed.0;
        let mut splitmix = || {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        // Four successive SplitMix64 outputs are never all zero.
        Rng {
            s: [splitmix(), splitmix(), splitmix(), splitmix()],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// The upper 32 bits of the next output.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)` with 53 random bits. Exact.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// `lo + (hi - lo) * next_f64()`. Exact IEEE arithmetic, no fused ops.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Uniform integer in `0..n` without modulo bias (rejection sampling).
    /// Panics if `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "Rng::below(0)");
        // Reject the lowest (2^64 mod n) values so the rest divide evenly.
        let threshold = n.wrapping_neg() % n;
        loop {
            let x = self.next_u64();
            if x >= threshold {
                return x % n;
            }
        }
    }
}

/// Everything random about a painting. Cheap to copy; contains no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeedBundle {
    pub digest: TextDigest,
    pub variation: Variation,
    streams: [StreamSeed; 4],
}

impl SeedBundle {
    /// Every stream for one digest and variation (`pigment-seed/1`).
    pub fn derive(digest: TextDigest, variation: Variation) -> SeedBundle {
        SeedBundle::from_streams(
            digest,
            variation,
            Domain::ALL.map(|d| (d, StreamSeed::derive(digest, variation, d))),
        )
    }

    /// Assemble a bundle from already-derived stream seeds. [`derive`] is
    /// the only production caller; diagnostics and tests may use it directly.
    ///
    /// [`derive`]: SeedBundle::derive
    pub fn from_streams(
        digest: TextDigest,
        variation: Variation,
        streams: [(Domain, StreamSeed); 4],
    ) -> SeedBundle {
        let mut out = [StreamSeed(0); 4];
        let mut seen = [false; 4];
        for (d, s) in streams {
            assert!(!seen[d.index()], "domain {:?} given twice", d);
            seen[d.index()] = true;
            out[d.index()] = s;
        }
        SeedBundle {
            digest,
            variation,
            streams: out,
        }
    }

    pub fn stream(&self, domain: Domain) -> StreamSeed {
        self.streams[domain.index()]
    }

    /// The seeds that determine structure (everything except paint detail).
    pub fn structural(&self) -> [StreamSeed; 3] {
        [
            self.stream(Domain::Composition),
            self.stream(Domain::Terrain),
            self.stream(Domain::Vegetation),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XOSHIRO_1234: [u64; 6] = [
        11520,
        0,
        1509978240,
        1215971899390074240,
        1216172134540287360,
        607988272756665600,
    ];

    #[test]
    fn digest_hex_round_trips_and_rejects_bad_input() {
        let d = TextDigest(std::array::from_fn(|i| (i * 7) as u8));
        assert_eq!(TextDigest::from_hex(&d.to_hex()), Some(d));
        assert_eq!(TextDigest::from_hex("00"), None);
        assert_eq!(TextDigest::from_hex(&"G".repeat(64)), None);
        assert_eq!(
            TextDigest::from_hex(&"AB".repeat(32)),
            None,
            "uppercase is not canonical"
        );
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<TextDigest>(&json).unwrap(), d);
    }

    #[test]
    fn labels_are_distinct_and_only_paint_detail_ignores_variation() {
        let labels: std::collections::HashSet<_> = Domain::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels.len(), 4);
        assert!(!Domain::PaintDetail.uses_variation());
        assert!(Domain::Composition.uses_variation());
    }

    fn shore_a() -> SeedBundle {
        SeedBundle::derive(
            TextDigest::from_source("A pebble rests by the shore.").unwrap(),
            Variation(0),
        )
    }

    #[test]
    fn derivation_is_repeatable() {
        assert_eq!(shore_a(), shore_a());
        let d = TextDigest::from_source("Blue dusk.").unwrap();
        assert_eq!(d, TextDigest::of(&text::normalize("Blue dusk.").unwrap()));
    }

    #[test]
    fn streams_are_distinct() {
        let b = shore_a();
        let seeds: std::collections::HashSet<_> = Domain::ALL.map(|d| b.stream(d)).into();
        assert_eq!(seeds.len(), 4);
    }

    #[test]
    fn variation_changes_only_the_structural_streams() {
        let d = shore_a().digest;
        let v0 = SeedBundle::derive(d, Variation(0));
        for v in [1, 2, u32::MAX] {
            let vn = SeedBundle::derive(d, Variation(v));
            for domain in Domain::ALL {
                assert_eq!(
                    v0.stream(domain) == vn.stream(domain),
                    !domain.uses_variation(),
                    "{domain:?} at variation {v}"
                );
            }
        }
    }

    #[test]
    fn advancing_one_stream_leaves_the_others_alone() {
        let b = shore_a();
        let terrain = |n| -> Vec<u64> {
            let mut r = b.stream(Domain::Terrain).rng();
            (0..n).map(|_| r.next_u64()).collect()
        };
        let before = terrain(64);
        let mut paint = b.stream(Domain::PaintDetail).rng();
        let mut t = b.stream(Domain::Terrain).rng();
        let mut interleaved = Vec::new();
        for _ in 0..64 {
            for _ in 0..1000 {
                paint.next_u64();
            }
            interleaved.push(t.next_u64());
        }
        assert_eq!(interleaved, before);
        assert_eq!(terrain(64), before, "a fresh terrain rng restarts");
    }

    #[test]
    fn one_character_edit_gives_an_unrelated_digest() {
        // Policy, not a collision claim: no similarity is preserved.
        let a = TextDigest::from_source("A pebble rests by the shore.").unwrap();
        let b = TextDigest::from_source("A pebble rests by the shores.").unwrap();
        assert_ne!(a, b);
        assert_ne!(
            SeedBundle::derive(a, Variation(0)).structural(),
            SeedBundle::derive(b, Variation(0)).structural()
        );
    }

    #[test]
    fn rng_helpers_stay_in_range() {
        let mut r = StreamSeed(42).rng();
        for _ in 0..10_000 {
            let f = r.next_f64();
            assert!((0.0..1.0).contains(&f));
            let g = r.range_f64(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&g));
            assert!(r.below(7) < 7);
        }
        assert_eq!(r.below(1), 0);
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            counts[r.below(3) as usize] += 1;
        }
        assert!(
            counts.iter().all(|&c| (9_000..11_000).contains(&c)),
            "{counts:?}"
        );
    }

    /// Frozen tier-0 vectors, generated by the independent reference
    /// implementation `scripts/seed-vectors.py`.
    const VECTORS: &str = include_str!("../../../fixtures/seed-vectors.json");

    fn hex_u64(v: &serde_json::Value) -> u64 {
        u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
    }

    #[test]
    fn frozen_vectors_reproduce_exactly() {
        let doc: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
        assert_eq!(doc["normalization"], crate::version::NORMALIZATION_ID);
        assert_eq!(doc["seed_algorithm"], crate::version::SEED_ALGORITHM_ID);
        let vectors = doc["vectors"].as_array().unwrap();
        assert!(vectors.len() >= 20);
        for v in vectors {
            let id = v["id"].as_str().unwrap();
            let normalized = text::normalize(v["text"].as_str().unwrap()).unwrap();
            assert_eq!(
                normalized.as_bytes().len() as u64,
                v["normalized_utf8_bytes"].as_u64().unwrap(),
                "{id}"
            );
            let digest = TextDigest::of(&normalized);
            assert_eq!(digest.to_hex(), v["digest"].as_str().unwrap(), "{id}");
            for s in v["streams"].as_array().unwrap() {
                let variation = Variation(s["variation"].as_u64().unwrap() as u32);
                let bundle = SeedBundle::derive(digest, variation);
                for d in Domain::ALL {
                    assert_eq!(bundle.stream(d).0, hex_u64(&s[d.label()]), "{id} {d:?}");
                }
            }
        }
        for r in doc["rng"].as_array().unwrap() {
            let mut rng = StreamSeed(hex_u64(&r["seed"])).rng();
            for want in r["next_u64"].as_array().unwrap() {
                assert_eq!(rng.next_u64(), hex_u64(want));
            }
        }
        for bad in doc["rejected"].as_array().unwrap() {
            assert!(TextDigest::from_source(bad.as_str().unwrap()).is_err());
        }
    }

    #[test]
    fn declared_equivalences_hold() {
        let doc: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
        let vectors = doc["vectors"].as_array().unwrap();
        let digest_of = |id: &str| {
            let v = vectors.iter().find(|v| v["id"] == id).unwrap();
            TextDigest::from_source(v["text"].as_str().unwrap()).unwrap()
        };
        let mut pairs = 0;
        for v in vectors {
            if let Some(other) = v["same_as"].as_str() {
                assert_eq!(digest_of(v["id"].as_str().unwrap()), digest_of(other));
                pairs += 1;
            }
        }
        assert!(pairs >= 5);
        // Every fixture digest is otherwise distinct.
        let unique: std::collections::HashSet<_> = vectors
            .iter()
            .map(|v| v["digest"].as_str().unwrap())
            .collect();
        assert_eq!(unique.len(), vectors.len() - pairs);
    }

    #[test]
    fn corpus_passages_are_all_in_the_vectors() {
        let passages: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/passages.json")).unwrap();
        let vectors: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
        for p in passages.as_array().unwrap() {
            assert!(
                vectors["vectors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["id"] == p["id"] && v["text"] == p["text"]),
                "{}",
                p["id"]
            );
        }
    }

    #[test]
    fn rng_matches_the_xoshiro_reference() {
        // State fixed to {1, 2, 3, 4} as in the C reference implementation
        // (prng.di.unimi.it/xoshiro256starstar.c); values computed from it.
        let mut r = Rng { s: [1, 2, 3, 4] };
        let got: Vec<u64> = (0..6).map(|_| r.next_u64()).collect();
        assert_eq!(got, XOSHIRO_1234);
    }

    #[test]
    fn from_streams_places_each_domain() {
        let b = SeedBundle::from_streams(
            TextDigest([0; 32]),
            Variation(3),
            [
                (Domain::PaintDetail, StreamSeed(4)),
                (Domain::Composition, StreamSeed(1)),
                (Domain::Vegetation, StreamSeed(3)),
                (Domain::Terrain, StreamSeed(2)),
            ],
        );
        assert_eq!(
            b.structural(),
            [StreamSeed(1), StreamSeed(2), StreamSeed(3)]
        );
        assert_eq!(b.stream(Domain::PaintDetail), StreamSeed(4));
    }
}
