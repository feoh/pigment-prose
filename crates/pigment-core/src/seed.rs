//! Seed contract: one text digest plus a variation index fan out into
//! independent, domain-separated streams.
//!
//! Task 04 implements, in this file:
//!
//! ```ignore
//! impl TextDigest { pub fn of(text: &NormalizedText) -> TextDigest; }
//! impl SeedBundle { pub fn derive(digest: TextDigest, variation: Variation) -> SeedBundle; }
//! impl StreamSeed { pub fn rng(self) -> Rng; } // documented, platform-independent PRNG
//! ```
//!
//! Rules that task 04 must satisfy and test:
//! - Each stream seed is a function of `(algorithm id, domain label,
//!   digest, variation-if-applicable)` with unambiguous (length-prefixed)
//!   encoding. No stream is derived from another stream's state.
//! - [`Domain::uses_variation`] decides which streams "Another Composition"
//!   changes. Paint detail ignores variation, so the paper and pigment
//!   texture stays the same while the layout changes.
//! - No language default/randomized hasher; results must match across
//!   platforms and builds (tier 0 in docs/architecture.md).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 256-bit digest of the normalized text. Serialized as 64 lowercase hex
/// digits. A digest of guessable text can be brute-forced: it is not
/// encryption and not a privacy guarantee (task 10 documents this).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextDigest(pub [u8; 32]);

impl TextDigest {
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

/// Seed of one stream. Consumers build their own PRNG from it (task 04's
/// `StreamSeed::rng`) and never share mutable RNG state across domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamSeed(pub u64);

/// Everything random about a painting. Cheap to copy; contains no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeedBundle {
    pub digest: TextDigest,
    pub variation: Variation,
    streams: [StreamSeed; 4],
}

impl SeedBundle {
    /// Assemble a bundle from already-derived stream seeds. Task 04's
    /// `derive` is the only production caller; tests may use it directly.
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
