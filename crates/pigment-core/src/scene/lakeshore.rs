//! The first product scene: a rocky wooded lakeshore below a mountain ridge.
//!
//! Specification: `docs/scene-generation.md`. In brief:
//!
//! - **Streams.** The composition stream picks the template, mirroring,
//!   light side, horizon and the placement of every major form. The terrain
//!   stream seeds the fractal detail, shorelines and rocks. The vegetation
//!   stream places woodland regions. Form settings never change which values
//!   are drawn, so moving a slider morphs one composition instead of
//!   reshuffling it. The paint-detail stream is never read.
//! - **Topology.** Every layer is an *x-monotone band*: a top chain over a
//!   bottom chain sampled on one grid, strictly apart between its ends. Such
//!   a polygon is simple by construction, and planes laid over a mass reuse
//!   the mass's own samples, so no slivers open between them.
//! - **Exactness.** Only `+ − × ÷`, `sqrt`, `floor`, comparisons and integer
//!   hashing, in `f64`, then one rounding to `f32`. No platform
//!   transcendental functions, so geometry is identical on every OS (tier 1).

use super::noise::{Fbm, unit};
use super::{CanvasPoint, LayerRole, Plant, Scene, SceneGenerator, SceneKey, SceneLayer};
use crate::error::ValidationError;
use crate::frame::AspectRatio;
use crate::seed::{Domain, Rng, SeedBundle};
use crate::settings::FormSettings;
use crate::version;

/// Horizontal vertex spacing of silhouettes and shorelines, canvas units.
pub const PROFILE_STEP: f64 = 1.0 / 320.0;
/// Finest detail wavelength on any silhouette, canvas units: about three
/// pixels at the 960 px interaction preview, so outlines stay readable.
pub const MIN_WAVELENGTH: f64 = 1.0 / 160.0;
/// Every full-width layer extends this far past the frame on all sides.
pub const MARGIN: f64 = 0.1;
/// Vertex spacing of cloud outlines (soft masses, billows ≥ 0.1 across).
pub const CLOUD_STEP: f64 = 1.0 / 120.0;
/// Highest primary summit above the horizon, canvas units (short side = 1).
pub const MAX_SUMMIT: f64 = 0.75;

/// Highest summit of the `TowerPeak` template, canvas units.
pub const MAX_TOWER: f64 = 1.3;

pub const MAX_ROCKS: usize = 6;
pub const MAX_CUMULUS: usize = 3;
pub const MAX_SPURS: usize = 7;
pub const MAX_FAR_WOODS: usize = 6;
pub const MAX_NEAR_WOODS: usize = 5;
pub const MAX_MOUNTAIN_PLANES: usize = 24;

/// Layer depths, back to front.
pub mod depth {
    pub const SKY: f32 = 1.0;
    pub const CLOUDS: f32 = 0.98;
    pub const FAR_RANGE: f32 = 0.95;
    pub const MOUNTAIN: f32 = 0.85;
    pub const FOOTHILLS: f32 = 0.66;
    pub const FAR_WOODS: f32 = 0.6;
    pub const FRAMING: f32 = 0.55;
    pub const BEACH: f32 = 0.52;
    pub const WATER: f32 = 0.45;
    pub const NEAR_SHORE: f32 = 0.25;
    pub const NEAR_WOODS: f32 = 0.2;
    /// Rocks run from this (farthest) toward [`ROCK_NEAREST`].
    pub const ROCK_FARTHEST: f32 = 0.15;
    pub const ROCK_NEAREST: f32 = 0.03;
    /// Valley spurs (`HighVantage`) run from this toward [`SPUR_NEAREST`].
    pub const SPUR_FARTHEST: f32 = 0.44;
    pub const SPUR_NEAREST: f32 = 0.14;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Template {
    /// One dominant off-centre peak over open water; the near shore and its
    /// rocks sit in the opposite lower corner.
    PeakOverWater,
    /// Ridges come down to the lake from both sides and frame a distant peak
    /// in the valley; a low near shore with a bay runs across the bottom.
    FramingRidges,
    /// A long ridge with two summits of similar height.
    TwinSummits,
    /// "High distance": a low horizon and one steep giant summit rising
    /// over a tiny forested shore, so the trees measure the mountain.
    TowerPeak,
    /// "Level distance" from above: a high horizon, ridge spurs stepping
    /// down from both sides with the lake winding between them, ending on
    /// the valley's nearest spur.
    HighVantage,
}

impl Template {
    pub const ALL: [Template; 5] = [
        Template::PeakOverWater,
        Template::FramingRidges,
        Template::TwinSummits,
        Template::TowerPeak,
        Template::HighVantage,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Template::PeakOverWater => "peak-over-water",
            Template::FramingRidges => "framing-ridges",
            Template::TwinSummits => "twin-summits",
            Template::TowerPeak => "tower-peak",
            Template::HighVantage => "high-vantage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShoreKind {
    /// The shoreline enters from one side and leaves through the bottom.
    Corner { right: bool },
    /// The shoreline crosses the whole width with a central bay.
    Bay,
    /// No near shore (`HighVantage`): the valley runs to the frame's edge,
    /// ending on its nearest spur.
    Valley,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cumulus {
    /// Canvas x of the centre and full width.
    pub x: f64,
    pub width: f64,
    /// Canvas y of the flat base, and billow height above it.
    pub base: f64,
    pub height: f64,
    /// Billow wavelength, canvas units.
    pub billow: f64,
}

/// A storm deck across the top of the frame with a break in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Deck {
    /// Canvas y of the deck's lower edge.
    pub base: f64,
    /// Centre and width of the break, canvas units.
    pub gap_x: f64,
    pub gap_width: f64,
    /// How far the lower edge lifts inside the break, canvas units.
    pub lift: f64,
}

/// Where the light falls: a sun break through the clouds. Layers inside the
/// pool are lifted in `shade`, the rest sink into cloud shadow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightPool {
    /// Centre on the primary summit (true) or at `(x, y)` in the valley.
    pub on_summit: bool,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// 0 = even light, 1 = strongest contrast.
    pub strength: f64,
}

/// One valley spur of `HighVantage`, far to near.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spur {
    pub from_right: bool,
    /// 0 (at the horizon) to 1 (at the frame bottom): how near it is.
    pub nearness: f64,
    /// How far across the width its tip reaches, fraction of the width.
    pub reach: f64,
    pub height: f64,
    pub thickness: f64,
}

/// One summit of the main massif. Heights are relative to the primary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hump {
    /// Canvas x of the summit.
    pub x: f64,
    /// Height relative to the primary summit (primary = 1).
    pub rel: f64,
    /// Multipliers on the left and right half-widths (asymmetry).
    pub left: f64,
    pub right: f64,
    /// 0 = straight flanks, up to 0.6 = concave flanks (steep summit,
    /// spreading foot).
    pub concave: f64,
}

/// Every decision taken from the composition stream. A pure function of
/// the composition seed and the aspect ratio; form settings are applied
/// afterwards, so they never change it.
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    pub template: Template,
    pub mirrored: bool,
    pub light_from_left: bool,
    /// Canvas y of the far shore. Everything above it is "sky space".
    pub horizon: f64,
    /// 0–1 factor on the primary summit's height.
    pub peak_scale: f64,
    /// Primary summit first.
    pub humps: Vec<Hump>,
    pub shore: ShoreKind,
    /// Corner: edge height (fraction of foreground depth), reach (fraction
    /// of width), curvature. Bay: shoreline depth, bay centre, bay width,
    /// bay depth.
    pub shore_params: [f64; 4],
    /// Framing ridges: left height (fraction of sky), left foot (fraction of
    /// width), right height, right foot. Drawn for every template, used by
    /// `FramingRidges` only.
    pub framing: [f64; 4],
    /// How far the lake's edge bends forward under the framing ridges.
    pub framing_bend: f64,
    /// Where along the visible near shore the rocks cluster, 0–1.
    pub rock_cluster: f64,
    /// Sky structure. `dramatic` scenes get a storm deck more often and a
    /// stronger light pool.
    pub dramatic: bool,
    pub cumulus: Vec<Cumulus>,
    pub deck: Option<Deck>,
    pub light: LightPool,
    /// `HighVantage` meander: centre, amplitude (fractions of the width),
    /// bends over the depth of the view, phase. Zero otherwise.
    pub river: [f64; 4],
    /// `HighVantage` only (empty otherwise).
    pub spurs: Vec<Spur>,
}

impl Composition {
    pub fn draw(seeds: &SeedBundle, aspect: AspectRatio) -> Composition {
        let ext = aspect.extents();
        let (w, h) = (ext.width, ext.height);
        let mut r = seeds.stream(Domain::Composition).rng();
        // The vista templates (tower and high vantage) take half the draws.
        let template = match r.below(100) {
            0..25 => Template::TowerPeak,
            25..50 => Template::HighVantage,
            50..67 => Template::PeakOverWater,
            67..84 => Template::TwinSummits,
            _ => Template::FramingRidges,
        };
        let mirrored = r.below(2) == 1;
        let light_from_left = r.below(2) == 1;
        // Wider frames get a higher horizon share (less sky); portrait and
        // square frames keep more sky for a taller mountain. The tower sits
        // on a low horizon; the high vantage looks down on a high one.
        let wide = (w / h - 1.0).clamp(0.0, 1.0);
        let lo = match template {
            Template::TowerPeak => 0.66 + 0.04 * wide,
            Template::HighVantage => 0.28 + 0.04 * wide,
            _ => 0.52 + 0.08 * wide,
        };
        let horizon = h * r.range_f64(lo, lo + 0.08);
        let peak_scale = r.next_f64();

        let fx = |u: f64| if mirrored { (1.0 - u) * w } else { u * w };
        let hump = |r: &mut Rng, x: f64, rel: f64| Hump {
            x,
            rel,
            left: r.range_f64(0.75, 1.35),
            right: r.range_f64(0.75, 1.35),
            concave: r.range_f64(0.0, 0.6),
        };
        let mut humps = Vec::new();
        let secondary = match template {
            Template::PeakOverWater => {
                let x = fx(r.range_f64(0.24, 0.40));
                humps.push(hump(&mut r, x, 1.0));
                (0.3, 0.6)
            }
            Template::FramingRidges => {
                let x = fx(r.range_f64(0.40, 0.60));
                humps.push(hump(&mut r, x, 1.0));
                (0.4, 0.7)
            }
            Template::TwinSummits => {
                let x = fx(r.range_f64(0.22, 0.36));
                humps.push(hump(&mut r, x, 1.0));
                let x2 = fx(r.range_f64(0.62, 0.78));
                let rel = r.range_f64(0.8, 0.97);
                humps.push(hump(&mut r, x2, rel));
                (0.3, 0.6)
            }
            Template::TowerPeak => {
                let x = fx(r.range_f64(0.35, 0.62));
                let mut main = hump(&mut r, x, 1.0);
                main.left = r.range_f64(0.6, 1.5);
                main.right = r.range_f64(0.6, 1.5);
                humps.push(main);
                // Shoulders and a subsidiary top make a massif, not a
                // pyramid.
                for side in [-1.0, 1.0] {
                    let dx = side * r.range_f64(0.1, 0.3) * w.min(1.0);
                    let rel = r.range_f64(0.5, 0.82);
                    humps.push(hump(&mut r, x + dx, rel));
                }
                (0.15, 0.4)
            }
            Template::HighVantage => {
                let x = fx(r.range_f64(0.3, 0.7));
                humps.push(hump(&mut r, x, 1.0));
                (0.35, 0.7)
            }
        };
        // Secondary summits walk outward in canvas units, so wider frames
        // carry more of the range rather than stretched summits.
        let (min_x, max_x) = humps
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p.x), b.max(p.x))
            });
        let mut x = min_x - r.range_f64(0.3, 0.6);
        while x > -0.4 {
            let rel = r.range_f64(secondary.0, secondary.1);
            humps.push(hump(&mut r, x, rel));
            x -= r.range_f64(0.3, 0.6);
        }
        let mut x = max_x + r.range_f64(0.3, 0.6);
        while x < w + 0.4 {
            let rel = r.range_f64(secondary.0, secondary.1);
            humps.push(hump(&mut r, x, rel));
            x += r.range_f64(0.3, 0.6);
        }

        let shore_roll = r.next_f64();
        let corner_right = humps[0].x < 0.5 * w;
        let shore = match template {
            Template::PeakOverWater | Template::TowerPeak => ShoreKind::Corner {
                right: corner_right,
            },
            Template::HighVantage => ShoreKind::Valley,
            Template::FramingRidges => ShoreKind::Bay,
            Template::TwinSummits if shore_roll < 0.6 => ShoreKind::Corner {
                right: corner_right,
            },
            Template::TwinSummits => ShoreKind::Bay,
        };
        let shore_params = match shore {
            // The tower's foreground stays small so the summit dominates.
            ShoreKind::Corner { .. } if template == Template::TowerPeak => [
                r.range_f64(0.35, 0.65),
                r.range_f64(0.3, 0.55),
                r.range_f64(0.2, 0.7),
                r.next_f64(),
            ],
            ShoreKind::Corner { .. } => [
                r.range_f64(0.15, 0.45),
                r.range_f64(0.55, 0.85),
                r.range_f64(0.2, 0.7),
                r.next_f64(),
            ],
            ShoreKind::Bay => [
                r.range_f64(0.45, 0.65),
                r.range_f64(0.3, 0.7),
                r.range_f64(0.35, 0.6),
                r.range_f64(0.08, 0.25),
            ],
            ShoreKind::Valley => [0.0; 4],
        };
        let framing = [
            r.range_f64(0.35, 0.6),
            r.range_f64(0.22, 0.36),
            r.range_f64(0.35, 0.6),
            r.range_f64(0.22, 0.36),
        ];
        let framing_bend = r.range_f64(0.02, 0.05);
        let rock_cluster = r.next_f64();

        // Sky: cumulus banks sit in the sky space (often behind the peaks);
        // dramatic skies usually add a storm deck with a break near the
        // focal summit, and the light pool falls through it.
        let dramatic = r.next_f64() < 0.75;
        let n_cumulus = 1 + r.below(MAX_CUMULUS as u64) as usize;
        let mut cumulus = Vec::with_capacity(MAX_CUMULUS);
        for _ in 0..MAX_CUMULUS {
            cumulus.push(Cumulus {
                x: r.range_f64(-0.05, 1.05) * w,
                width: r.range_f64(0.25, 0.7) * w.clamp(1.0, 1.8),
                base: horizon * r.range_f64(0.3, 0.8),
                height: r.range_f64(0.08, 0.22) * h.min(w).max(horizon.min(1.0)),
                billow: r.range_f64(0.14, 0.26),
            });
        }
        cumulus.truncate(n_cumulus);
        let deck_roll = r.next_f64();
        let deck_params = Deck {
            base: horizon * r.range_f64(0.12, 0.3),
            gap_x: humps[0].x + r.range_f64(-0.2, 0.2),
            gap_width: r.range_f64(0.3, 0.7) * w.min(1.5),
            lift: r.range_f64(0.4, 0.8),
        };
        let deck = (deck_roll < if dramatic { 0.65 } else { 0.15 }).then_some(Deck {
            lift: deck_params.base * deck_params.lift,
            ..deck_params
        });
        let on_summit = template != Template::HighVantage || r.next_f64() < 0.4;
        let light = LightPool {
            on_summit,
            x: r.range_f64(0.2, 0.8) * w,
            y: horizon + (h - horizon) * r.range_f64(0.1, 0.4),
            radius: r.range_f64(0.25, 0.5),
            strength: if dramatic {
                r.range_f64(0.5, 0.9)
            } else {
                r.range_f64(0.1, 0.25)
            },
        };

        let mut spurs = Vec::new();
        let mut river = [0.0; 4];
        if template == Template::HighVantage {
            // A meandering channel: smooth bends of even wavelength that
            // widen toward the viewer. Each spur grows from the bank the
            // channel swings away from (the inside of the bend) and ends at
            // the channel's edge, so the tips line up along smooth banks.
            river = [
                r.range_f64(0.4, 0.6),
                r.range_f64(0.12, 0.22),
                r.range_f64(1.0, 1.8),
                r.next_f64(),
            ];
            let n = 4 + r.below((MAX_SPURS - 3) as u64) as usize;
            for k in 0..n {
                // The nearest spur sits on the frame's bottom edge, so the
                // view ends on the valley's nearest ridge.
                let t = (k + 1) as f64 / n as f64;
                let jitter = r.range_f64(-0.03, 0.03);
                let height = r.range_f64(0.6, 1.4);
                let thickness = r.range_f64(0.5, 1.2);
                let (left, right, centre) = channel(&river, t);
                let from_right = centre < river[0];
                let reach = if from_right { 1.0 - right } else { left };
                spurs.push(Spur {
                    from_right,
                    nearness: t,
                    reach: (reach + jitter).clamp(0.12, 0.85),
                    height,
                    thickness,
                });
            }
        }
        Composition {
            river,
            template,
            mirrored,
            light_from_left,
            horizon,
            peak_scale,
            humps,
            shore,
            shore_params,
            framing,
            framing_bend,
            rock_cluster,
            dramatic,
            cumulus,
            deck,
            light,
            spurs,
        }
    }
}

/// The product scene generator (`GENERATOR_VERSION`).
#[derive(Debug, Clone, Copy, Default)]
pub struct LakeshoreGenerator;

impl SceneGenerator for LakeshoreGenerator {
    fn version(&self) -> u32 {
        version::GENERATOR_VERSION
    }

    fn generate(
        &self,
        seeds: &SeedBundle,
        form: &FormSettings,
        aspect: AspectRatio,
    ) -> Result<Scene, ValidationError> {
        form.validate()?;
        let comp = Composition::draw(seeds, aspect);
        let layers = Builder::new(seeds, form, aspect, &comp).build();
        Scene::new(SceneKey::new(self.version(), seeds, *form, aspect), layers)
    }
}

// ---------------------------------------------------------------------------

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// `f(t) = t` blended toward a smooth ease by `1 − angular`.
fn ease(t: f64, angular: f64) -> f64 {
    let smooth = t * t * (3.0 - 2.0 * t);
    lerp(smooth, t, angular)
}

/// Polynomial smooth maximum; `k = 0` is the plain maximum.
fn smax(a: f64, b: f64, k: f64) -> f64 {
    if k <= 0.0 {
        return a.max(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.max(b) + h * h * k * 0.25
}

/// Shading from a face's chord `(dx, dy)` (canvas, +y down) and the light.
fn face_shade(dx: f64, dy: f64, light_from_left: bool) -> f64 {
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return 0.5;
    }
    // Normal pointing up (negative y).
    let (mut nx, mut ny) = (dy / len, -dx / len);
    if ny > 0.0 {
        nx = -nx;
        ny = -ny;
    }
    let (lx, ly): (f64, f64) = (if light_from_left { -0.66 } else { 0.66 }, -0.75);
    let ll = (lx * lx + ly * ly).sqrt();
    (0.5 + 0.5 * (nx * lx + ny * ly) / ll).clamp(0.08, 0.92)
}

enum Bottom<'a> {
    /// A straight bottom edge at this y (full-width masses hidden below).
    Flat(f64),
    /// A bottom chain that meets the top chain at both ends.
    Pinched(&'a [f64]),
    /// A bottom chain strictly below the top chain everywhere.
    Open(&'a [f64]),
}

/// Outline of the band between `top` and `bottom` over increasing `xs`.
fn band(xs: &[f64], top: &[f64], bottom: Bottom) -> Vec<CanvasPoint> {
    let pt = |x: f64, y: f64| CanvasPoint {
        x: x as f32,
        y: y as f32,
    };
    let n = xs.len();
    let mut out: Vec<CanvasPoint> = (0..n).map(|i| pt(xs[i], top[i])).collect();
    match bottom {
        Bottom::Flat(y) => {
            out.push(pt(xs[n - 1], y));
            out.push(pt(xs[0], y));
        }
        Bottom::Pinched(b) => {
            for i in (1..n - 1).rev() {
                out.push(pt(xs[i], b[i]));
            }
        }
        Bottom::Open(b) => {
            for i in (0..n).rev() {
                out.push(pt(xs[i], b[i]));
            }
        }
    }
    out
}

/// Evenly spaced samples covering `[a, b]` with spacing at most `step`.
fn samples(a: f64, b: f64, step: f64) -> Vec<f64> {
    let n = (((b - a) / step).ceil() as usize).max(1) + 1;
    let d = (b - a) / (n - 1) as f64;
    (0..n).map(|i| a + d * i as f64).collect()
}

struct Builder<'a> {
    form: &'a FormSettings,
    comp: &'a Composition,
    w: f64,
    h: f64,
    /// Sky space: the horizon's canvas y.
    sky: f64,
    xs: Vec<f64>,
    terrain: Rng,
    vegetation: Rng,
    layers: Vec<SceneLayer>,
    /// Height of the primary summit above the horizon.
    primary: f64,
    /// Canvas position of the highest visible summit.
    summit: (f64, f64),
}

/// Noise seeds drawn once from the terrain stream, in a fixed order.
struct TerrainSeeds {
    mountain: u64,
    far_range: u64,
    foothills: u64,
    framing: [u64; 2],
    waterline: u64,
    shore: u64,
    beach: u64,
    planes: u64,
    rocks: u64,
    clouds: u64,
    spurs: u64,
}

impl<'a> Builder<'a> {
    fn new(
        seeds: &SeedBundle,
        form: &'a FormSettings,
        aspect: AspectRatio,
        comp: &'a Composition,
    ) -> Builder<'a> {
        let ext = aspect.extents();
        Builder {
            form,
            comp,
            w: ext.width,
            h: ext.height,
            sky: comp.horizon,
            xs: samples(-MARGIN, ext.width + MARGIN, PROFILE_STEP),
            terrain: seeds.stream(Domain::Terrain).rng(),
            vegetation: seeds.stream(Domain::Vegetation).rng(),
            layers: Vec::new(),
            primary: primary_height(form, comp),
            summit: (0.5 * ext.width, comp.horizon),
        }
    }

    fn push(&mut self, role: LayerRole, depth: f32, shade: f64, outline: Vec<CanvasPoint>) {
        self.layers.push(SceneLayer {
            role,
            depth,
            shade: shade as f32,
            plant: Plant::None,
            outline,
        });
    }

    fn push_woods(&mut self, plant: Plant, depth: f32, outline: Vec<CanvasPoint>) {
        self.layers.push(SceneLayer {
            role: LayerRole::Woodland,
            depth,
            shade: 0.5,
            plant,
            outline,
        });
    }

    fn fbm(&self, seed: u64, base: f64, min: f64, gain: f64, angular: f64) -> Fbm {
        Fbm {
            seed,
            base_wavelength: base,
            min_wavelength: min.max(MIN_WAVELENGTH),
            gain,
            angular,
        }
    }

    fn build(mut self) -> Vec<SceneLayer> {
        let t = &mut self.terrain;
        let ts = TerrainSeeds {
            mountain: t.next_u64(),
            far_range: t.next_u64(),
            foothills: t.next_u64(),
            framing: [t.next_u64(), t.next_u64()],
            waterline: t.next_u64(),
            shore: t.next_u64(),
            beach: t.next_u64(),
            planes: t.next_u64(),
            rocks: t.next_u64(),
            clouds: t.next_u64(),
            spurs: t.next_u64(),
        };
        // Reserved: formerly the cliff-edge seed. Still drawn so the rock
        // draws that follow, and so every other template, stay unchanged.
        let _reserved = t.next_u64();
        let (w, bottom) = (self.w, self.h + MARGIN);
        let sky = [
            CanvasPoint {
                x: -MARGIN as f32,
                y: -MARGIN as f32,
            },
            CanvasPoint {
                x: (w + MARGIN) as f32,
                y: -MARGIN as f32,
            },
            CanvasPoint {
                x: (w + MARGIN) as f32,
                y: bottom as f32,
            },
            CanvasPoint {
                x: -MARGIN as f32,
                y: bottom as f32,
            },
        ];
        self.push(LayerRole::Sky, depth::SKY, 0.5, sky.to_vec());
        self.clouds(&ts);

        let ridge = self.mountain_ridge(&ts);
        let far = self.far_range(&ts, &ridge);
        let cap = |y: f64| y.min(bottom - 0.01);
        let far_top: Vec<f64> = far.iter().map(|r| cap(self.sky - r)).collect();
        let outline = band(&self.xs, &far_top, Bottom::Flat(bottom));
        self.push(LayerRole::FarRidge, depth::FAR_RANGE, 0.5, outline);

        let sil: Vec<f64> = ridge.iter().map(|r| cap(self.sky - r)).collect();
        if let Some(i) = (0..self.xs.len())
            .filter(|&i| (0.0..=w).contains(&self.xs[i]))
            .min_by(|&i, &j| sil[i].total_cmp(&sil[j]))
        {
            self.summit = (self.xs[i], sil[i]);
        }
        let outline = band(&self.xs, &sil, Bottom::Flat(bottom));
        self.push(LayerRole::Mountain, depth::MOUNTAIN, 0.5, outline);
        self.mountain_planes(&ts, &ridge, &sil);

        let hills = self.foothills(&ts);
        let outline = band(&self.xs, &hills, Bottom::Flat(bottom));
        self.push(LayerRole::MidRidge, depth::FOOTHILLS, 0.5, outline);

        let waterline: Vec<f64> = self.xs.iter().map(|&x| self.waterline(&ts, x)).collect();
        self.far_woods(&ts);
        if self.comp.template == Template::FramingRidges {
            self.framing_ridges(&ts);
        }

        let fb = self.fbm(ts.beach, 0.3, 1.0 / 80.0, 0.5, 0.5);
        let beach_top: Vec<f64> = self
            .xs
            .iter()
            .zip(&waterline)
            .map(|(&x, &y)| y - 0.0015 - 0.005 * fb.eval(x).max(0.0))
            .collect();
        let beach_bottom: Vec<f64> = waterline.iter().map(|y| y + 0.004).collect();
        let outline = band(&self.xs, &beach_top, Bottom::Open(&beach_bottom));
        self.push(LayerRole::Shore, depth::BEACH, 0.5, outline);

        let outline = band(&self.xs, &waterline, Bottom::Flat(bottom));
        self.push(LayerRole::Water, depth::WATER, 0.5, outline);

        if self.comp.shore == ShoreKind::Valley {
            self.spurs(&ts);
        } else {
            let run = self.near_shore(&ts);
            self.near_woods(&ts, run);
            self.rocks(&ts, run);
        }
        self.apply_light();
        self.layers
    }

    /// Storm deck, then cumulus banks, each a band with structural planes.
    fn clouds(&mut self, ts: &TerrainSeeds) {
        let c = self.comp;
        let (w, top) = (self.w, -MARGIN);
        if let Some(d) = c.deck {
            let fb = self.fbm(ts.clouds, 0.4, 1.0 / 60.0, 0.5, 0.3);
            let bottom: Vec<f64> = self
                .xs
                .iter()
                .map(|&x| {
                    let g = (x - d.gap_x) / (0.5 * d.gap_width);
                    let q = (1.0 - g * g).max(0.0);
                    // A sagging, lumpy underside that lifts in the break.
                    let lumps = crowns(ts.clouds ^ 7, x, 0.05) - 0.65;
                    let y = d.base * (1.0 + 0.2 * fb.eval(x)) - d.lift * q * q + 0.012 * lumps;
                    y.max(top + 0.02)
                })
                .collect();
            let pt = |x: f64, y: f64| CanvasPoint {
                x: x as f32,
                y: y as f32,
            };
            let mut outline = vec![pt(-MARGIN, top), pt(w + MARGIN, top)];
            for i in (0..self.xs.len()).rev() {
                outline.push(pt(self.xs[i], bottom[i]));
            }
            self.push(LayerRole::Cloud, depth::CLOUDS, 0.3, outline);
        }
        for (k, cu) in c.cumulus.iter().enumerate() {
            let (a, b) = (
                (cu.x - 0.5 * cu.width).max(-MARGIN),
                (cu.x + 0.5 * cu.width).min(w + MARGIN),
            );
            if b - a < 0.05 {
                continue;
            }
            let seed = ts.clouds.wrapping_add(k as u64 + 1);
            // Billows are at least 0.1 across, so a coarser grid suffices.
            let xs = samples(a, b, CLOUD_STEP);
            let swell = self.fbm(seed ^ 5, 2.5 * cu.billow, 0.5 * cu.billow, 0.5, 0.0);
            let env: Vec<f64> = xs.iter().map(|&x| envelope((x - a) / (b - a))).collect();
            let top: Vec<f64> = xs
                .iter()
                .zip(&env)
                .map(|(&x, &e)| {
                    // Two billow scales, offset, with a slow swell: towers
                    // and hollows rather than a row of equal teeth.
                    let b1 = billow(seed, x, cu.billow);
                    let b2 = billow(seed ^ 3, x + 0.37 * cu.billow, 0.43 * cu.billow);
                    let swell = swell.eval(x);
                    cu.base - cu.height * e * (0.3 + 0.35 * b1 + 0.2 * b2 + 0.35 * swell)
                })
                .collect();
            let bottom: Vec<f64> = env
                .iter()
                .map(|e| cu.base + 0.004 + 0.05 * cu.height * e)
                .collect();
            self.push(
                LayerRole::Cloud,
                depth::CLOUDS,
                0.62,
                band(&xs, &top, Bottom::Open(&bottom)),
            );
            // Sunlit crown and shadowed base.
            let cap: Vec<f64> = top
                .iter()
                .zip(&bottom)
                .map(|(t, b)| t + 0.4 * (b - t))
                .collect();
            self.push(
                LayerRole::Cloud,
                depth::CLOUDS,
                0.88,
                band(&xs, &top, Bottom::Open(&cap)),
            );
            let base_top: Vec<f64> = env.iter().map(|e| cu.base - 0.18 * cu.height * e).collect();
            self.push(
                LayerRole::Cloud,
                depth::CLOUDS,
                0.34,
                band(&xs, &base_top, Bottom::Open(&bottom)),
            );
        }
    }

    /// `HighVantage`: ridge spurs from the sides, far to near, each with a
    /// band of trees sized for its distance, so the eye steps down the
    /// valley and the trees measure it.
    fn spurs(&mut self, ts: &TerrainSeeds) {
        let f = *self.form;
        let fg = self.h - self.sky;
        let spurs = self.comp.spurs.clone();
        for (k, sp) in spurs.iter().enumerate() {
            let v = &mut self.vegetation;
            let (wood_roll, wood_at, wood_len, wood_wl, wood_seed) = (
                v.next_f64(),
                v.range_f64(0.05, 0.45),
                v.range_f64(0.2, 0.5),
                v.next_f64(),
                v.next_u64(),
            );
            let t = sp.nearness;
            // Perspective: spurs bunch up toward the horizon.
            let y_base = self.sky + fg * t * t.sqrt();
            let size = (0.05 + 0.32 * t * t) * (0.6 + 0.8 * f.relief);
            let (height, thick) = (size * sp.height, 0.8 * size * sp.thickness);
            let reach = sp.reach * self.w;
            let span = reach + MARGIN;
            let sw = self.w;
            let dist = move |x: f64| {
                if sp.from_right {
                    sw + MARGIN - x
                } else {
                    x + MARGIN
                }
            };
            // Distant spurs are small and low in contrast: sample them more
            // coarsely (twice the grid spacing at the horizon, the full grid
            // in the foreground) and band-limit their detail to match.
            let step = PROFILE_STEP * (2.0 - t);
            let xs = if sp.from_right {
                samples(self.w - reach, self.w + MARGIN, step)
            } else {
                samples(-MARGIN, reach, step)
            };
            if xs.len() < 4 {
                continue;
            }
            let detail = self.fbm(
                ts.spurs.wrapping_add(k as u64),
                0.15 * (0.5 + t),
                2.0 * step,
                0.5,
                f.faceting,
            );
            // Falls from the frame edge to the tip, with a rounded or
            // faceted hill along the way so the spur has volume.
            let profile = |x: f64| {
                let u = (dist(x) / span).clamp(0.0, 1.0);
                let fall = 1.0 - ease(u, f.faceting);
                let hill = 1.0 - (2.0 * u - 0.7) * (2.0 * u - 0.7);
                fall * (0.65 + 0.35 * hill.max(0.0))
            };
            // Blunt, rounded tips (water erodes points): the last 30 % of
            // the spur closes on an elliptical cap. The water line curves
            // gently instead of running straight.
            let cap = |x: f64| {
                let u = ((dist(x) / span - 0.7) / 0.3).clamp(0.0, 1.0);
                (1.0 - u * u).sqrt()
            };
            let bank = self.fbm(
                ts.spurs.wrapping_add(100 + k as u64),
                0.2,
                4.0 * step,
                0.4,
                0.0,
            );
            let top: Vec<f64> = xs
                .iter()
                .map(|&x| {
                    let upper = height * profile(x) * (1.0 + 0.4 * detail.eval(x)) + 0.35 * thick;
                    y_base - upper * cap(x)
                })
                .collect();
            let bottom: Vec<f64> = xs
                .iter()
                .map(|&x| {
                    let u = (dist(x) / span).clamp(0.0, 1.0);
                    let lower =
                        thick * (0.55 + 0.45 * (1.0 - u) * (1.0 - u)) * (1.0 + 0.1 * bank.eval(x));
                    y_base + 0.001 + lower * cap(x)
                })
                .collect();
            let d = depth::SPUR_FARTHEST - (depth::SPUR_FARTHEST - depth::SPUR_NEAREST) * t as f32;
            let (x_edge, x_tip) = if sp.from_right {
                (self.w + MARGIN, self.w - reach)
            } else {
                (-MARGIN, reach)
            };
            let shade = face_shade(x_tip - x_edge, height, self.comp.light_from_left);
            self.push(
                LayerRole::NearRidge,
                d,
                shade,
                band(&xs, &top, Bottom::Open(&bottom)),
            );
            // Trees along the spur's upper slope.
            if wood_roll >= 0.2 + 0.7 * f.woodland_density {
                continue;
            }
            let (s0, s1) = (wood_at, (wood_at + wood_len).min(0.95));
            let sel: Vec<usize> = (0..xs.len())
                .filter(|&i| {
                    let s = dist(xs[i]) / span;
                    s >= s0 && s <= s1
                })
                .collect();
            if sel.len() < 4 {
                continue;
            }
            let plant = Plant::pick(unit(wood_seed, 0));
            let (tall_mul, wl_mul) = plant_shape(plant);
            let tree = (0.003 + 0.03 * t * t) * (0.6 + 0.8 * f.woodland_density) * tall_mul;
            let wl = (0.004 + 0.02 * t) * (0.8 + 0.4 * wood_wl) * wl_mul;
            let (a, b) = (xs[sel[0]], xs[sel[sel.len() - 1]]);
            let wxs: Vec<f64> = sel.iter().map(|&i| xs[i]).collect();
            let wtop: Vec<f64> = sel
                .iter()
                .map(|&i| {
                    let x = xs[i];
                    top[i] - tree * envelope((x - a) / (b - a)) * canopy(plant, wood_seed, x, wl)
                })
                .collect();
            let wbot: Vec<f64> = sel.iter().map(|&i| top[i] + 0.8 * tree + 0.001).collect();
            self.push_woods(plant, d, band(&wxs, &wtop, Bottom::Open(&wbot)));
        }
    }

    /// Structural light: layers inside the light pool are lifted, the rest
    /// sink into cloud shadow, by the pool's strength. The pool is judged
    /// from each layer's vertices inside the frame.
    fn apply_light(&mut self) {
        let lp = self.comp.light;
        let (cx, cy) = if lp.on_summit {
            (self.summit.0, self.summit.1 + 0.3 * lp.radius)
        } else {
            (lp.x, lp.y)
        };
        let (w, h, r2) = (self.w, self.h, lp.radius * lp.radius);
        for l in self.layers.iter_mut().skip(1) {
            let (mut acc, mut n) = (0.0, 0usize);
            for p in &l.outline {
                let (x, y) = (p.x as f64, p.y as f64);
                if (0.0..=w).contains(&x) && (-0.05..=h + 0.05).contains(&y) {
                    let d2 = ((x - cx) * (x - cx) + (y - cy) * (y - cy)) / r2;
                    let g = (1.0 - d2).max(0.0);
                    acc += g * g;
                    n += 1;
                }
            }
            let lit = if n > 0 {
                (3.0 * acc / n as f64).min(1.0)
            } else {
                0.0
            };
            let factor = lerp(1.0 - 0.45 * lp.strength, 1.0 + 0.4 * lp.strength, lit);
            l.shade = ((l.shade as f64) * factor).clamp(0.03, 0.97) as f32;
        }
    }

    /// Height above the horizon of the main massif at every grid sample.
    fn mountain_ridge(&self, ts: &TerrainSeeds) -> Vec<f64> {
        let f = self.form;
        let c = self.comp;
        let primary = self.primary;
        // Half-width / height: the tower is steep.
        let ratio = if c.template == Template::TowerPeak {
            1.25 - 0.5 * f.relief
        } else {
            3.0 - 1.8 * f.relief
        };
        let k = primary * (0.25 * (1.0 - f.faceting) + 0.02);
        let shoulder = self.fbm(ts.mountain ^ 1, 0.9, 0.2, 0.5, f.faceting);
        let detail = self.fbm(
            ts.mountain,
            0.3,
            MIN_WAVELENGTH,
            0.45 + 0.13 * f.faceting,
            f.faceting,
        );
        let amp = (0.14 + 0.16 * f.relief)
            * if c.template == Template::TowerPeak {
                1.4
            } else {
                1.0
            };
        self.xs
            .iter()
            .map(|&x| {
                // A continuous low shoulder so the range never breaks.
                let mut m = primary * 0.14 * (1.0 + 0.5 * shoulder.eval(x));
                for hp in &c.humps {
                    let height = primary * hp.rel;
                    let hw = height * ratio * if x < hp.x { hp.left } else { hp.right };
                    let d = ((x - hp.x) / hw).abs();
                    let tri = if d < 1.0 {
                        (1.0 - d) * (1.0 - hp.concave * d)
                    } else {
                        0.0
                    };
                    let q = (1.0 - d * d).max(0.0);
                    let shape = height * lerp(q * q, tri, f.faceting);
                    m = smax(m, shape, k);
                }
                let mut r = m + primary * amp * (0.4 + 0.6 * m / primary) * detail.eval(x);
                let cap = 0.9 * self.sky;
                if r > cap {
                    r = cap + (r - cap) * 0.3;
                }
                r
            })
            .collect()
    }

    fn far_range(&self, ts: &TerrainSeeds, ridge: &[f64]) -> Vec<f64> {
        let f = self.form;
        let peak = ridge.iter().cloned().fold(0.0, f64::max);
        let low = self.fbm(ts.far_range, 0.6, 1.0 / 60.0, 0.5, f.faceting);
        let detail = self.fbm(ts.far_range ^ 1, 0.12, 1.0 / 90.0, 0.5, f.faceting);
        self.xs
            .iter()
            .map(|&x| peak * (0.38 + 0.22 * low.eval(x) + 0.05 * detail.eval(x)))
            .collect()
    }

    /// Light and shadow planes on each face of the main summits.
    fn mountain_planes(&mut self, ts: &TerrainSeeds, ridge: &[f64], sil: &[f64]) {
        let f = *self.form;
        let n = self.xs.len();
        let idx = |x: f64| -> usize {
            (((x + MARGIN) / (self.xs[1] - self.xs[0])).round().max(0.0) as usize).min(n - 1)
        };
        // Locate actual summits near each significant hump.
        let mut peaks: Vec<(usize, usize)> = Vec::new(); // (grid index, hump index)
        for (hi, hp) in self.comp.humps.iter().enumerate() {
            if hp.rel < 0.3 || hp.x < -0.2 || hp.x > self.w + 0.2 {
                continue;
            }
            let reach = 0.12;
            let (a, b) = (idx(hp.x - reach), idx(hp.x + reach));
            let best = (a..=b)
                .max_by(|&i, &j| ridge[i].total_cmp(&ridge[j]))
                .unwrap();
            // Summits closer than this would make sliver facets.
            let min_gap = (0.06 / PROFILE_STEP) as usize;
            if peaks.iter().all(|&(p, _)| p.abs_diff(best) > min_gap) {
                peaks.push((best, hi));
            }
        }
        peaks.sort();
        let mut faces: Vec<(usize, usize, usize, bool)> = Vec::new(); // (peak, saddle, hump, left face)
        for (k, &(p, hi)) in peaks.iter().enumerate() {
            let floor = 0.35 * ridge[p];
            let left = if k > 0 {
                let q = peaks[k - 1].0;
                (q..p).max_by(|&i, &j| sil[i].total_cmp(&sil[j])).unwrap()
            } else {
                (0..p).rev().find(|&i| ridge[i] < floor).unwrap_or(0)
            };
            let right = if k + 1 < peaks.len() {
                let q = peaks[k + 1].0;
                (p + 1..=q)
                    .max_by(|&i, &j| sil[i].total_cmp(&sil[j]))
                    .unwrap()
            } else {
                (p + 1..n).find(|&i| ridge[i] < floor).unwrap_or(n - 1)
            };
            faces.push((p, left, hi, true));
            faces.push((p, right, hi, false));
        }
        // Each face is split along the ridge into 1–2 facets. A facet hangs
        // from the silhouette: its lower edge falls from one end to a low
        // point and rises to the other, so planes interlock as wedges with
        // the mass showing between them, never as horizontal bands. The
        // facet at the summit is the flank: its crease runs from the summit
        // down to the foot, so the flank reads as one lit or shadowed plane
        // and the mass between the two flanks as the summit's front spur.
        // Further facets hang shallowly and break the flank up.
        let facets_per_face = 1 + f.faceting.round() as usize;
        let contrast = 0.6 + 0.4 * f.faceting;
        let mut planes = 0;
        for (p, s, hi, is_left) in faces {
            let (i0, i1) = if is_left { (s, p) } else { (p, s) };
            if i1 < i0 + 4 || self.xs[i1] < 0.0 || self.xs[i0] > self.w {
                continue;
            }
            let face = face_shade(
                self.xs[s] - self.xs[p],
                sil[s] - sil[p],
                self.comp.light_from_left,
            );
            let key = (hi as i64 * 2 + i64::from(is_left)) * 32;
            let h = |k: i64| unit(ts.planes, key + k);
            // Split points, measured from the summit end.
            let mut cuts = vec![0.0];
            let weights: Vec<f64> = (0..facets_per_face)
                .map(|j| if j == 0 { 1.2 } else { 0.5 } + h(j as i64))
                .collect();
            let total: f64 = weights.iter().sum();
            let mut acc = 0.0;
            for wgt in &weights {
                acc += wgt / total;
                cuts.push(acc);
            }
            let m = (i1 - i0) as f64;
            // Narrow faces get one facet; no facet is narrower than this.
            let min_facet = (0.08 / PROFILE_STEP) as usize;
            let facets = if i1 - i0 >= 3 * min_facet {
                facets_per_face
            } else {
                1
            };
            let cuts = if facets == facets_per_face {
                cuts
            } else {
                vec![0.0, 1.0]
            };
            for j in 0..facets {
                if planes >= MAX_MOUNTAIN_PLANES {
                    return;
                }
                // Local index range of this facet within [i0, i1].
                let (u0, u1) = (cuts[j], cuts[j + 1]);
                let (a, b) = if is_left {
                    (
                        i1 - (u1 * m).round() as usize,
                        i1 - (u0 * m).round() as usize,
                    )
                } else {
                    (
                        i0 + (u0 * m).round() as usize,
                        i0 + (u1 * m).round() as usize,
                    )
                };
                if b < a + min_facet.min(i1 - i0) {
                    continue;
                }
                // Skewed low points make slanted planes (one steep edge,
                // one shallow) rather than symmetric teeth.
                let (low_at, drop) = if j == 0 {
                    (0.6 + 0.3 * h(8), 0.75 + 0.25 * h(16))
                } else {
                    (0.15 + 0.7 * h(8 + j as i64), 0.15 + 0.3 * h(16 + j as i64))
                };
                // `low_at` is measured from the summit end of the face.
                let low_at = if is_left { 1.0 - low_at } else { low_at };
                let width = self.xs[b] - self.xs[a];
                // Flanks may be deep; secondary facets stay broad and shallow.
                let max_drop = width * if j == 0 { 3.0 } else { 1.2 };
                let bottom = self.facet_bottom(&sil[a..=b], max_drop, low_at, drop, f.faceting);
                let vary = 0.7 * (h(24 + j as i64) - 0.5);
                let shade = (0.5 + (face - 0.5) * contrast * (1.0 + vary)).clamp(0.05, 0.95);
                let outline = band(&self.xs[a..=b], &sil[a..=b], Bottom::Pinched(&bottom));
                self.push(LayerRole::Mountain, depth::MOUNTAIN, shade, outline);
                planes += 1;
            }
        }
    }

    /// Lower edge of a facet under `top`: falls from the first sample to a
    /// low point `low_at` of the way along, `drop` of the way from the
    /// silhouette to just below the horizon, then rises to the last sample.
    /// The drop is capped at `max_drop` canvas units, so narrow facets never
    /// become sliver gullies. Strictly below the top in the interior.
    fn facet_bottom(
        &self,
        top: &[f64],
        max_drop: f64,
        low_at: f64,
        drop: f64,
        angular: f64,
    ) -> Vec<f64> {
        let last = top.len() - 1;
        let floor = self.sky + 0.02;
        let il = ((low_at * last as f64).round() as usize).clamp(1, last - 1);
        let y_low = (top[il] + (floor - top[il]) * drop).min(top[il] + max_drop);
        (0..=last)
            .map(|i| {
                let raw = if i <= il {
                    lerp(top[0], y_low, ease(i as f64 / il as f64, angular))
                } else {
                    lerp(
                        y_low,
                        top[last],
                        ease((i - il) as f64 / (last - il) as f64, angular),
                    )
                };
                let gap = (floor - top[i]).max(0.004);
                raw.max(top[i] + 0.04 * gap).min(floor).max(top[i] + 0.002)
            })
            .collect()
    }

    fn foothills(&self, ts: &TerrainSeeds) -> Vec<f64> {
        let f = self.form;
        let fb = self.fbm(ts.foothills, 0.35, 1.0 / 120.0, 0.5, 0.6 * f.faceting);
        let base = self.sky * (0.07 + 0.08 * f.relief);
        self.xs
            .iter()
            .map(|&x| self.sky - base * (0.55 + 0.45 * fb.eval(x)))
            .collect()
    }

    /// Canvas y of the far shoreline (the top edge of the lake).
    fn waterline(&self, ts: &TerrainSeeds, x: f64) -> f64 {
        let fw = self.fbm(
            ts.waterline,
            0.2,
            1.0 / 100.0,
            0.5,
            0.5 * self.form.faceting,
        );
        let mut y = self.sky + 0.006 * fw.eval(x);
        if self.comp.template == Template::FramingRidges {
            let c = self.comp;
            let (le, re) = (c.framing[1] * self.w, (1.0 - c.framing[3]) * self.w);
            let l = (1.0 - x / le).max(0.0);
            let r = (1.0 - (self.w - x) / (self.w - re)).max(0.0);
            y += c.framing_bend * (l * l + r * r);
        }
        y
    }

    fn framing_ridges(&mut self, ts: &TerrainSeeds) {
        let f = *self.form;
        let c = self.comp;
        let bottom = self.h + MARGIN;
        for side in 0..2 {
            let (height, foot) = if side == 0 {
                (c.framing[0], c.framing[1] * self.w)
            } else {
                (c.framing[2], (1.0 - c.framing[3]) * self.w)
            };
            let hl = self.sky * height * (0.55 + 0.45 * f.relief);
            let detail = self.fbm(ts.framing[side], 0.2, MIN_WAVELENGTH, 0.48, f.faceting);
            let xs: Vec<f64> = self
                .xs
                .iter()
                .cloned()
                .filter(|&x| if side == 0 { x <= foot } else { x >= foot })
                .collect();
            if xs.len() < 4 {
                continue;
            }
            let span = if side == 0 {
                foot + MARGIN
            } else {
                self.w + MARGIN - foot
            };
            let top: Vec<f64> = xs
                .iter()
                .map(|&x| {
                    let d = if side == 0 {
                        x + MARGIN
                    } else {
                        self.w + MARGIN - x
                    };
                    let s = (d / span).clamp(0.0, 1.0);
                    // 1 at the frame edge falling to 0 at the foot.
                    let p = 1.0 - ease(s, f.faceting);
                    let y = self.waterline(ts, x) + 0.012 - hl * p * (1.0 + 0.3 * detail.eval(x));
                    y.min(bottom - 0.01)
                })
                .collect();
            let outline = band(&xs, &top, Bottom::Flat(bottom));
            self.push(LayerRole::NearRidge, depth::FRAMING, 0.5, outline);
        }
    }

    fn far_woods(&mut self, ts: &TerrainSeeds) {
        let density = self.form.woodland_density;
        let span = self.w + 2.0 * MARGIN;
        let v = &mut self.vegetation;
        let params: Vec<(f64, f64, f64, f64, f64, u64)> = (0..MAX_FAR_WOODS)
            .map(|_| {
                (
                    v.range_f64(0.1, 0.9),
                    v.next_f64(),
                    v.next_f64(),
                    v.next_f64(),
                    v.range_f64(0.008, 0.016),
                    v.next_u64(),
                )
            })
            .collect();
        // Any woodland at all means at least one far wood: the trees at the
        // mountain's foot are its scale.
        let surest = (0..MAX_FAR_WOODS)
            .min_by(|&a, &b| params[a].3.total_cmp(&params[b].3))
            .unwrap();
        for (j, &(jit, width, height, roll, crown, seed)) in params.iter().enumerate() {
            let forced = density > 0.0 && j == surest;
            if roll >= 0.15 + 0.8 * density && !forced {
                continue;
            }
            let centre = -MARGIN + (j as f64 + jit) / MAX_FAR_WOODS as f64 * span;
            let hw = (0.06 + 0.18 * width) * (0.6 + 0.8 * density);
            // Trees at the mountain's foot are its scale: tiny under the
            // tower and seen from the high vantage.
            let scale = match self.comp.template {
                Template::TowerPeak => 0.35,
                Template::HighVantage => 0.5,
                _ => 1.0,
            };
            let plant = Plant::pick(unit(seed, 0));
            let (tall_mul, wl_mul) = plant_shape(plant);
            let crown = crown * wl_mul;
            let tall = (0.012 + 0.035 * height) * (0.6 + 0.8 * density) * scale * tall_mul;
            let (a, b) = (
                (centre - hw).max(-MARGIN),
                (centre + hw).min(self.w + MARGIN),
            );
            if b - a < 0.02 {
                continue;
            }
            let xs = samples(a, b, PROFILE_STEP);
            let wl: Vec<f64> = xs.iter().map(|&x| self.waterline(ts, x)).collect();
            let top: Vec<f64> = xs
                .iter()
                .zip(&wl)
                .map(|(&x, &y)| {
                    y - tall * envelope((x - a) / (b - a)) * canopy(plant, seed, x, crown)
                })
                .collect();
            let bottom: Vec<f64> = wl.iter().map(|y| y + 0.006).collect();
            let outline = band(&xs, &top, Bottom::Open(&bottom));
            self.push_woods(plant, depth::FAR_WOODS, outline);
        }
    }

    /// Canvas y of the near shoreline at `x` (land below, water above).
    fn shore_y(&self, ts: &TerrainSeeds, x: f64) -> f64 {
        let f = self.form;
        let c = self.comp;
        let fg = self.h - self.sky; // foreground depth
        let detail = self.fbm(ts.shore, 0.3, 1.0 / 100.0, 0.5, f.faceting);
        let base = match c.shore {
            ShoreKind::Corner { right } => {
                let [edge, reach, k, _] = c.shore_params;
                let u = if right { self.w - x } else { x };
                let s = u / (reach * self.w);
                let y_edge = self.sky + fg * edge;
                y_edge + (self.h - y_edge) * s * (k + (1.0 - k) * s)
            }
            ShoreKind::Bay => {
                let [mid, centre, width, depth] = c.shore_params;
                let d = (x - centre * self.w) / (0.5 * width * self.w);
                let q = (1.0 - d * d).max(0.0);
                let edge = |t: f64| (1.0 - t / (0.25 * self.w)).max(0.0);
                let headland = edge(x) * edge(x) + edge(self.w - x) * edge(self.w - x);
                self.sky + fg * (mid + depth * q * q - 0.2 * headland)
            }
            ShoreKind::Valley => self.h + MARGIN,
        };
        base + 0.012 * detail.eval(x)
    }

    /// Adds the near-shore land and returns the longest run of grid x where
    /// its shoreline is visible in the lower frame.
    fn near_shore(&mut self, ts: &TerrainSeeds) -> Option<(f64, f64)> {
        let bottom = self.h + MARGIN;
        let ys: Vec<f64> = self.xs.iter().map(|&x| self.shore_y(ts, x)).collect();
        let mut reached = false;
        let (xs, top): (Vec<f64>, Vec<f64>) = match self.comp.shore {
            ShoreKind::Corner { right } => {
                // Walk from the corner edge until the shore leaves the frame.
                let order: Vec<usize> = if right {
                    (0..self.xs.len()).rev().collect()
                } else {
                    (0..self.xs.len()).collect()
                };
                let mut pts = Vec::new();
                for i in order {
                    if ys[i] >= bottom {
                        pts.push((self.xs[i], bottom));
                        reached = true;
                        break;
                    }
                    pts.push((self.xs[i], ys[i]));
                }
                if right {
                    pts.reverse();
                }
                pts.into_iter().unzip()
            }
            ShoreKind::Bay => (
                self.xs.clone(),
                ys.iter().map(|y| y.min(bottom - 0.01)).collect(),
            ),
            ShoreKind::Valley => return None,
        };
        if xs.len() >= 3 {
            let outline = match self.comp.shore {
                ShoreKind::Corner { right } => {
                    let mut o = band(&xs, &top, Bottom::Flat(bottom));
                    // If the far end already lies on the bottom edge, drop
                    // the duplicate corner the flat bottom adds there.
                    if reached {
                        if right {
                            o.remove(o.len() - 1);
                        } else {
                            o.remove(o.len() - 2);
                        }
                    }
                    o
                }
                ShoreKind::Bay | ShoreKind::Valley => band(&xs, &top, Bottom::Flat(bottom)),
            };
            self.push(LayerRole::Shore, depth::NEAR_SHORE, 0.5, outline);
        }
        // Visible run: shoreline inside the lower frame.
        let lo = self.sky + 0.05 * (self.h - self.sky);
        let hi = self.h - 0.03;
        let mut best: Option<(f64, f64)> = None;
        let mut start: Option<f64> = None;
        for (i, &x) in self.xs.iter().enumerate() {
            let visible = (0.0..=self.w).contains(&x) && ys[i] > lo && ys[i] < hi;
            match (visible, start) {
                (true, None) => start = Some(x),
                (false, Some(a)) => {
                    let prev = self.xs[i - 1];
                    if best.is_none_or(|(p, q)| prev - a > q - p) {
                        best = Some((a, prev));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(a) = start {
            let last = *self.xs.last().unwrap();
            let end = last.min(self.w);
            if best.is_none_or(|(p, q)| end - a > q - p) {
                best = Some((a, end));
            }
        }
        best.filter(|(a, b)| b - a > 0.05)
    }

    /// Which end of the visible shore run is next to the frame edge.
    fn edge_side_is_left(&self, run: (f64, f64)) -> bool {
        match self.comp.shore {
            ShoreKind::Corner { right } => !right,
            ShoreKind::Bay | ShoreKind::Valley => run.0 < self.w - run.1,
        }
    }

    fn near_woods(&mut self, ts: &TerrainSeeds, run: Option<(f64, f64)>) {
        let density = self.form.woodland_density;
        for _ in 0..MAX_NEAR_WOODS {
            let v = &mut self.vegetation;
            let (pos, width, height, roll, crown, seed) = (
                v.range_f64(0.0, 0.9),
                v.next_f64(),
                v.next_f64(),
                v.next_f64(),
                v.range_f64(0.015, 0.03),
                v.next_u64(),
            );
            let Some(run) = run else { continue };
            if roll >= 0.2 + 0.8 * density {
                continue;
            }
            let len = run.1 - run.0;
            let pos = if self.comp.template == Template::TowerPeak {
                0.35 * pos
            } else {
                pos
            };
            let centre = if self.edge_side_is_left(run) {
                run.0 + pos * len
            } else {
                run.1 - pos * len
            };
            let hw = 0.5 * (0.08 + 0.2 * width) * (0.6 + 0.6 * density);
            // Near trees must not rival the mountain, or it reads as a hill.
            let plant = Plant::pick(unit(seed, 0));
            let (tall_mul, wl_mul) = plant_shape(plant);
            let crown = crown * wl_mul;
            let tall = ((0.06 + 0.16 * height) * (0.5 + 0.7 * density) * self.sky * tall_mul)
                .min(0.35 * self.primary);
            let (a, b) = (
                (centre - hw).max(-MARGIN),
                (centre + hw).min(self.w + MARGIN),
            );
            if b - a < 0.03 {
                continue;
            }
            let xs = samples(a, b, PROFILE_STEP * 0.75);
            let sy: Vec<f64> = xs
                .iter()
                .map(|&x| self.shore_y(ts, x).min(self.h + MARGIN - 0.03))
                .collect();
            let top: Vec<f64> = xs
                .iter()
                .zip(&sy)
                .map(|(&x, &y)| {
                    y - tall * envelope((x - a) / (b - a)) * canopy(plant, seed, x, crown)
                })
                .collect();
            let bottom: Vec<f64> = sy.iter().map(|y| y + 0.02).collect();
            let outline = band(&xs, &top, Bottom::Open(&bottom));
            self.push_woods(plant, depth::NEAR_WOODS, outline);
        }
    }

    fn rocks(&mut self, ts: &TerrainSeeds, run: Option<(f64, f64)>) {
        let f = *self.form;
        let t = &mut self.terrain;
        let count = 3 + t.below(4) as usize;
        let mut params = Vec::with_capacity(MAX_ROCKS);
        for _ in 0..MAX_ROCKS {
            params.push(RockParams {
                pos: t.next_f64(),
                size: t.next_f64(),
                height: t.range_f64(0.25, 0.5),
                apex: t.range_f64(0.3, 0.7),
                left: [
                    t.range_f64(0.1, 0.4),
                    t.range_f64(0.45, 0.8),
                    t.range_f64(0.5, 0.8),
                    t.range_f64(0.8, 1.0),
                ],
                right: [
                    t.range_f64(0.2, 0.5),
                    t.range_f64(0.75, 1.0),
                    t.range_f64(0.6, 0.9),
                    t.range_f64(0.35, 0.75),
                ],
                offset: t.range_f64(-0.25, 0.4),
                row: t.next_f64(),
                crease: [t.range_f64(0.15, 0.45), t.range_f64(0.15, 0.45)],
                cap: t.range_f64(0.12, 0.25),
            });
        }
        let Some((ra, rb)) = run else { return };
        let fg = self.h - self.sky;
        let mut rocks: Vec<(f64, f64, f64, RockParams)> = Vec::new(); // (x, y_base, width, params)
        for p in params.iter().take(count) {
            let s = (self.comp.rock_cluster + (p.pos - 0.5) * 0.6).clamp(0.03, 0.97);
            let x = ra + (rb - ra) * s;
            let shore = self.shore_y(ts, x);
            let near = ((shore - self.sky) / fg).clamp(0.0, 1.0);
            let small = if self.comp.template == Template::TowerPeak {
                0.6
            } else {
                1.0
            };
            let width = (0.06 + 0.16 * p.size) * (0.5 + 0.8 * near) * small;
            // Most rocks sit on the waterline; a few stand further forward.
            let forward = (self.h - shore).max(0.0) * p.row * p.row * p.row;
            let y_base = shore + width * p.offset + forward;
            rocks.push((x, y_base, width * (1.0 + 0.8 * p.row * p.row * p.row), *p));
        }
        rocks.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (ri, (x, y_base, width, p)) in rocks.into_iter().enumerate() {
            let near = ((y_base - self.sky) / (self.h + MARGIN - self.sky)).clamp(0.0, 1.0);
            // Sorted far to near, so depth never increases.
            let d =
                depth::ROCK_FARTHEST - (depth::ROCK_FARTHEST - depth::ROCK_NEAREST) * near as f32;
            self.rock(ts, ri, x, y_base, width, &p, d, f.faceting);
        }
    }

    /// One boulder: a body and three planes (two flanks and a top cap).
    #[allow(clippy::too_many_arguments)]
    fn rock(
        &mut self,
        ts: &TerrainSeeds,
        index: usize,
        x: f64,
        y_base: f64,
        width: f64,
        p: &RockParams,
        d: f32,
        angular: f64,
    ) {
        let tall = width * p.height;
        let n = ((width / (PROFILE_STEP * 0.5)).ceil() as usize).clamp(12, 128);
        let xs = samples(x - 0.5 * width, x + 0.5 * width, width / n as f64);
        let last = xs.len() - 1;
        let apex = p.apex;
        let knots = [
            (0.0, 0.0),
            (apex * p.left[0], p.left[1]),
            (apex * p.left[2], p.left[3]),
            (apex, 1.0),
            (apex + (1.0 - apex) * p.right[0], p.right[1]),
            (apex + (1.0 - apex) * p.right[2], p.right[3]),
            (1.0, 0.0),
        ];
        let detail = self.fbm(ts.rocks ^ index as u64, 0.03, 1.0 / 200.0, 0.5, angular);
        let top: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &xx)| {
                let s = i as f64 / last as f64;
                let facet = piecewise(s, &knots);
                let dd = if s < apex {
                    (apex - s) / apex
                } else {
                    (s - apex) / (1.0 - apex)
                };
                let dome = 1.0 - dd * dd;
                let prof = lerp(dome, facet, angular);
                let bump = 0.05 * 4.0 * s * (1.0 - s) * detail.eval(xx);
                y_base - tall * (prof + bump)
            })
            .collect();
        let bottom: Vec<f64> = (0..=last)
            .map(|i| {
                let s = i as f64 / last as f64;
                y_base + tall * 0.06 * 4.0 * s * (1.0 - s)
            })
            .collect();
        self.push(
            LayerRole::ForegroundRock,
            d,
            0.5,
            band(&xs, &top, Bottom::Pinched(&bottom)),
        );

        // Flanks: creases fall from the apex to the base.
        let ia = ((apex * last as f64).round() as usize).clamp(3, last - 3);
        for left in [true, false] {
            let (i0, i1) = if left { (0, ia) } else { (ia, last) };
            let m = i1 - i0;
            let crease = p.crease[usize::from(!left)];
            let kc = ((crease * m as f64).round() as usize).clamp(1, m);
            let at = if left { i1 - kc } else { i0 + kc };
            let seg_top = &top[i0..=i1];
            let seg_bot = &bottom[i0..=i1];
            let b: Vec<f64> = (0..=m)
                .map(|k| {
                    // Distance from the apex end, 0–1.
                    let u = if left {
                        (m - k) as f64 / m as f64
                    } else {
                        k as f64 / m as f64
                    };
                    let raw = if u < crease {
                        lerp(top[ia], bottom[at], ease(u / crease, angular))
                    } else {
                        seg_bot[k]
                    };
                    let gap = seg_bot[k] - seg_top[k];
                    raw.max(seg_top[k] + 0.02 * gap).min(seg_bot[k])
                })
                .collect();
            let (x0, y0, x1, y1) = if left {
                (xs[0], top[0], xs[ia], top[ia])
            } else {
                (xs[ia], top[ia], xs[last], top[last])
            };
            let shade = face_shade(x1 - x0, y1 - y0, self.comp.light_from_left);
            let shade = 0.5 + (shade - 0.5) * (0.6 + 0.4 * angular);
            self.push(
                LayerRole::ForegroundRock,
                d,
                shade,
                band(&xs[i0..=i1], seg_top, Bottom::Pinched(&b)),
            );
        }

        // Cap: the upper plane around the apex, facing the sky.
        let half = ((p.cap * last as f64).round() as usize).max(2);
        let (c0, c1) = (ia.saturating_sub(half).max(1), (ia + half).min(last - 1));
        if c1 >= c0 + 4 {
            let seg = &top[c0..=c1];
            let (a, z) = (seg[0], seg[seg.len() - 1]);
            let k = seg.len() - 1;
            let b: Vec<f64> = (0..=k)
                .map(|i| {
                    let u = i as f64 / k as f64;
                    // A shallow V below the chord between the cap's ends.
                    let chord = lerp(a, z, u) + tall * 0.12 * (1.0 - (2.0 * u - 1.0).abs());
                    let gap = bottom[c0 + i] - seg[i];
                    chord.max(seg[i] + 0.05 * gap).min(bottom[c0 + i])
                })
                .collect();
            let shade = face_shade(xs[c1] - xs[c0], z - a, self.comp.light_from_left);
            let shade = (shade + 0.1).min(0.95);
            self.push(
                LayerRole::ForegroundRock,
                d,
                0.5 + (shade - 0.5) * (0.6 + 0.4 * angular),
                band(&xs[c0..=c1], seg, Bottom::Pinched(&b)),
            );
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RockParams {
    pos: f64,
    size: f64,
    height: f64,
    apex: f64,
    /// Left flank knots: (x fraction of apex, height) twice.
    left: [f64; 4],
    /// Right flank knots: (x fraction of the rest, height) twice.
    right: [f64; 4],
    /// Base offset in widths; negative stands in the water.
    offset: f64,
    /// How far forward of the waterline (cubed, so most stay on it).
    row: f64,
    crease: [f64; 2],
    cap: f64,
}

/// A smooth periodic wave in `[-1, 1]` with period 1: two parabolic
/// half-waves, continuous in value and slope (a sine stand-in built from
/// exact arithmetic).
fn wave(u: f64) -> f64 {
    let f = u - u.floor();
    if f < 0.5 {
        let g = 2.0 * f;
        4.0 * g * (1.0 - g)
    } else {
        let g = 2.0 * f - 1.0;
        -4.0 * g * (1.0 - g)
    }
}

/// The meander's left bank, right bank and centre (fractions of the width)
/// at nearness `t`: bends swing wider and the channel widens toward the
/// viewer.
fn channel(river: &[f64; 4], t: f64) -> (f64, f64, f64) {
    let [centre, amp, bends, phase] = *river;
    let x = centre + amp * (0.4 + 0.6 * t) * wave(phase + bends * t);
    let half = 0.05 + 0.1 * t;
    (x - half, x + half, x)
}

/// Height of the primary summit above the horizon. Capped in short-side
/// units too, so tall portrait frames keep sky above the summit; the tower
/// may rise higher than the other templates.
fn primary_height(f: &FormSettings, c: &Composition) -> f64 {
    let sky = c.horizon;
    match c.template {
        Template::TowerPeak => (sky * (0.62 + 0.28 * f.relief) * (0.9 + 0.1 * c.peak_scale))
            .min(0.9 * sky)
            .min(MAX_TOWER),
        Template::HighVantage => (sky * (0.45 + 0.4 * f.relief) * (0.85 + 0.3 * c.peak_scale))
            .min(0.85 * sky)
            .min(MAX_SUMMIT),
        _ => (sky * (0.3 + 0.5 * f.relief) * (0.85 + 0.3 * c.peak_scale))
            .min(0.85 * sky)
            .min(MAX_SUMMIT),
    }
}

/// Rises from 0 at both ends to 1 in the middle, flat-topped.
fn envelope(s: f64) -> f64 {
    let q = (2.0 * s - 1.0) * (2.0 * s - 1.0);
    (1.0 - q * q).max(0.0)
}

/// Scalloped tree-crown edge in `(0, 1]`: rounded bumps of wavelength
/// about `wl` with per-crown heights; continuous at the cusps.
fn crowns(seed: u64, x: f64, wl: f64) -> f64 {
    let t = x / wl;
    let k = t.floor();
    let c = 2.0 * (t - k) - 1.0;
    let bump = 1.0 - c * c;
    let hv = 0.6 + 0.4 * unit(seed, k as i64);
    0.65 + 0.35 * bump * hv
}

/// Height and crown-width multipliers of a plant's silhouette.
fn plant_shape(p: Plant) -> (f64, f64) {
    match p {
        Plant::Broadleaf => (1.15, 1.3),
        Plant::Conifer => (1.45, 0.7),
        Plant::Birch => (1.2, 0.8),
        Plant::Shrub => (0.45, 1.4),
        Plant::Flowering => (0.85, 1.0),
        Plant::Copper => (1.1, 1.25),
        Plant::None => (1.0, 1.0),
    }
}

/// A stand's canopy edge in `(0, 1]` by plant: rounded crowns for leafy
/// trees, pointed spires for conifers, low even mounds for shrubs. Each
/// crown has its own height, and the profile is continuous where crowns
/// meet.
fn canopy(p: Plant, seed: u64, x: f64, wl: f64) -> f64 {
    let t = x / wl;
    let k = t.floor();
    let c = 2.0 * (t - k) - 1.0;
    let hv = 0.55 + 0.45 * unit(seed, k as i64);
    match p {
        Plant::Conifer => 0.3 + 0.7 * (1.0 - c.abs()) * hv,
        Plant::Shrub => 0.72 + 0.28 * (1.0 - c * c) * hv,
        Plant::Birch => 0.55 + 0.45 * (1.0 - c * c) * hv,
        Plant::Broadleaf | Plant::Copper | Plant::Flowering => {
            0.5 + 0.5 * (1.0 - c * c).sqrt() * hv
        }
        Plant::None => crowns(seed, x, wl),
    }
}

/// Rounded cloud billows in `[0, 1]`: semicircular bumps of wavelength
/// about `wl` with per-billow heights (`sqrt` is IEEE-exact).
fn billow(seed: u64, x: f64, wl: f64) -> f64 {
    let t = x / wl;
    let k = t.floor();
    let c = 2.0 * (t - k) - 1.0;
    (1.0 - c * c).sqrt() * (0.55 + 0.45 * unit(seed, k as i64))
}

/// Piecewise-linear interpolation through `(s, v)` knots sorted by `s`.
fn piecewise(s: f64, knots: &[(f64, f64)]) -> f64 {
    for pair in knots.windows(2) {
        let ((s0, v0), (s1, v1)) = (pair[0], pair[1]);
        if s <= s1 {
            return lerp(v0, v1, ((s - s0) / (s1 - s0)).clamp(0.0, 1.0));
        }
    }
    knots.last().map_or(0.0, |k| k.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, UHD_4K, UHD_8K};
    use crate::recipe::Recipe;
    use crate::scene::raster;
    use crate::seed::{StreamSeed, TextDigest, Variation};

    const CORPUS: &str = include_str!("../../../../fixtures/passages.json");

    fn corpus() -> Vec<(String, SeedBundle)> {
        let doc: serde_json::Value = serde_json::from_str(CORPUS).unwrap();
        doc.as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let text = p["text"].as_str().unwrap();
                let digest = TextDigest::from_source(text).unwrap();
                (
                    p["id"].as_str().unwrap().to_string(),
                    SeedBundle::derive(digest, Variation(0)),
                )
            })
            .collect()
    }

    fn sample(i: u32) -> SeedBundle {
        let digest = TextDigest::from_source(&format!("sample passage {i}")).unwrap();
        SeedBundle::derive(digest, Variation(0))
    }

    fn scene(seeds: &SeedBundle, form: FormSettings, aspect: AspectRatio) -> Scene {
        LakeshoreGenerator.generate(seeds, &form, aspect).unwrap()
    }

    const ASPECTS: [(u32, u32); 6] = [(16, 9), (9, 16), (1, 1), (3, 2), (4, 1), (1, 4)];

    /// Sample grid following the frame's aspect, fine enough for small
    /// trees even in 1:4 frames.
    fn grid(s: &Scene) -> (usize, usize) {
        let e = s.extents();
        let (w, h) = if e.width >= e.height {
            (128.0 * e.width / e.height, 128.0)
        } else {
            (128.0, 128.0 * e.height / e.width)
        };
        (w as usize, h as usize)
    }

    fn coverage(s: &Scene, role: LayerRole) -> f64 {
        let (gw, gh) = grid(s);
        let cov = raster::role_coverage(s, gw, gh);
        cov[LayerRole::ALL.iter().position(|r| *r == role).unwrap()]
    }

    /// Everything the task asks of every scene. Returns a description of
    /// the first failure.
    fn check(s: &Scene, form: &FormSettings) -> Result<(), String> {
        for (i, l) in s.layers().iter().enumerate() {
            if !raster::is_simple(&l.outline) {
                return Err(format!("layer {i} ({:?}) is not a simple polygon", l.role));
            }
        }
        let first = s.layers().first().map(|l| l.role);
        if first != Some(LayerRole::Sky) {
            return Err("the sky is not the back layer".into());
        }
        let empty = raster::empty_fraction(s, 96, 96);
        if empty != 0.0 {
            return Err(format!("{empty} of the frame is uncovered"));
        }
        let min = [
            (&[LayerRole::Sky, LayerRole::Cloud][..], 0.10),
            (&[LayerRole::Mountain][..], 0.015),
            (&[LayerRole::Water][..], 0.05),
            // Foreground: a near shore with rocks, or the valley's spurs.
            (
                &[
                    LayerRole::Shore,
                    LayerRole::ForegroundRock,
                    LayerRole::NearRidge,
                ][..],
                0.01,
            ),
            (
                &[LayerRole::ForegroundRock, LayerRole::NearRidge][..],
                0.0005,
            ),
        ];
        for (roles, at_least) in min {
            let c: f64 = roles.iter().map(|r| coverage(s, *r)).sum();
            if c < at_least {
                return Err(format!("{roles:?} cover {c:.4}, below {at_least}"));
            }
        }
        if form.woodland_density > 0.0 && coverage(s, LayerRole::Woodland) == 0.0 {
            return Err("no visible woodland".into());
        }
        let (gw, gh) = grid(s);
        let cov = raster::role_coverage(s, gw, gh);
        if let Some(c) = cov.iter().find(|&&c| c > 0.7) {
            return Err(format!("one role covers {c} of the frame"));
        }
        Ok(())
    }

    #[test]
    fn corpus_scenes_are_valid_in_every_orientation() {
        let form = FormSettings::default();
        for (id, seeds) in corpus() {
            for (w, h) in ASPECTS {
                let s = scene(&seeds, form, AspectRatio::of(w, h));
                check(&s, &form).unwrap_or_else(|e| panic!("{id} {w}:{h}: {e}"));
            }
        }
    }

    #[test]
    fn a_larger_seed_sample_is_valid_at_extreme_forms() {
        // 120 seeds × 3 orientations, cycling through the corners of the
        // form space. Measured over 1500 scenes while tuning: sky ≥ 12%,
        // mountain ≥ 2.4%, water ≥ 10%, shore ≥ 1.7%, rocks always visible.
        let corners = [0.0, 1.0];
        for i in 0..120u32 {
            let form = FormSettings {
                faceting: corners[(i % 2) as usize],
                relief: corners[((i / 2) % 2) as usize],
                woodland_density: corners[((i / 4) % 2) as usize],
            };
            let seeds = sample(i);
            for (w, h) in [(16, 9), (9, 16), (1, 1)] {
                let s = scene(&seeds, form, AspectRatio::of(w, h));
                check(&s, &form).unwrap_or_else(|e| panic!("sample {i} {w}:{h} {form:?}: {e}"));
            }
        }
    }

    #[test]
    fn counts_stay_within_bounds() {
        // The widest frame at the densest settings carries the most.
        let form = FormSettings {
            faceting: 1.0,
            relief: 1.0,
            woodland_density: 1.0,
        };
        for i in 0..40 {
            let s = scene(&sample(i), form, AspectRatio::of(4, 1));
            let verts: usize = s.layers().iter().map(|l| l.outline.len()).sum();
            assert!(s.layers().len() <= 90, "{} layers", s.layers().len());
            assert!(verts <= 30_000, "{verts} vertices");
            let planes = s
                .layers()
                .iter()
                .filter(|l| l.role == LayerRole::Mountain)
                .count();
            assert!(planes <= 1 + MAX_MOUNTAIN_PLANES);
        }
    }

    #[test]
    fn silhouettes_stay_coherent() {
        // The mountain's top chain: no spikes between neighbouring samples
        // and bounded total variation, so outlines read as forms, not noise.
        let form = FormSettings {
            faceting: 1.0,
            relief: 1.0,
            ..Default::default()
        };
        for i in 0..60 {
            for (w, h) in [(16, 9), (9, 16)] {
                let aspect = AspectRatio::of(w, h);
                let s = scene(&sample(i), form, aspect);
                let m = s
                    .layers()
                    .iter()
                    .find(|l| l.role == LayerRole::Mountain)
                    .unwrap();
                let top = &m.outline[..m.outline.len() - 2];
                let mut tv = 0.0;
                for p in top.windows(2) {
                    let dy = (p[1].y - p[0].y).abs() as f64;
                    assert!(dy < 0.03, "sample {i}: step of {dy}");
                    tv += dy;
                }
                let width = aspect.extents().width + 2.0 * MARGIN;
                assert!(tv / width < 2.5, "sample {i}: total variation {tv}");
            }
        }
    }

    #[test]
    fn same_recipe_same_scene() {
        for (id, seeds) in corpus() {
            let a = scene(&seeds, FormSettings::default(), UHD_4K.aspect());
            let b = scene(&seeds, FormSettings::default(), UHD_4K.aspect());
            assert_eq!(a, b, "{id}");
            assert_eq!(a.geometry_checksum(), b.geometry_checksum());
        }
    }

    #[test]
    fn checksums_are_frozen() {
        // Exact arithmetic only, so these hold on every OS; portable CI
        // checks them on Linux, Windows and macOS. GENERATOR_VERSION is 0
        // (pre-approval): update them freely until the task 08 visual gate,
        // after which any change needs a version bump.
        let c = corpus();
        let get = |id: &str| &c.iter().find(|(i, _)| i == id).unwrap().1;
        let got = [
            scene(
                get("shore-a"),
                FormSettings::default(),
                AspectRatio::of(16, 9),
            ),
            scene(
                get("non-latin"),
                FormSettings::default(),
                AspectRatio::of(9, 16),
            ),
            scene(
                get("emoji"),
                FormSettings {
                    faceting: 0.0,
                    relief: 1.0,
                    woodland_density: 0.2,
                },
                AspectRatio::of(1, 1),
            ),
        ]
        .map(|s| s.geometry_checksum());
        assert_eq!(got, FROZEN, "{got:x?}");
    }

    const FROZEN: [u64; 3] = [
        0x91a2_336a_07d0_1989,
        0x9143_581e_72ed_10df,
        0xba92_4177_4bd8_86d9,
    ];

    #[test]
    fn appearance_and_paint_seed_never_touch_geometry() {
        let digest = TextDigest::from_source("A pebble rests by the shore.").unwrap();
        let base = Recipe::new(digest, UHD_4K);
        let build = |r: &Recipe| {
            LakeshoreGenerator
                .generate(&r.seeds(), &r.form, r.frame.aspect())
                .unwrap()
                .geometry_checksum()
        };
        let reference = build(&base);
        let mut r = base.clone();
        r.palette.intensity = 0.0;
        r.atmosphere.haze = 1.0;
        r.painting.edge_looseness = 1.0;
        r.painting.wash_gouache = 1.0;
        r.painting.mark_scale = 2.0;
        r.painting.granulation = 1.0;
        r.painting.paper_grain = 1.0;
        assert_eq!(build(&r), reference);
        // A different paint-detail stream with the same structural streams.
        let s = base.seeds();
        let other = SeedBundle::from_streams(
            s.digest,
            s.variation,
            [
                (Domain::Composition, s.stream(Domain::Composition)),
                (Domain::Terrain, s.stream(Domain::Terrain)),
                (Domain::Vegetation, s.stream(Domain::Vegetation)),
                (Domain::PaintDetail, StreamSeed(1)),
            ],
        );
        let g = LakeshoreGenerator
            .generate(&other, &base.form, UHD_4K.aspect())
            .unwrap();
        assert_eq!(g.geometry_checksum(), reference);
    }

    #[test]
    fn pixel_size_keeps_the_scene_and_aspect_recomposes_it() {
        let seeds = &corpus()[0].1;
        let f = FormSettings::default();
        let at = |fr: Frame| scene(seeds, f, fr.aspect()).geometry_checksum();
        assert_eq!(at(UHD_4K), at(UHD_8K));
        assert_eq!(at(UHD_4K), at(Frame::new(1920, 1080).unwrap()));
        assert_ne!(at(UHD_4K), at(UHD_4K.rotated()));
        assert_ne!(at(UHD_4K), at(Frame::new(2000, 2000).unwrap()));
    }

    #[test]
    fn variation_changes_the_composition() {
        let seeds = &corpus()[0].1;
        let f = FormSettings::default();
        let aspect = UHD_4K.aspect();
        let mut templates = std::collections::HashSet::new();
        let mut sums = std::collections::HashSet::new();
        for v in 0..12 {
            let s = SeedBundle::derive(seeds.digest, Variation(v));
            templates.insert(Composition::draw(&s, aspect).template);
            sums.insert(scene(&s, f, aspect).geometry_checksum());
        }
        assert!(templates.len() >= 2, "{templates:?}");
        assert_eq!(sums.len(), 12);
        // Across a seed sample every template appears.
        let all: std::collections::HashSet<_> = (0..60)
            .map(|i| Composition::draw(&sample(i), aspect).template)
            .collect();
        assert_eq!(all.len(), Template::ALL.len());
    }

    #[test]
    fn form_morphs_the_same_composition() {
        let aspect = UHD_4K.aspect();
        for (id, seeds) in corpus() {
            let a = FormSettings::default();
            let b = FormSettings {
                relief: a.relief + 0.01,
                ..a
            };
            let (sa, sb) = (scene(&seeds, a, aspect), scene(&seeds, b, aspect));
            // The main massif moves a little, never jumps.
            let body = |s: &Scene| {
                s.layers()
                    .iter()
                    .find(|l| l.role == LayerRole::Mountain)
                    .unwrap()
                    .outline
                    .clone()
            };
            let (ma, mb) = (body(&sa), body(&sb));
            assert_eq!(ma.len(), mb.len(), "{id}");
            let moved = ma
                .iter()
                .zip(&mb)
                .map(|(p, q)| (p.y - q.y).abs())
                .fold(0.0f32, f32::max);
            assert!(moved > 0.0 && moved < 0.01, "{id}: {moved}");
        }
    }

    #[test]
    fn relief_raises_the_mountain() {
        let aspect = UHD_4K.aspect();
        let summit = |s: &Scene| {
            s.layers()
                .iter()
                .find(|l| l.role == LayerRole::Mountain)
                .unwrap()
                .outline
                .iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min)
        };
        for (id, seeds) in corpus() {
            let low = scene(
                &seeds,
                FormSettings {
                    relief: 0.0,
                    ..Default::default()
                },
                aspect,
            );
            let high = scene(
                &seeds,
                FormSettings {
                    relief: 1.0,
                    ..Default::default()
                },
                aspect,
            );
            assert!(summit(&high) < summit(&low) - 0.1, "{id}");
        }
    }

    #[test]
    fn faceting_changes_shape_and_planes() {
        let aspect = UHD_4K.aspect();
        let planes = |s: &Scene| {
            s.layers()
                .iter()
                .filter(|l| l.role == LayerRole::Mountain)
                .count()
        };
        let mut more = 0;
        for (id, seeds) in corpus() {
            let round = scene(
                &seeds,
                FormSettings {
                    faceting: 0.0,
                    ..Default::default()
                },
                aspect,
            );
            let sharp = scene(
                &seeds,
                FormSettings {
                    faceting: 1.0,
                    ..Default::default()
                },
                aspect,
            );
            assert_ne!(round.geometry_checksum(), sharp.geometry_checksum(), "{id}");
            if planes(&sharp) > planes(&round) {
                more += 1;
            }
        }
        assert!(
            more >= 8,
            "faceted scenes had more planes in only {more} of 10"
        );
    }

    #[test]
    fn woodland_density_scales_the_woodland_regions() {
        let aspect = UHD_4K.aspect();
        let mut denser = 0;
        for (_, seeds) in corpus() {
            let sparse = scene(
                &seeds,
                FormSettings {
                    woodland_density: 0.0,
                    ..Default::default()
                },
                aspect,
            );
            let dense = scene(
                &seeds,
                FormSettings {
                    woodland_density: 1.0,
                    ..Default::default()
                },
                aspect,
            );
            let (a, b) = (
                coverage(&sparse, LayerRole::Woodland),
                coverage(&dense, LayerRole::Woodland),
            );
            assert!(a < 0.05, "sparse woodland covers {a}");
            if b > a {
                denser += 1;
            }
        }
        assert_eq!(denser, 10);
    }

    #[test]
    fn layers_run_back_to_front_by_role() {
        let order = |r: LayerRole| match r {
            LayerRole::Sky | LayerRole::Cloud => 0,
            LayerRole::FarRidge => 1,
            LayerRole::Mountain => 2,
            LayerRole::MidRidge => 3,
            LayerRole::Woodland | LayerRole::NearRidge | LayerRole::Shore => 4,
            LayerRole::Water => 5,
            LayerRole::ForegroundRock => 7,
        };
        for (id, seeds) in corpus() {
            let s = scene(&seeds, FormSettings::default(), UHD_4K.aspect());
            let roles: Vec<LayerRole> = s.layers().iter().map(|l| l.role).collect();
            let water = roles.iter().position(|r| *r == LayerRole::Water).unwrap();
            // Before the water everything is behind it; after it, only the
            // near shore, valley spurs, near woods and rocks.
            for r in &roles[..water] {
                assert!(order(*r) <= 4, "{id}: {r:?} before the water");
            }
            for r in &roles[water + 1..] {
                assert!(
                    matches!(
                        r,
                        LayerRole::Shore
                            | LayerRole::Woodland
                            | LayerRole::ForegroundRock
                            | LayerRole::NearRidge
                    ),
                    "{id}: {r:?} in front of the water"
                );
            }
            // Clouds only ever sit directly in front of the sky.
            let clouds = roles.iter().filter(|r| **r == LayerRole::Cloud).count();
            assert!(
                roles[1..=clouds].iter().all(|r| *r == LayerRole::Cloud),
                "{id}"
            );
            assert!(
                roles[..water]
                    .windows(2)
                    .all(|p| order(p[0]) <= order(p[1])),
                "{id}"
            );
        }
    }

    #[test]
    fn vista_templates_score_higher_on_their_devices() {
        // Averages over a seed sample at 16:9, default form. The tower must
        // win on height and scale, the high vantage on planes and expanse.
        use crate::scene::metrics::measure;
        let aspect = UHD_4K.aspect();
        let mut by: std::collections::HashMap<Template, Vec<crate::scene::metrics::AweMetrics>> =
            Default::default();
        for i in 0..80 {
            let seeds = sample(i);
            let t = Composition::draw(&seeds, aspect).template;
            let m = measure(&scene(&seeds, FormSettings::default(), aspect));
            by.entry(t).or_default().push(m);
        }
        let mean = |ts: &[Template], f: &dyn Fn(&crate::scene::metrics::AweMetrics) -> f64| {
            let v: Vec<f64> = ts
                .iter()
                .flat_map(|t| by.get(t).into_iter().flatten())
                .map(f)
                .collect();
            v.iter().sum::<f64>() / v.len() as f64
        };
        let classic = [
            Template::PeakOverWater,
            Template::FramingRidges,
            Template::TwinSummits,
        ];
        let tower = [Template::TowerPeak];
        let high = [Template::HighVantage];
        let rise = |m: &crate::scene::metrics::AweMetrics| m.summit_rise;
        let scale = |m: &crate::scene::metrics::AweMetrics| m.scale_ratio.unwrap_or(0.0);
        let planes = |m: &crate::scene::metrics::AweMetrics| m.planes as f64;
        let expanse = |m: &crate::scene::metrics::AweMetrics| m.expanse;
        assert!(mean(&tower, &rise) > 1.3 * mean(&classic, &rise));
        assert!(mean(&tower, &scale) > 2.5 * mean(&classic, &scale));
        assert!(mean(&high, &planes) > mean(&classic, &planes) + 3.0);
        assert!(mean(&high, &expanse) > 1.5 * mean(&classic, &expanse));
    }

    #[test]
    fn dramatic_light_stages_contrast() {
        use crate::scene::metrics::measure;
        let aspect = UHD_4K.aspect();
        let (mut calm, mut drama) = (Vec::new(), Vec::new());
        for i in 0..60 {
            let seeds = sample(i);
            let c = Composition::draw(&seeds, aspect);
            let m = measure(&scene(&seeds, FormSettings::default(), aspect));
            if c.dramatic { &mut drama } else { &mut calm }.push(m.light_contrast);
        }
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(!calm.is_empty() && !drama.is_empty());
        assert!(
            avg(&drama) > avg(&calm),
            "{} vs {}",
            avg(&drama),
            avg(&calm)
        );
    }

    #[test]
    fn woodland_stands_have_varied_plants() {
        let aspect = UHD_4K.aspect();
        let mut seen = std::collections::HashSet::new();
        // (plant, depth band) -> canopy heights, to compare like with like.
        let mut heights: std::collections::HashMap<Plant, Vec<f64>> = Default::default();
        for i in 0..80 {
            let s = scene(&sample(i), FormSettings::default(), aspect);
            for l in s.layers() {
                if l.role == LayerRole::Woodland {
                    assert_ne!(l.plant, Plant::None, "sample {i}: woodland without a plant");
                    seen.insert(l.plant);
                    if l.depth == depth::FAR_WOODS {
                        // An open band: the top chain, then the bottom chain
                        // reversed. Canopy height is their largest gap,
                        // less the 0.006 the band reaches below the water.
                        let o = &l.outline;
                        let n = o.len() / 2;
                        let canopy = (0..n)
                            .map(|i| (o[o.len() - 1 - i].y - o[i].y) as f64 - 0.006)
                            .fold(0.0, f64::max);
                        heights.entry(l.plant).or_default().push(canopy);
                    }
                } else {
                    assert_eq!(l.plant, Plant::None, "sample {i}: {:?} has a plant", l.role);
                }
            }
        }
        assert_eq!(seen.len(), Plant::TREES.len(), "{seen:?}");
        let mean = |p: Plant| {
            let v = &heights[&p];
            v.iter().sum::<f64>() / v.len() as f64
        };
        // Tall trees stand over squat shrubs at the same distance.
        assert!(mean(Plant::Conifer) > 1.8 * mean(Plant::Shrub));
        assert!(mean(Plant::Broadleaf) > 1.5 * mean(Plant::Shrub));
    }

    #[test]
    fn plant_weights_cover_the_unit_interval() {
        let mut counts = std::collections::HashMap::new();
        for k in 0..1000 {
            *counts.entry(Plant::pick(k as f64 / 1000.0)).or_insert(0) += 1;
        }
        assert_eq!(counts.len(), 6);
        assert_eq!(counts[&Plant::Broadleaf], 260);
        assert_eq!(counts[&Plant::Copper], 80);
    }

    #[test]
    fn meanders_are_smooth() {
        // The channel wave is continuous in value and slope (no kinks), so
        // bends are smooth like an eroded river's.
        let h = 1e-6;
        for k in 0..400 {
            let u = k as f64 / 100.0;
            let (a, b) = (wave(u - h), wave(u + h));
            // |slope| <= 8, so a continuous wave moves at most 16h over 2h.
            assert!((a - b).abs() <= 16.0 * h + 1e-12, "jump at {u}");
            let s1 = (wave(u) - wave(u - h)) / h;
            let s2 = (wave(u + h) - wave(u)) / h;
            assert!((s1 - s2).abs() < 1e-3, "kink at {u}: {s1} vs {s2}");
            assert!((-1.0..=1.0).contains(&wave(u)));
        }
    }

    #[test]
    fn valley_spurs_have_blunt_tips() {
        // A wedge closes linearly toward its tip; an eroded spur keeps most
        // of its thickness until it rounds off. Measured on the band's gap
        // (bottom chain minus top chain) at 85 % of the way to the tip.
        let aspect = UHD_4K.aspect();
        let mut spurs = 0;
        for i in 0..80 {
            let seeds = sample(i);
            if Composition::draw(&seeds, aspect).template != Template::HighVantage {
                continue;
            }
            let s = scene(&seeds, FormSettings::default(), aspect);
            for l in s.layers().iter().filter(|l| l.role == LayerRole::NearRidge) {
                let o = &l.outline;
                let n = o.len() / 2;
                let gap = |i: usize| (o[o.len() - 1 - i].y - o[i].y) as f64;
                // Chains run from the frame edge to the tip or the reverse.
                let (edge, tip) = if o[0].x < o[n - 1].x && o[0].x < 0.0 {
                    (0, n - 1)
                } else {
                    (n - 1, 0)
                };
                let at = |f: f64| {
                    let i = edge as f64 + (tip as f64 - edge as f64) * f;
                    gap(i.round() as usize)
                };
                assert!(
                    at(0.85) > 0.3 * at(0.5),
                    "sample {i}: tip closes like a wedge ({} vs {})",
                    at(0.85),
                    at(0.5)
                );
                spurs += 1;
            }
        }
        assert!(spurs > 40, "only {spurs} spurs checked");
    }

    #[test]
    fn generators_use_only_exact_arithmetic() {
        // Tier 1 forbids platform transcendental functions in generators.
        for (name, src) in [
            ("lakeshore.rs", include_str!("lakeshore.rs")),
            ("noise.rs", include_str!("noise.rs")),
        ] {
            // Only the code before the test module (this list is in it).
            let code = src.split("#[cfg(test)]").next().unwrap();
            let code: String = code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for f in [
                ".sin(",
                ".cos(",
                ".tan(",
                ".exp(",
                ".ln(",
                ".log",
                ".pow",
                ".atan",
                ".cbrt(",
                ".hypot(",
                ".mul_add(",
                ".exp2(",
                ".asin(",
                ".acos(",
            ] {
                assert!(!code.contains(f), "{name} calls {f}");
            }
        }
    }
}
