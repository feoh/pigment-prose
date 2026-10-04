//! Paint-space envelopes, not new geometry. Facets of one mass share its
//! silhouette so relief, cloud density and their shadows don't restart at
//! the old flat colour-plane boundaries. Sampling is independent of output
//! resolution and tile layout; the immutable scene/checksum is untouched.

use pigment_core::scene::{LayerRole, Scene};

fn bounds(scene: &Scene, i: usize) -> [f32; 4] {
    scene.layers()[i]
        .outline
        .iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
            [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]
        })
}

fn supported(role: LayerRole) -> bool {
    matches!(
        role,
        LayerRole::Cloud
            | LayerRole::Mountain
            | LayerRole::ForegroundRock
            | LayerRole::Mesa
            | LayerRole::FarRidge
    )
}

/// Two vec4 headers per layer: owner bounds; (column offset, column count,
/// owner layer, inverse column spacing). The remaining vec4s are vertical
/// silhouette spans (top, bottom, 0, 0). Unused roles have zero columns.
pub(crate) fn envelopes(scene: &Scene) -> Vec<f32> {
    let layers = scene.layers();
    let boxes: Vec<_> = (0..layers.len()).map(|i| bounds(scene, i)).collect();
    let mut owners: Vec<usize> = Vec::with_capacity(layers.len());
    let mut data = vec![0.0; layers.len() * 8];
    for (i, layer) in layers.iter().enumerate() {
        let b = boxes[i];
        let owner = if supported(layer.role) {
            (0..i)
                .rev()
                .find(|&j| {
                    let a = boxes[j];
                    layers[j].role == layer.role
                    && layers[j].depth == layer.depth
                    && b[0] >= a[0] && b[1] >= a[1]
                    && b[2] <= a[2] && b[3] <= a[3]
                    // Cumulus cap/base planes share the body's complete x
                    // range. An overlapping, smaller bank is NOT a facet.
                    && (layer.role != LayerRole::Cloud || (a[0] == b[0] && a[2] == b[2]))
                })
                .map(|j| owners[j])
                .unwrap_or(i)
        } else {
            i
        };
        owners.push(owner);
        if owner != i {
            let header: [f32; 8] = data[owner * 8..owner * 8 + 8].try_into().unwrap();
            data[i * 8..i * 8 + 8].copy_from_slice(&header);
            continue;
        }
        data[i * 8..i * 8 + 4].copy_from_slice(&b);
        data[i * 8 + 6] = i as f32;
        if !supported(layer.role) {
            continue;
        }
        let width = (b[2] - b[0]).max(1e-6);
        let count = ((width * 1024.0).ceil() as usize + 1).clamp(65, 4097);
        let first = data.len() / 4;
        data[i * 8 + 4..i * 8 + 8].copy_from_slice(&[
            first as f32,
            count as f32,
            i as f32,
            (count - 1) as f32 / width,
        ]);
        for k in 0..count {
            // Endpoints just inside the polygon, to avoid a vertex's
            // ambiguous parity. This is paint data, not scene geometry.
            let t = (k as f32 / (count - 1) as f32).clamp(1e-6, 1.0 - 1e-6);
            let x = b[0] + width * t;
            let (mut top, mut bottom) = (f32::MAX, f32::MIN);
            for j in 0..layer.outline.len() {
                let (a, z) = (
                    layer.outline[j],
                    layer.outline[(j + 1) % layer.outline.len()],
                );
                if (a.x > x) != (z.x > x) {
                    let y = a.y + (x - a.x) / (z.x - a.x) * (z.y - a.y);
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
            }
            if top > bottom {
                (top, bottom) = (b[1], b[3]);
            }
            data.extend([top, bottom, 0.0, 0.0]);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use pigment_core::biome::BiomeId;
    use pigment_core::frame::AspectRatio;
    use pigment_core::scene::{diagnostic_seeds, generator};
    use pigment_core::settings::FormSettings;

    #[test]
    fn envelopes_are_finite_bounded_repeatable_and_do_not_change_geometry() {
        for biome in pigment_core::biome::PROFILES.iter().map(|p| p.id) {
            for aspect in [
                AspectRatio::of(16, 9),
                AspectRatio::of(9, 16),
                AspectRatio::of(4, 1),
            ] {
                for seed in 0..8 {
                    let scene = generator(biome)
                        .generate(&diagnostic_seeds(seed), &FormSettings::default(), aspect)
                        .unwrap();
                    let checksum = scene.geometry_checksum();
                    let a = envelopes(&scene);
                    assert_eq!(a, envelopes(&scene));
                    assert_eq!(checksum, scene.geometry_checksum());
                    assert!(a.iter().all(|v| v.is_finite()));
                    assert!(a.len() * 4 < 4 * 1024 * 1024);
                    for i in 0..scene.layers().len() {
                        let h = &a[i * 8..i * 8 + 8];
                        let (first, count, owner) = (h[4] as usize, h[5] as usize, h[6] as usize);
                        assert!(owner <= i);
                        assert!(first + count <= a.len() / 4);
                        for k in first..first + count {
                            assert!(a[k * 4] <= a[k * 4 + 1]);
                        }
                        if owner != i {
                            assert_eq!(h, &a[owner * 8..owner * 8 + 8]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cloud_bands_and_rock_facets_share_the_whole_mass() {
        let mut cloud_facets = 0;
        let mut rock_facets = 0;
        for seed in 0..16 {
            let scene = generator(BiomeId::Alpine)
                .generate(
                    &diagnostic_seeds(seed),
                    &FormSettings::default(),
                    AspectRatio::of(16, 9),
                )
                .unwrap();
            let data = envelopes(&scene);
            for (i, layer) in scene.layers().iter().enumerate() {
                let owner = data[i * 8 + 6] as usize;
                if owner != i {
                    assert_eq!(scene.layers()[owner].role, layer.role);
                    assert_eq!(scene.layers()[owner].depth, layer.depth);
                    cloud_facets += usize::from(layer.role == LayerRole::Cloud);
                    rock_facets += usize::from(layer.role == LayerRole::ForegroundRock);
                }
            }
        }
        assert!(
            cloud_facets >= 32,
            "cumulus bands must not become separate volumes"
        );
        assert!(
            rock_facets > 50,
            "rock relief must continue across its planes"
        );
    }
}
