//! Authored palettes (task 06).
//!
//! A palette is a small set of coordinated colors per *material*, each with
//! a light and a shadow side. A layer's structural `shade` picks between
//! them, so light and shadow come from the scene's geometry and light pool
//! rather than from random colors per polygon. Colors are authored in sRGB
//! and uploaded in linear light.
//!
//! `palette.intensity` scales saturation only (around luminance), never
//! brightness; `atmosphere.haze` mixes distant layers toward the haze color.

use crate::settings::PaletteId;

/// An sRGB color, authored as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Srgb(pub u32);

impl Srgb {
    /// Linear-light RGB (the exact sRGB transfer function).
    pub fn linear(self) -> [f32; 3] {
        let ch = |shift: u32| {
            let c = ((self.0 >> shift) & 0xff) as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [ch(16), ch(8), ch(0)]
    }
}

/// A material's lit and shadowed colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    pub light: Srgb,
    pub shadow: Srgb,
}

const fn mat(light: u32, shadow: u32) -> Material {
    Material {
        light: Srgb(light),
        shadow: Srgb(shadow),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub id: PaletteId,
    pub name: &'static str,
    /// The paper that washes glaze over.
    pub paper: Srgb,
    pub sky_zenith: Srgb,
    pub sky_horizon: Srgb,
    /// Warm glow toward the light.
    pub sun: Srgb,
    /// The color distance dissolves into.
    pub haze: Srgb,
    pub cloud: Material,
    pub storm: Srgb,
    /// Distant ranges.
    pub far: Material,
    pub rock: Material,
    pub snow: Material,
    pub forest: Material,
    /// Sunlit crowns (warm) and cool depths inside foliage.
    pub foliage_warm: Srgb,
    pub foliage_cool: Srgb,
    pub meadow: Material,
    pub moss: Srgb,
    /// Water near the viewer (deep) and toward the far shore (sky light).
    pub water_deep: Srgb,
    pub water_far: Srgb,
    pub water_sheen: Srgb,
    pub sand: Material,
    /// Per plant, in `Plant::TREES` order: broadleaf, conifer, birch,
    /// shrub, flowering, copper.
    pub plants: [Material; 6],
    /// Pale birch bark, and the woody stems of shrubs.
    pub bark: Srgb,
    pub wood: Srgb,
    /// Blossom colors of flowering trees.
    pub blossom: [Srgb; 2],
    /// Autumn leaves (task 16): gold, orange, red. Each deciduous crown
    /// turns toward one of them, by its own stable attribute.
    pub autumn: [Srgb; 3],
    /// Bare winter branches, lit and shadowed.
    pub twigs: Material,
    /// Grass cured to straw in late summer and autumn.
    pub dry_grass: Srgb,
}

/// Verdant high summer: sap and viridian greens with yellow-green light,
/// turquoise water, warm ochre rock against violet shadows.
pub const LAKESHORE: Palette = Palette {
    id: PaletteId::Lakeshore,
    name: "Lakeshore",
    paper: Srgb(0xf4eedf),
    sky_zenith: Srgb(0x4a82c4),
    sky_horizon: Srgb(0xd3e6ea),
    sun: Srgb(0xffe6ad),
    haze: Srgb(0xb9d2dc),
    cloud: mat(0xfffdf5, 0x97a3c0),
    storm: Srgb(0x5b6379),
    far: mat(0x93a8c8, 0x6a7aa8),
    rock: mat(0xd2bf98, 0x5d5876),
    snow: mat(0xffffff, 0xa6bad8),
    forest: mat(0x5f9a2e, 0x173f2c),
    foliage_warm: Srgb(0xb9d655),
    foliage_cool: Srgb(0x2a6b62),
    meadow: mat(0xaccb4f, 0x4c7c34),
    moss: Srgb(0x7ea83a),
    water_deep: Srgb(0x155a70),
    water_far: Srgb(0x8cc8cf),
    water_sheen: Srgb(0xe8f5ef),
    sand: mat(0xe6d2a2, 0x9a8c77),
    plants: [
        mat(0x62a832, 0x1b4a26), // broadleaf: bright sap green
        mat(0x2f7a5a, 0x0d3024), // conifer: deep blue-green
        mat(0xc2dc5e, 0x6a9a3e), // birch: light yellow-green
        mat(0x8f9c3c, 0x3c4222), // shrub: olive
        mat(0x7cbc4c, 0x2a5a30), // flowering
        mat(0x9a4e62, 0x421c2e), // copper beech: red-purple
    ],
    bark: Srgb(0xe8e2d2),
    wood: Srgb(0x6e4a2c),
    blossom: [Srgb(0xf08cbc), Srgb(0xfff4f0)],
    autumn: [Srgb(0xe8c03a), Srgb(0xe07a2a), Srgb(0xb8362c)],
    twigs: mat(0x7a6660, 0x3a2c34),
    dry_grass: Srgb(0xc9a85a),
};

/// Evening: low gold light, rose clouds, deep blue-green shadows.
pub const GOLDEN_EVENING: Palette = Palette {
    id: PaletteId::GoldenEvening,
    name: "Golden evening",
    paper: Srgb(0xf6ecdb),
    sky_zenith: Srgb(0x3d5b95),
    sky_horizon: Srgb(0xf5c68c),
    sun: Srgb(0xffc56e),
    haze: Srgb(0xd9b9a6),
    cloud: mat(0xffe0b3, 0x8b7a9c),
    storm: Srgb(0x4e4a66),
    far: mat(0xb89aa8, 0x6c6590),
    rock: mat(0xe3a978, 0x584866),
    snow: mat(0xffe6c9, 0x9a95c0),
    forest: mat(0x8ea23c, 0x1f3d31),
    foliage_warm: Srgb(0xe0c85a),
    foliage_cool: Srgb(0x2c5a5c),
    meadow: mat(0xc9bb55, 0x5a6d38),
    moss: Srgb(0x9aa640),
    water_deep: Srgb(0x21496a),
    water_far: Srgb(0xeab88e),
    water_sheen: Srgb(0xfff0d0),
    sand: mat(0xecc796, 0x8e7a70),
    plants: [
        mat(0x9eaa3a, 0x223e2a),
        mat(0x4a7258, 0x142c26),
        mat(0xe2d05e, 0x7e8a42),
        mat(0xa8963e, 0x44402a),
        mat(0xa8b84c, 0x34502e),
        mat(0xc8584a, 0x4c1c2a),
    ],
    bark: Srgb(0xf4e2c8),
    wood: Srgb(0x7a4a2a),
    blossom: [Srgb(0xf7a0b0), Srgb(0xfff0dc)],
    autumn: [Srgb(0xf0b848), Srgb(0xe8702e), Srgb(0xc0382e)],
    twigs: mat(0x8c6e5c, 0x3e2e36),
    dry_grass: Srgb(0xd8a860),
};

pub const PALETTES: [&Palette; 2] = [&LAKESHORE, &GOLDEN_EVENING];

pub fn palette(id: PaletteId) -> &'static Palette {
    match id {
        PaletteId::Lakeshore => &LAKESHORE,
        PaletteId::GoldenEvening => &GOLDEN_EVENING,
    }
}

/// Number of `vec4` entries [`Palette::gpu`] produces; the shader's
/// constants index into this layout.
pub const GPU_ENTRIES: usize = 48;

impl Palette {
    /// Linear RGB, one `[r, g, b, 0]` per entry, in the order the painting
    /// shader expects (see `paint.wgsl`, "palette layout").
    pub fn gpu(&self) -> Vec<[f32; 4]> {
        let e = |c: Srgb| {
            let [r, g, b] = c.linear();
            [r, g, b, 0.0]
        };
        let m = |m: Material| [e(m.light), e(m.shadow)];
        let mut out = vec![
            e(self.paper),
            e(self.sky_zenith),
            e(self.sky_horizon),
            e(self.sun),
            e(self.haze),
        ];
        out.extend(m(self.cloud));
        out.push(e(self.storm));
        out.extend(m(self.far));
        out.extend(m(self.rock));
        out.extend(m(self.snow));
        out.extend(m(self.forest));
        out.push(e(self.foliage_warm));
        out.push(e(self.foliage_cool));
        out.extend(m(self.meadow));
        out.push(e(self.moss));
        out.push(e(self.water_deep));
        out.push(e(self.water_far));
        out.push(e(self.water_sheen));
        out.extend(m(self.sand));
        for p in self.plants {
            out.extend(m(p));
        }
        out.push(e(self.bark));
        out.push(e(self.wood));
        out.extend(self.blossom.map(e));
        out.extend(self.autumn.map(e));
        out.extend(m(self.twigs));
        out.push(e(self.dry_grass));
        // Padding entries keep the count fixed for future materials.
        while out.len() < GPU_ENTRIES {
            out.push([0.0; 4]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_id_has_a_palette() {
        for p in PALETTES {
            assert_eq!(palette(p.id), p);
            let g = p.gpu();
            assert_eq!(g.len(), GPU_ENTRIES);
            assert!(
                g.iter()
                    .flatten()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            );
        }
    }

    #[test]
    fn srgb_to_linear_endpoints() {
        assert_eq!(Srgb(0x000000).linear(), [0.0; 3]);
        let w = Srgb(0xffffff).linear();
        assert!(w.iter().all(|c| (c - 1.0).abs() < 1e-6));
        // sRGB mid-gray 0x80 is about 0.216 linear.
        assert!((Srgb(0x808080).linear()[0] - 0.2158).abs() < 1e-3);
    }

    #[test]
    fn lit_sides_are_lighter_than_shadows() {
        let lum = |c: Srgb| {
            let [r, g, b] = c.linear();
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        for p in PALETTES {
            for (name, m) in [
                ("cloud", p.cloud),
                ("far", p.far),
                ("rock", p.rock),
                ("snow", p.snow),
                ("forest", p.forest),
                ("meadow", p.meadow),
                ("sand", p.sand),
                ("broadleaf", p.plants[0]),
                ("conifer", p.plants[1]),
                ("birch", p.plants[2]),
                ("shrub", p.plants[3]),
                ("flowering", p.plants[4]),
                ("copper", p.plants[5]),
                ("twigs", p.twigs),
            ] {
                assert!(lum(m.light) > lum(m.shadow), "{} {name}", p.name);
            }
        }
    }

    #[test]
    fn the_default_palette_is_green_where_it_should_be() {
        // "Verdant": forest and meadow greens dominate their channels.
        let [r, g, b] = LAKESHORE.forest.light.linear();
        assert!(g > r && g > b);
        let [r, g, b] = LAKESHORE.meadow.light.linear();
        assert!(g > r && g > b);
    }
}
