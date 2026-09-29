//! The open document (task 10): the effective recipe, the prose in the
//! editor, the explicit keep-source-text choice, the file path and whether
//! there are unsaved changes. A small model owned by the UI, independent of
//! the renderer.
//!
//! Privacy: the prose lives in memory while the document is open. It is
//! written to the recipe file **only** when [`Document::keep_source_text`]
//! is on (off by default, and off after opening a recipe that had none). It
//! is never part of a render request, so it cannot reach an exported image.
//! `Debug` shows only its length.

use std::fmt;
use std::path::{Path, PathBuf};

use pigment_core::error::{TextError, ValidationError};
use pigment_core::frame::Frame;
use pigment_core::recipe::{Recipe, RecipeVersions, VersionNotice};
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::text;

use crate::recipe_file::{RecipeFileError, read_recipe, write_recipe};

/// Why [`Document::save`] could not run.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// The document has never been saved; ask for a path (`save_as`).
    NoPath,
    File(RecipeFileError),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::NoPath => f.write_str("choose where to save the recipe"),
            SaveError::File(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Clone, PartialEq)]
pub struct Document {
    /// Seed, variation, frame and settings. `source_text` is always `None`
    /// here; the prose is held in `prose` and added only when saving.
    recipe: Recipe,
    prose: Option<String>,
    keep_source_text: bool,
    path: Option<PathBuf>,
    dirty: bool,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("recipe", &self.recipe)
            .field(
                "prose",
                &self.prose.as_ref().map(|p| format!("<{} bytes>", p.len())),
            )
            .field("keep_source_text", &self.keep_source_text)
            .field("path", &self.path)
            .field("dirty", &self.dirty)
            .finish()
    }
}

impl Document {
    /// A new, unsaved document seeded by `prose`, with default settings.
    /// Starts dirty (nothing is on disk) and without keeping the prose.
    pub fn from_prose(prose: &str, frame: Frame) -> Result<Document, TextError> {
        let digest = TextDigest::from_source(prose)?;
        Ok(Document {
            recipe: Recipe::new(digest, frame),
            prose: Some(prose.to_string()),
            keep_source_text: false,
            path: None,
            dirty: true,
        })
    }

    /// Reads and validates `path`. The caller's current document is not
    /// involved, so a failure leaves it exactly as it was. A recipe that
    /// kept its prose opens with `keep_source_text` on; one without opens
    /// with no prose and the choice off. Version notices are for the UI.
    pub fn open(path: &Path) -> Result<(Document, Vec<VersionNotice>), RecipeFileError> {
        let mut recipe = read_recipe(path)?;
        let notices = recipe.version_notices();
        let prose = recipe.source_text.take();
        Ok((
            Document {
                keep_source_text: prose.is_some(),
                prose,
                recipe,
                path: Some(path.to_path_buf()),
                dirty: false,
            },
            notices,
        ))
    }

    /// Replaces this document with the one at `path`, or leaves it
    /// untouched and returns the error.
    pub fn replace_from_file(
        &mut self,
        path: &Path,
    ) -> Result<Vec<VersionNotice>, RecipeFileError> {
        let (doc, notices) = Document::open(path)?;
        *self = doc;
        Ok(notices)
    }

    /// The effective recipe without source text (what renders use).
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub fn seeds(&self) -> SeedBundle {
        self.recipe.seeds()
    }

    pub fn appearance(&self) -> Appearance {
        self.recipe.appearance()
    }

    /// The prose in the editor, if known. `None` after opening a recipe
    /// saved without its source text: the painting reproduces from the
    /// digest, but the words are not recoverable from it.
    pub fn prose(&self) -> Option<&str> {
        self.prose.as_deref()
    }

    pub fn keep_source_text(&self) -> bool {
        self.keep_source_text
    }

    /// Whether the next save writes `source_text`: the choice is on and the
    /// prose is known.
    pub fn saves_source_text(&self) -> bool {
        self.keep_source_text && self.prose.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Unsaved changes: saving now would write a different file.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn update<T: PartialEq>(dirty: &mut bool, slot: &mut T, value: T) {
        if *slot != value {
            *slot = value;
            *dirty = true;
        }
    }

    /// New prose re-seeds the painting (the variation is kept). Rejected
    /// prose (empty, blank, oversized) changes nothing. Text with the same
    /// digest (for example CRLF vs LF) updates the editor's copy without
    /// changing the painting.
    pub fn set_prose(&mut self, prose: &str) -> Result<(), TextError> {
        let digest = TextDigest::from_source(prose)?;
        Self::update(&mut self.dirty, &mut self.recipe.seed.digest, digest);
        if self.prose.as_deref() != Some(prose) {
            if self.keep_source_text {
                // The kept text in the file would change.
                self.dirty = true;
            }
            self.prose = Some(prose.to_string());
        }
        Ok(())
    }

    /// The explicit include-source-text choice.
    pub fn set_keep_source_text(&mut self, keep: bool) {
        let before = self.saves_source_text();
        self.keep_source_text = keep;
        if self.saves_source_text() != before {
            self.dirty = true;
        }
    }

    pub fn set_variation(&mut self, v: Variation) {
        Self::update(&mut self.dirty, &mut self.recipe.seed.variation, v);
    }

    /// "Another Composition": the next variation, wrapping at `u32::MAX`.
    pub fn next_variation(&mut self) -> Variation {
        let v = Variation(self.recipe.seed.variation.0.wrapping_add(1));
        self.set_variation(v);
        v
    }

    pub fn set_frame(&mut self, frame: Frame) -> Result<(), ValidationError> {
        frame.validate()?;
        Self::update(&mut self.dirty, &mut self.recipe.frame, frame);
        Ok(())
    }

    pub fn set_form(&mut self, form: FormSettings) -> Result<(), ValidationError> {
        form.validate()?;
        Self::update(&mut self.dirty, &mut self.recipe.form, form);
        Ok(())
    }

    /// Select a supported landscape and apply its structural, palette and
    /// atmosphere defaults while preserving the user's painting and season.
    pub fn set_biome(
        &mut self,
        biome: pigment_core::biome::BiomeId,
    ) -> Result<(), ValidationError> {
        let profile = pigment_core::biome::profile(biome);
        profile
            .validate()
            .expect("registered biome profiles are valid");
        let mut appearance = self.appearance();
        appearance.palette.id = profile.palette;
        appearance.palette.intensity = profile.palette_intensity;
        appearance.atmosphere.haze = profile.haze;
        appearance.validate()?;
        profile.form.validate()?;
        Self::update(&mut self.dirty, &mut self.recipe.biome, biome);
        Self::update(&mut self.dirty, &mut self.recipe.form, profile.form);
        self.set_appearance(appearance)?;
        Ok(())
    }

    /// Painting, palette, atmosphere and season together. Invalid values
    /// change nothing.
    pub fn set_appearance(&mut self, a: Appearance) -> Result<(), ValidationError> {
        a.validate()?;
        Self::update(&mut self.dirty, &mut self.recipe.painting, a.painting);
        Self::update(&mut self.dirty, &mut self.recipe.palette, a.palette);
        Self::update(&mut self.dirty, &mut self.recipe.atmosphere, a.atmosphere);
        Self::update(&mut self.dirty, &mut self.recipe.season, a.season);
        Ok(())
    }

    /// The recipe a save would write: current algorithm and component
    /// versions, and `source_text` only if [`saves_source_text`](Self::saves_source_text).
    pub fn to_saved_recipe(&self) -> Recipe {
        let mut r = self.recipe.clone();
        r.versions = RecipeVersions::current();
        r.source_text = if self.saves_source_text() {
            self.prose.clone()
        } else {
            None
        };
        r
    }

    /// Saves to the current path.
    pub fn save(&mut self) -> Result<(), SaveError> {
        let path = self.path.clone().ok_or(SaveError::NoPath)?;
        self.save_as(&path).map_err(SaveError::File)
    }

    /// Saves to `path` and makes it the current path. On failure nothing
    /// changes: the path, dirty flag and any existing file stay as they were.
    /// A successful save records this build's generator and renderer
    /// versions.
    pub fn save_as(&mut self, path: &Path) -> Result<(), RecipeFileError> {
        let saved = self.to_saved_recipe();
        write_recipe(path, &saved)?;
        self.recipe.versions = saved.versions;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Checks prose against the input gate without changing anything (for
    /// live UI feedback).
    pub fn check_prose(prose: &str) -> Result<(), TextError> {
        text::check_source(prose)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pigment_core::frame::UHD_4K;
    use pigment_core::recipe::Component;
    use pigment_core::settings::PaletteId;

    use super::*;
    use crate::test_dir::TestDir;

    const MARKER: &str = "The zebra lantern glows over quiet marigolds.";

    #[test]
    fn dirty_state_follows_real_changes_only() {
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        assert!(d.is_dirty(), "new documents are unsaved");
        let dir = TestDir::new("doc-dirty");
        let path = dir.path().join("d.recipe.json");
        d.save_as(&path).unwrap();
        assert!(!d.is_dirty());
        assert_eq!(d.path(), Some(path.as_path()));

        // Setting a value to what it already is does not dirty.
        d.set_form(d.recipe().form).unwrap();
        d.set_appearance(d.appearance()).unwrap();
        d.set_variation(Variation(0));
        d.set_prose(MARKER).unwrap();
        d.set_keep_source_text(false);
        assert!(!d.is_dirty());
        // Same digest (CRLF vs LF is one normalized text) is not a change.
        let crlf = "a\r\nb";
        d.set_prose(crlf).unwrap();
        d.save().unwrap();
        d.set_prose("a\nb").unwrap();
        assert!(!d.is_dirty(), "same digest, prose not kept");

        let mut a = d.appearance();
        a.palette.id = PaletteId::GoldenEvening;
        d.set_appearance(a).unwrap();
        assert!(d.is_dirty());
        d.save().unwrap();
        d.next_variation();
        assert!(d.is_dirty());
        assert_eq!(d.recipe().seed.variation, Variation(1));
    }

    #[test]
    fn biome_selection_applies_profile_defaults_and_survives_reopening() {
        let mut doc = Document::from_prose(MARKER, UHD_4K).unwrap();
        let mut appearance = doc.appearance();
        appearance.painting.mark_scale = 0.72;
        appearance.season.year = 0.18;
        doc.set_appearance(appearance).unwrap();
        doc.set_biome(pigment_core::biome::BiomeId::Desert).unwrap();
        assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Desert);
        assert_eq!(doc.recipe().form, pigment_core::biome::DESERT.form);
        assert_eq!(doc.appearance().palette.id, PaletteId::Desert);
        assert_eq!(
            doc.appearance().atmosphere.haze,
            pigment_core::biome::DESERT.haze
        );
        assert_eq!(doc.appearance().painting.mark_scale, 0.72);
        assert_eq!(doc.appearance().season.year, 0.18);

        let dir = TestDir::new("doc-biome");
        let path = dir.path().join("desert.recipe.json");
        doc.save_as(&path).unwrap();
        let (opened, _) = Document::open(&path).unwrap();
        assert_eq!(opened.recipe(), doc.recipe());
    }

    #[test]
    fn tundra_selection_applies_approved_defaults_and_survives_reopening() {
        let mut doc = Document::from_prose(MARKER, UHD_4K).unwrap();
        doc.set_biome(pigment_core::biome::BiomeId::Tundra).unwrap();
        assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Tundra);
        assert_eq!(doc.recipe().form, pigment_core::biome::TUNDRA.form);
        assert_eq!(doc.appearance().palette.id, PaletteId::Tundra);
        assert_eq!(
            doc.appearance().season.year,
            pigment_core::season::DEFAULT_YEAR
        );

        let dir = TestDir::new("doc-tundra");
        let path = dir.path().join("tundra.recipe.json");
        doc.save_as(&path).unwrap();
        let (opened, _) = Document::open(&path).unwrap();
        assert_eq!(opened.recipe(), doc.recipe());
    }

    #[test]
    fn jungle_selection_applies_approved_defaults_and_survives_reopening() {
        let mut doc = Document::from_prose(MARKER, UHD_4K).unwrap();
        doc.set_biome(pigment_core::biome::BiomeId::Jungle).unwrap();
        assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Jungle);
        assert_eq!(doc.recipe().form, pigment_core::biome::JUNGLE.form);
        assert_eq!(doc.appearance().palette.id, PaletteId::Jungle);
        assert_eq!(
            doc.appearance().season.year,
            pigment_core::season::DEFAULT_YEAR
        );

        let dir = TestDir::new("doc-jungle");
        let path = dir.path().join("jungle.recipe.json");
        doc.save_as(&path).unwrap();
        let (opened, _) = Document::open(&path).unwrap();
        assert_eq!(opened.recipe(), doc.recipe());
    }

    #[test]
    fn invalid_edits_change_nothing() {
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        let dir = TestDir::new("doc-invalid");
        d.save_as(&dir.path().join("d.recipe.json")).unwrap();
        let before = d.clone();
        let mut form = d.recipe().form;
        form.relief = 1.5;
        assert!(d.set_form(form).is_err());
        let mut a = d.appearance();
        a.painting.mark_scale = f64::NAN;
        assert!(d.set_appearance(a).is_err());
        assert!(
            d.set_frame(Frame {
                width: 10,
                height: 10
            })
            .is_err()
        );
        assert!(d.set_prose("   \n").is_err());
        assert!(d.set_prose("").is_err());
        assert_eq!(d, before);
    }

    #[test]
    fn source_free_recipes_truly_omit_the_prose() {
        let dir = TestDir::new("doc-omit");
        let path = dir.path().join("omit.recipe.json");
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        assert!(!d.keep_source_text(), "privacy default");
        d.save_as(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("source_text"));
        // Not the prose, nor any word of it, nor an encoding of it.
        for word in MARKER.split_whitespace().filter(|w| w.len() > 3) {
            let w = word.trim_matches('.');
            assert!(!text.to_lowercase().contains(&w.to_lowercase()), "{w}");
        }
        let hex: String = MARKER.bytes().map(|b| format!("{b:02x}")).collect();
        assert!(!text.contains(&hex[..16]));
        // Only these keys exist; the digest is the only seed material.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "atmosphere",
                "biome",
                "form",
                "frame",
                "painting",
                "palette",
                "schema",
                "season",
                "seed",
                "versions"
            ]
        );

        // Reopened: reproduces the same seeds, but the words are gone.
        let (back, notices) = Document::open(&path).unwrap();
        assert!(notices.is_empty());
        assert_eq!(back.prose(), None);
        assert!(!back.keep_source_text());
        assert_eq!(back.seeds(), d.seeds());
        assert_eq!(back.recipe(), d.recipe());
    }

    #[test]
    fn kept_prose_round_trips_and_can_be_dropped() {
        let dir = TestDir::new("doc-keep");
        let path = dir.path().join("keep.recipe.json");
        let prose = "Café 🌲\r\nsnow on 山\u{00a0}ridge";
        let mut d = Document::from_prose(prose, UHD_4K).unwrap();
        d.set_keep_source_text(true);
        assert!(d.saves_source_text());
        d.save_as(&path).unwrap();
        let (back, _) = Document::open(&path).unwrap();
        assert_eq!(back.prose(), Some(prose), "exact, not normalized");
        assert!(back.keep_source_text() && !back.is_dirty());

        let mut dropped = back.clone();
        dropped.set_keep_source_text(false);
        assert!(dropped.is_dirty(), "turning it off changes the file");
        dropped.save().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("source_text"));
        assert_eq!(Document::open(&path).unwrap().0.prose(), None);
    }

    #[test]
    fn keep_choice_without_prose_saves_no_source() {
        let dir = TestDir::new("doc-noprose");
        let path = dir.path().join("n.recipe.json");
        Document::from_prose(MARKER, UHD_4K)
            .unwrap()
            .save_as(&path)
            .unwrap();
        let (mut d, _) = Document::open(&path).unwrap();
        d.set_keep_source_text(true);
        assert!(!d.saves_source_text() && !d.is_dirty(), "nothing to keep");
        d.set_prose(MARKER).unwrap();
        assert!(d.saves_source_text() && d.is_dirty());
    }

    #[test]
    fn failed_open_leaves_the_current_document_unchanged() {
        let dir = TestDir::new("doc-open-fail");
        let good = dir.path().join("good.recipe.json");
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        d.set_keep_source_text(true);
        d.save_as(&good).unwrap();
        d.next_variation(); // unsaved edit
        let before = d.clone();
        let json = fs::read_to_string(&good).unwrap();
        let bad = dir.path().join("bad.recipe.json");
        for contents in [
            json[..json.len() / 2].to_string(),
            json.replace("\"schema\": 3", "\"schema\": 4"),
            json.replace("0.55", "5.5"),
            "not json".to_string(),
        ] {
            fs::write(&bad, contents).unwrap();
            assert!(d.replace_from_file(&bad).is_err());
            assert_eq!(d, before);
        }
        assert!(d.replace_from_file(&dir.path().join("missing")).is_err());
        assert_eq!(d, before);
        assert!(d.is_dirty(), "the unsaved edit is still there");
    }

    #[test]
    fn failed_save_keeps_path_dirty_state_and_file() {
        let dir = TestDir::new("doc-save-fail");
        let path = dir.path().join("s.recipe.json");
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        d.save_as(&path).unwrap();
        d.next_variation();
        let before = d.clone();
        assert!(
            d.save_as(&dir.path().join("no/such/dir/x.recipe.json"))
                .is_err()
        );
        assert_eq!(d, before);
        assert_eq!(d.path(), Some(path.as_path()));
        assert!(d.is_dirty());
        let mut unsaved = Document::from_prose(MARKER, UHD_4K).unwrap();
        assert_eq!(unsaved.save(), Err(SaveError::NoPath));
    }

    #[test]
    fn saving_records_current_versions_and_notices_old_ones() {
        let dir = TestDir::new("doc-versions");
        let path = dir.path().join("v.recipe.json");
        let mut r = Recipe::new(TextDigest::from_source(MARKER).unwrap(), UHD_4K);
        r.versions.generator = 0;
        r.versions.renderer = 0;
        write_recipe(&path, &r).unwrap();
        let (mut d, notices) = Document::open(&path).unwrap();
        assert_eq!(
            notices.iter().map(|n| n.component).collect::<Vec<_>>(),
            [Component::Generator, Component::Renderer]
        );
        assert_eq!(d.recipe().versions.generator, 0, "kept until saved");
        d.save().unwrap();
        let (_, notices) = Document::open(&path).unwrap();
        assert!(notices.is_empty());
    }

    #[test]
    fn debug_never_shows_the_prose() {
        let mut d = Document::from_prose(MARKER, UHD_4K).unwrap();
        d.set_keep_source_text(true);
        let shown = format!("{d:?} {:?}", d.to_saved_recipe());
        assert!(!shown.contains("zebra"), "{shown}");
    }
}
