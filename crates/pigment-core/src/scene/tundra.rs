//! Open tundra candidate: low rolling land, exposed stone and sparse shrubs.
//!
//! This is intentionally neither a snow overlay on the alpine massif nor a
//! variation of the lakeshore. The scene has no peak, lake or tree stands.

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

const STEP: f64 = 1.0 / 128.0;
const MARGIN: f64 = 0.1;
const GENERATOR_VERSION: u32 = crate::version::GENERATOR_VERSION;

/// A broad, low-relief tundra with exposed peat/stone and dwarf shrubs.
#[derive(Debug, Clone, Copy, Default)]
pub struct TundraGenerator;

impl SceneGenerator for TundraGenerator {
    fn version(&self) -> u32 {
        GENERATOR_VERSION
    }

    fn biome(&self) -> BiomeId {
        BiomeId::Tundra
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
        let plants = seeds.stream(Domain::Vegetation).0;
        let light = if unit(composition, 17) < 0.5 {
            LightSide::Left
        } else {
            LightSide::Right
        };
        let horizon = h * (0.32 + 0.08 * unit(composition, 0));
        let far_seed = terrain ^ 0x4641_525f_5455_4e44;
        let middle_seed = terrain ^ 0x4d49_445f_5455_4e44;
        let foreground_seed = terrain ^ 0x4e45_4152_5455_4e44;
        let mut layers = vec![land(
            w,
            h,
            LandSpec {
                role: LayerRole::Sky,
                depth: 1.0,
                shade: 0.5,
                top: -MARGIN,
                bottom: h + MARGIN,
                amplitude: 0.0,
                roughness: 0.0,
            },
            form.faceting,
            terrain,
        )];
        layers.push(land(
            w,
            h,
            LandSpec {
                role: LayerRole::TundraGround,
                depth: 0.82,
                shade: 0.47,
                top: horizon,
                bottom: horizon + h * 0.23,
                amplitude: h * 0.14,
                roughness: 0.12,
            },
            form.faceting,
            far_seed,
        ));
        layers.push(land(
            w,
            h,
            LandSpec {
                role: LayerRole::TundraGround,
                depth: 0.59,
                shade: 0.53,
                top: horizon + h * 0.16,
                bottom: horizon + h * 0.44,
                amplitude: h * 0.17,
                roughness: 0.14,
            },
            form.faceting,
            middle_seed,
        ));
        layers.push(land(
            w,
            h,
            LandSpec {
                role: LayerRole::TundraGround,
                depth: 0.33,
                shade: 0.58,
                top: horizon + h * 0.25,
                bottom: h + MARGIN,
                amplitude: h * (0.13 + 0.04 * form.relief),
                roughness: 0.15,
            },
            form.faceting,
            foreground_seed,
        ));

        // Stable, very low silhouettes ride the foreground contour. There
        // are no tall crowns and season never enters this placement stream.
        let count = (form.woodland_density * 36.0).round() as usize;
        for i in 0..count {
            let x = MARGIN + (w - 2.0 * MARGIN) * unit(plants, i as i64 * 3);
            let ground = foreground_top(
                x,
                h,
                horizon + h * 0.25,
                h * (0.13 + 0.04 * form.relief),
                form.faceting,
                foreground_seed,
            );
            let radius = h * (0.009 + 0.011 * unit(plants, i as i64 * 3 + 2));
            layers.push(SceneLayer {
                role: LayerRole::Woodland,
                depth: 0.18,
                shade: 0.52,
                plant: Plant::Shrub,
                outline: low_shrub_outline(
                    x,
                    ground + radius * 0.2,
                    radius,
                    unit(plants, i as i64 * 7 + 80),
                ),
            });
        }

        layers.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        Ok(Scene::new(
            SceneKey {
                generator_version: GENERATOR_VERSION,
                biome: BiomeId::Tundra,
                structural_seeds: seeds.structural(),
                form: *form,
                aspect,
            },
            layers,
        )?
        .with_light(light))
    }
}

struct LandSpec {
    role: LayerRole,
    depth: f32,
    shade: f32,
    top: f64,
    bottom: f64,
    amplitude: f64,
    roughness: f64,
}

fn land(width: f64, height: f64, spec: LandSpec, angular: f64, seed: u64) -> SceneLayer {
    let LandSpec {
        role,
        depth,
        shade,
        top,
        bottom,
        amplitude,
        roughness,
    } = spec;
    let field = Fbm {
        seed,
        base_wavelength: 0.66,
        min_wavelength: STEP * 3.0,
        gain: 0.52,
        angular,
    };
    let count = (width / STEP).ceil() as usize + 1;
    let mut outline = Vec::with_capacity(count + 2);
    for i in 0..count {
        let x = -MARGIN + (width + 2.0 * MARGIN) * i as f64 / (count - 1) as f64;
        let y = if role == LayerRole::Sky {
            top
        } else {
            let broad = field.eval(x * roughness.max(0.35));
            let grain = field.eval(x * 4.7 + 11.0) * roughness * 0.16;
            (top - amplitude * broad - height * grain).clamp(-MARGIN, bottom)
        };
        outline.push(point(x, y));
    }
    outline.extend([point(width + MARGIN, bottom), point(-MARGIN, bottom)]);
    SceneLayer {
        role,
        depth,
        shade,
        plant: Plant::None,
        outline,
    }
}

fn foreground_top(x: f64, height: f64, base: f64, amplitude: f64, angular: f64, seed: u64) -> f64 {
    let field = Fbm {
        seed,
        base_wavelength: 0.66,
        min_wavelength: STEP * 3.0,
        gain: 0.52,
        angular,
    };
    let broad = field.eval(x * 0.35);
    let grain = field.eval(x * 4.7 + 11.0) * 0.065 * 0.16;
    base - amplitude * broad - height * grain
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
    use crate::frame::AspectRatio;
    use crate::scene::diagnostic_seeds;

    #[test]
    fn zero_vegetation_density_produces_an_unplanted_open_scene() {
        let mut form = crate::biome::TUNDRA.form;
        form.woodland_density = 0.0;
        let scene = TundraGenerator
            .generate(&diagnostic_seeds(9), &form, AspectRatio::of(16, 9))
            .unwrap();
        assert!(
            scene
                .layers()
                .iter()
                .all(|layer| layer.plant == Plant::None)
        );
    }

    #[test]
    fn form_controls_morph_only_their_structural_features_within_bounds() {
        let seeds = diagnostic_seeds(12);
        let aspect = AspectRatio::of(4, 1);
        let form = crate::biome::TUNDRA.form;
        let base = TundraGenerator.generate(&seeds, &form, aspect).unwrap();
        let mut angular = form;
        angular.faceting = 1.0;
        let angular_scene = TundraGenerator.generate(&seeds, &angular, aspect).unwrap();
        assert_ne!(base.geometry_checksum(), angular_scene.geometry_checksum());
        let mut maxed = form;
        maxed.faceting = 1.0;
        maxed.relief = 1.0;
        maxed.woodland_density = 1.0;
        let extreme = TundraGenerator.generate(&seeds, &maxed, aspect).unwrap();
        assert!(extreme.layers().len() < 96);
        assert!(
            extreme
                .layers()
                .iter()
                .map(|layer| layer.outline.len())
                .sum::<usize>()
                < 32_768
        );
    }

    #[test]
    fn scenes_are_open_low_and_reproducible() {
        let form = crate::biome::TUNDRA.form;
        for seed in 0..10 {
            let seeds = diagnostic_seeds(seed);
            let a = TundraGenerator
                .generate(&seeds, &form, AspectRatio::of(16, 9))
                .unwrap();
            let b = TundraGenerator
                .generate(&seeds, &form, AspectRatio::of(16, 9))
                .unwrap();
            assert_eq!(a.geometry_checksum(), b.geometry_checksum());
            assert_eq!(a.key().biome, BiomeId::Tundra);
            assert!(
                a.layers()
                    .iter()
                    .all(|layer| layer.role != LayerRole::Mountain)
            );
            assert!(
                a.layers()
                    .iter()
                    .any(|layer| layer.role == LayerRole::TundraGround)
            );
            assert!(a.layers().iter().all(|layer| {
                !matches!(layer.role, LayerRole::MidRidge | LayerRole::NearRidge)
            }));
            assert!(
                a.layers()
                    .iter()
                    .filter(|layer| layer.plant == Plant::Shrub)
                    .count()
                    <= 40
            );
        }
    }
}
