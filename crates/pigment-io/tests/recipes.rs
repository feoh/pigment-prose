//! Recipe persistence against the approved task 25 baseline, and across a
//! real process restart (task 10). Portable: no GPU.

use std::path::{Path, PathBuf};
use std::process::Command;

use pigment_core::frame::Frame;
use pigment_core::scene::SceneGenerator;
use pigment_core::scene::lakeshore::LakeshoreGenerator;
use pigment_core::seed::Variation;
use pigment_io::{Document, read_recipe};

fn baseline() -> PathBuf {
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
        assert!(
            notices.is_empty(),
            "{path:?} was made by this generator/renderer"
        );
        assert_eq!(doc.prose(), None, "baseline recipes keep no source text");
        let copy = dir.0.join(format!("{i}.recipe.json"));
        let mut doc2 = doc.clone();
        doc2.save_as(&copy).unwrap();
        // The approved files are schema 1. Saving writes schema 2 with
        // exactly two changes: the schema number, and the midsummer season
        // every schema 1 painting was made in (task 16).
        let original = String::from_utf8(original).unwrap();
        let body = original
            .replacen("\"schema\": 1,", "\"schema\": 2,", 1)
            .strip_suffix("  }\n}\n")
            .expect("atmosphere is the last section")
            .to_string();
        let migrated = format!("{body}  }},\n  \"season\": {{\n    \"year\": 0.5\n  }}\n}}\n");
        let saved = std::fs::read_to_string(&copy).unwrap();
        assert_eq!(saved, migrated, "{path:?}: schema 1 migrates to schema 2");
        // And the schema 2 file saves again byte for byte.
        let (again, _) = Document::open(&copy).unwrap();
        let copy2 = dir.0.join(format!("{i}-again.recipe.json"));
        again.clone().save_as(&copy2).unwrap();
        assert_eq!(std::fs::read_to_string(&copy2).unwrap(), saved);
        let back = read_recipe(&copy).unwrap();
        assert_eq!(&back, doc.recipe());
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
