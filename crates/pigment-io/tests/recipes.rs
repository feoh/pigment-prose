//! Recipe persistence against the approved baseline (schema 2), migrations
//! from schemas 1 and 2, and a real process restart
//! (task 10). Portable: no GPU.

use std::path::{Path, PathBuf};
use std::process::Command;

use pigment_core::frame::Frame;
use pigment_core::recipe::Component;
use pigment_core::scene::SceneGenerator;
use pigment_core::scene::lakeshore::LakeshoreGenerator;
use pigment_core::seed::Variation;
use pigment_io::{Document, read_recipe};

fn baseline() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/visual-review/baseline-16")
}

/// The task 25 baseline: schema 1 recipes, generator and renderer 2.
fn schema_1_baseline() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/visual-review/baseline-25")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("pigment-io-it-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `(cell, geometry checksum, image fnv)` from a baseline sheet's notes.
fn sheet_rows(txt: &Path) -> Vec<(usize, String, String)> {
    let text = std::fs::read_to_string(txt).unwrap();
    let mut lines = text.lines().skip_while(|l| !l.starts_with("cell\t"));
    let header: Vec<&str> = lines.next().expect("header").split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let (c_cell, c_sum, c_fnv) = (col("cell"), col("checksum"), col("image fnv"));
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (
                f[c_cell].parse().unwrap(),
                f[c_sum].to_string(),
                f[c_fnv].to_string(),
            )
        })
        .collect()
}

/// Every approved recipe: `(recipe path, expected checksum, image fnv)`.
fn approved() -> Vec<(PathBuf, String, String)> {
    let mut out = Vec::new();
    for sheet in [
        "corpus-16x9",
        "corpus-9x16",
        "corpus-1x1",
        "round-06-scenes",
    ] {
        let rows = sheet_rows(&baseline().join(format!("{sheet}.txt")));
        // Cells are numbered from the sheet's --first; files are NN.
        for (cell, sum, fnv) in rows {
            let path = baseline()
                .join(sheet)
                .join(format!("{cell:02}.recipe.json"));
            assert!(path.exists(), "{path:?}");
            out.push((path, sum, fnv));
        }
    }
    assert_eq!(out.len(), 47, "the approved baseline has 47 recipes");
    out
}

#[test]
fn approved_recipes_round_trip_and_reproduce_their_scenes() {
    let dir = Scratch::new("baseline");
    for (i, (path, checksum, _)) in approved().iter().enumerate() {
        let original = std::fs::read(path).unwrap();
        let (doc, notices) = Document::open(path).unwrap();
        assert_eq!(
            notices,
            [pigment_core::recipe::VersionNotice {
                component: Component::Renderer,
                recorded: 3,
                current: pigment_core::version::RENDERER_VERSION,
            }],
            "{path:?} must warn that its recorded renderer differs; geometry remains reproducible"
        );
        assert_eq!(doc.prose(), None, "baseline recipes keep no source text");
        assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Alpine);
        let copy = dir.0.join(format!("{i}.recipe.json"));
        let mut doc2 = doc.clone();
        doc2.save_as(&copy).unwrap();
        let back = read_recipe(&copy).unwrap();
        let mut expected = doc.recipe().clone();
        expected.versions.generator = pigment_core::version::GENERATOR_VERSION;
        expected.versions.renderer = pigment_core::version::RENDERER_VERSION;
        assert_eq!(back, expected);
        assert_eq!(back.schema, pigment_core::version::RECIPE_SCHEMA_VERSION);
        assert_ne!(
            std::fs::read(&copy).unwrap(),
            original,
            "legacy schema migrates"
        );
        let scene = LakeshoreGenerator
            .generate(&back.seeds(), &back.form, back.frame.aspect())
            .unwrap();
        assert_eq!(
            format!("{:016x}", scene.geometry_checksum()),
            *checksum,
            "{path:?}"
        );
    }
}

const CHILD: &str = "PIGMENT_IO_RESTART_CHILD";

/// Saves in a separate process, then reopens here: nothing but the file
/// carries the painting across.
#[test]
fn a_saved_recipe_reproduces_after_restart() {
    if let Ok(dir) = std::env::var(CHILD) {
        // The "first run" of the app.
        let dir = PathBuf::from(dir);
        let mut doc = Document::from_prose(
            "Morning fog lifts off the water.",
            Frame::new(2400, 1600).unwrap(),
        )
        .unwrap();
        doc.set_variation(Variation(3));
        let mut form = doc.recipe().form;
        form.relief = 0.8;
        doc.set_form(form).unwrap();
        let mut a = doc.appearance();
        a.painting.edge_looseness = 0.7;
        a.atmosphere.haze = 0.2;
        doc.set_appearance(a).unwrap();
        doc.save_as(&dir.join("saved.recipe.json")).unwrap();
        let scene = LakeshoreGenerator
            .generate(
                &doc.seeds(),
                &doc.recipe().form,
                doc.recipe().frame.aspect(),
            )
            .unwrap();
        let expect = format!(
            "{:016x} {:?} {:?}",
            scene.geometry_checksum(),
            doc.seeds(),
            doc.appearance()
        );
        std::fs::write(dir.join("expected.txt"), expect).unwrap();
        return;
    }
    let dir = Scratch::new("restart");
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_saved_recipe_reproduces_after_restart",
            "--test-threads=1",
        ])
        .env(CHILD, &dir.0)
        .status()
        .unwrap();
    assert!(status.success(), "child process failed");
    let (doc, notices) = Document::open(&dir.0.join("saved.recipe.json")).unwrap();
    assert!(notices.is_empty());
    assert_eq!(doc.prose(), None, "saved without its prose");
    assert!(!doc.is_dirty());
    let scene = LakeshoreGenerator
        .generate(
            &doc.seeds(),
            &doc.recipe().form,
            doc.recipe().frame.aspect(),
        )
        .unwrap();
    let got = format!(
        "{:016x} {:?} {:?}",
        scene.geometry_checksum(),
        doc.seeds(),
        doc.appearance()
    );
    assert_eq!(
        got,
        std::fs::read_to_string(dir.0.join("expected.txt")).unwrap()
    );
    assert_eq!(doc.recipe().seed.variation, Variation(3));
}

#[test]
fn schema_1_recipes_migrate_to_midsummer_and_alpine_schema_3() {
    // The task 25 baseline was saved in schema 1 (before seasons). Each
    // opens with notices for its older generator and renderer, and saves
    // as schema 3 with the midsummer season (task 16) and Alpine biome (task 18).
    let dir = Scratch::new("schema-1");
    let mut n = 0;
    for sheet in [
        "corpus-16x9",
        "corpus-9x16",
        "corpus-1x1",
        "round-06-scenes",
    ] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(schema_1_baseline().join(sheet))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".recipe.json"))
            .collect();
        files.sort();
        for path in files {
            let original = std::fs::read_to_string(&path).unwrap();
            assert!(original.contains("\"schema\": 1,"), "{path:?}");
            let (doc, notices) = Document::open(&path).unwrap();
            assert_eq!(notices.len(), 2, "{path:?}: older generator and renderer");
            assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Alpine);
            assert_eq!(doc.recipe().season.year, pigment_core::season::DEFAULT_YEAR);
            let copy = dir.0.join(format!("{n}.recipe.json"));
            doc.clone().save_as(&copy).unwrap();
            let migrated = read_recipe(&copy).unwrap();
            assert_eq!(
                migrated.schema,
                pigment_core::version::RECIPE_SCHEMA_VERSION
            );
            assert_eq!(migrated.biome, pigment_core::biome::BiomeId::Alpine);
            assert_eq!(migrated.season.year, pigment_core::season::DEFAULT_YEAR);
            assert!(original.contains("\"schema\": 1,"));
            n += 1;
        }
    }
    assert_eq!(n, 47);
}
