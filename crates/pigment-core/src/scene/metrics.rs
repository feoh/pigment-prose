//! Awe proxies computed from scene geometry (task 05b).
//!
//! Awe is judged by a person. These numbers are the *measurable parts* of
//! the devices landscape painters use for vastness (scale cues, height, the
//! expanse laid out below a high viewpoint, framing, sky, light), so a
//! contact sheet can say how strongly a scene uses each one. They are logged
//! next to the user's ratings in `docs/visual-review/` and recalibrated
//! against them; a high number is never a substitute for that judgement.
//! See `docs/art-direction.md`, "Awe".

use std::fmt;

use super::raster::{self, NONE};
use super::{LayerRole, Scene};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AweMetrics {
    /// Highest summit above the far shoreline, as a fraction of the frame
    /// height ("high distance").
    pub summit_rise: f64,
    /// Summit rise ÷ median height of the trees at the mountain's foot
    /// (woodland at depth ≥ 0.5). `None` without such trees. The eye reads
    /// the mountain's size from them: near 1 it reads as a hill.
    pub scale_ratio: Option<f64>,
    /// Distinct visible depth planes among ridges and spurs.
    pub planes: usize,
    /// Visible fraction of the frame that is lake and valley laid out below
    /// the horizon: water, valley spurs and their trees ("level distance").
    pub expanse: f64,
    /// Fraction of the central third of the frame taken by near foreground
    /// (rocks and trees at depth ≤ 0.25). Low keeps the view open; framing
    /// belongs at the edges.
    pub centre_clutter: f64,
    /// Visible cloud coverage of the frame.
    pub sky_structure: f64,
    /// Spread of `shade` across layers covering at least 0.5 % of the frame
    /// (sky excluded): how strongly light and shadow are staged.
    pub light_contrast: f64,
    /// Share of the bottom third of the frame that is vegetated land
    /// (spurs, ridges, woodland, meadow shore): a foreground rich enough to
    /// draw the eye and hold it (round 4 favourites, 2026-09-27).
    pub foreground: f64,
}

impl fmt::Display for AweMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = self
            .scale_ratio
            .map_or("-".to_string(), |r| format!("{r:.0}"));
        write!(
            f,
            "rise {:.2} scale {scale} planes {} expanse {:.2} clutter {:.2} sky {:.2} light {:.2} foreground {:.2}",
            self.summit_rise,
            self.planes,
            self.expanse,
            self.centre_clutter,
            self.sky_structure,
            self.light_contrast,
            self.foreground
        )
    }
}

/// Column headers matching [`AweMetrics::columns`].
pub const COLUMNS: &str = "rise\tscale\tplanes\texpanse\tclutter\tsky\tlight\tforeground";

impl AweMetrics {
    /// Tab-separated values for notes and rating sheets.
    pub fn columns(&self) -> String {
        format!(
            "{:.2}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
            self.summit_rise,
            self.scale_ratio
                .map_or("-".to_string(), |r| format!("{r:.0}")),
            self.planes,
            self.expanse,
            self.centre_clutter,
            self.sky_structure,
            self.light_contrast,
            self.foreground
        )
    }
}

/// Canvas y of the far shoreline (the top of the water inside the frame)
/// and of the highest visible summit. Used by the metrics and by the
/// painting renderer (treeline and snowline).
pub fn horizon_and_summit(scene: &Scene) -> (f64, f64) {
    let ext = scene.extents();
    let (w, h) = (ext.width, ext.height);
    let top_of = |role: LayerRole| {
        scene
            .layers()
            .iter()
            .filter(|l| l.role == role)
            .flat_map(|l| l.outline.iter())
            .filter(|p| (0.0..=w as f32).contains(&p.x))
            .map(|p| p.y as f64)
            .fold(f64::INFINITY, f64::min)
    };
    let horizon = top_of(LayerRole::Water).min(h);
    (horizon, top_of(LayerRole::Mountain).min(horizon))
}

pub fn measure(scene: &Scene) -> AweMetrics {
    let ext = scene.extents();
    let (w, h) = (ext.width, ext.height);
    let layers = scene.layers();
    let (horizon, summit) = horizon_and_summit(scene);
    let rise = (horizon - summit).max(0.0);

    let mut trees: Vec<f64> = layers
        .iter()
        .filter(|l| l.role == LayerRole::Woodland && l.depth >= 0.5)
        .map(|l| {
            let (lo, hi) = l
                .outline
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                    (a.min(p.y as f64), b.max(p.y as f64))
                });
            (hi - lo - 0.006).max(0.001)
        })
        .collect();
    trees.sort_by(f64::total_cmp);
    let scale_ratio = (!trees.is_empty()).then(|| rise / trees[trees.len() / 2]);

    let gw = 128usize;
    let gh = ((gw as f64 * h / w).round() as usize).clamp(32, 512);
    let ids = raster::front_layers(scene, gw, gh);
    let mut counts = vec![0usize; layers.len()];
    let mut centre = (0usize, 0usize);
    let mut bottom = (0usize, 0usize);
    for (i, &id) in ids.iter().enumerate() {
        if id == NONE {
            continue;
        }
        if i / gw >= gh * 2 / 3 {
            bottom.1 += 1;
            let l = &layers[id as usize];
            let land = match l.role {
                LayerRole::NearRidge | LayerRole::MidRidge | LayerRole::Woodland => true,
                LayerRole::Shore => l.depth < 0.45,
                _ => false,
            };
            if land {
                bottom.0 += 1;
            }
        }
        counts[id as usize] += 1;
        let col = i % gw;
        if col >= gw / 3 && col < 2 * gw / 3 {
            centre.1 += 1;
            let l = &layers[id as usize];
            if matches!(l.role, LayerRole::ForegroundRock | LayerRole::Woodland) && l.depth <= 0.25
            {
                centre.0 += 1;
            }
        }
    }
    let total = ids.len() as f64;
    let cov = |i: usize| counts[i] as f64 / total;
    let mut depths: Vec<f32> = Vec::new();
    let (mut expanse, mut sky_structure) = (0.0, 0.0);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (i, l) in layers.iter().enumerate() {
        let c = cov(i);
        match l.role {
            LayerRole::FarRidge
            | LayerRole::Mountain
            | LayerRole::MidRidge
            | LayerRole::NearRidge
                if c >= 0.002 && !depths.contains(&l.depth) =>
            {
                depths.push(l.depth);
            }
            _ => {}
        }
        let valley = l.role == LayerRole::Water
            || (matches!(l.role, LayerRole::NearRidge | LayerRole::Woodland)
                && (0.14..=0.45).contains(&l.depth));
        if valley {
            expanse += c;
        }
        if l.role == LayerRole::Cloud {
            sky_structure += c;
        }
        if l.role != LayerRole::Sky && c >= 0.005 {
            lo = lo.min(l.shade as f64);
            hi = hi.max(l.shade as f64);
        }
    }
    AweMetrics {
        summit_rise: rise / h,
        scale_ratio,
        planes: depths.len(),
        expanse,
        centre_clutter: if centre.1 > 0 {
            centre.0 as f64 / centre.1 as f64
        } else {
            0.0
        },
        sky_structure,
        light_contrast: if hi >= lo { hi - lo } else { 0.0 },
        foreground: if bottom.1 > 0 {
            bottom.0 as f64 / bottom.1 as f64
        } else {
            0.0
        },
    }
}
