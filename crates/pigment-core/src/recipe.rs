//! The versioned recipe: everything needed to reproduce a painting.
//!
//! This file owns the format and the load/save policy (task 04); task 10
//! owns files on disk (atomic save, the explicit include-source-text
//! choice). The full policy is in `docs/seeds-and-recipes.md`. In short:
//!
//! - [`Recipe::from_json`] is the only way to read a recipe. It checks, in
//!   order: size, JSON syntax and duplicate keys, the `schema` number, then
//!   every key against [`SCHEMA`] (unknown and missing keys, JSON types),
//!   then algorithm ids, value ranges and `source_text`. The first problem
//!   is reported and nothing is applied. Nothing is clamped, defaulted or
//!   ignored, so a file can never quietly produce a different painting.
//! - [`Recipe::to_canonical_json`] runs the same checks and writes the one
//!   canonical form: fields in schema order, two-space indentation, shortest
//!   round-trip numbers, a trailing newline, and no `source_text` key when
//!   the prose was not kept.
//! - Generator and renderer version differences are *not* errors; they are
//!   reported by [`Recipe::version_notices`] for the UI to show.
//! - **Schema 1** (before task 16's seasons) is read through one explicit
//!   migration: it is checked against its own table, then gains
//!   `season.year = 0.5`, midsummer, the look every schema 1 painting was
//!   made with, so it paints exactly as before. Saving writes schema 2.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::error::{MalformedKind, RecipeError, ValidationError, sanitized_key};
use crate::frame::Frame;
use crate::seed::{SeedBundle, TextDigest, Variation};
use crate::settings::{
    Appearance, AtmosphereSettings, FormSettings, PaintingSettings, PaletteId, PaletteSettings,
    SeasonSettings,
};
use crate::version;

/// Upper bound on a recipe document, in bytes. `source_text` is at most
/// [`crate::text::MAX_SOURCE_BYTES`] (1 MiB) of prose, and JSON escaping
/// grows a byte to at most six (`\u001f`), so 8 MiB covers every valid file.
pub const MAX_RECIPE_BYTES: usize = 8 << 20;

/// `Debug` is written by hand so a kept `source_text` shows only its length:
/// a recipe can be logged or appear in a test failure without its prose.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// The time of year (schema 2, task 16).
    pub season: SeasonSettings,
    /// Original prose, present **only** if the user explicitly chose to keep
    /// it (task 10). Never required to reproduce the seed and never copied
    /// into exported images. Omitted from the file entirely when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

impl fmt::Debug for Recipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = self
            .source_text
            .as_ref()
            .map(|t| format!("<{} bytes>", t.len()));
        f.debug_struct("Recipe")
            .field("schema", &self.schema)
            .field("versions", &self.versions)
            .field("seed", &self.seed)
            .field("frame", &self.frame)
            .field("form", &self.form)
            .field("painting", &self.painting)
            .field("palette", &self.palette)
            .field("atmosphere", &self.atmosphere)
            .field("season", &self.season)
            .field("source_text", &source)
            .finish()
    }
}

/// Algorithms and versions that produced this recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RecipeSeed {
    pub digest: TextDigest,
    pub variation: Variation,
}

/// A recorded generator or renderer version that differs from this build's.
/// The recipe still loads; the UI says the painting may differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionNotice {
    pub component: Component,
    pub recorded: u32,
    pub current: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Generator,
    Renderer,
}

impl fmt::Display for VersionNotice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, verb) = match self.component {
            Component::Generator => ("generator", "compose"),
            Component::Renderer => ("renderer", "paint"),
        };
        write!(
            f,
            "made with {name} v{}; this version (v{}) may {verb} differently",
            self.recorded, self.current
        )
    }
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
            season: SeasonSettings::default(),
            source_text: None,
        }
    }

    /// Parses and fully validates a recipe document. See the module docs
    /// for the order of checks. Errors never contain values from the file.
    pub fn from_json(json: &str) -> Result<Recipe, RecipeError> {
        Self::from_json_with_schema(json).map(|(r, _)| r)
    }

    /// As [`from_json`](Recipe::from_json), and the schema the file was
    /// written in (1 if it was migrated), so the UI can say that saving will
    /// update the format.
    pub fn from_json_with_schema(json: &str) -> Result<(Recipe, u32), RecipeError> {
        if json.len() > MAX_RECIPE_BYTES {
            return Err(RecipeError::TooLarge {
                bytes: json.len(),
                max: MAX_RECIPE_BYTES,
            });
        }
        let Strict(mut value) = serde_json::from_str(json).map_err(malformed)?;
        let found = check_schema_number(&value)?;
        if found == 1 {
            check_object(&value, SCHEMA_V1, "")?;
            migrate_v1(&mut value);
        }
        check_object(&value, SCHEMA, "")?;
        // The walk above has checked every key and type, so typing cannot
        // fail unless SCHEMA and the structs disagree (a test prevents it).
        let recipe: Recipe = serde_json::from_value(value).map_err(|_| RecipeError::WrongType {
            path: String::new(),
            expected: "a Pigment Prose recipe",
        })?;
        recipe.validate()?;
        Ok((recipe, found))
    }

    /// The canonical serialization, after the same checks as
    /// [`from_json`](Recipe::from_json), so every saved file loads again.
    pub fn to_canonical_json(&self) -> Result<String, RecipeError> {
        self.validate()?;
        let mut out = serde_json::to_string_pretty(self).expect("recipe serializes");
        out.push('\n');
        Ok(out)
    }

    /// Everything [`from_json`](Recipe::from_json) checks after typing:
    /// schema and algorithm ids, value ranges, and that a kept
    /// `source_text` reproduces `seed.digest`.
    pub fn validate(&self) -> Result<(), RecipeError> {
        if self.schema != version::RECIPE_SCHEMA_VERSION {
            return Err(RecipeError::UnsupportedSchema {
                found: self.schema.into(),
                supported: version::RECIPE_SCHEMA_VERSION,
            });
        }
        for (field, found, ours) in [
            (
                "normalization",
                &self.versions.normalization,
                version::NORMALIZATION_ID,
            ),
            (
                "seed_algorithm",
                &self.versions.seed_algorithm,
                version::SEED_ALGORITHM_ID,
            ),
        ] {
            if found != ours {
                return Err(RecipeError::UnsupportedAlgorithm {
                    field,
                    found: sanitized_key(found),
                });
            }
        }
        self.validate_values()?;
        if let Some(text) = &self.source_text {
            let digest = TextDigest::from_source(text).map_err(RecipeError::SourceText)?;
            if digest != self.seed.digest {
                return Err(RecipeError::SourceTextMismatch);
            }
        }
        Ok(())
    }

    /// Range checks on the frame and every setting.
    pub fn validate_values(&self) -> Result<(), ValidationError> {
        self.frame.validate()?;
        self.form.validate()?;
        self.appearance().validate()
    }

    /// The seed bundle. Depends only on `seed`, never on `source_text` or
    /// any setting.
    pub fn seeds(&self) -> SeedBundle {
        SeedBundle::derive(self.seed.digest, self.seed.variation)
    }

    /// Generator and renderer versions that differ from this build's.
    pub fn version_notices(&self) -> Vec<VersionNotice> {
        [
            (
                Component::Generator,
                self.versions.generator,
                version::GENERATOR_VERSION,
            ),
            (
                Component::Renderer,
                self.versions.renderer,
                version::RENDERER_VERSION,
            ),
        ]
        .into_iter()
        .filter(|(_, recorded, current)| recorded != current)
        .map(|(component, recorded, current)| VersionNotice {
            component,
            recorded,
            current,
        })
        .collect()
    }

    pub fn appearance(&self) -> Appearance {
        Appearance {
            painting: self.painting,
            palette: self.palette,
            atmosphere: self.atmosphere,
            season: self.season,
        }
    }
}

// ---------------------------------------------------------------------------
// Structural check against the schema table.

/// One key of the recipe format.
#[derive(Debug)]
pub struct Field {
    pub key: &'static str,
    pub kind: Kind,
    pub required: bool,
}

/// The JSON shape a key must have.
#[derive(Debug)]
pub enum Kind {
    Object(&'static [Field]),
    /// Any JSON number; ranges are checked by `validate_values`.
    Number,
    /// A non-negative integer that fits in `u32`.
    Uint32,
    Text,
    /// 64 lowercase hex digits.
    Digest,
    Palette,
}

const fn req(key: &'static str, kind: Kind) -> Field {
    Field {
        key,
        kind,
        required: true,
    }
}

const F_SCHEMA: Field = req("schema", Kind::Uint32);
const F_VERSIONS: Field = req(
    "versions",
    Kind::Object(&[
        req("normalization", Kind::Text),
        req("seed_algorithm", Kind::Text),
        req("generator", Kind::Uint32),
        req("renderer", Kind::Uint32),
    ]),
);
const F_SEED: Field = req(
    "seed",
    Kind::Object(&[req("digest", Kind::Digest), req("variation", Kind::Uint32)]),
);
const F_FRAME: Field = req(
    "frame",
    Kind::Object(&[req("width", Kind::Uint32), req("height", Kind::Uint32)]),
);
const F_FORM: Field = req(
    "form",
    Kind::Object(&[
        req("faceting", Kind::Number),
        req("relief", Kind::Number),
        req("woodland_density", Kind::Number),
    ]),
);
const F_PAINTING: Field = req(
    "painting",
    Kind::Object(&[
        req("edge_looseness", Kind::Number),
        req("wash_gouache", Kind::Number),
        req("mark_scale", Kind::Number),
        req("granulation", Kind::Number),
        req("paper_grain", Kind::Number),
    ]),
);
const F_PALETTE: Field = req(
    "palette",
    Kind::Object(&[req("id", Kind::Palette), req("intensity", Kind::Number)]),
);
const F_ATMOSPHERE: Field = req("atmosphere", Kind::Object(&[req("haze", Kind::Number)]));
const F_SEASON: Field = req("season", Kind::Object(&[req("year", Kind::Number)]));
const F_SOURCE_TEXT: Field = Field {
    key: "source_text",
    kind: Kind::Text,
    required: false,
};

/// Schema 2 (the current format), in canonical order. `pub` so
/// documentation and tests can walk it; a test checks it against the serde
/// structs.
pub const SCHEMA: &[Field] = &[
    F_SCHEMA,
    F_VERSIONS,
    F_SEED,
    F_FRAME,
    F_FORM,
    F_PAINTING,
    F_PALETTE,
    F_ATMOSPHERE,
    F_SEASON,
    F_SOURCE_TEXT,
];

/// Schema 1: schema 2 without `season`.
pub const SCHEMA_V1: &[Field] = &[
    F_SCHEMA,
    F_VERSIONS,
    F_SEED,
    F_FRAME,
    F_FORM,
    F_PAINTING,
    F_PALETTE,
    F_ATMOSPHERE,
    F_SOURCE_TEXT,
];

/// Schema 1 → 2: every schema 1 painting was made at midsummer, the look
/// before seasons existed, so that is its season.
fn migrate_v1(root: &mut Value) {
    let obj = root.as_object_mut().expect("checked against SCHEMA_V1");
    obj.insert("schema".into(), version::RECIPE_SCHEMA_VERSION.into());
    let mut season = Map::new();
    season.insert(
        "year".into(),
        Number::from_f64(crate::season::DEFAULT_YEAR)
            .expect("finite")
            .into(),
    );
    obj.insert("season".into(), Value::Object(season));
}

fn join(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

/// Read `schema` before anything else, so a future format gets a clear
/// "unsupported schema" rather than a confusing missing-field error.
/// Returns it: 1 (migrated) or the current schema.
fn check_schema_number(root: &Value) -> Result<u32, RecipeError> {
    let Some(obj) = root.as_object() else {
        return Err(RecipeError::WrongType {
            path: String::new(),
            expected: "a JSON object",
        });
    };
    let Some(v) = obj.get("schema") else {
        return Err(RecipeError::MissingField {
            path: "schema".into(),
        });
    };
    let Some(found) = v.as_u64() else {
        return Err(RecipeError::WrongType {
            path: "schema".into(),
            expected: "a whole number",
        });
    };
    if found != 1 && found != u64::from(version::RECIPE_SCHEMA_VERSION) {
        return Err(RecipeError::UnsupportedSchema {
            found,
            supported: version::RECIPE_SCHEMA_VERSION,
        });
    }
    Ok(found as u32)
}

fn check_object(value: &Value, fields: &[Field], path: &str) -> Result<(), RecipeError> {
    let Some(obj) = value.as_object() else {
        return Err(RecipeError::WrongType {
            path: path.to_string(),
            expected: "an object",
        });
    };
    if let Some(key) = obj.keys().find(|k| !fields.iter().any(|f| f.key == *k)) {
        return Err(RecipeError::UnknownField {
            path: join(path, &sanitized_key(key)),
        });
    }
    for field in fields {
        let here = join(path, field.key);
        match obj.get(field.key) {
            None if field.required => return Err(RecipeError::MissingField { path: here }),
            None => {}
            Some(v) => check_value(v, &field.kind, here)?,
        }
    }
    Ok(())
}

fn check_value(v: &Value, kind: &Kind, path: String) -> Result<(), RecipeError> {
    let wrong = |expected| RecipeError::WrongType {
        path: path.clone(),
        expected,
    };
    let invalid = |reason| RecipeError::InvalidValue {
        path: path.clone(),
        reason,
    };
    match kind {
        Kind::Object(fields) => check_object(v, fields, &path),
        Kind::Number if v.is_number() => Ok(()),
        Kind::Number => Err(wrong("a number")),
        Kind::Uint32 => match v.as_u64() {
            Some(n) if u32::try_from(n).is_ok() => Ok(()),
            _ => Err(wrong("a whole number from 0 to 4294967295")),
        },
        Kind::Text if v.is_string() => Ok(()),
        Kind::Text => Err(wrong("a string")),
        Kind::Digest => match v.as_str() {
            Some(s) if TextDigest::from_hex(s).is_some() => Ok(()),
            Some(_) => Err(invalid("must be 64 lowercase hex digits")),
            None => Err(wrong("a string")),
        },
        Kind::Palette => match v.as_str() {
            Some(_) if PaletteId::deserialize(v).is_ok() => Ok(()),
            Some(_) => Err(invalid("is not a known palette")),
            None => Err(wrong("a string")),
        },
    }
}

// ---------------------------------------------------------------------------
// Strict JSON: a `serde_json::Value` that rejects duplicate keys, which
// `Value` itself silently collapses (last one wins).

struct Strict(Value);

const DUPLICATE_KEY: &str = "duplicate key";

fn malformed(e: serde_json::Error) -> RecipeError {
    let kind = if e.is_eof() {
        MalformedKind::Truncated
    } else if e.is_data() && e.to_string().starts_with(DUPLICATE_KEY) {
        MalformedKind::DuplicateKey
    } else {
        MalformedKind::Syntax
    };
    RecipeError::Malformed {
        line: e.line(),
        column: e.column(),
        kind,
    }
}

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Strict;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_unit<E>(self) -> Result<Strict, E> {
        Ok(Strict(Value::Null))
    }

    fn visit_bool<E>(self, v: bool) -> Result<Strict, E> {
        Ok(Strict(Value::Bool(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Strict, E> {
        Ok(Strict(Value::Number(v.into())))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Strict, E> {
        Ok(Strict(Value::Number(v.into())))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
        Number::from_f64(v)
            .map(|n| Strict(Value::Number(n)))
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Strict, E> {
        Ok(Strict(Value::String(v.to_string())))
    }

    fn visit_string<E>(self, v: String) -> Result<Strict, E> {
        Ok(Strict(Value::String(v)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
        let mut out = Vec::new();
        while let Some(Strict(v)) = seq.next_element()? {
            out.push(v);
        }
        Ok(Strict(Value::Array(out)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(de::Error::custom(DUPLICATE_KEY));
            }
            let Strict(v) = map.next_value()?;
            out.insert(key, v);
        }
        Ok(Strict(Value::Object(out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Problem, TextError};
    use crate::frame::UHD_4K;
    use crate::seed::Domain;

    const EXAMPLE: &str = include_str!("../../../docs/examples/recipe.example.json");
    const SHORE_A: &str = "A pebble rests by the shore.";

    fn sample() -> Recipe {
        Recipe::new(TextDigest::from_source(SHORE_A).unwrap(), UHD_4K)
    }

    fn json_of(r: &Recipe) -> Value {
        serde_json::to_value(r).unwrap()
    }

    /// Load a `sample()` edited as JSON.
    fn load_edited(edit: impl FnOnce(&mut Value)) -> Result<Recipe, RecipeError> {
        let mut v = json_of(&sample());
        edit(&mut v);
        Recipe::from_json(&v.to_string())
    }

    #[test]
    fn documented_example_is_canonical_and_loads() {
        let r = Recipe::from_json(EXAMPLE).unwrap();
        assert_eq!(r.schema, version::RECIPE_SCHEMA_VERSION);
        assert!(r.source_text.is_none());
        assert_eq!(r.seed.digest, TextDigest::from_source(SHORE_A).unwrap());
        assert_eq!(r.seed.variation, Variation(2));
        assert_eq!(r.to_canonical_json().unwrap(), EXAMPLE, "byte-identical");
    }

    #[test]
    fn canonical_json_round_trips() {
        let mut r = sample();
        r.seed.variation = Variation(u32::MAX);
        r.painting.mark_scale = 0.5;
        r.form.faceting = 1.0 / 3.0;
        r.frame = Frame::new(1000, 4000).unwrap();
        let json = r.to_canonical_json().unwrap();
        assert!(!json.contains("source_text"), "absent, not null or empty");
        assert!(json.ends_with("}\n"));
        let back = Recipe::from_json(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.to_canonical_json().unwrap(), json);
    }

    #[test]
    fn omitted_source_text_still_reproduces_the_seed() {
        let mut kept = sample();
        kept.source_text = Some(SHORE_A.to_string());
        let with = Recipe::from_json(&kept.to_canonical_json().unwrap()).unwrap();
        let without = Recipe::from_json(&sample().to_canonical_json().unwrap()).unwrap();
        assert_eq!(with.source_text.as_deref(), Some(SHORE_A));
        assert_eq!(with.seeds(), without.seeds());
        assert_eq!(
            without.seeds(),
            SeedBundle::derive(TextDigest::from_source(SHORE_A).unwrap(), Variation(0))
        );
    }

    #[test]
    fn source_text_must_match_the_digest() {
        let mut r = sample();
        r.source_text = Some("A pebble rests by the shores.".into());
        assert_eq!(r.validate(), Err(RecipeError::SourceTextMismatch));
        // Normalization-equivalent text is the same prose.
        r.source_text = Some("A pebble rests by the shore.".into());
        assert_eq!(r.validate(), Ok(()));
        r.source_text = Some("   ".into());
        assert_eq!(
            r.validate(),
            Err(RecipeError::SourceText(TextError::WhitespaceOnly))
        );
        assert!(
            r.to_canonical_json().is_err(),
            "never save what cannot load"
        );
    }

    #[test]
    fn palette_and_paint_settings_do_not_touch_seeds() {
        let base = sample();
        let mut r = base.clone();
        r.palette.intensity = 0.0;
        r.atmosphere.haze = 1.0;
        r.painting.edge_looseness = 1.0;
        r.painting.wash_gouache = 1.0;
        r.form.faceting = 0.0;
        r.frame = r.frame.rotated();
        assert_eq!(r.seeds(), base.seeds());
    }

    #[test]
    fn variation_changes_the_scene_streams_only() {
        let a = sample();
        let mut b = a.clone();
        b.seed.variation = Variation(1);
        for d in Domain::ALL {
            assert_eq!(
                a.seeds().stream(d) == b.seeds().stream(d),
                !d.uses_variation()
            );
        }
    }

    #[test]
    fn schema_table_matches_the_structs() {
        fn collect(fields: &[Field], prefix: &str, out: &mut Vec<String>) {
            for f in fields {
                let p = join(prefix, f.key);
                if let Kind::Object(inner) = &f.kind {
                    collect(inner, &p, out);
                } else {
                    out.push(p);
                }
            }
        }
        fn leaves(v: &Value, prefix: &str, out: &mut Vec<String>) {
            for (k, v) in v.as_object().unwrap() {
                let p = join(prefix, k);
                if v.is_object() {
                    leaves(v, &p, out);
                } else {
                    out.push(p);
                }
            }
        }
        let mut r = sample();
        r.source_text = Some(SHORE_A.into());
        let mut from_table = Vec::new();
        collect(SCHEMA, "", &mut from_table);
        // Serialization follows struct order, which must be the table's.
        let json: Value = serde_json::from_str(&r.to_canonical_json().unwrap()).unwrap();
        let mut from_structs = Vec::new();
        leaves(&json, "", &mut from_structs);
        from_table.sort();
        from_structs.sort();
        assert_eq!(from_table, from_structs);
        // Canonical key order is schema order.
        let text = r.to_canonical_json().unwrap();
        let positions: Vec<_> = SCHEMA
            .iter()
            .map(|f| text.find(&format!("\n  \"{}\":", f.key)).unwrap())
            .collect();
        assert!(positions.is_sorted(), "{positions:?}");
    }

    #[test]
    fn future_and_unknown_schemas_are_rejected_first() {
        // A future schema is reported as such even if its shape is unknown.
        let future = r#"{"schema": 3, "layers": []}"#;
        assert_eq!(
            Recipe::from_json(future),
            Err(RecipeError::UnsupportedSchema {
                found: 3,
                supported: 2
            })
        );
        for (schema, want) in [
            (
                "0",
                RecipeError::UnsupportedSchema {
                    found: 0,
                    supported: 2,
                },
            ),
            (
                "99999999999",
                RecipeError::UnsupportedSchema {
                    found: 99999999999,
                    supported: 2,
                },
            ),
            (
                "1.5",
                RecipeError::WrongType {
                    path: "schema".into(),
                    expected: "a whole number",
                },
            ),
            (
                "-1",
                RecipeError::WrongType {
                    path: "schema".into(),
                    expected: "a whole number",
                },
            ),
            (
                "\"1\"",
                RecipeError::WrongType {
                    path: "schema".into(),
                    expected: "a whole number",
                },
            ),
        ] {
            assert_eq!(
                load_edited(|v| v["schema"] = serde_json::from_str(schema).unwrap()),
                Err(want),
                "{schema}"
            );
        }
        assert_eq!(
            load_edited(|v| {
                v.as_object_mut().unwrap().remove("schema");
            }),
            Err(RecipeError::MissingField {
                path: "schema".into()
            })
        );
        let mut r = sample();
        r.schema = 1;
        assert!(matches!(
            r.to_canonical_json(),
            Err(RecipeError::UnsupportedSchema { .. })
        ));
    }

    /// `sample()` as schema 1 wrote it: no `season`.
    fn schema_1_json() -> Value {
        let mut v = json_of(&sample());
        v["schema"] = 1.into();
        v.as_object_mut().unwrap().remove("season");
        v
    }

    #[test]
    fn schema_1_recipes_migrate_to_midsummer() {
        let v1 = schema_1_json();
        let (r, found) = Recipe::from_json_with_schema(&v1.to_string()).unwrap();
        assert_eq!(found, 1);
        assert_eq!(r.schema, version::RECIPE_SCHEMA_VERSION);
        assert_eq!(r.season.year, crate::season::DEFAULT_YEAR);
        // Everything else is as written, so it paints exactly as before.
        assert_eq!(r, sample());
        // Saving writes the current schema, which loads as itself.
        let json = r.to_canonical_json().unwrap();
        assert!(json.contains("\"schema\": 2") && json.contains("\"season\""));
        assert_eq!(Recipe::from_json_with_schema(&json).unwrap(), (r, 2));
    }

    #[test]
    fn each_schema_is_checked_against_its_own_table() {
        // A schema 1 file with a season is not schema 1.
        let mut v = schema_1_json();
        v["season"] = serde_json::json!({"year": 0.1});
        assert_eq!(
            Recipe::from_json(&v.to_string()),
            Err(RecipeError::UnknownField {
                path: "season".into()
            })
        );
        // A schema 2 file must say its season.
        let e = load_edited(|v| {
            v.as_object_mut().unwrap().remove("season");
        });
        assert_eq!(
            e,
            Err(RecipeError::MissingField {
                path: "season".into()
            })
        );
        // Schema 1 problems are still reported as such.
        let mut v = schema_1_json();
        v["atmosphere"]["mist"] = 1.into();
        assert_eq!(
            Recipe::from_json(&v.to_string()),
            Err(RecipeError::UnknownField {
                path: "atmosphere.mist".into()
            })
        );
    }

    #[test]
    fn the_season_is_range_checked_and_leaves_the_seeds_alone() {
        for bad in [-0.01, 1.01] {
            let e = load_edited(|v| v["season"]["year"] = bad.into()).unwrap_err();
            assert!(
                matches!(
                    e,
                    RecipeError::Validation(ValidationError {
                        field: "season.year",
                        ..
                    })
                ),
                "{e:?}"
            );
        }
        let mut r = sample();
        r.season.year = 0.0;
        assert_eq!(r.seeds(), sample().seeds());
        let back = Recipe::from_json(&r.to_canonical_json().unwrap()).unwrap();
        assert_eq!(back.season.year, 0.0);
    }

    #[test]
    fn unknown_algorithms_are_rejected() {
        let e = load_edited(|v| v["versions"]["seed_algorithm"] = "pigment-seed/2".into());
        assert_eq!(
            e,
            Err(RecipeError::UnsupportedAlgorithm {
                field: "seed_algorithm",
                found: "pigment-seed/2".into()
            })
        );
        let e = load_edited(|v| v["versions"]["normalization"] = "a secret phrase".into());
        assert_eq!(
            e,
            Err(RecipeError::UnsupportedAlgorithm {
                field: "normalization",
                found: "<unrecognized>".into()
            })
        );
    }

    #[test]
    fn newer_generator_and_renderer_load_with_notices() {
        assert!(sample().version_notices().is_empty());
        let r = load_edited(|v| {
            v["versions"]["generator"] = 7.into();
            v["versions"]["renderer"] = 9.into();
        })
        .unwrap();
        let notices = r.version_notices();
        assert_eq!(notices.len(), 2);
        assert_eq!(
            notices[0].to_string(),
            format!(
                "made with generator v7; this version (v{}) may compose differently",
                version::GENERATOR_VERSION
            )
        );
        assert_eq!(notices[1].component, Component::Renderer);
    }

    #[test]
    fn unknown_and_missing_fields_are_rejected() {
        let e = load_edited(|v| v["painting"]["edge_loosness"] = 0.4.into());
        assert_eq!(
            e,
            Err(RecipeError::UnknownField {
                path: "painting.edge_loosness".into()
            })
        );
        let e = load_edited(|v| v["weather"] = "autumn".into());
        assert_eq!(
            e,
            Err(RecipeError::UnknownField {
                path: "weather".into()
            })
        );
        let e = load_edited(|v| {
            v["form"]["the lake was still"] = 1.into();
        });
        assert_eq!(
            e,
            Err(RecipeError::UnknownField {
                path: "form.<unrecognized>".into()
            })
        );
        for (parent, key) in [
            ("form", "relief"),
            ("painting", "paper_grain"),
            ("seed", "digest"),
            ("versions", "renderer"),
            ("palette", "id"),
        ] {
            let e = load_edited(|v| {
                v[parent].as_object_mut().unwrap().remove(key);
            });
            assert_eq!(
                e,
                Err(RecipeError::MissingField {
                    path: format!("{parent}.{key}")
                })
            );
        }
        let e = load_edited(|v| {
            v.as_object_mut().unwrap().remove("atmosphere");
        });
        assert_eq!(
            e,
            Err(RecipeError::MissingField {
                path: "atmosphere".into()
            })
        );
    }

    #[test]
    fn wrong_types_and_bad_values_are_rejected() {
        type Case = (fn(&mut Value), RecipeError);
        let cases: [Case; 9] = [
            (
                |v| v["form"]["faceting"] = "0.5".into(),
                RecipeError::WrongType {
                    path: "form.faceting".into(),
                    expected: "a number",
                },
            ),
            (
                |v| v["frame"]["width"] = 3840.5.into(),
                RecipeError::WrongType {
                    path: "frame.width".into(),
                    expected: "a whole number from 0 to 4294967295",
                },
            ),
            (
                |v| v["seed"]["variation"] = (u64::from(u32::MAX) + 1).into(),
                RecipeError::WrongType {
                    path: "seed.variation".into(),
                    expected: "a whole number from 0 to 4294967295",
                },
            ),
            (
                |v| v["frame"]["height"] = (-1).into(),
                RecipeError::WrongType {
                    path: "frame.height".into(),
                    expected: "a whole number from 0 to 4294967295",
                },
            ),
            (
                |v| v["seed"]["digest"] = "AB".repeat(32).into(),
                RecipeError::InvalidValue {
                    path: "seed.digest".into(),
                    reason: "must be 64 lowercase hex digits",
                },
            ),
            (
                |v| v["palette"]["id"] = "sunset".into(),
                RecipeError::InvalidValue {
                    path: "palette.id".into(),
                    reason: "is not a known palette",
                },
            ),
            (
                |v| v["painting"] = Value::Null,
                RecipeError::WrongType {
                    path: "painting".into(),
                    expected: "an object",
                },
            ),
            (
                |v| v["source_text"] = Value::Null,
                RecipeError::WrongType {
                    path: "source_text".into(),
                    expected: "a string",
                },
            ),
            (
                |v| v["versions"]["generator"] = true.into(),
                RecipeError::WrongType {
                    path: "versions.generator".into(),
                    expected: "a whole number from 0 to 4294967295",
                },
            ),
        ];
        for (edit, want) in cases {
            assert_eq!(load_edited(edit), Err(want));
        }
    }

    #[test]
    fn out_of_range_values_are_rejected_not_clamped() {
        let e = load_edited(|v| v["painting"]["edge_looseness"] = 1.5.into()).unwrap_err();
        assert_eq!(
            e,
            RecipeError::Validation(ValidationError {
                field: "painting.edge_looseness",
                problem: Problem::OutOfRange {
                    value: 1.5,
                    min: 0.0,
                    max: 1.0
                }
            })
        );
        let e = load_edited(|v| v["frame"]["width"] = 20000.into()).unwrap_err();
        assert!(matches!(
            e,
            RecipeError::Validation(ValidationError {
                field: "frame.width",
                ..
            })
        ));
        let e = load_edited(|v| v["frame"]["height"] = 0.into()).unwrap_err();
        assert!(matches!(
            e,
            RecipeError::Validation(ValidationError {
                field: "frame.height",
                ..
            })
        ));
        let e = load_edited(|v| {
            v["frame"]["width"] = 8000.into();
            v["frame"]["height"] = 1000.into();
        })
        .unwrap_err();
        assert!(matches!(
            e,
            RecipeError::Validation(ValidationError { field: "frame", .. })
        ));
        // JSON cannot spell NaN or infinity, and overflowing literals are
        // syntax errors rather than infinities.
        let text = sample().to_canonical_json().unwrap();
        for bad in ["NaN", "Infinity", "1e400", "-1e999"] {
            let broken = text.replace("\"haze\": 0.4", &format!("\"haze\": {bad}"));
            assert!(
                matches!(
                    Recipe::from_json(&broken),
                    Err(RecipeError::Malformed {
                        kind: MalformedKind::Syntax,
                        ..
                    })
                ),
                "{bad}"
            );
        }
        let mut r = sample();
        r.atmosphere.haze = f64::NAN;
        assert!(
            r.to_canonical_json().is_err(),
            "NaN is never written as null"
        );
    }

    #[test]
    fn corrupt_and_oversized_files_are_rejected() {
        let text = sample().to_canonical_json().unwrap();
        let truncated = &text[..text.len() / 2];
        assert!(matches!(
            Recipe::from_json(truncated),
            Err(RecipeError::Malformed {
                kind: MalformedKind::Truncated,
                ..
            })
        ));
        for bad in ["", "   "] {
            assert!(matches!(
                Recipe::from_json(bad),
                Err(RecipeError::Malformed {
                    kind: MalformedKind::Truncated,
                    ..
                })
            ));
        }
        for bad in ["{,}", "{\"schema\": 1,}", "nonsense", &format!("{text}x")] {
            assert!(
                matches!(
                    Recipe::from_json(bad),
                    Err(RecipeError::Malformed {
                        kind: MalformedKind::Syntax,
                        ..
                    })
                ),
                "{bad:?}"
            );
        }
        for bad in ["[]", "1", "\"recipe\"", "null"] {
            assert_eq!(
                Recipe::from_json(bad),
                Err(RecipeError::WrongType {
                    path: String::new(),
                    expected: "a JSON object"
                })
            );
        }
        // Deep nesting hits serde_json's recursion limit, not the stack.
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(matches!(
            Recipe::from_json(&deep),
            Err(RecipeError::Malformed { .. })
        ));
        // Size is checked before parsing.
        let big = " ".repeat(MAX_RECIPE_BYTES + 1);
        assert_eq!(
            Recipe::from_json(&big),
            Err(RecipeError::TooLarge {
                bytes: MAX_RECIPE_BYTES + 1,
                max: MAX_RECIPE_BYTES
            })
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_at_any_depth() {
        let text = sample().to_canonical_json().unwrap();
        for (from, to) in [
            ("\"haze\": 0.4", "\"haze\": 0.4, \"haze\": 0.9"),
            ("\"schema\": 2,", "\"schema\": 2, \"schema\": 2,"),
        ] {
            let dup = text.replace(from, to);
            let e = Recipe::from_json(&dup).unwrap_err();
            assert!(
                matches!(
                    e,
                    RecipeError::Malformed {
                        kind: MalformedKind::DuplicateKey,
                        line: 2..,
                        ..
                    }
                ),
                "{e:?}"
            );
        }
    }

    #[test]
    fn the_largest_valid_source_text_fits_the_size_bound() {
        // Worst-case escaping: every byte a control character (\u001f).
        let prose = format!("a{}", "\u{1f}".repeat(crate::text::MAX_SOURCE_BYTES - 1));
        let mut r = Recipe::new(TextDigest::from_source(&prose).unwrap(), UHD_4K);
        r.source_text = Some(prose);
        let json = r.to_canonical_json().unwrap();
        assert!(json.len() <= MAX_RECIPE_BYTES, "{}", json.len());
        assert_eq!(Recipe::from_json(&json).unwrap(), r);
    }

    #[test]
    fn errors_never_contain_file_contents() {
        const MARKER: &str = "zebra lantern";
        let mut kept = sample();
        kept.source_text = Some(MARKER.into());
        kept.seed.digest = TextDigest::from_source(MARKER).unwrap();
        let text = kept.to_canonical_json().unwrap();
        let attempts: Vec<Result<Recipe, RecipeError>> = vec![
            load_edited(|v| v["form"]["faceting"] = MARKER.into()),
            load_edited(|v| v["seed"]["digest"] = MARKER.into()),
            load_edited(|v| v["palette"]["id"] = MARKER.into()),
            load_edited(|v| v["versions"]["normalization"] = MARKER.into()),
            load_edited(|v| v[MARKER] = 1.into()),
            load_edited(|v| v["source_text"] = MARKER.into()),
            Recipe::from_json(&text.replace("\"schema\": 2", &format!("\"schema\": \"{MARKER}\""))),
            Recipe::from_json(&text.replace("\"id\": \"lakeshore\"", "\"id\": zebra lantern")),
            Recipe::from_json(&text[..text.len() - 20]),
        ];
        for a in attempts {
            let e = a.unwrap_err();
            for shown in [e.to_string(), format!("{e:?}")] {
                assert!(!shown.contains("zebra"), "{shown}");
            }
        }
    }

    #[test]
    fn debug_shows_the_length_of_kept_source_text_only() {
        let mut kept = sample();
        kept.source_text = Some("zebra lantern".into());
        let shown = format!("{kept:?}");
        assert!(!shown.contains("zebra"), "{shown}");
        assert!(shown.contains("<13 bytes>"), "{shown}");
        assert!(format!("{:?}", sample()).contains("source_text: None"));
    }
}
