//! Registered landscape families. A biome profile supplies defaults and
//! capabilities to the shared scene, season and paint systems; it is not a
//! renderer or a copy of one.
//!
//! Only fully integrated profiles are advertised; stable IDs remain part of
//! recipe identity so selection survives save/load and scene caching.

use serde::{Deserialize, Serialize};

use crate::scene::Plant;
use crate::season::{self, SeasonProfile};
use crate::settings::{FormSettings, HAZE, PALETTE_INTENSITY, PaletteId, RELIEF, WOODLAND_DENSITY};

/// Stable internal identity for a supported landscape family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum BiomeId {
    #[default]
    Alpine,
    Desert,
    Tundra,
    Jungle,
}

impl BiomeId {
    /// Stable spelling persisted in schema-3 recipes.
    pub const fn as_str(self) -> &'static str {
        match self {
            BiomeId::Alpine => "alpine",
            BiomeId::Desert => "desert",
            BiomeId::Tundra => "tundra",
            BiomeId::Jungle => "jungle",
        }
    }

    /// Parse known stable IDs. An ID is not necessarily user-selectable yet:
    /// only profiles in `PROFILES` are complete and may be advertised.
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "alpine" => Some(BiomeId::Alpine),
            "desert" => Some(BiomeId::Desert),
            "tundra" => Some(BiomeId::Tundra),
            "jungle" => Some(BiomeId::Jungle),
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
    /// No persistent lake; any pools are isolated and use the shared water role.
    Sparse,
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
    palette_intensity: 0.82,
    haze: 0.24,
    season: &season::DESERT,
};

/// Approved low-growing vegetation and exposed tundra ground.
const JUNGLE_PLANTS: &[Plant] = &[
    Plant::Broadleaf,
    Plant::Birch,
    Plant::Shrub,
    Plant::Flowering,
];

pub const JUNGLE: BiomeProfile = BiomeProfile {
    id: BiomeId::Jungle,
    display_name: "Tropical jungle",
    form: FormSettings {
        faceting: 0.42,
        relief: 0.58,
        woodland_density: 0.82,
    },
    vegetation: JUNGLE_PLANTS,
    water: WaterForm::Sparse,
    palette: PaletteId::Jungle,
    palette_intensity: 0.78,
    haze: 0.48,
    season: &season::JUNGLE,
};

pub const TUNDRA: BiomeProfile = BiomeProfile {
    id: BiomeId::Tundra,
    display_name: "Open tundra",
    form: FormSettings {
        faceting: 0.38,
        relief: 0.30,
        woodland_density: 0.28,
    },
    vegetation: &[Plant::Shrub],
    water: WaterForm::Sparse,
    palette: PaletteId::Tundra,
    palette_intensity: 0.62,
    haze: 0.32,
    season: &season::TUNDRA,
};

/// Profiles available to application code. The UI should expose a selector
/// only when this registry contains more than one fully implemented profile.
/// Only profiles with owner-approved visual studies are selectable.
pub const PROFILES: &[BiomeProfile] = &[ALPINE, DESERT, TUNDRA, JUNGLE];

pub fn profile(id: BiomeId) -> &'static BiomeProfile {
    match id {
        BiomeId::Alpine => &ALPINE,
        BiomeId::Desert => &DESERT,
        BiomeId::Tundra => &TUNDRA,
        BiomeId::Jungle => &JUNGLE,
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
        if self.season.cycle == season::Cycle::DryWet
            && !self.season.keys.iter().all(|(_, state)| {
                state.snow == 0.0 && state.ground_snow == 0.0 && state.tree_snow == 0.0
            })
        {
            return Err("dry/wet seasonal profiles cannot use snow channels");
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
        assert_eq!(PROFILES.len(), 4);
        assert_eq!(profile(BiomeId::Alpine).id, BiomeId::Alpine);
        assert_eq!(BiomeId::parse("alpine"), Some(BiomeId::Alpine));
        assert_eq!(BiomeId::parse("desert"), Some(BiomeId::Desert));
        assert_eq!(BiomeId::parse("tundra"), Some(BiomeId::Tundra));
        assert_eq!(BiomeId::parse("jungle"), Some(BiomeId::Jungle));
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
    fn approved_tundra_profile_is_complete_and_advertised() {
        assert!(TUNDRA.validate().is_ok());
        assert_eq!(TUNDRA.id.as_str(), "tundra");
        assert_eq!(TUNDRA.palette, PaletteId::Tundra);
        assert_eq!(TUNDRA.season.cycle, season::Cycle::Temperate);
        assert!(PROFILES.iter().any(|profile| profile.id == BiomeId::Tundra));
        assert_eq!(TUNDRA.water, WaterForm::Sparse);
        assert_eq!(TUNDRA.vegetation, &[Plant::Shrub]);
    }

    #[test]
    fn approved_jungle_profile_is_complete_and_advertised() {
        assert!(JUNGLE.validate().is_ok());
        assert_eq!(JUNGLE.id.as_str(), "jungle");
        assert_eq!(JUNGLE.season.cycle, season::Cycle::DryWet);
        assert_eq!(JUNGLE.palette, PaletteId::Jungle);
        assert!(PROFILES.iter().any(|profile| profile.id == BiomeId::Jungle));
        assert_eq!(
            JUNGLE.vegetation,
            &[
                Plant::Broadleaf,
                Plant::Birch,
                Plant::Shrub,
                Plant::Flowering
            ]
        );
    }

    #[test]
    fn dry_wet_profiles_cannot_introduce_snow() {
        static SNOW_KEYS: &[(f64, season::SeasonState)] = &[
            (0.0, season::SeasonState::NEUTRAL),
            (
                0.5,
                season::SeasonState {
                    snow: 0.1,
                    ..season::SeasonState::NEUTRAL
                },
            ),
        ];
        static SNOW_PROFILE: season::SeasonProfile = season::SeasonProfile {
            name: "invalid dry/wet profile",
            cycle: season::Cycle::DryWet,
            keys: SNOW_KEYS,
        };
        let invalid = BiomeProfile {
            season: &SNOW_PROFILE,
            ..DESERT
        };
        assert_eq!(
            invalid.validate(),
            Err("dry/wet seasonal profiles cannot use snow channels")
        );
        assert!(DESERT.validate().is_ok());
    }

    #[test]
    fn desert_profile_is_registered_with_its_bounded_climate_and_landscape() {
        assert!(DESERT.validate().is_ok());
        assert_eq!(DESERT.id.as_str(), "desert");
        assert_eq!(DESERT.water, WaterForm::Arid);
        assert_eq!(DESERT.vegetation, &[Plant::Shrub]);
        assert_eq!(DESERT.palette, PaletteId::Desert);
        assert_eq!(DESERT.season.cycle, season::Cycle::DryWet);
        assert_eq!(DESERT.season.at(0.58).snow, 0.0);
        assert_eq!(DESERT.season.at(0.58).dry, 1.0);
        assert_eq!(PROFILES.len(), 4);
        assert!(PROFILES.iter().any(|profile| profile.id == BiomeId::Desert));
    }
}
