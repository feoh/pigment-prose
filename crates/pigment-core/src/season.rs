//! The cyclic year (task 16; docs/seasons-and-biomes.md).
//!
//! A season is a point on a normalized year `t` in `[0, 1]`, where 0 and 1
//! are the same moment (midwinter): 0.25 is the spring equinox, 0.5
//! midsummer and 0.75 the autumn equinox. A biome's [`SeasonProfile`] gives
//! a few keyframes of its [`SeasonState`], and [`SeasonProfile::at`] blends
//! the two around `t` with a smoothstep. So every channel:
//!
//! - is **periodic**: the last keyframe blends into the first across the
//!   wrap, so `at(0) == at(1)` and the values meet with a matching (zero)
//!   slope;
//! - is **C¹ continuous**, with no jumps anywhere in the year;
//! - is **bounded**: a convex blend of two keyframes, never overshooting
//!   the values the profile authored;
//! - lands **exactly** on a keyframe at its time. The alpine profile's
//!   midsummer keyframe is [`SeasonState::NEUTRAL`], which the painter
//!   treats as "no seasonal change", so the default season paints exactly
//!   the approved midsummer look.
//!
//! Seasons change appearance only. They never touch the scene: geography,
//! plant placement and the geometry checksum are the same all year.
//! Stylized, not simulated: the channels are painterly quantities, not
//! weather or ecology.

/// A biome's seasonal channels at one moment, each in `[0, 1]`. Kept apart
/// so each can be reasoned about (and later biomes can use only some):
///
/// - `snow`: how far down the terrain seasonal snow reaches. 0 is only the
///   permanent snow on high peaks; 1 reaches the valley floor.
/// - `ground_snow`: snow lying on meadows, shores and rocks.
/// - `tree_snow`: snow on crowns.
/// - `leaf`: how much of the deciduous canopy is in leaf (0 bare).
/// - `autumn`: how far deciduous leaves have turned (0 green).
/// - `fresh`: spring's pale, fresh green on new leaves and grass.
/// - `dry`: grass cured to straw.
/// - `bloom`: wildflowers and blossom, relative to midsummer (1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonState {
    pub snow: f64,
    pub ground_snow: f64,
    pub tree_snow: f64,
    pub leaf: f64,
    pub autumn: f64,
    pub fresh: f64,
    pub dry: f64,
    pub bloom: f64,
}

impl SeasonState {
    /// No seasonal change: the approved midsummer painting.
    pub const NEUTRAL: SeasonState = SeasonState {
        snow: 0.0,
        ground_snow: 0.0,
        tree_snow: 0.0,
        leaf: 1.0,
        autumn: 0.0,
        fresh: 0.0,
        dry: 0.0,
        bloom: 1.0,
    };

    pub fn channels(&self) -> [f64; 8] {
        [
            self.snow,
            self.ground_snow,
            self.tree_snow,
            self.leaf,
            self.autumn,
            self.fresh,
            self.dry,
            self.bloom,
        ]
    }

    fn from_channels(c: [f64; 8]) -> SeasonState {
        SeasonState {
            snow: c[0],
            ground_snow: c[1],
            tree_snow: c[2],
            leaf: c[3],
            autumn: c[4],
            fresh: c[5],
            dry: c[6],
            bloom: c[7],
        }
    }

    /// The painter's uniform: two `vec4`s in channel order.
    pub fn gpu(&self) -> [[f32; 4]; 2] {
        let c = self.channels().map(|v| v as f32);
        [[c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]]
    }
}

/// What kind of year a biome has (the extension contract for later biomes,
/// docs/seasons-and-biomes.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    /// Winter, spring, summer, autumn: snow and deciduous leaf fall.
    Temperate,
    /// A dry season and a wet/growth season, with no snow.
    DryWet,
}

/// A biome's year: keyframes `(t, state)` in increasing `t` within
/// `[0, 1)`. Between keyframes (and across the wrap from the last to the
/// first) the state is a smoothstep blend of its two neighbours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonProfile {
    pub name: &'static str,
    pub cycle: Cycle,
    pub keys: &'static [(f64, SeasonState)],
}

/// A keyframe's state from its channels, in [`SeasonState`] order.
const fn state(c: [f64; 8]) -> SeasonState {
    SeasonState {
        snow: c[0],
        ground_snow: c[1],
        tree_snow: c[2],
        leaf: c[3],
        autumn: c[4],
        fresh: c[5],
        dry: c[6],
        bloom: c[7],
    }
}

/// The alpine lakeshore and wooded valleys (the approved landscapes). A
/// stylized northern mountain year: a long snowy winter, a late green
/// spring with the snow line retreating up the slopes, a short dry late
/// summer, a bright autumn, and the first snow on the heights before the
/// leaves are all down.
pub const ALPINE: SeasonProfile = SeasonProfile {
    name: "alpine lakeshore",
    cycle: Cycle::Temperate,
    keys: &[
        //            snow  ground tree  leaf  autumn fresh dry   bloom
        (
            0.00,
            state([1.00, 1.00, 0.70, 0.00, 1.00, 0.00, 1.00, 0.00]),
        ), // midwinter
        (
            0.14,
            state([0.85, 0.70, 0.30, 0.00, 1.00, 0.00, 0.80, 0.00]),
        ), // late winter
        (
            0.23,
            state([0.50, 0.05, 0.00, 0.55, 0.00, 1.00, 0.15, 0.80]),
        ), // early spring
        (
            0.35,
            state([0.25, 0.00, 0.00, 0.95, 0.00, 0.60, 0.00, 1.00]),
        ), // late spring
        (0.50, SeasonState::NEUTRAL), // midsummer
        (
            0.62,
            state([0.00, 0.00, 0.00, 1.00, 0.08, 0.00, 0.40, 0.55]),
        ), // late summer
        (
            0.75,
            state([0.10, 0.00, 0.00, 0.90, 0.80, 0.00, 0.65, 0.10]),
        ), // autumn
        (
            0.86,
            state([0.45, 0.15, 0.10, 0.35, 1.00, 0.00, 0.90, 0.00]),
        ), // late autumn
    ],
};

/// A restrained stylized rocky-desert cycle: brief, subdued wet-season
/// greening and flowering; a long dry season. It does not imply snow, a
/// tropical wet season or an ecological/weather simulation. The phase is an
/// authored appearance control, independent of scene geometry.
pub const DESERT: SeasonProfile = SeasonProfile {
    name: "rocky desert",
    cycle: Cycle::DryWet,
    keys: &[
        //            snow  ground tree  leaf  autumn fresh dry   bloom
        (0.00, state([0.0, 0.0, 0.0, 0.72, 0.0, 0.0, 0.15, 0.45])), // cool season
        (0.20, state([0.0, 0.0, 0.0, 0.88, 0.0, 0.75, 0.0, 0.85])), // brief growth
        (0.38, state([0.0, 0.0, 0.0, 0.76, 0.0, 0.2, 0.55, 0.35])), // drying
        (0.58, state([0.0, 0.0, 0.0, 0.62, 0.0, 0.0, 1.0, 0.05])),  // peak dry
        (0.78, state([0.0, 0.0, 0.0, 0.68, 0.0, 0.0, 0.75, 0.1])),  // late dry
    ],
};

/// Tropical jungle: a lush wet/growth period followed by a shorter dry spell.
/// Wet/dry is an appearance cycle, not a precipitation or ecological model.
pub const JUNGLE: SeasonProfile = SeasonProfile {
    name: "tropical jungle",
    cycle: Cycle::DryWet,
    keys: &[
        //            snow  ground tree  leaf  autumn fresh dry   bloom
        (0.00, state([0.0, 0.0, 0.0, 0.96, 0.0, 0.15, 0.0, 0.65])),
        (0.20, state([0.0, 0.0, 0.0, 1.00, 0.0, 0.35, 0.0, 0.85])),
        (0.46, state([0.0, 0.0, 0.0, 1.00, 0.0, 0.0, 0.05, 0.70])),
        (0.66, state([0.0, 0.0, 0.0, 0.88, 0.0, 0.0, 0.62, 0.20])),
        (0.82, state([0.0, 0.0, 0.0, 0.82, 0.0, 0.0, 1.00, 0.05])),
    ],
};

/// Open tundra: a long, bright snow season and a very short growing window.
/// Snow covers the low ground from autumn through late spring; at midsummer
/// the new biome's base painting is the neutral, briefly snow-free look.
pub const TUNDRA: SeasonProfile = SeasonProfile {
    name: "open tundra",
    cycle: Cycle::Temperate,
    keys: &[
        //            snow  ground tree  leaf  autumn fresh dry   bloom
        (
            0.00,
            state([1.00, 1.00, 0.20, 0.00, 0.00, 0.00, 0.85, 0.00]),
        ), // midwinter
        (
            0.15,
            state([0.90, 0.90, 0.15, 0.00, 0.00, 0.00, 0.65, 0.00]),
        ), // deep snow
        (
            0.27,
            state([0.55, 0.58, 0.05, 0.25, 0.00, 0.25, 0.30, 0.10]),
        ), // thaw
        (
            0.39,
            state([0.12, 0.12, 0.00, 0.72, 0.00, 0.88, 0.00, 0.65]),
        ), // short spring
        (0.50, SeasonState::NEUTRAL), // brief, snow-free high summer
        (
            0.63,
            state([0.02, 0.00, 0.00, 1.00, 0.12, 0.00, 0.25, 0.35]),
        ), // late summer
        (
            0.75,
            state([0.20, 0.18, 0.05, 0.70, 0.70, 0.00, 0.55, 0.02]),
        ), // first snow
        (
            0.88,
            state([0.72, 0.70, 0.15, 0.18, 0.15, 0.00, 0.80, 0.00]),
        ), // long autumn
    ],
};

/// The default season: midsummer, the approved look.
pub const DEFAULT_YEAR: f64 = 0.5;

/// The season names, one per twelfth of the year, centred on their times.
const LABELS: [&str; 12] = [
    "midwinter",
    "late winter",
    "early spring",
    "spring",
    "late spring",
    "early summer",
    "midsummer",
    "late summer",
    "early autumn",
    "autumn",
    "late autumn",
    "early winter",
];

/// `t` wrapped into `[0, 1)`.
pub fn wrap(t: f64) -> f64 {
    let w = t - t.floor();
    if w >= 1.0 { 0.0 } else { w }
}

/// The name of the season at `t`, for labels and value text.
pub fn label(t: f64) -> &'static str {
    LABELS[((wrap(t) * 12.0).round() as usize) % 12]
}

impl SeasonProfile {
    /// The blended state at `t` (any real; wrapped).
    pub fn at(&self, t: f64) -> SeasonState {
        let t = wrap(t);
        let n = self.keys.len();
        // The keyframe at or before t (the last one, across the wrap, if t
        // is before the first).
        let i = self
            .keys
            .iter()
            .rposition(|(k, _)| *k <= t)
            .unwrap_or(n - 1);
        let (t0, a) = self.keys[i];
        let (t1, b) = self.keys[(i + 1) % n];
        let span = if t1 > t0 { t1 - t0 } else { t1 + 1.0 - t0 };
        let since = if t >= t0 { t - t0 } else { t + 1.0 - t0 };
        let u = (since / span).clamp(0.0, 1.0);
        let w = u * u * (3.0 - 2.0 * u);
        let (ca, cb) = (a.channels(), b.channels());
        SeasonState::from_channels(std::array::from_fn(|k| ca[k] + (cb[k] - ca[k]) * w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: SeasonState, b: SeasonState, tol: f64) -> bool {
        a.channels()
            .iter()
            .zip(b.channels())
            .all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn keyframes_are_ordered_in_range_and_bounded() {
        let k = ALPINE.keys;
        assert!(k.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(k.iter().all(|(t, _)| (0.0..1.0).contains(t)));
        for (_, s) in k {
            assert!(
                s.channels().iter().all(|v| (0.0..=1.0).contains(v)),
                "{s:?}"
            );
        }
    }

    #[test]
    fn midsummer_is_exactly_neutral() {
        assert_eq!(ALPINE.at(DEFAULT_YEAR), SeasonState::NEUTRAL);
        assert_eq!(label(DEFAULT_YEAR), "midsummer");
    }

    #[test]
    fn the_year_wraps_continuously() {
        assert_eq!(ALPINE.at(0.0), ALPINE.at(1.0));
        assert_eq!(ALPINE.at(0.25), ALPINE.at(1.25));
        assert_eq!(ALPINE.at(-0.75), ALPINE.at(0.25));
        // Either side of the wrap meet: values and (zero) slope.
        for eps in [1e-3, 1e-5] {
            assert!(close(ALPINE.at(1.0 - eps), ALPINE.at(eps), 1e-4), "{eps}");
        }
        let d = |t: f64| {
            let h = 1e-6;
            let (a, b) = (ALPINE.at(t - h).channels(), ALPINE.at(t + h).channels());
            a.iter()
                .zip(b)
                .map(|(x, y)| (y - x) / (2.0 * h))
                .collect::<Vec<_>>()
        };
        for (l, r) in d(1.0 - 1e-4).iter().zip(d(1e-4)) {
            // Both sides of the wrap sit next to the midwinter keyframe,
            // where every blend's slope goes to zero.
            assert!(l.abs() < 0.05 && r.abs() < 0.05, "slope {l} vs {r}");
        }
    }

    #[test]
    fn every_channel_is_continuous_and_bounded_all_year() {
        // Dense sampling: no step bigger than the steepest smoothstep allows,
        // and every value between the two keyframes it blends.
        let steps = 20_000;
        let k = ALPINE.keys;
        let min_span = (0..k.len())
            .map(|i| {
                let (a, b) = (k[i].0, k[(i + 1) % k.len()].0);
                if b > a { b - a } else { b + 1.0 - a }
            })
            .fold(1.0, f64::min);
        let mut prev = ALPINE.at(0.0);
        let (lo, hi) = ALPINE
            .keys
            .iter()
            .fold(([1.0f64; 8], [0.0f64; 8]), |(lo, hi), (_, s)| {
                let c = s.channels();
                (
                    std::array::from_fn(|k| lo[k].min(c[k])),
                    std::array::from_fn(|k| hi[k].max(c[k])),
                )
            });
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let s = ALPINE.at(t);
            for (k, v) in s.channels().iter().enumerate() {
                assert!(
                    *v >= lo[k] - 1e-12 && *v <= hi[k] + 1e-12,
                    "t {t} channel {k}"
                );
            }
            // Max slope of a smoothstep blend is 1.5 × Δ / span, with Δ ≤ 1.
            assert!(
                close(s, prev, 1.5 / min_span / steps as f64 + 1e-9),
                "jump at {t}"
            );
            prev = s;
        }
    }

    #[test]
    fn keyframes_are_hit_exactly() {
        for (t, s) in ALPINE.keys {
            assert_eq!(ALPINE.at(*t), *s);
        }
    }

    #[test]
    fn tundra_cycle_has_a_long_snow_season_and_a_short_growth_window() {
        assert_eq!(TUNDRA.at(0.0), TUNDRA.at(1.0));
        assert_eq!(TUNDRA.at(0.5), SeasonState::NEUTRAL);
        assert!(TUNDRA.at(0.15).snow > TUNDRA.at(0.5).snow);
        assert!(TUNDRA.at(0.39).fresh > TUNDRA.at(0.15).fresh);
        assert!(TUNDRA.at(0.88).ground_snow > TUNDRA.at(0.63).ground_snow);
        for i in 0..=10_000 {
            assert!(
                TUNDRA
                    .at(i as f64 / 10_000.0)
                    .channels()
                    .iter()
                    .all(|v| (0.0..=1.0).contains(v))
            );
        }
    }

    #[test]
    fn desert_cycle_is_dry_wet_continuous_and_never_snowy() {
        assert_eq!(DESERT.at(0.0), DESERT.at(1.0));
        assert!(DESERT.at(0.20).fresh > DESERT.at(0.58).fresh);
        assert!(DESERT.at(0.58).dry > DESERT.at(0.20).dry);
        assert!(DESERT.at(0.20).bloom > DESERT.at(0.58).bloom);

        for i in 0..=10_000 {
            let state = DESERT.at(i as f64 / 10_000.0);
            assert_eq!(state.snow, 0.0);
            assert_eq!(state.ground_snow, 0.0);
            assert_eq!(state.tree_snow, 0.0);
            assert!(
                state
                    .channels()
                    .iter()
                    .all(|value| (0.0..=1.0).contains(value))
            );
        }
    }

    #[test]
    fn labels_name_the_quarters() {
        assert_eq!(label(0.0), "midwinter");
        assert_eq!(label(1.0), "midwinter");
        assert_eq!(label(0.999), "midwinter");
        assert_eq!(label(0.25), "spring");
        assert_eq!(label(0.75), "autumn");
        assert_eq!(label(0.62), "late summer");
    }

    #[test]
    fn winter_has_snow_and_bare_trees_and_summer_has_neither() {
        let w = ALPINE.at(0.0);
        assert!(w.snow > 0.9 && w.ground_snow > 0.9 && w.leaf < 0.1);
        let s = ALPINE.at(0.5);
        assert!(s.snow == 0.0 && s.ground_snow == 0.0 && s.leaf == 1.0);
        let a = ALPINE.at(0.75);
        assert!(a.autumn > 0.5 && a.ground_snow == 0.0);
    }
}
