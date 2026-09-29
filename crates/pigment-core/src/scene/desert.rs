//! Deterministic rocky-desert candidate using the shared scene and paint contracts.
//!
//! The composition is a layered basin with broad flat-topped mesas, exposed
//! foreground shelves, and sparse low shrubs. It deliberately contains no
//! lake, forest, climate simulation, or new paint primitive.

use super::noise::{Fbm, unit};
use super::{
    CanvasPoint, LayerRole, LightSide, Plant, Scene, SceneGenerator, SceneKey, SceneLayer,
    low_shrub_outline,
};
use crate::biome::BiomeId;
use crate::error::ValidationError;
use crate::frame::AspectRatio;
use crate::seed::{Domain, SeedBundle};
use crate::settings::FormSettings;

const STEP: f64 = 1.0 / 160.0;
const MARGIN: f64 = 0.1;
const GENERATOR_VERSION: u32 = crate::version::GENERATOR_VERSION;

/// Rocky desert with wide exposed rock masses and sparse, low vegetation.
#[derive(Debug, Clone, Copy, Default)]
pub struct DesertGenerator;

impl SceneGenerator for DesertGenerator {
    fn version(&self) -> u32 {
        GENERATOR_VERSION
    }

    fn biome(&self) -> BiomeId {
        BiomeId::Desert
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
        let composition = seeds.stream(Domain::Composition).0;
        let terrain = seeds.stream(Domain::Terrain).0;
        let vegetation = seeds.stream(Domain::Vegetation).0;
        let light = if unit(composition, 5) < 0.5 {
            LightSide::Left
        } else {
            LightSide::Right
        };
        let horizon = h * (0.37 + 0.06 * unit(composition, 0));
        let mut layers = vec![band(
            LayerRole::Sky,
            1.0,
            0.5,
            BandSpec {
                x0: -MARGIN,
                top: -MARGIN,
                bottom: horizon + 0.03,
                width: w,
                noise: None,
            },
        )];

        // Distant tablelands: low, wide silhouettes with broad level crowns.
        layers.extend(mesa(
            terrain ^ 0x4449_5354_0000_0001,
            LayerRole::Mesa,
            0.93,
            0.57,
            w,
            h,
            horizon,
            0.20 + 0.60 * unit(composition, 3),
            0.12 + 0.12 * unit(composition, 4),
            0.05,
            0.10,
            form.faceting,
            0.5,
        ));
        layers.extend(mesa(
            terrain ^ 0x4d49_4452_0000_0002,
            LayerRole::Mesa,
            0.68,
            0.60,
            w,
            h,
            horizon + 0.04,
            0.20 + 0.60 * unit(composition, 12),
            0.16 + 0.16 * unit(composition, 13),
            0.05 + 0.07 * unit(composition, 14),
            0.08 + 0.12 * unit(composition, 15),
            form.faceting,
            0.35,
        ));

        // A pair of broad mesas at different depths makes the focal plateau
        // interlock with a lower, offset tableland instead of filling the frame.
        let primary_x = 0.18 + 0.64 * unit(composition, 1);
        let secondary_x = if primary_x > 0.5 {
            0.14 + 0.20 * unit(composition, 6)
        } else {
            0.66 + 0.20 * unit(composition, 6)
        };
        layers.extend(mesa(
            terrain ^ 0x5345_434f_0000_0003,
            LayerRole::Mesa,
            0.53,
            0.54,
            w,
            h,
            horizon + 0.04,
            secondary_x,
            0.08 + 0.10 * unit(composition, 7),
            0.08,
            0.12,
            form.faceting,
            0.46,
        ));
        layers.extend(mesa(
            terrain ^ 0x5052_494d_0000_0004,
            LayerRole::Mesa,
            0.47,
            0.69,
            w,
            h,
            horizon - 0.03,
            primary_x,
            0.08 + 0.16 * unit(composition, 10) + 0.035 * form.relief,
            0.04 + 0.12 * unit(composition, 8),
            0.16,
            form.faceting,
            0.70,
        ));
        // Open basin floor and a near shelf replace the lakeshore/water mass.
        layers.push(band(
            LayerRole::Mesa,
            0.58,
            0.40,
            BandSpec {
                x0: -MARGIN,
                top: horizon - h * 0.02,
                bottom: h + MARGIN,
                width: w,
                noise: Some(Fbm {
                    seed: terrain ^ 0x4241_5349_0000_0004,
                    base_wavelength: 0.60,
                    min_wavelength: STEP * 4.0,
                    gain: 0.48,
                    angular: form.faceting,
                }),
            },
        ));
        layers.push(band(
            LayerRole::Mesa,
            0.36,
            0.48,
            BandSpec {
                x0: -MARGIN,
                top: horizon + h * 0.26,
                bottom: h + MARGIN,
                width: w,
                noise: Some(Fbm {
                    seed: terrain ^ 0x464f_4f54_0000_0006,
                    base_wavelength: 0.92,
                    min_wavelength: STEP * 6.0,
                    gain: 0.56,
                    angular: form.faceting,
                }),
            },
        ));
        layers.push(dry_wash(
            w,
            h,
            horizon + h * 0.25,
            terrain ^ 0x5741_5348_0000_0007,
            0.31,
            0.30 + 0.40 * unit(composition, 11),
        ));
        // Sparse shrubs are individual stable forms drawn only from the
        // vegetation stream; appearance and seasonal settings cannot move them.
        let count = (3.0 + form.woodland_density * 18.0).round() as usize;
        for i in 0..count {
            let x = MARGIN + (w - 2.0 * MARGIN) * unit(vegetation, i as i64 * 3);
            let y0 = h * (0.76 + 0.14 * unit(vegetation, i as i64 * 3 + 1));
            let radius = h * (0.009 + 0.009 * unit(vegetation, i as i64 * 3 + 2));
            let outline = low_shrub_outline(x, y0, radius, unit(vegetation, i as i64 * 7 + 80));
            layers.push(SceneLayer {
                role: LayerRole::Woodland,
                depth: (0.24 - (y0 / h) as f32 * 0.08).clamp(0.08, 0.18),
                shade: 0.58,
                plant: Plant::Shrub,
                outline,
            });
        }
        layers.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        Ok(Scene::new(
            SceneKey {
                generator_version: GENERATOR_VERSION,
                biome: BiomeId::Desert,
                structural_seeds: seeds.structural(),
                form: *form,
                aspect,
            },
            layers,
        )?
        .with_light(light))
    }
}

struct BandSpec {
    x0: f64,
    top: f64,
    bottom: f64,
    width: f64,
    noise: Option<Fbm>,
}

fn band(role: LayerRole, depth: f32, shade: f32, band: BandSpec) -> SceneLayer {
    let BandSpec {
        x0,
        top,
        bottom,
        width,
        noise,
    } = band;
    let n = (width / STEP).ceil() as usize + 1;
    let mut outline = Vec::with_capacity(2 * n);
    for i in 0..n {
        let x = x0 + (width - x0 + MARGIN) * i as f64 / (n - 1) as f64;
        let y = top + noise.map_or(0.0, |f| f.eval(x) * 0.15);
        outline.push(point(x, y));
    }
    outline.extend([point(width + MARGIN, bottom), point(x0, bottom)]);
    SceneLayer {
        role,
        depth,
        shade,
        plant: Plant::None,
        outline,
    }
}

fn dry_wash(
    width: f64,
    height: f64,
    start_y: f64,
    seed: u64,
    depth: f32,
    start_x: f64,
) -> SceneLayer {
    let meander = Fbm {
        seed,
        base_wavelength: 0.8,
        min_wavelength: 0.1,
        gain: 0.55,
        angular: 0.2,
    };
    let n = 81;
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let y = start_y + (height + MARGIN - start_y) * t;
        let x = width * start_x + width * 0.055 * t * meander.eval(t * 1.8);
        let half = height * (0.018 + 0.07 * t * t);
        left.push(point(x - half, y));
        right.push(point(x + half, y));
    }
    left.extend(right.into_iter().rev());
    SceneLayer {
        role: LayerRole::Mesa,
        depth,
        shade: 0.50,
        plant: Plant::None,
        outline: left,
    }
}

#[allow(clippy::too_many_arguments)]
fn mesa(
    seed: u64,
    role: LayerRole,
    depth: f32,
    shade: f32,
    width: f64,
    height: f64,
    horizon: f64,
    center: f64,
    half_width: f64,
    cap: f64,
    drop: f64,
    faceting: f64,
    relief: f64,
) -> Vec<SceneLayer> {
    let center = center * width;
    let span = half_width * width;
    let left = center - span;
    let right = center + span;
    let cap_width = cap * width;
    let cap_left = cap_width * (0.65 + 0.7 * unit(seed, 20));
    let cap_right = cap_width * (0.65 + 0.7 * unit(seed, 21));
    let base = horizon + height * (0.22 + 0.35 * drop);
    let fuzz = Fbm {
        seed,
        base_wavelength: 0.28,
        min_wavelength: STEP * 3.0,
        gain: 0.45,
        angular: faceting,
    };
    let x0 = left - cap_left;
    let x1 = right + cap_right;
    let n = ((x1 - x0) / STEP).ceil() as usize + 1;
    let mut crest = Vec::with_capacity(n);
    for i in 0..n {
        let x = x0 + (x1 - x0) * i as f64 / (n - 1) as f64;
        let edge = if x < left {
            weathered_edge((x - x0) / cap_left, seed, 0)
        } else if x > right {
            weathered_edge((x1 - x) / cap_right, seed, 1)
        } else {
            1.0
        };
        // A near-level cap drops through irregular ledges; noise swells the
        // escarpment without roughening the broad summit into a peak.
        let side_noise = if edge < 1.0 {
            fuzz.eval(x) * 0.05 * (edge * (1.0 - edge) * 2.0)
        } else {
            fuzz.eval(x) * 0.014
        };
        let top = base - height * (0.28 + 0.24 * relief) * edge + side_noise;
        crest.push((x, top));
    }

    let mut body = crest.iter().map(|(x, y)| point(*x, *y)).collect::<Vec<_>>();
    body.extend([point(x1, height + MARGIN), point(x0, height + MARGIN)]);
    let mut layers = vec![SceneLayer {
        role,
        depth,
        shade,
        plant: Plant::None,
        outline: body,
    }];

    // Sedimentary ledges follow the same landform contour, in irregular
    // bands. Their shades create warm/cool strata without baking colors into
    // geometry or adding an artwork-derived texture.
    const STRATA: [(f64, f64, f32); 10] = [
        (0.006, 0.012, 0.20),
        (0.035, 0.018, 0.35),
        (0.070, 0.028, 0.65),
        (0.112, 0.020, 0.44),
        (0.154, 0.035, 0.72),
        (0.205, 0.018, 0.31),
        (0.247, 0.040, 0.62),
        (0.305, 0.020, 0.42),
        (0.360, 0.036, 0.75),
        (0.420, 0.022, 0.32),
    ];
    for (index, (base_offset, base_thickness, band_shade)) in STRATA.into_iter().enumerate() {
        if unit(seed, index as i64 + 256) < 0.16 {
            continue;
        }
        let offset = base_offset + (unit(seed, index as i64 + 48) - 0.5) * 0.026;
        let thickness = base_thickness * (0.60 + 0.80 * unit(seed, index as i64 + 64));
        let drift = Fbm {
            seed: seed ^ (0x5354_5241_5441_0000 + index as u64),
            base_wavelength: 0.72,
            min_wavelength: STEP * 8.0,
            gain: 0.55,
            angular: 0.35,
        };
        let trim_left = (n as f64 * 0.12 * unit(seed, index as i64 + 144)) as usize;
        let trim_right = (n as f64 * 0.12 * unit(seed, index as i64 + 160)) as usize;
        let end = n - trim_right - 1;
        let mut band_crest = (trim_left..=end)
            .step_by(2)
            .map(|i| crest[i])
            .collect::<Vec<_>>();
        if band_crest.last().is_some_and(|last| last.0 != crest[end].0) {
            band_crest.push(crest[end]);
        }
        let mut outline = Vec::with_capacity(2 * band_crest.len());
        for (x, y) in &band_crest {
            let warp = drift.eval(*x) * (0.025 + 0.020 * unit(seed, index as i64 + 96));
            outline.push(point(*x, *y + offset * height + warp));
        }
        for (x, y) in band_crest.iter().rev() {
            let warp = drift.eval(*x) * (0.025 + 0.020 * unit(seed, index as i64 + 96));
            outline.push(point(*x, *y + (offset + thickness) * height + warp));
        }
        layers.push(SceneLayer {
            role,
            depth: depth - (index as f32 + 1.0) * 0.001,
            shade: (band_shade + 0.18 * (2.0 * unit(seed, index as i64 + 128) as f32 - 1.0))
                .clamp(0.0, 1.0),
            plant: Plant::None,
            outline,
        });
    }
    layers
}

fn weathered_edge(t: f64, seed: u64, side: i64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let smooth = |x: f64, lo: f64, hi: f64| {
        let u = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    };
    let jitter = |salt: i64| unit(seed, side * 17 + salt);
    let shelf_start = 0.18 + 0.10 * jitter(1);
    let shelf_end = shelf_start + 0.10 + 0.05 * jitter(2);
    let lower_start = 0.52 + 0.12 * jitter(3);
    let lower_end = lower_start + 0.08 + 0.06 * jitter(4);
    (t * t * (3.0 - 2.0 * t)
        + 0.09
            * (smooth(t, shelf_start, shelf_end) - smooth(t, shelf_end + 0.08, shelf_end + 0.16))
        - 0.05
            * (smooth(t, lower_start, lower_end) - smooth(t, lower_end + 0.06, lower_end + 0.12)))
    .clamp(0.0, 1.0)
}

fn point(x: f64, y: f64) -> CanvasPoint {
    CanvasPoint {
        x: x as f32,
        y: y as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::{TextDigest, Variation};

    fn seeds(passage: &str, variation: u32) -> SeedBundle {
        SeedBundle::derive(
            TextDigest::from_source(passage).unwrap(),
            Variation(variation),
        )
    }

    #[test]
    fn same_seed_reproduces_broad_dry_landforms_without_a_lake() {
        let seed = seeds("Desert candidate", 0);
        let form = FormSettings::default();
        let aspect = AspectRatio::of(16, 9);
        let a = DesertGenerator.generate(&seed, &form, aspect).unwrap();
        let b = DesertGenerator.generate(&seed, &form, aspect).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.geometry_checksum(), b.geometry_checksum());
        assert!(
            a.layers()
                .iter()
                .filter(|l| l.role == LayerRole::Mesa)
                .count()
                >= 5
        );
        assert!(!a.layers().iter().any(|l| l.role == LayerRole::Water));
        assert!(
            a.layers()
                .iter()
                .filter(|l| l.plant == Plant::Shrub)
                .count()
                >= 3
        );
        assert!(a.layers().iter().all(|l| l.plant != Plant::Conifer));
    }

    #[test]
    fn varied_mesa_strata_remain_simple_bounded_scene_layers() {
        for i in 0..10 {
            let scene = DesertGenerator
                .generate(
                    &seeds("stratified desert", i),
                    &FormSettings::default(),
                    AspectRatio::of(16, 9),
                )
                .unwrap();
            for (layer_index, layer) in scene.layers().iter().enumerate() {
                assert!(
                    super::super::raster::is_simple(&layer.outline),
                    "seed {i}, layer {layer_index}, role {:?}, vertices {}",
                    layer.role,
                    layer.outline.len()
                );
            }
            assert!(scene.layers().len() <= super::super::MAX_LAYERS);
        }
    }

    #[test]
    fn extreme_wide_scene_stays_within_shared_bounds() {
        let form = FormSettings {
            faceting: 1.0,
            relief: 1.0,
            woodland_density: 1.0,
        };
        let scene = DesertGenerator
            .generate(&seeds("extreme desert", 0), &form, AspectRatio::of(4, 1))
            .unwrap();
        let vertices: usize = scene.layers().iter().map(|layer| layer.outline.len()).sum();
        assert!(scene.layers().len() <= super::super::MAX_LAYERS);
        assert!(vertices <= super::super::MAX_SCENE_VERTICES);
    }

    #[test]
    fn variation_recomposes_desert_structure() {
        let a = DesertGenerator
            .generate(
                &seeds("desert fixture", 0),
                &FormSettings::default(),
                AspectRatio::of(16, 9),
            )
            .unwrap();
        let b = DesertGenerator
            .generate(
                &seeds("desert fixture", 1),
                &FormSettings::default(),
                AspectRatio::of(16, 9),
            )
            .unwrap();
        assert_ne!(a.geometry_checksum(), b.geometry_checksum());
    }
}
