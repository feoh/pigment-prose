//! Artistic settings and the control specification.
//!
//! Three independent channels, never coupled by one "style" slider:
//! - **Structure** ([`FormSettings`]): may move geometry. Rebuilds the scene.
//! - **Paint handling** ([`PaintingSettings`]): edge behaviour, opacity,
//!   mark scale, texture. Repaints only; geometry must not change.
//! - **Appearance** ([`PaletteSettings`], [`AtmosphereSettings`],
//!   [`SeasonSettings`]): colour, haze and depth appearance, and the time of
//!   year. Repaints only; geometry must not change.
//!
//! [`CONTROLS`] is the single source of ranges, defaults and labels for the
//! UI (task 12), recipe validation and the renderer (tasks 05–07). Values
//! outside a range are *rejected* at the boundary (recipe load, API); the UI
//! clamps by construction. All values are dimensionless `f64`.

use serde::{Deserialize, Serialize};

use crate::error::{Problem, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Structure,
    PaintHandling,
    Appearance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Shown on the main panel.
    Main,
    /// Collapsed "Advanced" section.
    Advanced,
}

/// One slider. `key` is the dotted recipe field path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub channel: Channel,
    pub group: Group,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// What `min` means to the painter.
    pub low: &'static str,
    /// What `max` means to the painter.
    pub high: &'static str,
    /// Owning task that implements the visual effect.
    pub implemented_by: &'static str,
    /// `min` and `max` are the same moment (the year): keyboard steps wrap
    /// around instead of stopping at the ends.
    pub cyclic: bool,
}

/// Everything a slider edits: the structural settings and the appearance.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControlValues {
    pub form: FormSettings,
    pub appearance: Appearance,
}

impl ControlSpec {
    /// The field this control edits.
    fn slot<'a>(&self, v: &'a mut ControlValues) -> &'a mut f64 {
        let (f, a) = (&mut v.form, &mut v.appearance);
        match self.key {
            "form.faceting" => &mut f.faceting,
            "form.relief" => &mut f.relief,
            "form.woodland_density" => &mut f.woodland_density,
            "painting.edge_looseness" => &mut a.painting.edge_looseness,
            "painting.wash_gouache" => &mut a.painting.wash_gouache,
            "painting.mark_scale" => &mut a.painting.mark_scale,
            "painting.granulation" => &mut a.painting.granulation,
            "painting.paper_grain" => &mut a.painting.paper_grain,
            "palette.intensity" => &mut a.palette.intensity,
            "atmosphere.haze" => &mut a.atmosphere.haze,
            "season.year" => &mut a.season.year,
            other => unreachable!("control {other} has no field"),
        }
    }

    pub fn get(&self, v: &ControlValues) -> f64 {
        let mut copy = *v;
        *self.slot(&mut copy)
    }

    /// Sets this control's field and nothing else. Not validated: the UI
    /// clamps by construction and the document validates.
    pub fn set(&self, v: &mut ControlValues, value: f64) {
        *self.slot(v) = value;
    }

    pub fn check(&self, value: f64) -> Result<(), ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError {
                field: self.key,
                problem: Problem::NotFinite,
            });
        }
        if value < self.min || value > self.max {
            return Err(ValidationError {
                field: self.key,
                problem: Problem::OutOfRange {
                    value,
                    min: self.min,
                    max: self.max,
                },
            });
        }
        Ok(())
    }
}

macro_rules! control {
    ($key:literal, $label:literal, $ch:ident, $grp:ident, $min:literal..=$max:literal, default $def:literal, $low:literal => $high:literal, $task:literal) => {
        ControlSpec {
            key: $key,
            label: $label,
            channel: Channel::$ch,
            group: Group::$grp,
            min: $min,
            max: $max,
            default: $def,
            low: $low,
            high: $high,
            implemented_by: $task,
            cyclic: false,
        }
    };
}

pub const FACETING: ControlSpec = control!("form.faceting", "Form", Structure, Main,
    0.0..=1.0, default 0.55, "rounded, eroded masses" => "angular, faceted planes", "05");
pub const RELIEF: ControlSpec = control!("form.relief", "Relief", Structure, Advanced,
    0.0..=1.0, default 0.5, "low rolling hills" => "high, steep ridge", "05");
pub const WOODLAND_DENSITY: ControlSpec = control!("form.woodland_density", "Woodland density", Structure, Advanced,
    0.0..=1.0, default 0.5, "open, sparse groves" => "dense wooded masses", "07");

pub const EDGE_LOOSENESS: ControlSpec = control!("painting.edge_looseness", "Edge Looseness", PaintHandling, Main,
    0.0..=1.0, default 0.4, "controlled, crisp edges" => "soft, bleeding edges", "06");
pub const WASH_GOUACHE: ControlSpec = control!("painting.wash_gouache", "Wash / Gouache", PaintHandling, Main,
    0.0..=1.0, default 0.25, "translucent watercolor washes" => "opaque gouache body color", "06");
pub const MARK_SCALE: ControlSpec = control!("painting.mark_scale", "Mark scale", PaintHandling, Advanced,
    0.5..=2.0, default 1.0, "fine, small marks" => "broad, large marks", "06");
pub const GRANULATION: ControlSpec = control!("painting.granulation", "Granulation", PaintHandling, Advanced,
    0.0..=1.0, default 0.3, "smooth pigment" => "strongly settled pigment", "06");
pub const PAPER_GRAIN: ControlSpec = control!("painting.paper_grain", "Paper grain", PaintHandling, Advanced,
    0.0..=1.0, default 0.3, "hot-press, smooth" => "rough, visible tooth", "06");

pub const PALETTE_INTENSITY: ControlSpec = control!("palette.intensity", "Color intensity", Appearance, Main,
    0.0..=1.0, default 0.6, "muted, near-neutral" => "vivid, saturated", "06");
pub const HAZE: ControlSpec = control!("atmosphere.haze", "Atmosphere", Appearance, Main,
    0.0..=1.0, default 0.4, "clear, crisp distance" => "hazy, dissolving distance", "06");

/// The time of year (task 16): a cycle, so both ends are midwinter.
pub const SEASON: ControlSpec = ControlSpec {
    cyclic: true,
    ..control!("season.year", "Season", Appearance, Main,
        0.0..=1.0, default 0.5, "midwinter" => "midwinter, a year on", "16")
};

/// Every slider, in display order.
pub const CONTROLS: [ControlSpec; 11] = [
    FACETING,
    EDGE_LOOSENESS,
    WASH_GOUACHE,
    SEASON,
    HAZE,
    PALETTE_INTENSITY,
    RELIEF,
    WOODLAND_DENSITY,
    MARK_SCALE,
    GRANULATION,
    PAPER_GRAIN,
];

/// Maximum soft-edge/bleed radius in canvas units, reached at
/// `edge_looseness = 1`. Renderers must include it in their declared
/// support (docs/architecture.md, "Halo accounting").
pub const MAX_EDGE_BLEED_RADIUS: f64 = 0.02;

/// Structural settings. Changing any of these rebuilds the scene.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormSettings {
    pub faceting: f64,
    pub relief: f64,
    pub woodland_density: f64,
}

/// Paint handling. Must never change scene geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintingSettings {
    pub edge_looseness: f64,
    pub wash_gouache: f64,
    pub mark_scale: f64,
    pub granulation: f64,
    pub paper_grain: f64,
}

/// Authored palettes. Task 06 adds variants; ids are stable once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaletteId {
    /// Verdant high summer (the default).
    Lakeshore,
    /// Low gold evening light.
    GoldenEvening,
    /// Warm stone and sand with cool violet shadows.
    Desert,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteSettings {
    pub id: PaletteId,
    pub intensity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtmosphereSettings {
    pub haze: f64,
}

/// The time of year (task 16): `year` is the normalized year of
/// [`crate::season`], 0 and 1 both midwinter, 0.5 midsummer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonSettings {
    pub year: f64,
}

/// Everything the painter can change without rebuilding the scene.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Appearance {
    pub painting: PaintingSettings,
    pub palette: PaletteSettings,
    pub atmosphere: AtmosphereSettings,
    pub season: SeasonSettings,
}

impl Default for FormSettings {
    fn default() -> Self {
        FormSettings {
            faceting: FACETING.default,
            relief: RELIEF.default,
            woodland_density: WOODLAND_DENSITY.default,
        }
    }
}

impl Default for PaintingSettings {
    fn default() -> Self {
        PaintingSettings {
            edge_looseness: EDGE_LOOSENESS.default,
            wash_gouache: WASH_GOUACHE.default,
            mark_scale: MARK_SCALE.default,
            granulation: GRANULATION.default,
            paper_grain: PAPER_GRAIN.default,
        }
    }
}

impl Default for PaletteSettings {
    fn default() -> Self {
        PaletteSettings {
            id: PaletteId::Lakeshore,
            intensity: PALETTE_INTENSITY.default,
        }
    }
}

impl Default for AtmosphereSettings {
    fn default() -> Self {
        AtmosphereSettings { haze: HAZE.default }
    }
}

impl Default for SeasonSettings {
    fn default() -> Self {
        SeasonSettings {
            year: SEASON.default,
        }
    }
}

impl FormSettings {
    pub fn validate(&self) -> Result<(), ValidationError> {
        FACETING.check(self.faceting)?;
        RELIEF.check(self.relief)?;
        WOODLAND_DENSITY.check(self.woodland_density)
    }
}

impl PaintingSettings {
    pub fn validate(&self) -> Result<(), ValidationError> {
        EDGE_LOOSENESS.check(self.edge_looseness)?;
        WASH_GOUACHE.check(self.wash_gouache)?;
        MARK_SCALE.check(self.mark_scale)?;
        GRANULATION.check(self.granulation)?;
        PAPER_GRAIN.check(self.paper_grain)
    }

    /// Soft-edge radius in canvas units for this looseness.
    pub fn edge_bleed_radius(&self) -> f64 {
        MAX_EDGE_BLEED_RADIUS * self.edge_looseness
    }
}

impl Appearance {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.painting.validate()?;
        PALETTE_INTENSITY.check(self.palette.intensity)?;
        HAZE.check(self.atmosphere.haze)?;
        SEASON.check(self.season.year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_are_well_formed_and_unique() {
        let mut keys = std::collections::HashSet::new();
        for c in CONTROLS {
            assert!(keys.insert(c.key), "duplicate {}", c.key);
            assert!(c.min < c.max, "{}", c.key);
            assert!(c.check(c.default).is_ok(), "{} default out of range", c.key);
        }
        assert_eq!(keys.len(), CONTROLS.len());
    }

    #[test]
    fn the_three_named_channels_are_separate_main_controls() {
        assert_eq!(FACETING.channel, Channel::Structure);
        assert_eq!(EDGE_LOOSENESS.channel, Channel::PaintHandling);
        assert_eq!(WASH_GOUACHE.channel, Channel::PaintHandling);
        for c in [FACETING, EDGE_LOOSENESS, WASH_GOUACHE] {
            assert_eq!(c.group, Group::Main, "{}", c.key);
        }
        assert_ne!(EDGE_LOOSENESS.key, WASH_GOUACHE.key);
    }

    #[test]
    fn defaults_validate_and_bad_values_are_rejected_not_clamped() {
        assert!(FormSettings::default().validate().is_ok());
        assert!(Appearance::default().validate().is_ok());

        let mut a = Appearance::default();
        a.painting.edge_looseness = 1.5;
        let e = a.validate().unwrap_err();
        assert_eq!(e.field, "painting.edge_looseness");
        a.painting.edge_looseness = f64::NAN;
        assert_eq!(a.validate().unwrap_err().problem, Problem::NotFinite);

        let f = FormSettings {
            relief: -0.01,
            ..Default::default()
        };
        assert_eq!(f.validate().unwrap_err().field, "form.relief");
    }

    #[test]
    fn each_control_edits_only_its_own_field_in_its_channel() {
        let base = ControlValues::default();
        for c in CONTROLS {
            assert_eq!(c.get(&base), c.default, "{}", c.key);
            let mut v = base;
            let other = if c.default == c.max { c.min } else { c.max };
            c.set(&mut v, other);
            assert_eq!(c.get(&v), other, "{}", c.key);
            for d in CONTROLS.iter().filter(|d| d.key != c.key) {
                assert_eq!(d.get(&v), d.get(&base), "{} moved {}", c.key, d.key);
            }
            match c.channel {
                Channel::Structure => assert_eq!(v.appearance, base.appearance, "{}", c.key),
                _ => assert_eq!(v.form, base.form, "{}", c.key),
            }
        }
    }

    #[test]
    fn range_endpoints_are_inclusive() {
        for c in CONTROLS {
            assert!(
                c.check(c.min).is_ok() && c.check(c.max).is_ok(),
                "{}",
                c.key
            );
        }
    }
}
