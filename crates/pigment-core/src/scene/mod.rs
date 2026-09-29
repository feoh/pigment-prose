//! Immutable structural scene.
//!
//! A `Scene` is a pure function of `(generator version, biome, structural seeds,
//! FormSettings, AspectRatio)`: the [`SceneKey`]. It never depends on paint
//! handling, palette, atmosphere, pixel size, tile layout or device. It is
//! built once on the CPU, validated, wrapped in `Arc` and shared read-only by
//! preview and export.
//!
//! Generation runs in `f64` and quantizes to `f32` at construction. The
//! stored `f32` values are exactly what the GPU receives, and
//! [`Scene::geometry_checksum`] hashes them. Generators must use only
//! IEEE-exact operations (`+ - * /`, `sqrt`, comparisons) or a pure-Rust
//! deterministic implementation of transcendental functions (e.g. the `libm`
//! crate), never platform `f64::sin`/`exp`, whose results differ between
//! operating systems (tier 1 in docs/architecture.md).
//!
//! The product generator is [`lakeshore::LakeshoreGenerator`] (task 05,
//! documented in `docs/scene-generation.md`). [`raster`] is a CPU reference
//! rasterizer and topology checker for tests and diagnostics.

pub mod desert;
pub mod jungle;
pub mod lakeshore;
pub mod metrics;
mod noise;
pub mod raster;
pub mod tundra;

use crate::biome::BiomeId;
use crate::error::{Problem, ValidationError};
use crate::frame::{AspectRatio, CanvasExtents};
use crate::seed::{Domain, SeedBundle, StreamSeed};
use crate::settings::FormSettings;

pub const MAX_LAYERS: usize = 96;
pub const MAX_LAYER_VERTICES: usize = 4096;
/// Bound on the sum of all layer vertices (GPU upload and per-pixel cost).
pub const MAX_SCENE_VERTICES: usize = 32768;

/// Reusable low-shrub silhouette for open landscapes. `phase` is a stable
/// seed draw, not an animated angle, so both profile and tile geometry repeat.
pub(crate) fn low_shrub_outline(x: f64, y: f64, radius: f64, phase: f64) -> Vec<CanvasPoint> {
    let profile = [0.55, 0.70, 0.88, 1.0, 0.78, 0.92, 0.72, 0.61, 0.55];
    profile
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let t = i as f64 / (profile.len() - 1) as f64;
            let wobble = if i == 0 || i + 1 == profile.len() {
                1.0
            } else {
                0.96 + 0.04 * phase
            };
            CanvasPoint {
                x: (x + (2.0 * t - 1.0) * radius) as f32,
                y: (y - radius * r * wobble) as f32,
            }
        })
        .collect()
}

/// Inputs that fully determine a scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneKey {
    pub generator_version: u32,
    pub biome: BiomeId,
    pub structural_seeds: [StreamSeed; 3],
    pub form: FormSettings,
    pub aspect: AspectRatio,
}

impl SceneKey {
    pub fn new(
        generator_version: u32,
        seeds: &SeedBundle,
        biome: BiomeId,
        form: FormSettings,
        aspect: AspectRatio,
    ) -> SceneKey {
        SceneKey {
            generator_version,
            biome,
            structural_seeds: seeds.structural(),
            form,
            aspect,
        }
    }
}

/// Semantic role of a layer. Later passes (vegetation, water paint) select
/// regions by role, never by inspecting painted colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LayerRole {
    Sky = 0,
    /// Distant range behind the main mountain.
    FarRidge = 1,
    /// Foothills along the far shore.
    MidRidge = 2,
    /// Ridges that frame a valley from the sides and come down to the lake.
    NearRidge = 3,
    Water = 4,
    /// Far beach strip and the near-shore land.
    Shore = 5,
    /// Placement region for trees (task 07 fills it; debug views show a mass).
    Woodland = 6,
    ForegroundRock = 7,
    /// The main mountain massif and its facet planes.
    Mountain = 8,
    /// Cloud masses in the sky (behind all terrain).
    Cloud = 9,
    /// Layered, exposed mesa forms without alpine snow or forest response.
    Mesa = 10,
    /// Open tundra ground; paint it as low cover rather than forest.
    TundraGround = 11,
    /// Humid jungle understory and seasonal clearings.
    JungleGround = 12,
    /// Dense, species-varied tropical canopy in shared paint space.
    JungleCanopy = 13,
}

impl LayerRole {
    pub const ALL: [LayerRole; 14] = [
        LayerRole::Sky,
        LayerRole::FarRidge,
        LayerRole::MidRidge,
        LayerRole::NearRidge,
        LayerRole::Water,
        LayerRole::Shore,
        LayerRole::Woodland,
        LayerRole::ForegroundRock,
        LayerRole::Mountain,
        LayerRole::Cloud,
        LayerRole::Mesa,
        LayerRole::TundraGround,
        LayerRole::JungleGround,
        LayerRole::JungleCanopy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LayerRole::Sky => "sky",
            LayerRole::FarRidge => "far-ridge",
            LayerRole::MidRidge => "mid-ridge",
            LayerRole::NearRidge => "near-ridge",
            LayerRole::Water => "water",
            LayerRole::Shore => "shore",
            LayerRole::Woodland => "woodland",
            LayerRole::ForegroundRock => "foreground-rock",
            LayerRole::Mountain => "mountain",
            LayerRole::Cloud => "cloud",
            LayerRole::Mesa => "mesa",
            LayerRole::TundraGround => "tundra-ground",
            LayerRole::JungleGround => "jungle-ground",
            LayerRole::JungleCanopy => "jungle-canopy",
        }
    }
}

/// What grows in a woodland layer. Chosen per stand from the vegetation
/// stream; it shapes the stand's silhouette (in the generator) and its
/// crowns and colors (in the painting). `None` for every other layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Plant {
    #[default]
    None = 0,
    /// Tall, leafy, rounded crowns.
    Broadleaf = 1,
    /// Tall, narrow, dark spires.
    Conifer = 2,
    /// Light, airy crowns on pale trunks.
    Birch = 3,
    /// Squat, woody, low and wide.
    Shrub = 4,
    /// Blossoming trees.
    Flowering = 5,
    /// Red-purple summer foliage (copper beech).
    Copper = 6,
}

impl Plant {
    pub const TREES: [Plant; 6] = [
        Plant::Broadleaf,
        Plant::Conifer,
        Plant::Birch,
        Plant::Shrub,
        Plant::Flowering,
        Plant::Copper,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Plant::None => "none",
            Plant::Broadleaf => "broadleaf",
            Plant::Conifer => "conifer",
            Plant::Birch => "birch",
            Plant::Shrub => "shrub",
            Plant::Flowering => "flowering",
            Plant::Copper => "copper",
        }
    }

    /// Picks a plant from `u` in `[0, 1)` by natural-looking weights.
    pub fn pick(u: f64) -> Plant {
        const W: [(Plant, f64); 6] = [
            (Plant::Broadleaf, 0.26),
            (Plant::Conifer, 0.22),
            (Plant::Birch, 0.14),
            (Plant::Shrub, 0.18),
            (Plant::Flowering, 0.12),
            (Plant::Copper, 0.08),
        ];
        let mut acc = 0.0;
        for (p, w) in W {
            acc += w;
            if u < acc {
                return p;
            }
        }
        Plant::Broadleaf
    }
}

/// The side of the sky the scene's light comes from. Planes' `shade` is
/// computed from it; painting uses it to light tree crowns and cast stand
/// shadows the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum LightSide {
    #[default]
    Left = 0,
    Right = 1,
}

/// The wind over the water (task 25): a scene attribute drawn from the
/// composition stream, so paint settings never change it and a new
/// composition may bring a different breeze. `(x, z)` is the unit direction
/// the wind blows toward on the water plane: `x` across the image (+ right),
/// `z` into the distance (+ away from the viewer). `strength` runs from 0
/// (glassy calm) to 1 (a fresh breeze with whitecaps' texture, not waves).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wind {
    pub x: f32,
    pub z: f32,
    pub strength: f32,
}

impl Default for Wind {
    /// Calm, blowing across the image.
    fn default() -> Wind {
        Wind {
            x: 1.0,
            z: 0.0,
            strength: 0.0,
        }
    }
}

/// A point in canvas units (short side = 1, +y down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPoint {
    pub x: f32,
    pub y: f32,
}

/// One depth plane: a closed simple polygon in canvas units. It may extend
/// beyond the frame (the frame is a window onto an unbounded canvas).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLayer {
    pub role: LayerRole,
    /// 0 = nearest, 1 = farthest. Drives atmospheric perspective.
    pub depth: f32,
    /// Structural illumination, 0–1: how directly this plane faces the
    /// scene's light, from its geometry (0 = turned away, 0.5 = neutral or
    /// not a facet, 1 = facing the light). Painting maps it to value and
    /// temperature; it is not a color.
    pub shade: f32,
    /// What grows here (woodland layers only).
    pub plant: Plant,
    pub outline: Vec<CanvasPoint>,
}

/// Validated, immutable scene. Share as `Arc<Scene>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    key: SceneKey,
    extents: CanvasExtents,
    /// Back to front: depth never increases along the list.
    layers: Vec<SceneLayer>,
    light: LightSide,
    wind: Wind,
}

impl Scene {
    pub fn new(key: SceneKey, layers: Vec<SceneLayer>) -> Result<Scene, ValidationError> {
        let err = |field, problem| Err(ValidationError { field, problem });
        if layers.len() > MAX_LAYERS {
            return err(
                "scene.layers",
                Problem::TooLarge {
                    value: layers.len() as u64,
                    max: MAX_LAYERS as u64,
                },
            );
        }
        let total: usize = layers.iter().map(|l| l.outline.len()).sum();
        if total > MAX_SCENE_VERTICES {
            return err(
                "scene.vertices",
                Problem::TooLarge {
                    value: total as u64,
                    max: MAX_SCENE_VERTICES as u64,
                },
            );
        }
        let mut prev_depth = f32::INFINITY;
        for l in &layers {
            if !(0.0..=1.0).contains(&l.shade) {
                return err(
                    "scene.layer.shade",
                    Problem::OutOfRange {
                        value: l.shade.into(),
                        min: 0.0,
                        max: 1.0,
                    },
                );
            }
            if !(0.0..=1.0).contains(&l.depth) {
                return err(
                    "scene.layer.depth",
                    Problem::OutOfRange {
                        value: l.depth.into(),
                        min: 0.0,
                        max: 1.0,
                    },
                );
            }
            if l.depth > prev_depth {
                return err(
                    "scene.layer.depth",
                    Problem::OutOfRange {
                        value: l.depth.into(),
                        min: 0.0,
                        max: prev_depth.into(),
                    },
                );
            }
            prev_depth = l.depth;
            if l.outline.len() < 3 {
                return err(
                    "scene.layer.outline",
                    Problem::TooSmall {
                        value: l.outline.len() as u64,
                        min: 3,
                    },
                );
            }
            if l.outline.len() > MAX_LAYER_VERTICES {
                return err(
                    "scene.layer.outline",
                    Problem::TooLarge {
                        value: l.outline.len() as u64,
                        max: MAX_LAYER_VERTICES as u64,
                    },
                );
            }
            if l.outline
                .iter()
                .any(|p| !(p.x.is_finite() && p.y.is_finite()))
            {
                return err("scene.layer.outline", Problem::NotFinite);
            }
        }
        Ok(Scene {
            key,
            extents: key.aspect.extents(),
            layers,
            light: LightSide::default(),
            wind: Wind::default(),
        })
    }

    /// The same scene lit from `light` (the default is [`LightSide::Left`]).
    pub fn with_light(mut self, light: LightSide) -> Scene {
        self.light = light;
        self
    }

    pub fn light(&self) -> LightSide {
        self.light
    }

    /// The same scene with `wind` over its water (the default is calm).
    pub fn with_wind(mut self, wind: Wind) -> Scene {
        self.wind = wind;
        self
    }

    pub fn wind(&self) -> Wind {
        self.wind
    }

    pub fn key(&self) -> &SceneKey {
        &self.key
    }

    pub fn extents(&self) -> CanvasExtents {
        self.extents
    }

    pub fn layers(&self) -> &[SceneLayer] {
        &self.layers
    }

    /// FNV-1a 64 over the canonical little-endian encoding of everything the
    /// GPU receives (aspect, light side, wind bits, then per layer: role,
    /// plant, depth bits, shade bits, vertex count, vertex bits). A
    /// regression checksum, not a security hash.
    pub fn geometry_checksum(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.u32(self.key.aspect.width);
        h.u32(self.key.aspect.height);
        h.bytes(&[self.light as u8]);
        h.u32(self.wind.x.to_bits());
        h.u32(self.wind.z.to_bits());
        h.u32(self.wind.strength.to_bits());
        h.u32(self.layers.len() as u32);
        for l in &self.layers {
            h.bytes(&[l.role as u8, l.plant as u8]);
            h.u32(l.depth.to_bits());
            h.u32(l.shade.to_bits());
            h.u32(l.outline.len() as u32);
            for p in &l.outline {
                h.u32(p.x.to_bits());
                h.u32(p.y.to_bits());
            }
        }
        h.finish()
    }
}

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// Builds scenes. Implementations must be deterministic and must read only
/// the structural streams, form settings and aspect ratio.
pub trait SceneGenerator: Send + Sync {
    fn version(&self) -> u32;
    fn biome(&self) -> BiomeId {
        BiomeId::Alpine
    }
    fn generate(
        &self,
        seeds: &SeedBundle,
        form: &FormSettings,
        aspect: AspectRatio,
    ) -> Result<Scene, ValidationError>;
}

/// The single generator dispatch used by previews, exports, and tools.
pub fn generator(biome: BiomeId) -> &'static dyn SceneGenerator {
    match biome {
        BiomeId::Alpine => &lakeshore::LakeshoreGenerator,
        BiomeId::Desert => &desert::DesertGenerator,
        BiomeId::Tundra => &tundra::TundraGenerator,
        BiomeId::Jungle => &jungle::JungleGenerator,
    }
}

/// Diagnostic scene for the GPU smoke path and tests: sky plus three angular
/// ridge bands. **Not the product generator** ([`lakeshore`]) and not an art
/// candidate. Uses its own SplitMix64 on the terrain stream and only exact
/// arithmetic, so its checksum is identical on every platform.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestCard;

impl TestCard {
    /// 1: layers gained `shade` (always 0.5 here); 2: and `plant` (always
    /// `None`); 3: the checksum covers the scene's light side (always
    /// left here); 4: and its wind (always calm here).
    pub const VERSION: u32 = 4;
}

impl SceneGenerator for TestCard {
    fn version(&self) -> u32 {
        Self::VERSION
    }

    fn generate(
        &self,
        seeds: &SeedBundle,
        form: &FormSettings,
        aspect: AspectRatio,
    ) -> Result<Scene, ValidationError> {
        form.validate()?;
        let ext = aspect.extents();
        let (w, h) = (ext.width, ext.height);
        let mut rng = SplitMix64(seeds.stream(Domain::Terrain).0);
        let pt = |x: f64, y: f64| CanvasPoint {
            x: x as f32,
            y: y as f32,
        };
        let mut layers = vec![SceneLayer {
            role: LayerRole::Sky,
            depth: 1.0,
            shade: 0.5,
            plant: Plant::None,
            outline: vec![
                pt(-0.1, -0.1),
                pt(w + 0.1, -0.1),
                pt(w + 0.1, h + 0.1),
                pt(-0.1, h + 0.1),
            ],
        }];
        let bands = [
            (LayerRole::FarRidge, 0.8f32, 0.40),
            (LayerRole::MidRidge, 0.5, 0.55),
            (LayerRole::NearRidge, 0.2, 0.75),
        ];
        const N: usize = 24;
        for (role, depth, base) in bands {
            let amp = 0.08 + 0.14 * form.relief;
            let mut outline = Vec::with_capacity(N + 2);
            for k in 0..N {
                let f = k as f64 / (N - 1) as f64;
                // Faceting blends a jagged zigzag with a smoother profile.
                let zig = if k % 2 == 0 { 1.0 } else { 0.35 };
                let jag = form.faceting * zig + (1.0 - form.faceting) * 0.65;
                let y = h * (base - amp * jag * (0.4 + 0.6 * rng.unit()));
                outline.push(pt(-0.1 + (w + 0.2) * f, y));
            }
            outline.push(pt(w + 0.1, h + 0.1));
            outline.push(pt(-0.1, h + 0.1));
            layers.push(SceneLayer {
                role,
                depth,
                shade: 0.5,
                plant: Plant::None,
                outline,
            });
        }
        Scene::new(
            SceneKey::new(self.version(), seeds, self.biome(), *form, aspect),
            layers,
        )
    }
}

/// Test-card PRNG only, frozen with its checksum. Product generators use
/// `StreamSeed::rng`.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

/// A bundle for diagnostics and tests, derived from a number rather than
/// text. Never used for user paintings.
pub fn diagnostic_seeds(seed: u64) -> SeedBundle {
    use crate::seed::{TextDigest, Variation};
    SeedBundle::from_streams(
        TextDigest([0; 32]),
        Variation(0),
        [
            (Domain::Composition, StreamSeed(seed)),
            (Domain::Terrain, StreamSeed(seed ^ 0x7465_7272_6169_6e00)),
            (Domain::Vegetation, StreamSeed(seed ^ 0x7665_6765_7461_7400)),
            (
                Domain::PaintDetail,
                StreamSeed(seed ^ 0x7061_696e_7400_0000),
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{UHD_4K, UHD_8K};

    fn card(seed: u64, form: FormSettings, aspect: AspectRatio) -> Scene {
        TestCard
            .generate(&diagnostic_seeds(seed), &form, aspect)
            .unwrap()
    }

    #[test]
    fn same_key_same_checksum() {
        let f = FormSettings::default();
        let a = card(7, f, UHD_4K.aspect());
        let b = card(7, f, UHD_4K.aspect());
        assert_eq!(a, b);
        assert_eq!(a.geometry_checksum(), b.geometry_checksum());
    }

    #[test]
    fn pixel_size_does_not_change_the_scene_but_aspect_does() {
        let f = FormSettings::default();
        let a = card(7, f, UHD_4K.aspect());
        assert_eq!(
            a.geometry_checksum(),
            card(7, f, UHD_8K.aspect()).geometry_checksum()
        );
        assert_ne!(
            a.geometry_checksum(),
            card(7, f, UHD_4K.rotated().aspect()).geometry_checksum()
        );
    }

    #[test]
    fn structure_inputs_change_geometry() {
        let f = FormSettings::default();
        let base = card(7, f, UHD_4K.aspect()).geometry_checksum();
        assert_ne!(base, card(8, f, UHD_4K.aspect()).geometry_checksum());
        let faceted = FormSettings { faceting: 1.0, ..f };
        assert_ne!(base, card(7, faceted, UHD_4K.aspect()).geometry_checksum());
    }

    #[test]
    fn paint_detail_stream_does_not_affect_geometry() {
        let f = FormSettings::default();
        let a = diagnostic_seeds(7);
        let b = SeedBundle::from_streams(
            a.digest,
            a.variation,
            [
                (Domain::Composition, a.stream(Domain::Composition)),
                (Domain::Terrain, a.stream(Domain::Terrain)),
                (Domain::Vegetation, a.stream(Domain::Vegetation)),
                (Domain::PaintDetail, StreamSeed(12345)),
            ],
        );
        let sa = TestCard.generate(&a, &f, UHD_4K.aspect()).unwrap();
        let sb = TestCard.generate(&b, &f, UHD_4K.aspect()).unwrap();
        assert_eq!(sa.geometry_checksum(), sb.geometry_checksum());
    }

    #[test]
    fn test_card_checksum_is_frozen() {
        // Exact arithmetic only, so this holds on every OS and CPU. If it
        // changes, TestCard::VERSION must change too.
        let s = card(7, FormSettings::default(), UHD_4K.aspect());
        assert_eq!(s.geometry_checksum(), TEST_CARD_SEED7_4K_CHECKSUM);
    }

    // Captured on Linux x86_64 for TestCard v3 (v0 0x54e3_8098_64ea_446c had
    // no `shade`, v1 0x2257_2651_a5cc_f378 no `plant`, v2
    // 0x52b0_6c7e_571a_a4b4 no light side); portable CI re-checks it on
    // Windows and macOS.
    const TEST_CARD_SEED7_4K_CHECKSUM: u64 = 0x11a3_019b_5cdd_f4a1;

    #[test]
    fn invalid_scenes_are_rejected() {
        let key = card(7, FormSettings::default(), UHD_4K.aspect()).key;
        let tri = |depth: f32, x: f32| SceneLayer {
            role: LayerRole::FarRidge,
            depth,
            shade: 0.5,
            plant: Plant::None,
            outline: vec![
                CanvasPoint { x, y: 0.0 },
                CanvasPoint { x: 1.0, y: 0.0 },
                CanvasPoint { x: 0.0, y: 1.0 },
            ],
        };
        assert!(Scene::new(key, vec![tri(0.5, 0.0)]).is_ok());
        assert_eq!(
            Scene::new(key, vec![tri(1.5, 0.0)]).unwrap_err().field,
            "scene.layer.depth"
        );
        assert!(
            Scene::new(key, vec![tri(0.2, 0.0), tri(0.8, 0.0)]).is_err(),
            "front-to-back"
        );
        assert_eq!(
            Scene::new(key, vec![tri(0.5, f32::NAN)])
                .unwrap_err()
                .problem,
            Problem::NotFinite
        );
        let mut two = tri(0.5, 0.0);
        two.outline.pop();
        assert!(Scene::new(key, vec![two]).is_err());
        let mut dark = tri(0.5, 0.0);
        dark.shade = -0.1;
        assert_eq!(
            Scene::new(key, vec![dark]).unwrap_err().field,
            "scene.layer.shade"
        );
    }
}
