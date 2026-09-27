//! The versioned recipe: everything needed to reproduce a painting.
//!
//! This file fixes the *shape* and field names. Task 04 owns parsing policy
//! (unknown fields, unsupported/future versions, corrupt input, canonical
//! serialization) and task 10 owns files on disk (atomic save, the explicit
//! include-source-text choice). The example in
//! `docs/examples/recipe.example.json` is checked by a test below.

use serde::{Deserialize, Serialize};

use crate::error::ValidationError;
use crate::frame::Frame;
use crate::seed::{TextDigest, Variation};
use crate::settings::{
    Appearance, AtmosphereSettings, FormSettings, PaintingSettings, PaletteSettings,
};
use crate::version;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// Recipe format version (`version::RECIPE_SCHEMA_VERSION`).
    pub schema: u32,
    pub versions: RecipeVersions,
    pub seed: RecipeSeed,
    /// The document's frame. Its aspect ratio is part of the scene key; its
    /// pixel size is the default export size.
    pub frame: Frame,
    pub form: FormSettings,
    pub painting: PaintingSettings,
    pub palette: PaletteSettings,
    pub atmosphere: AtmosphereSettings,
    /// Original prose, present **only** if the user explicitly chose to keep
    /// it (task 10). Never required to reproduce the seed and never copied
    /// into exported images. Omitted from the file entirely when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

/// Algorithms and versions that produced this recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeVersions {
    pub normalization: String,
    pub seed_algorithm: String,
    pub generator: u32,
    pub renderer: u32,
}

impl RecipeVersions {
    pub fn current() -> RecipeVersions {
        RecipeVersions {
            normalization: version::NORMALIZATION_ID.to_string(),
            seed_algorithm: version::SEED_ALGORITHM_ID.to_string(),
            generator: version::GENERATOR_VERSION,
            renderer: version::RENDERER_VERSION,
        }
    }
}

/// The effective seed. Enough to rebuild every stream without the prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeSeed {
    pub digest: TextDigest,
    pub variation: Variation,
}

impl Recipe {
    /// A new document with default settings for an already-computed digest.
    pub fn new(digest: TextDigest, frame: Frame) -> Recipe {
        Recipe {
            schema: version::RECIPE_SCHEMA_VERSION,
            versions: RecipeVersions::current(),
            seed: RecipeSeed {
                digest,
                variation: Variation(0),
            },
            frame,
            form: FormSettings::default(),
            painting: PaintingSettings::default(),
            palette: PaletteSettings::default(),
            atmosphere: AtmosphereSettings::default(),
            source_text: None,
        }
    }

    pub fn appearance(&self) -> Appearance {
        Appearance {
            painting: self.painting,
            palette: self.palette,
            atmosphere: self.atmosphere,
        }
    }

    /// Value checks shared by every entry point. Version and format policy
    /// is task 04's.
    pub fn validate_values(&self) -> Result<(), ValidationError> {
        self.frame.validate()?;
        self.form.validate()?;
        self.appearance().validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../docs/examples/recipe.example.json");

    #[test]
    fn documented_example_parses_and_validates() {
        let r: Recipe = serde_json::from_str(EXAMPLE).unwrap();
        r.validate_values().unwrap();
        assert_eq!(r.schema, version::RECIPE_SCHEMA_VERSION);
        assert!(r.source_text.is_none());
    }

    #[test]
    fn round_trips_and_omits_absent_source_text() {
        let r = Recipe::new(TextDigest([0xab; 32]), crate::frame::UHD_4K);
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("source_text"), "absent, not null or empty");
        assert_eq!(serde_json::from_str::<Recipe>(&json).unwrap(), r);
    }
}
