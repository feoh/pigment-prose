//! Dense tropical canopy as a broad, layered painterly landscape.
//!
//! Canopy masses use the shared forest painting primitives, arranged as a
//! few coherent depth bands with bounded understory accents. The dry/wet
//! cycle is appearance-only; no seasonal input affects placement.

use super::noise::{Fbm, unit};
use super::{
    CanvasPoint, LayerRole, LightSide, Plant, Scene, SceneGenerator, SceneKey, SceneLayer,
};
use crate::biome::BiomeId;
use crate::error::ValidationError;
use crate::frame::AspectRatio;
use crate::seed::{Domain, SeedBundle};
use crate::settings::FormSettings;

const STEP: f64 = 1.0 / 144.0;
const MARGIN: f64 = 0.08;
const GENERATOR_VERSION: u32 = crate::version::GENERATOR_VERSION;

#[derive(Debug, Clone, Copy, Default)]
pub struct JungleGenerator;

impl SceneGenerator for JungleGenerator {
    fn version(&self) -> u32 {
        GENERATOR_VERSION
    }

    fn biome(&self) -> BiomeId {
        BiomeId::Jungle
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
        let light = if unit(composition, 4) < 0.5 {
            LightSide::Left
        } else {
            LightSide::Right
        };
        let horizon = h * (0.21 + 0.06 * unit(composition, 0));
        let mut layers = vec![band(
            LayerRole::Sky,
            1.0,
            0.52,
            w,
            h,
            -MARGIN,
            horizon + 0.12 * h,
            h + MARGIN,
            terrain,
        )];

        // A broken overhanging canopy creates a view from beneath the trees,
        // with one continuous, seed-shaped foliage edge rather than cutout blocks.
        layers.push(overhang(w, h, terrain ^ 0x4f56_4552_4841_4e47));

        // Three overlapping canopy horizons create depth while leaving small,
        // seed-stable shafts of pale air between the silhouettes.
        for (i, (role, depth, top, amplitude)) in [
            (LayerRole::JungleCanopy, 0.88, horizon, 0.045),
            (LayerRole::JungleCanopy, 0.64, horizon + 0.05 * h, 0.075),
            (LayerRole::JungleCanopy, 0.40, horizon + 0.11 * h, 0.11),
        ]
        .into_iter()
        .enumerate()
        {
            let seed = terrain ^ (0x4a55_4e47_4c45_0000u64 | i as u64);
            layers.push(band(
                role,
                depth,
                (0.40 + 0.2 * unit(seed, 13)) as f32,
                w,
                h,
                top,
                amplitude * h,
                h + MARGIN,
                seed,
            ));
            // Offset canopy masses interlock and prevent a single unbroken
            // horizontal tree line.
            if i > 0 {
                let center = 0.18 + 0.64 * unit(composition, 20 + i as i64);
                let width = 0.16 + 0.14 * unit(composition, 24 + i as i64);
                layers.push(crown_mass(
                    seed ^ 0x4341_4e4f_5059,
                    w,
                    h,
                    center,
                    width,
                    top - h * 0.03,
                    amplitude * h * 0.48,
                    depth - 0.025,
                    role,
                ));
            }
        }

        // Ground plane: a narrow, winding opening guides the eye into the
        // scene without turning the generator into a water landscape.
        layers.push(band(
            LayerRole::JungleGround,
            0.30,
            0.50,
            w,
            h,
            horizon + 0.34 * h,
            0.018 * h,
            h + MARGIN,
            terrain ^ 0x4752_4f55_4e44,
        ));
        // A seed-varied sunlit clearing gives the layered canopy an opening
        // and a focal path through the understory, without implying a lake.
        layers.push(clearing(
            w,
            h,
            horizon + 0.20 * h,
            h + MARGIN,
            0.18 + 0.64 * unit(composition, 31),
            terrain ^ 0x434c_4541_5249_4e47,
        ));

        // Tropical broad-leaf fans punctuate the near understory. Each fan is
        // a different bundle of tapered fronds, mixed with broad and airy forms.
        let plant_kinds = [
            Plant::Broadleaf,
            Plant::Birch,
            Plant::Shrub,
            Plant::Flowering,
        ];
        let species_shift = (unit(vegetation, 519) * plant_kinds.len() as f64) as usize;
        for (group, (x, y, scale)) in [
            (0.08, 0.91, 1.05),
            (0.30, 0.99, 0.78),
            (0.70, 0.97, 0.86),
            (0.92, 0.88, 1.12),
        ]
        .into_iter()
        .enumerate()
        {
            let base_x = w * (x + 0.05 * (unit(vegetation, group as i64 + 518) - 0.5));
            let base_y = h * (y + 0.07 * unit(vegetation, group as i64 + 520));
            let plant = plant_kinds[(group + species_shift) % plant_kinds.len()];
            for (leaf, (dx, dy)) in [
                (0.78, -0.62),
                (0.56, -0.83),
                (0.26, -0.97),
                (-0.18, -0.98),
                (-0.55, -0.84),
                (-0.82, -0.57),
            ]
            .into_iter()
            .enumerate()
            {
                let direction = if group % 2 == 0 { dx } else { -dx };
                let length = h
                    * scale
                    * (0.18 + 0.12 * unit(vegetation, group as i64 * 17 + leaf as i64 + 540));
                let width = h
                    * scale
                    * (0.012 + 0.012 * unit(vegetation, group as i64 * 17 + leaf as i64 + 560));
                layers.push(SceneLayer {
                    role: LayerRole::JungleCanopy,
                    depth: 0.10
                        + 0.08 * unit(vegetation, group as i64 * 17 + leaf as i64 + 580) as f32,
                    shade: (0.34 + 0.38 * unit(vegetation, group as i64 * 17 + leaf as i64 + 600))
                        as f32,
                    plant,
                    outline: leaf_blade(base_x, base_y, direction, dy, length, width),
                });
            }
        }

        // A bounded number of varied, stable understory plants. Main canopy
        // topology comes from shared, overlapping forest masses rather than
        // repeated identical tree stamps.
        let count = (12.0 + form.woodland_density * 30.0).round() as usize;
        for i in 0..count {
            let x = MARGIN + (w - 2.0 * MARGIN) * unit(vegetation, i as i64 * 4);
            let y = horizon + h * (0.26 + 0.42 * unit(vegetation, i as i64 * 4 + 1));
            let radius = h * (0.018 + 0.025 * unit(vegetation, i as i64 * 4 + 2));
            let plant = jungle_plant(unit(vegetation, i as i64 * 4 + 3));
            layers.push(SceneLayer {
                role: LayerRole::Woodland,
                depth: (0.36 - 0.22 * ((y - horizon) / (h - horizon)).clamp(0.0, 1.0)) as f32,
                shade: (0.38 + 0.32 * unit(vegetation, i as i64 * 9 + 70)) as f32,
                plant,
                outline: tree_outline(x, y, radius, unit(vegetation, i as i64 * 7 + 90), plant),
            });
        }

        layers.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        Ok(Scene::new(
            SceneKey {
                generator_version: GENERATOR_VERSION,
                biome: BiomeId::Jungle,
                structural_seeds: seeds.structural(),
                form: *form,
                aspect,
            },
            layers,
        )?
        .with_light(light))
    }
}

#[allow(clippy::too_many_arguments)]
fn band(
    role: LayerRole,
    depth: f32,
    shade: f32,
    width: f64,
    height: f64,
    top: f64,
    amplitude: f64,
    bottom: f64,
    seed: u64,
) -> SceneLayer {
    let field = Fbm {
        seed,
        base_wavelength: 0.34,
        min_wavelength: STEP * 4.0,
        gain: 0.5,
        angular: 0.15,
    };
    let n = (width / STEP).ceil() as usize + 1;
    let mut outline = Vec::with_capacity(2 * n);
    for i in 0..n {
        let x = -MARGIN + (width + 2.0 * MARGIN) * i as f64 / (n - 1) as f64;
        let broad = field.eval(x * 0.42);
        let tooth = field.eval(x * 4.2 + 9.0) - 0.5;
        let y = top - amplitude * (0.35 + broad * 0.75) - height * tooth.abs() * 0.035;
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

fn clearing(
    width: f64,
    height: f64,
    start_y: f64,
    bottom: f64,
    start_x: f64,
    seed: u64,
) -> SceneLayer {
    let n = 33;
    let mut left = Vec::with_capacity(2 * n);
    let mut right = Vec::with_capacity(2 * n);
    let field = Fbm {
        seed,
        base_wavelength: 0.6,
        min_wavelength: STEP * 4.0,
        gain: 0.45,
        angular: 0.1,
    };
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let y = start_y + (bottom - start_y) * t;
        let center = width * start_x + width * 0.12 * t * (2.0 * field.eval(t * 1.3) - 1.0);
        let half = height * (0.015 + 0.12 * t * t);
        left.push(point(center - half, y));
        right.push(point(center + half, y));
    }
    left.extend(right.into_iter().rev());
    SceneLayer {
        role: LayerRole::JungleGround,
        depth: 0.22,
        shade: 0.62,
        plant: Plant::None,
        outline: left,
    }
}

#[allow(clippy::too_many_arguments)]
fn crown_mass(
    seed: u64,
    width: f64,
    height: f64,
    center: f64,
    half_width: f64,
    top: f64,
    amplitude: f64,
    depth: f32,
    role: LayerRole,
) -> SceneLayer {
    let left = center * width - half_width * width;
    let right = center * width + half_width * width;
    let n = 33;
    let mut outline = Vec::with_capacity(n + 2);
    let field = Fbm {
        seed,
        base_wavelength: 0.11,
        min_wavelength: STEP * 4.0,
        gain: 0.48,
        angular: 0.2,
    };
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let scallop = (1.0 - (2.0 * t - 1.0).powi(2)).max(0.0);
        let y = top - amplitude * scallop * (0.62 + field.eval(t * 0.55) * 0.76);
        outline.push(point(left + (right - left) * t, y));
    }
    let bottom = top + amplitude * 2.2;
    for i in (0..n).rev() {
        let t = i as f64 / (n - 1) as f64;
        let x = left + (right - left) * t;
        let y = bottom + height * 0.012 * field.eval(t * 0.7 + 3.0);
        outline.push(point(x, y));
    }
    SceneLayer {
        role,
        depth,
        shade: 0.58,
        plant: Plant::None,
        outline,
    }
}

fn overhang(width: f64, height: f64, seed: u64) -> SceneLayer {
    let (x0, x1) = (-MARGIN, width + MARGIN);
    let n = 41;
    let mut outline = Vec::with_capacity(2 * n);
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        outline.push(point(x0 + (x1 - x0) * t, -MARGIN));
    }
    let field = Fbm {
        seed,
        base_wavelength: 0.24,
        min_wavelength: STEP * 4.0,
        gain: 0.5,
        angular: 0.12,
    };
    for i in (0..n).rev() {
        let t = i as f64 / (n - 1) as f64;
        let scallop = 0.5 + 0.5 * field.eval(t * 3.2);
        let edge_drape = (2.0 * t - 1.0).abs();
        let fine = 0.5 + 0.5 * field.eval(t * 11.0 + 7.0);
        let y = height * (0.10 + 0.13 * scallop + 0.035 * fine + 0.22 * edge_drape);
        outline.push(point(x0 + (x1 - x0) * t, y));
    }
    SceneLayer {
        role: LayerRole::JungleCanopy,
        depth: 0.19,
        shade: 0.48,
        plant: Plant::None,
        outline,
    }
}

fn leaf_blade(x: f64, y: f64, dx: f64, dy: f64, length: f64, width: f64) -> Vec<CanvasPoint> {
    let mag = (dx * dx + dy * dy).sqrt();
    let (nx, ny) = (-dy / mag, dx / mag);
    let profile = [0.08, 0.34, 0.68, 0.92, 1.0, 0.91, 0.66, 0.31, 0.04];
    let mut outline = Vec::with_capacity(2 * profile.len());
    for i in 0..profile.len() {
        let t = i as f64 / (profile.len() - 1) as f64;
        let bow = width * 0.65 * t * (1.0 - t);
        let center_x = x + dx * length * t + bow;
        let center_y = y + dy * length * t;
        let edge = width * profile[i];
        outline.push(point(center_x + nx * edge, center_y + ny * edge));
    }
    for i in (0..profile.len()).rev() {
        let t = i as f64 / (profile.len() - 1) as f64;
        let bow = width * 0.65 * t * (1.0 - t);
        let center_x = x + dx * length * t + bow;
        let center_y = y + dy * length * t;
        let edge = -width * profile[i];
        outline.push(point(center_x + nx * edge, center_y + ny * edge));
    }
    outline
}

fn tree_outline(x: f64, y: f64, r: f64, phase: f64, plant: Plant) -> Vec<CanvasPoint> {
    let (wide, crown_y) = match plant {
        Plant::Conifer => (0.52, 1.65),
        Plant::Birch => (0.62, 1.72),
        Plant::Shrub => (1.35, 0.72),
        Plant::Flowering => (1.08, 1.05),
        _ => (1.0, 1.32),
    };
    let profile = match plant {
        // Broad multi-lobed crowns make the tropical canopy silhouette.
        Plant::Broadleaf => [
            0.38, 0.68, 0.94, 0.78, 0.70, 0.92, 1.0, 0.84, 0.72, 0.90, 0.42,
        ],
        // Tall, airy emergents rise above the broad crowns.
        Plant::Birch => [
            0.12, 0.34, 0.58, 0.82, 1.0, 0.92, 0.78, 0.62, 0.44, 0.26, 0.10,
        ],
        // Low, broad understory clumps.
        Plant::Shrub => [
            0.38, 0.62, 0.82, 0.94, 1.0, 0.88, 0.98, 0.84, 0.70, 0.54, 0.38,
        ],
        // A rounder crown with a few small blossom-bearing lobes.
        Plant::Flowering => [
            0.20, 0.56, 0.82, 0.95, 0.88, 1.0, 0.90, 0.96, 0.76, 0.42, 0.16,
        ],
        _ => [
            0.15, 0.42, 0.72, 0.92, 1.0, 0.86, 0.95, 0.76, 0.55, 0.28, 0.12,
        ],
    };
    profile
        .iter()
        .enumerate()
        .map(|(i, height)| {
            let t = i as f64 / (profile.len() - 1) as f64;
            let wobble = if i == 0 || i + 1 == profile.len() {
                1.0
            } else {
                0.9 + 0.1 * phase
            };
            point(
                x + (2.0 * t - 1.0) * r * wide,
                y - r * crown_y * height * wobble,
            )
        })
        .collect()
}

fn jungle_plant(value: f64) -> Plant {
    if value < 0.38 {
        Plant::Broadleaf
    } else if value < 0.66 {
        Plant::Birch
    } else if value < 0.86 {
        Plant::Shrub
    } else {
        Plant::Flowering
    }
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
    use crate::biome;
    use crate::frame::AspectRatio;
    use crate::scene::diagnostic_seeds;

    #[test]
    fn jungle_scenes_are_reproducible_layered_and_bounded() {
        let mut plants = std::collections::HashSet::new();
        for seed in 0..10 {
            let seeds = diagnostic_seeds(seed);
            let form = biome::JUNGLE.form;
            let a = JungleGenerator
                .generate(&seeds, &form, AspectRatio::of(16, 9))
                .unwrap();
            let b = JungleGenerator
                .generate(&seeds, &form, AspectRatio::of(16, 9))
                .unwrap();
            assert_eq!(a.geometry_checksum(), b.geometry_checksum());
            assert_eq!(a.key().biome, BiomeId::Jungle);
            assert!(
                a.layers()
                    .iter()
                    .any(|layer| layer.role == LayerRole::JungleCanopy)
            );
            assert!(a.layers().len() < 96);
            assert!(
                a.layers()
                    .iter()
                    .map(|layer| layer.outline.len())
                    .sum::<usize>()
                    < 32_768
            );
            assert!(
                a.layers()
                    .iter()
                    .filter(|layer| layer.plant != Plant::None)
                    .count()
                    <= 66
            );
            assert_eq!(
                a.layers()
                    .iter()
                    .filter(|layer| {
                        layer.role == LayerRole::JungleCanopy && layer.plant != Plant::None
                    })
                    .count(),
                24
            );
            plants.extend(
                a.layers()
                    .iter()
                    .filter_map(|layer| (layer.plant != Plant::None).then_some(layer.plant)),
            );
        }
        assert!(plants.contains(&Plant::Broadleaf));
        assert!(plants.contains(&Plant::Birch));
        assert!(plants.contains(&Plant::Shrub));
        assert!(plants.contains(&Plant::Flowering));
        assert!(!plants.contains(&Plant::Conifer));
        assert!(!plants.contains(&Plant::Copper));
    }

    #[test]
    fn jungle_profile_and_dry_wet_year_are_valid_and_snow_free() {
        assert!(biome::JUNGLE.validate().is_ok());
        assert_eq!(biome::JUNGLE.season.cycle, crate::season::Cycle::DryWet);
        for step in 0..=100 {
            let state = biome::JUNGLE.season.at(step as f64 / 100.0);
            assert_eq!(state.snow, 0.0);
            assert_eq!(state.ground_snow, 0.0);
            assert_eq!(state.tree_snow, 0.0);
            assert!((0.0..=1.0).contains(&state.dry));
        }
    }
}
