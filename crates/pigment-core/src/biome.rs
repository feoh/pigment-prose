//! Registered landscape families. A biome profile supplies defaults and
//! capabilities to the shared scene, season and paint systems; it is not a
//! renderer or a copy of one.
//!
//! There is intentionally only one registered biome today. Until a second
//! complete profile is approved, prose continues to seed the alpine scene
//! exactly as before and the UI does not show a redundant biome picker.

use crate::scene::Plant;
use crate::season::{self, SeasonProfile};
use crate::settings::{FormSettings, HAZE, PALETTE_INTENSITY, PaletteId, RELIEF, WOODLAND_DENSITY};

/// Stable internal identity for a supported landscape family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BiomeId {
    Alpine,
    Desert,
}

impl BiomeId {
    /// Stable spelling reserved for versioned recipes once multiple biomes
    /// are exposed. Existing schema-2 recipes remain byte-for-byte unchanged
    /// and resolve to this sole registered profile.
    pub const fn as_str(self) -> &'static str {
        match self {
            BiomeId::Alpine => "alpine",
            BiomeId::Desert => "desert",
        }
    }

    /// Parse known stable IDs. An ID is not necessarily user-selectable yet:
    /// only profiles in `PROFILES` are complete and may be advertised.
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "alpine" => Some(BiomeId::Alpine),
            "desert" => Some(BiomeId::Desert),
            _ => None,
        }
    }
}

/// Water form supplied by the shared scene generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaterForm {
    /// An inland lake with a shoreline, as in the approved scene family.
    Lake,
    /// No persistent open-water body in the rocky-desert family.
    Arid,
}

/// A complete, validated set of defaults/capabilities for one landscape.
/// Structural values may change composition; paint-only settings never enter
/// the `SceneKey` and must not affect its geometry.
#[derive(Debug, Clone, Copy)]
pub struct BiomeProfile {
    pub id: BiomeId,
    pub display_name: &'static str,
    pub form: FormSettings,
    pub vegetation: &'static [Plant],
    pub water: WaterForm,
    pub palette: PaletteId,
    pub palette_intensity: f64,
    pub haze: f64,
    pub season: &'static SeasonProfile,
}

/// The approved mountain-lakeshore landscape. Defaults deliberately match
/// the pre-profile application defaults so registering the profile cannot
/// alter existing recipes or approved renders.
pub const ALPINE: BiomeProfile = BiomeProfile {
    id: BiomeId::Alpine,
    display_name: "Alpine lakeshore",
    form: FormSettings {
        faceting: crate::settings::FACETING.default,
        relief: RELIEF.default,
        woodland_density: WOODLAND_DENSITY.default,
    },
    vegetation: &Plant::TREES,
    water: WaterForm::Lake,
    palette: PaletteId::Lakeshore,
    palette_intensity: PALETTE_INTENSITY.default,
    haze: HAZE.default,
    season: &season::ALPINE,
};

/// Desert vegetation is intentionally restricted to low shrubs; the shared
/// renderer already knows how to paint this plant form.
const DESERT_PLANTS: &[Plant] = &[Plant::Shrub];

/// Profile values are kept out of `PROFILES` until terrain, recipe selection,
/// the painter and visual-review fixtures are complete. This lets the profile
/// contract and authored season/palette be validated without advertising an
/// incomplete biome as a working choice.
pub const DESERT: BiomeProfile = BiomeProfile {
    id: BiomeId::Desert,
    display_name: "Rocky desert",
    form: FormSettings {
        faceting: 0.78,
        relief: 0.42,
        woodland_density: 0.14,
    },
    vegetation: DESERT_PLANTS,
    water: WaterForm::Arid,
    palette: PaletteId::Desert,
    palette_intensity: 0.58,
    haze: 0.34,
    season: &season::DESERT,
};

/// Profiles available to application code. The UI should expose a selector
/// only when this registry contains more than one fully implemented profile.
pub const PROFILES: &[BiomeProfile] = &[ALPINE];

pub fn profile(id: BiomeId) -> &'static BiomeProfile {
    match id {
        BiomeId::Alpine => &ALPINE,
        BiomeId::Desert => &DESERT,
    }
}

impl BiomeProfile {
    /// Validate profile data at registration/test boundaries. User-selected
    /// setting ranges remain owned by `settings::CONTROLS`.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.vegetation.is_empty() {
            return Err("a biome must permit at least one vegetation form");
        }
        if self.vegetation.contains(&Plant::None) {
            return Err("vegetation capabilities cannot include Plant::None");
        }
        if self.season.keys.len() < 2
            || !self
                .season
                .keys
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
            || !self.season.keys.iter().all(|(time, state)| {
                (0.0..1.0).contains(time)
                    && state
                        .channels()
                        .iter()
                        .all(|value| (0.0..=1.0).contains(value))
            })
        {
            return Err("season keyframes must be ordered, in range and bounded");
        }
        for (spec, value) in [
            ("form.faceting", self.form.faceting),
            ("form.relief", self.form.relief),
            ("form.woodland_density", self.form.woodland_density),
            ("palette.intensity", self.palette_intensity),
            ("atmosphere.haze", self.haze),
        ] {
            let control = crate::settings::CONTROLS
                .iter()
                .find(|control| control.key == spec)
                .expect("profile setting has a registered control");
            if !value.is_finite() || value < control.min || value > control.max {
                return Err("profile defaults must fit the registered control ranges");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_registered_biomes_are_advertised() {
        assert_eq!(PROFILES.len(), 1);
        assert_eq!(profile(BiomeId::Alpine).id, BiomeId::Alpine);
        assert_eq!(BiomeId::parse("alpine"), Some(BiomeId::Alpine));
        assert_eq!(BiomeId::parse("desert"), Some(BiomeId::Desert));
        assert_eq!(BiomeId::parse("unknown"), None);
        assert!(PROFILES.iter().all(|profile| profile.validate().is_ok()));
    }

    #[test]
    fn alpine_profile_preserves_existing_application_defaults() {
        assert_eq!(ALPINE.form, FormSettings::default());
        assert_eq!(ALPINE.palette, PaletteId::Lakeshore);
        assert_eq!(ALPINE.season.name, season::ALPINE.name);
        assert_eq!(ALPINE.vegetation, &Plant::TREES);
    }

    #[test]
    fn desert_profile_is_valid_but_not_advertised_before_its_generator_is_complete() {
        assert!(DESERT.validate().is_ok());
        assert_eq!(DESERT.id.as_str(), "desert");
        assert_eq!(DESERT.water, WaterForm::Arid);
        assert_eq!(DESERT.vegetation, &[Plant::Shrub]);
        assert_eq!(DESERT.palette, PaletteId::Desert);
        assert_eq!(DESERT.season.cycle, season::Cycle::DryWet);
        assert_eq!(DESERT.season.at(0.58).snow, 0.0);
        assert_eq!(DESERT.season.at(0.58).dry, 1.0);
        assert_eq!(PROFILES.len(), 1);
        assert!(!PROFILES.iter().any(|profile| profile.id == BiomeId::Desert));
    }
}
