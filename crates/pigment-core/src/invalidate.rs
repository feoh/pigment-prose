//! Which pipeline stages a document change invalidates.
//!
//! | Change | Seeds | Scene | Paint |
//! | --- | --- | --- | --- |
//! | prose (new digest), variation, normalization/seed algorithm | ✓ | ✓ | ✓ |
//! | biome, form settings, frame aspect ratio, generator version | | ✓ | ✓ |
//! | painting, palette, atmosphere, renderer version | | | ✓ |
//! | frame pixel size at the same aspect ratio | | | |
//! | keeping or dropping `source_text` | | | |
//!
//! Outside the recipe: resizing the preview area repaints only (same scene,
//! new target size); changing the export size never affects the preview.

use crate::recipe::Recipe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Invalidation {
    /// Re-derive the seed bundle.
    pub seeds: bool,
    /// Rebuild the `Scene` (geometry and placements).
    pub scene: bool,
    /// Repaint the preview with the existing scene.
    pub paint: bool,
}

impl Invalidation {
    pub const NONE: Invalidation = Invalidation {
        seeds: false,
        scene: false,
        paint: false,
    };

    pub fn between(old: &Recipe, new: &Recipe) -> Invalidation {
        let seeds = old.seed != new.seed
            || old.versions.normalization != new.versions.normalization
            || old.versions.seed_algorithm != new.versions.seed_algorithm;
        let scene = seeds
            || old.biome != new.biome
            || old.form != new.form
            || old.frame.aspect() != new.frame.aspect()
            || old.versions.generator != new.versions.generator;
        let paint = scene
            || old.appearance() != new.appearance()
            || old.versions.renderer != new.versions.renderer;
        Invalidation {
            seeds,
            scene,
            paint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, UHD_4K, UHD_8K};
    use crate::seed::{TextDigest, Variation};

    fn base() -> Recipe {
        Recipe::new(TextDigest([1; 32]), UHD_4K)
    }

    fn inv(edit: impl FnOnce(&mut Recipe)) -> Invalidation {
        let old = base();
        let mut new = old.clone();
        edit(&mut new);
        Invalidation::between(&old, &new)
    }

    const ALL: Invalidation = Invalidation {
        seeds: true,
        scene: true,
        paint: true,
    };
    const SCENE: Invalidation = Invalidation {
        seeds: false,
        scene: true,
        paint: true,
    };
    const PAINT: Invalidation = Invalidation {
        seeds: false,
        scene: false,
        paint: true,
    };

    #[test]
    fn seed_changes_invalidate_everything() {
        assert_eq!(inv(|r| r.seed.digest = TextDigest([2; 32])), ALL);
        assert_eq!(inv(|r| r.seed.variation = Variation(1)), ALL);
    }

    #[test]
    fn form_and_aspect_rebuild_the_scene() {
        assert_eq!(inv(|r| r.form.faceting = 0.9), SCENE);
        assert_eq!(inv(|r| r.form.woodland_density = 0.1), SCENE);
        assert_eq!(inv(|r| r.frame = UHD_4K.rotated()), SCENE);
        assert_eq!(inv(|r| r.versions.generator += 1), SCENE);
    }

    #[test]
    fn biome_changes_invalidate_scene_and_paint_but_not_seeds() {
        assert_eq!(inv(|r| r.biome = crate::biome::BiomeId::Desert), SCENE);
    }

    #[test]
    fn paint_only_changes_keep_the_scene() {
        assert_eq!(inv(|r| r.painting.edge_looseness = 0.9), PAINT);
        assert_eq!(inv(|r| r.painting.wash_gouache = 0.9), PAINT);
        assert_eq!(inv(|r| r.painting.mark_scale = 2.0), PAINT);
        assert_eq!(inv(|r| r.palette.intensity = 0.1), PAINT);
        assert_eq!(inv(|r| r.atmosphere.haze = 1.0), PAINT);
        assert_eq!(inv(|r| r.season.year = 0.0), PAINT);
    }

    #[test]
    fn resolution_and_source_text_change_nothing() {
        assert_eq!(inv(|r| r.frame = UHD_8K), Invalidation::NONE);
        assert_eq!(
            inv(|r| r.frame = Frame {
                width: 1920,
                height: 1080
            }),
            Invalidation::NONE
        );
        assert_eq!(
            inv(|r| r.source_text = Some("kept".into())),
            Invalidation::NONE
        );
    }
}
