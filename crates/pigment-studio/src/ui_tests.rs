//! Task 12 acceptance tests for the studio's state and interaction, run
//! headless through egui_kittest (AccessKit-driven input, no GPU): a CPU
//! stand-in renderer paints flat images and scripted dialogs answer the
//! file choosers. The real GPU and real window are covered by
//! `pigment-studio --script` and `tests/gpu_preview.rs`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pigment_core::capability::{
    AdapterReport, BackendKind, DeviceKind, GpuCapabilities, LimitsReport,
};
use pigment_core::frame::AspectRatio;
use pigment_core::invalidate::Invalidation;
use pigment_core::scene::lakeshore::LakeshoreGenerator;
use pigment_core::scene::{SceneGenerator, SceneKey};
use pigment_core::seed::{Domain, Variation};
use pigment_core::settings::{CONTROLS, Channel, ControlSpec, FACETING, PaletteId};
use pigment_core::version::GENERATOR_VERSION;
use pigment_io::Document;

use crate::app::{NoticeKind, StudioApp, StudioOptions};
use crate::files::tests::FakeDialogs;
use crate::files::{DialogAnswer, DialogRequest, Dialogs, Parent};
use crate::preview::Quality;
use crate::script::Report;
use crate::worker::tests::FlatRenderer;
use crate::worker::{PreviewWorker, WorkerOptions};

fn caps() -> GpuCapabilities {
    let limits = LimitsReport {
        max_texture_dimension_2d: 8192,
        max_buffer_size: 1 << 28,
        max_storage_buffer_binding_size: 1 << 27,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 256,
        max_compute_invocations_per_workgroup: 256,
        max_storage_textures_per_shader_stage: 4,
    };
    GpuCapabilities {
        adapter: AdapterReport {
            name: "test stand-in".into(),
            backend: BackendKind::Other,
            kind: DeviceKind::Cpu,
            software: true,
            vendor_id: 0,
            device_id: 0,
            driver: "none".into(),
            driver_info: String::new(),
            adapter_limits: limits,
        },
        device_limits: limits,
        wgpu_version: "none",
    }
}

/// Scripted dialogs the test can inspect after handing them to the app.
#[derive(Debug, Clone, Default)]
struct Shared(Arc<Mutex<FakeDialogs>>);

impl Shared {
    fn answering(answers: impl IntoIterator<Item = DialogAnswer>) -> Shared {
        Shared(Arc::new(Mutex::new(FakeDialogs::answering(answers))))
    }
    fn push(&self, a: DialogAnswer) {
        self.0.lock().unwrap().answers.push_back(a);
    }
    fn shown(&self) -> Vec<DialogRequest> {
        self.0.lock().unwrap().shown.clone()
    }
}

impl Dialogs for Shared {
    fn start(&mut self, request: DialogRequest, parent: Option<&dyn Parent>) {
        self.0.lock().unwrap().start(request, parent);
    }
    fn poll(&mut self) -> Option<DialogAnswer> {
        self.0.lock().unwrap().poll()
    }
}

fn app(dialogs: Shared) -> StudioApp {
    let worker = PreviewWorker::spawn(FlatRenderer::default(), WorkerOptions::default(), || {});
    StudioApp::new(
        worker,
        caps(),
        StudioOptions::default(),
        Arc::new(Mutex::new(Report::default())),
        Box::new(dialogs),
    )
}

fn harness(dialogs: Shared) -> Harness<'static, StudioApp> {
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .build_eframe(move |cc| {
            crate::theme::install(&cc.egui_ctx);
            app(dialogs)
        });
    settle(&mut h);
    h
}

/// Steps until no preview is pending, running or scheduled.
fn settle(h: &mut Harness<'static, StudioApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        let a = h.state();
        let quiet = a.view.shown.is_some()
            && !a.view.is_pending()
            && a.worker.in_flight() == 0
            && a.scheduler.wait(Instant::now()).is_none();
        if quiet && !a.files.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "the studio did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
    h.step();
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pigment-ui-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn picked(p: &Path) -> DialogAnswer {
    DialogAnswer::Picked(p.to_path_buf())
}

fn command(h: &Harness<'static, StudioApp>, k: Key) {
    h.key_press_modifiers(Modifiers::COMMAND, k);
}

fn geometry(doc: &Document) -> u64 {
    let r = doc.recipe();
    LakeshoreGenerator
        .generate(&doc.seeds(), &r.form, r.frame.aspect())
        .unwrap()
        .geometry_checksum()
}

fn viewport_commands(h: &Harness<'static, StudioApp>) -> Vec<egui::ViewportCommand> {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

/// A value for `spec` well away from its default.
fn other_value(spec: &ControlSpec) -> f64 {
    if spec.default - spec.min > spec.max - spec.default {
        spec.min
    } else {
        spec.max
    }
}

#[test]
fn each_control_invalidates_only_its_stage_and_paint_keeps_geometry() {
    let mut a = app(Shared::default());
    a.area_px = Some((640.0, 360.0));
    let now = Instant::now();
    a.submit(now, Quality::Settled);
    let wait_result = |a: &StudioApp| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(r) = a.worker.drain().pop() {
                return r;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    assert!(!wait_result(&a).scene_reused, "first render builds");

    for spec in CONTROLS {
        let before = a.doc.clone();
        a.set_control(&spec, other_value(&spec), now, false);
        let inv = Invalidation::between(before.recipe(), a.doc.recipe());
        let key = |d: &Document| {
            let r = d.recipe();
            SceneKey::new(GENERATOR_VERSION, &d.seeds(), r.form, r.frame.aspect())
        };
        let structural = spec.channel == Channel::Structure;
        assert_eq!(
            inv,
            Invalidation {
                seeds: false,
                scene: structural,
                paint: true
            },
            "{}",
            spec.key
        );
        assert_eq!(key(&before) != key(&a.doc), structural, "{}", spec.key);
        assert_eq!(
            geometry(&before) == geometry(&a.doc),
            !structural,
            "{}: geometry/placement checksum",
            spec.key
        );
        // The worker reruns exactly the invalidated stages.
        a.submit(now, Quality::Settled);
        let r = wait_result(&a);
        assert_eq!(r.scene_reused, !structural, "{}", spec.key);
        // Back to the default for the next control.
        a.set_control(&spec, spec.default, now, false);
        a.submit(now, Quality::Settled);
        wait_result(&a);
    }

    // The palette is paint only too.
    let before = a.doc.clone();
    let mut v = a.values();
    v.appearance.palette.id = PaletteId::GoldenEvening;
    a.doc.set_appearance(v.appearance).unwrap();
    assert!(!Invalidation::between(before.recipe(), a.doc.recipe()).scene);
    assert_eq!(geometry(&before), geometry(&a.doc));

    // Another Composition: same prose seed and settings, new layout, same
    // paint-detail stream.
    let before = a.doc.clone();
    a.another_composition(now);
    let (b, n) = (before.recipe(), a.doc.recipe());
    assert_eq!(n.seed.variation, Variation(b.seed.variation.0 + 1));
    assert_eq!(n.seed.digest, b.seed.digest);
    assert_eq!(
        (n.form, n.appearance(), n.frame),
        (b.form, b.appearance(), b.frame)
    );
    assert!(Invalidation::between(b, n).scene);
    assert_ne!(geometry(&before), geometry(&a.doc));
    assert_eq!(
        before.seeds().stream(Domain::PaintDetail),
        a.doc.seeds().stream(Domain::PaintDetail)
    );
    a.previous_composition(now);
    assert_eq!(a.doc.recipe().seed.variation, b.seed.variation);
    assert_eq!(geometry(&before), geometry(&a.doc));
}

#[test]
fn keyboard_alone_adjusts_a_slider_changes_composition_and_saves() {
    let dir = temp("keyboard");
    let path = dir.join("keys.recipe.json");
    let dialogs = Shared::answering([picked(&path)]);
    let mut h = harness(dialogs.clone());

    // Tab through the window until the Form slider has focus.
    let mut found = false;
    for _ in 0..80 {
        h.key_press(Key::Tab);
        h.step();
        if h.query_by(|n| {
            n.is_focused() && n.role() == Role::Slider && n.label().as_deref() == Some("Form")
        })
        .is_some()
        {
            found = true;
            break;
        }
    }
    assert!(found, "Form slider reachable with Tab");
    let faceting = |h: &Harness<'static, StudioApp>| h.state().values().form.faceting;
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(near(faceting(&h), FACETING.default));
    for _ in 0..3 {
        h.key_press(Key::ArrowRight);
        h.step();
    }
    assert!(near(faceting(&h), 0.58), "{}", faceting(&h));
    h.key_press(Key::PageUp);
    h.step();
    assert!(near(faceting(&h), 0.68), "{}", faceting(&h));
    h.key_press(Key::End);
    h.step();
    assert!(near(faceting(&h), 1.0));
    h.key_press(Key::Home);
    h.step();
    assert!(near(faceting(&h), 0.0));
    h.key_press(Key::Delete);
    h.step();
    assert!(near(faceting(&h), FACETING.default), "Delete resets");
    h.key_press(Key::ArrowLeft);
    h.step();
    assert!(near(faceting(&h), 0.54));
    // The accessible value names the setting and where it leans.
    let slider = h.get_by(|n| n.role() == Role::Slider && n.label().as_deref() == Some("Form"));
    assert_eq!(
        slider.value().as_deref(),
        Some("0.54 (nearer angular, faceted planes)")
    );

    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    h.step();
    assert_eq!(h.state().doc.recipe().seed.variation, Variation(2));
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    h.step();
    assert_eq!(h.state().doc.recipe().seed.variation, Variation(1));
    // Painting only: the controls fold away and come back.
    command(&h, Key::Backslash);
    h.step();
    assert!(!h.state().show_controls);
    assert!(h.query_by_label("Another composition").is_none());
    command(&h, Key::Backslash);
    h.step();
    assert!(h.state().show_controls);
    h.get_by_label("Another composition");
    settle(&mut h);
    assert!(h.state().preview_is_current());

    // Save from the keyboard: the dialog is asked once, then saves in place.
    command(&h, Key::S);
    settle(&mut h);
    assert_eq!(dialogs.shown().len(), 1);
    let (saved, _) = Document::open(&path).unwrap();
    assert!(near(saved.recipe().form.faceting, 0.54));
    assert_eq!(saved.recipe().seed.variation, Variation(1));
    assert!(!h.state().unsaved());
    assert_eq!(
        h.state().notice.as_ref().map(|n| n.kind),
        Some(NoticeKind::Info)
    );
}

#[test]
fn replacing_or_closing_unsaved_work_asks_first() {
    let dir = temp("confirm");
    let other = dir.join("other.recipe.json");
    let mut d = Document::from_prose(
        "A different passage for the other file.",
        pigment_core::frame::UHD_4K,
    )
    .unwrap();
    d.set_variation(Variation(9));
    d.save_as(&other).unwrap();

    let dialogs = Shared::default();
    let mut h = harness(dialogs.clone());
    // The untouched launch document asks nothing.
    assert!(!h.state().unsaved());
    h.state_mut()
        .set_control(&FACETING, 0.9, Instant::now(), false);
    settle(&mut h);
    assert!(h.state().unsaved());

    // Open → prompt → Cancel: no dialog, nothing changes.
    command(&h, Key::O);
    h.step();
    h.get_by_label("Save changes to this painting?");
    h.get_by_label("Cancel").click();
    h.step();
    h.step();
    assert!(dialogs.shown().is_empty());
    assert!(h.query_by_label("Save changes to this painting?").is_none());
    assert!((h.state().values().form.faceting - 0.9).abs() < 1e-9);

    // Open → prompt → Don't save → pick the other file.
    dialogs.push(picked(&other));
    command(&h, Key::O);
    h.step();
    h.get_by_label("Don't save").click();
    settle(&mut h);
    assert_eq!(dialogs.shown(), [DialogRequest::Open]);
    assert_eq!(h.state().doc.recipe().seed.variation, Variation(9));
    assert!(!h.state().unsaved());

    // Closing the window with unsaved work: the close is cancelled and the
    // prompt shown; Don't save closes.
    h.state_mut().another_composition(Instant::now());
    settle(&mut h);
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.step();
    assert!(viewport_commands(&h).contains(&egui::ViewportCommand::CancelClose));
    h.step();
    h.get_by_label("Save changes to this painting?");
    h.get_by_label("Don't save").click();
    h.step();
    assert!(h.state().allow_close);
    assert!(viewport_commands(&h).contains(&egui::ViewportCommand::Close));
}

#[test]
fn source_free_recipes_open_and_save_without_inventing_prose() {
    let dir = temp("source-free");
    let path = dir.join("free.recipe.json");
    let prose = "Lichen on the boulders by the outlet stream.";
    let mut d = Document::from_prose(prose, pigment_core::frame::UHD_4K).unwrap();
    d.set_variation(Variation(4));
    d.save_as(&path).unwrap();
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("source_text")
    );

    let copy = dir.join("copy.recipe.json");
    let dialogs = Shared::answering([picked(&path), picked(&copy)]);
    let mut h = harness(dialogs);
    command(&h, Key::O);
    settle(&mut h);
    let a = h.state();
    assert!(a.source_free());
    assert_eq!(a.draft, "");
    assert_eq!(a.doc.prose(), None);
    assert!(
        a.prose_error.is_none(),
        "an empty editor is not an error here"
    );
    assert!(a.preview_is_current(), "painted from the stored seed");
    assert_eq!(a.doc.seeds(), d.seeds());
    h.get_by_label_contains("stored seed; the words cannot be recovered");

    // Opening marks nothing as changed, and saving a copy keeps it source
    // free with the same seed.
    assert!(!h.state().unsaved());
    command(&h, Key::S);
    settle(&mut h); // saves in place, no dialog
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    settle(&mut h);
    let text = std::fs::read_to_string(&copy).unwrap();
    assert!(!text.contains("source_text"));
    assert_eq!(Document::open(&copy).unwrap().0.seeds(), d.seeds());
    // Opting in has nothing to keep: the words are unknown.
    h.get_by_label("Save the prose in the recipe file").click();
    h.step();
    assert!(!h.state().doc.saves_source_text());
}

#[test]
fn save_and_reopen_reproduce_settings_scene_and_kept_prose() {
    let dir = temp("roundtrip");
    let path = dir.join("round.recipe.json");
    let dialogs = Shared::answering([picked(&path)]);
    let mut h = harness(dialogs);
    let now = Instant::now();
    {
        let a = h.state_mut();
        for spec in CONTROLS {
            a.set_control(&spec, (spec.min + spec.max) * 0.37, now, false);
        }
        a.another_composition(now);
        a.another_composition(now);
    }
    h.get_by_label("Save the prose in the recipe file").click();
    settle(&mut h);
    command(&h, Key::S);
    settle(&mut h);
    let (values, recipe, geom) = {
        let a = h.state();
        (a.values(), a.doc.recipe().clone(), geometry(&a.doc))
    };

    // A new session opens it.
    let mut h2 = harness(Shared::answering([picked(&path)]));
    command(&h2, Key::O);
    settle(&mut h2);
    let b = h2.state();
    assert_eq!(b.values(), values);
    assert_eq!(b.doc.recipe(), &recipe);
    assert_eq!(geometry(&b.doc), geom);
    assert_eq!(b.draft, crate::app::DEFAULT_PROSE);
    assert!(b.doc.keep_source_text());
    assert!(b.preview_is_current());
}

#[test]
fn failed_opens_and_saves_are_reported_and_change_nothing() {
    let dir = temp("failures");
    let broken = dir.join("broken.recipe.json");
    std::fs::write(&broken, "{ \"schema\": 1, ").unwrap();
    let unwritable = dir.join("no/such/dir/x.recipe.json");
    let dialogs = Shared::answering([
        picked(&broken),
        DialogAnswer::Cancelled,
        picked(&unwritable),
    ]);
    let mut h = harness(dialogs.clone());
    h.state_mut()
        .set_control(&FACETING, 0.2, Instant::now(), false);
    settle(&mut h);
    let before = h.state().doc.clone();

    // A broken file (after discarding at the prompt).
    command(&h, Key::O);
    h.step();
    h.get_by_label("Don't save").click();
    settle(&mut h);
    let n = h.state().notice.clone().unwrap();
    assert_eq!(n.kind, NoticeKind::Error);
    assert!(
        n.text.contains("broken.recipe.json") && n.text.contains("unchanged"),
        "{}",
        n.text
    );
    assert_eq!(h.state().doc, before);

    // A cancelled save dialog: nothing written, nothing reported.
    h.get_by_label("Dismiss").click();
    command(&h, Key::S);
    settle(&mut h);
    assert!(h.state().notice.is_none());
    assert!(h.state().unsaved());

    // A write failure: reported, still unsaved, no path adopted.
    command(&h, Key::S);
    settle(&mut h);
    let n = h.state().notice.clone().unwrap();
    assert_eq!(n.kind, NoticeKind::Error);
    assert!(
        n.text.contains("x.recipe.json") && n.text.contains("still unsaved"),
        "{}",
        n.text
    );
    assert!(h.state().unsaved() && h.state().doc.path().is_none());
    assert_eq!(dialogs.shown().len(), 3);
}

#[test]
fn the_preview_says_when_it_shows_earlier_settings() {
    let mut h = harness(Shared::default());
    assert!(
        h.query_by_label_contains("Showing earlier settings")
            .is_none()
    );
    // A change the worker has not answered yet.
    let a = h.state_mut();
    a.set_control(&FACETING, 0.1, Instant::now(), false);
    let area = a.area_px;
    a.area_px = None; // hold the submission back
    h.step();
    assert!(!h.state().preview_is_current());
    h.get_by_label_contains("Showing earlier settings");
    h.state_mut().area_px = area;
    settle(&mut h);
    assert!(h.state().preview_is_current());
    assert!(
        h.query_by_label_contains("Showing earlier settings")
            .is_none()
    );
    // Shapes outside the presets are named, not hidden.
    let aspect = AspectRatio::of(21, 9);
    assert!(crate::app::Shape::of(aspect).is_none());
}

/// Renders the studio's states offscreen with the real painter and the
/// real theme, for design review and the task evidence. Hardware only:
/// `PIGMENT_SCREENS=DIR cargo test --release -p pigment-studio --lib
/// review_screens -- --ignored --nocapture`.
#[test]
#[ignore = "needs a hardware GPU; writes review screenshots"]
fn review_screens() {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupExisting};
    use pigment_core::capability::AdapterPolicy;
    use pigment_gpu::{GpuContext, PaintRenderer};

    let out = std::env::var_os("PIGMENT_SCREENS").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/studio-screens"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&out).unwrap();
    let ctx = Arc::new(GpuContext::new(&AdapterPolicy::default()).expect("hardware GPU"));
    let caps = ctx.capabilities.clone();
    let setup = || {
        WgpuSetup::Existing(WgpuSetupExisting {
            instance: ctx.instance.clone(),
            adapter: ctx.adapter.clone(),
            device: ctx.device.clone(),
            queue: ctx.queue.clone(),
        })
    };
    let dir = temp("screens");
    let make = |dialogs: Shared, size: [f32; 2]| {
        let renderer = PaintRenderer::new(ctx.clone()).unwrap();
        let worker = PreviewWorker::spawn(renderer, WorkerOptions::default(), || {});
        let caps = caps.clone();
        let mut h = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(2.0)
            .wgpu_setup(setup())
            .build_eframe(move |cc| {
                crate::theme::install(&cc.egui_ctx);
                StudioApp::new(
                    worker,
                    caps,
                    StudioOptions::default(),
                    Arc::new(Mutex::new(Report::default())),
                    Box::new(dialogs),
                )
            });
        settle(&mut h);
        h
    };
    let save = |h: &mut Harness<'static, StudioApp>, name: &str| {
        let img = h.render().expect("render");
        let (w, hgt) = (img.width(), img.height());
        let rgb: Vec<u8> = img
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let path = out.join(format!("{name}.png"));
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, hgt);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
        eprintln!("wrote {}", path.display());
    };
    let tab_to = |h: &mut Harness<'static, StudioApp>, label: &str| {
        for _ in 0..120 {
            h.key_press(Key::Tab);
            h.step();
            if h.query_by(|n| n.is_focused() && n.label().as_deref() == Some(label))
                .is_some()
            {
                return;
            }
        }
        panic!("could not Tab to {label}");
    };

    // 1. First launch, one slider off its default (Reset shown).
    let mut h = make(Shared::default(), [1440.0, 900.0]);
    h.remove_cursor();
    h.state_mut().set_control(
        &pigment_core::settings::EDGE_LOOSENESS,
        0.62,
        Instant::now(),
        false,
    );
    settle(&mut h);
    save(&mut h, "01-launch");

    // 2. Keyboard focus on a changed slider in the open Advanced section.
    h.state_mut().open_advanced = Some(true);
    h.step();
    tab_to(&mut h, "Granulation");
    h.key_press(Key::PageUp);
    h.key_press(Key::PageUp);
    settle(&mut h);
    save(&mut h, "02-advanced-focus");

    // 3. The unsaved-changes prompt.
    command(&h, Key::O);
    h.step();
    h.step();
    save(&mut h, "03-unsaved-prompt");
    h.get_by_label("Cancel").click();
    h.step();

    // 4. A failed open, and a preview still on its way.
    let broken = dir.join("broken.recipe.json");
    std::fs::write(&broken, "{").unwrap();
    let mut h = make(Shared::answering([picked(&broken)]), [1440.0, 900.0]);
    h.remove_cursor();
    command(&h, Key::O);
    settle(&mut h);
    let a = h.state_mut();
    a.set_control(&FACETING, 0.2, Instant::now(), false);
    let area = a.area_px.take();
    h.step();
    h.step();
    save(&mut h, "04-open-failed-and-earlier-settings");
    h.state_mut().area_px = area;
    settle(&mut h);

    // 5. An old source-free recipe: version notice and no prose.
    let old = dir.join("old.recipe.json");
    let mut r = pigment_core::recipe::Recipe::new(
        pigment_core::seed::TextDigest::from_source("Old recipe text.").unwrap(),
        pigment_core::frame::Frame::largest_with_aspect(AspectRatio::of(4, 5), 3840).unwrap(),
    );
    r.versions.renderer = 0;
    pigment_io::write_recipe(&old, &r).unwrap();
    let mut h = make(Shared::answering([picked(&old)]), [1440.0, 900.0]);
    h.remove_cursor();
    command(&h, Key::O);
    settle(&mut h);
    save(&mut h, "05-source-free-portrait-old-version");

    // 6. The smallest window, with a long message and keyboard focus on
    // the primary action.
    let mut h = make(Shared::answering([picked(&broken)]), [760.0, 480.0]);
    h.remove_cursor();
    command(&h, Key::O);
    settle(&mut h);
    tab_to(&mut h, "Another composition");
    h.step();
    save(&mut h, "06-minimum-window");

    // 7. Painting only.
    let mut h = make(Shared::default(), [1440.0, 900.0]);
    h.remove_cursor();
    command(&h, Key::Backslash);
    settle(&mut h);
    save(&mut h, "07-painting-only");
}
