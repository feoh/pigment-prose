//! `pigment-studio --script`: a scripted run of the real window, used as the
//! task 11 and 12 acceptance check. It types into the prose editor, resizes
//! the window twice, waits for the preview to settle, feeds the view a
//! deliberately out-of-order old result, drags a structure slider (Form)
//! and then a paint slider (Edge Looseness) every frame for a second each,
//! asks for Another Composition, captures the window with the Advanced
//! section open, then starts one more (delayed) render and closes the
//! window while it runs.
//!
//! After each drag it checks that the preview settles on the newest values
//! at the full preview size, that no older result was ever displayed after
//! a newer one, and that the paint drag rebuilt no scene.
//!
//! Task 13: it then exports an 8K PNG to a temporary folder while dragging
//! a slider the whole time, and checks that the UI kept drawing, that the
//! running job's snapshot never changed, and that the file decodes at
//! 7680×4320 with no temporary file left beside it. The folder is deleted.
//!
//! Frames are requested continuously, so the gap between frames measures
//! whether the UI thread was ever blocked, in particular while the worker
//! runs a render slowed by `--preview-delay-ms`.

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui;
use pigment_core::request::RequestId;
use pigment_core::settings::{ControlSpec, EDGE_LOOSENESS, FACETING};

use crate::app::StudioApp;
use crate::preview::preview_size;
use crate::worker::{PreviewOutcome, PreviewResult};

const TEXT: &str = " The wind moves over cold water and the pines lean toward the ridge.";
/// One keystroke every 60 ms: faster than the 300 ms prose debounce, so
/// previews wait for the pause at the end.
const KEY_INTERVAL: Duration = Duration::from_millis(60);
const SIZES: [(Duration, [f32; 2]); 2] = [
    (Duration::from_millis(800), [1100.0, 720.0]),
    (Duration::from_millis(1800), [1360.0, 860.0]),
];
const SETTLE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long each slider is dragged (one new value per frame).
const DRAG_FOR: Duration = Duration::from_millis(1000);

/// What a settled preview must show.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Settled {
    /// The image on screen was made from the document's current values.
    pub current: bool,
    /// At the settled preview size, not the interaction size.
    pub full_size: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub keystrokes: u32,
    pub resizes: u32,
    pub submitted: u32,
    pub superseded: u32,
    pub max_in_flight: usize,
    pub displayed: u32,
    pub latencies: Vec<Duration>,
    pub renders: Vec<Duration>,
    /// Longest gap between UI frames while a preview job was in flight.
    pub max_frame_gap_busy: Duration,
    /// The five longest busy gaps: (gap, time since the script started).
    pub worst_gaps: Vec<(Duration, Duration)>,
    pub frames_busy: u32,
    pub settled_on_newest: Option<bool>,
    pub stale_rejected: Option<bool>,
    pub screenshot: Option<Result<String, String>>,
    /// Close requested while a delayed render was running.
    pub closed_while_busy: Option<bool>,
    /// (time from close to worker joined, joined).
    pub shutdown: Option<(Duration, bool)>,
    pub device: String,
    /// A render delay was simulated, so busy-time checks apply.
    pub delayed: bool,
    /// A device loss was simulated (`--lose-device-after`): the check is
    /// that the user is told and previews stop, not that they settle.
    pub device_loss_expected: bool,
    pub device_loss_shown: Option<bool>,
    /// Previews displayed that were rendered at interaction quality.
    pub interaction_shown: u32,
    /// Results displayed although a newer one had already been shown.
    pub out_of_order_displays: u32,
    pub last_displayed: Option<RequestId>,
    /// Slider values submitted by the drags.
    pub drag_values: u32,
    pub after_form_drag: Option<Settled>,
    pub after_paint_drag: Option<Settled>,
    /// Scenes built while only paint was dragged (must be 0).
    pub paint_drag_scene_builds: Option<u32>,
    pub after_another: Option<Settled>,
    /// Another Composition changed the variation and nothing else.
    pub another_kept_seed_and_settings: Option<bool>,
    /// The 8K export: (file decoded at 7680×4320 and nothing else left in
    /// the folder, the job's snapshot never changed, elapsed).
    pub export: Option<(bool, bool, Duration)>,
    /// Longest gap between UI frames while the export ran.
    pub max_frame_gap_exporting: Duration,
    pub frames_exporting: u32,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn percentile(v: &[Duration], p: f64) -> Duration {
    let mut v = v.to_vec();
    v.sort();
    v.get(((v.len() as f64 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or_default()
}

impl Report {
    /// Checks and a printable summary.
    pub fn verdict(&self) -> (bool, String) {
        let mut checks = vec![
            ("typed into the editor", self.keystrokes > 20),
            ("resized the window twice", self.resizes == 2),
            (
                "at most one running + one pending job",
                self.max_in_flight <= 2,
            ),
            (
                "out-of-order old result rejected",
                self.stale_rejected == Some(true),
            ),
        ];
        let ok = |s: Option<Settled>| {
            s == Some(Settled {
                current: true,
                full_size: true,
            })
        };
        if self.device_loss_expected {
            checks.push((
                "device loss shown and previews stopped",
                self.device_loss_shown == Some(true),
            ));
        } else {
            checks.push((
                "settled on the newest request",
                self.settled_on_newest == Some(true),
            ));
            checks.push((
                "dragged Form: settled on the newest values at full size",
                ok(self.after_form_drag),
            ));
            checks.push((
                "dragged Edge Looseness: settled on the newest values at full size",
                ok(self.after_paint_drag),
            ));
            checks.push((
                "paint-only drag rebuilt no scene",
                self.paint_drag_scene_builds == Some(0),
            ));
            checks.push((
                "Another Composition: settled on the new variation at full size",
                ok(self.after_another),
            ));
            checks.push((
                "Another Composition kept the prose seed and every setting",
                self.another_kept_seed_and_settings == Some(true),
            ));
            checks.push((
                "8K export written (7680×4320, no temporary left)",
                self.export.is_some_and(|e| e.0),
            ));
            checks.push((
                "the export's snapshot never changed while sliders moved",
                self.export.is_some_and(|e| e.1),
            ));
            checks.push((
                "UI frames kept coming during the export (gap < 250 ms)",
                self.frames_exporting > 0
                    && self.max_frame_gap_exporting < Duration::from_millis(250),
            ));
            if !self.delayed {
                // With a slow simulated GPU every drag job is superseded
                // before it finishes, so only an undelayed run shows them.
                checks.push((
                    "interaction previews shown while dragging",
                    self.interaction_shown > 0,
                ));
            }
        }
        checks.push((
            "never displayed an older result after a newer one",
            self.out_of_order_displays == 0,
        ));
        if self.delayed {
            checks.push((
                "UI frames kept coming while rendering (gap < 250 ms)",
                self.frames_busy > 10 && self.max_frame_gap_busy < Duration::from_millis(250),
            ));
            checks.push((
                "closed while a render was in flight",
                self.closed_while_busy == Some(true),
            ));
        }
        let shutdown_ok = self
            .shutdown
            .is_some_and(|(d, joined)| joined && d < Duration::from_millis(1000));
        checks.push(("worker shut down within 1 s", shutdown_ok));
        if let Some(shot) = &self.screenshot {
            checks.push(("window captured", shot.is_ok()));
        }
        let mut out = format!("device: {}\n", self.device);
        out += &format!(
            "keystrokes {}, resizes {}, previews submitted {} (superseded before starting {}), displayed {}, max in flight {}\n",
            self.keystrokes,
            self.resizes,
            self.submitted,
            self.superseded,
            self.displayed,
            self.max_in_flight
        );
        out += &format!(
            "slider values dragged {}, interaction previews shown {}, scenes built during the paint drag {}\n",
            self.drag_values,
            self.interaction_shown,
            self.paint_drag_scene_builds
                .map_or("?".to_string(), |n| n.to_string()),
        );
        if let Some((_, _, t)) = self.export {
            out += &format!(
                "8K export: {:.0} ms; {} UI frames while it ran, longest gap {:.1} ms\n",
                ms(t),
                self.frames_exporting,
                ms(self.max_frame_gap_exporting)
            );
        }
        out += &format!(
            "UI frames while a job was in flight: {}, longest gap {:.1} ms\n",
            self.frames_busy,
            ms(self.max_frame_gap_busy)
        );
        if !self.worst_gaps.is_empty() {
            let list: Vec<String> = self
                .worst_gaps
                .iter()
                .map(|(g, at)| format!("{:.0} ms at {:.2} s", ms(*g), at.as_secs_f64()))
                .collect();
            out += &format!("longest busy gaps: {}\n", list.join(", "));
        }
        if !self.latencies.is_empty() {
            out += &format!(
                "request→shown latency: median {:.1} ms, p95 {:.1} ms, max {:.1} ms; render+readback median {:.2} ms, max {:.2} ms (n={})\n",
                ms(percentile(&self.latencies, 0.5)),
                ms(percentile(&self.latencies, 0.95)),
                ms(percentile(&self.latencies, 1.0)),
                ms(percentile(&self.renders, 0.5)),
                ms(percentile(&self.renders, 1.0)),
                self.latencies.len()
            );
        }
        if let Some((d, joined)) = self.shutdown {
            out += &format!(
                "close → worker stopped: {:.1} ms (joined: {joined})\n",
                ms(d)
            );
        }
        if let Some(s) = &self.screenshot {
            out += &match s {
                Ok(p) => format!("window captured: {p}\n"),
                Err(e) => format!("window capture FAILED: {e}\n"),
            };
        }
        let mut pass = true;
        for (name, ok) in checks {
            pass &= ok;
            out += &format!("[{}] {name}\n", if ok { "ok" } else { "FAIL" });
        }
        (pass, out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Typing,
    Settling,
    Drag(Slider),
    SettleDrag(Slider),
    SettleAnother,
    Exporting,
    Capturing,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slider {
    Form,
    Paint,
}

impl Slider {
    fn spec(self) -> &'static ControlSpec {
        match self {
            Slider::Form => &FACETING,
            Slider::Paint => &EDGE_LOOSENESS,
        }
    }
}

/// No preview is pending, running or scheduled.
fn settled(app: &StudioApp, now: Instant) -> bool {
    !app.view.is_pending() && app.worker.in_flight() == 0 && app.scheduler.wait(now).is_none()
}

fn check_settled(app: &StudioApp) -> Settled {
    let aspect = app.doc.recipe().frame.aspect();
    let full = app
        .area_px
        .and_then(|a| preview_size(aspect, a, app.settled_cap));
    Settled {
        current: app.preview_is_current(),
        full_size: app.view.shown.map(|s| (s.width, s.height)) == full && full.is_some(),
    }
}

#[derive(Debug)]
pub struct Script {
    start: Option<Instant>,
    last_frame: Option<Instant>,
    typed: usize,
    stage: Stage,
    stage_since: Instant,
    scenes_built: u32,
    before_another: Option<(
        pigment_core::recipe::Recipe,
        pigment_core::settings::ControlValues,
    )>,
    export_dir: Option<std::path::PathBuf>,
    export_job: Option<crate::export::ExportJob>,
    export_stable: bool,
}

impl Script {
    pub fn new() -> Script {
        Script {
            start: None,
            last_frame: None,
            typed: 0,
            stage: Stage::Typing,
            stage_since: Instant::now(),
            scenes_built: 0,
            before_another: None,
            export_dir: None,
            export_job: None,
            export_stable: true,
        }
    }

    fn to(&mut self, stage: Stage, now: Instant) {
        self.stage = stage;
        self.stage_since = now;
    }

    pub fn step(&mut self, app: &mut StudioApp, ctx: &egui::Context, now: Instant) {
        ctx.request_repaint(); // keep frames flowing so gaps are measurable
        app.allow_close = true; // the script's typing is not work to save
        let start = *self.start.get_or_insert(now);
        let t = now - start;
        let busy = app.worker.in_flight() > 0;
        {
            let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(prev) = self.last_frame
                && self.stage == Stage::Exporting
            {
                r.frames_exporting += 1;
                r.max_frame_gap_exporting = r.max_frame_gap_exporting.max(now - prev);
            }
            if let Some(prev) = self.last_frame
                && busy
            {
                r.frames_busy += 1;
                let gap = now - prev;
                r.max_frame_gap_busy = r.max_frame_gap_busy.max(gap);
                r.worst_gaps.push((gap, t));
                r.worst_gaps.sort_by_key(|g| std::cmp::Reverse(g.0));
                r.worst_gaps.truncate(5);
            }
        }
        self.last_frame = Some(now);

        match self.stage {
            Stage::Typing => {
                let due = (t.as_millis() / KEY_INTERVAL.as_millis()) as usize;
                let mut chars = TEXT.chars().skip(self.typed);
                while self.typed < due.min(TEXT.chars().count()) {
                    if let Some(c) = chars.next() {
                        app.draft.push(c);
                        app.scheduler.prose_edited(now);
                        self.typed += 1;
                        app.report
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .keystrokes += 1;
                    }
                }
                let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
                for (i, (at, size)) in SIZES.iter().enumerate() {
                    if t >= *at && r.resizes == i as u32 {
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            size[0], size[1],
                        )));
                        r.resizes += 1;
                    }
                }
                if self.typed == TEXT.chars().count() && r.resizes == 2 {
                    drop(r);
                    self.to(Stage::Settling, now);
                }
            }
            Stage::Settling => {
                let settled = !app.view.is_pending()
                    && app.worker.in_flight() == 0
                    && app.scheduler.wait(now).is_none();
                if settled || now - self.stage_since > SETTLE_TIMEOUT {
                    let newest = app.view.requested();
                    let shown = app.view.shown.map(|s| s.id);
                    // A deliberately late, out-of-order old result.
                    let stale = PreviewResult {
                        id: RequestId(1),
                        outcome: PreviewOutcome::Image {
                            width: 4,
                            height: 4,
                            rgba8: vec![255; 64],
                        },
                        scene_reused: false,
                        scene_time: Duration::ZERO,
                        timings: None,
                        submitted_at: start,
                    };
                    let replaced = app.accept(ctx, stale, now);
                    let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
                    r.settled_on_newest = Some(settled && newest.is_some() && newest == shown);
                    r.stale_rejected = Some(!replaced && app.view.shown.map(|s| s.id) == shown);
                    drop(r);
                    if app.view.device_lost {
                        self.capture(app, ctx, now);
                    } else {
                        self.to(Stage::Drag(Slider::Form), now);
                    }
                }
            }
            Stage::Drag(slider) => {
                let spec = slider.spec();
                let e = (now - self.stage_since).as_secs_f64();
                // Sweep across most of the range, a new value every frame.
                let x = 0.5 + 0.45 * (e * 7.0).sin();
                let v = spec.min + (spec.max - spec.min) * x;
                app.set_control(spec, (v * 100.0).round() / 100.0, now, true);
                app.report
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .drag_values += 1;
                if now - self.stage_since >= DRAG_FOR {
                    self.to(Stage::SettleDrag(slider), now);
                }
            }
            Stage::SettleDrag(slider) => {
                if settled(app, now) || now - self.stage_since > SETTLE_TIMEOUT {
                    let result = check_settled(app);
                    let built = app.worker.stats().scenes_built;
                    let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
                    match slider {
                        Slider::Form => {
                            r.after_form_drag = Some(result);
                            drop(r);
                            self.scenes_built = built;
                            self.to(Stage::Drag(Slider::Paint), now);
                        }
                        Slider::Paint => {
                            r.after_paint_drag = Some(result);
                            r.paint_drag_scene_builds = Some(built - self.scenes_built);
                            drop(r);
                            self.before_another = Some((app.doc.recipe().clone(), app.values()));
                            app.another_composition(now);
                            self.to(Stage::SettleAnother, now);
                        }
                    }
                }
            }
            Stage::SettleAnother => {
                if settled(app, now) || now - self.stage_since > SETTLE_TIMEOUT {
                    let result = check_settled(app);
                    let kept = self.before_another.take().is_some_and(|(before, values)| {
                        let after = app.doc.recipe();
                        after.seed.digest == before.seed.digest
                            && after.seed.variation.0 == before.seed.variation.0 + 1
                            && app.values() == values
                            && after.frame == before.frame
                    });
                    let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
                    r.after_another = Some(result);
                    r.another_kept_seed_and_settings = Some(kept);
                    drop(r);
                    self.start_export(app, ctx, now);
                }
            }
            Stage::Exporting => {
                // Keep dragging a slider for as long as the export runs.
                let e = (now - self.stage_since).as_secs_f64();
                let v = 0.5 + 0.45 * (e * 9.0).sin();
                app.set_control(&EDGE_LOOSENESS, (v * 100.0).round() / 100.0, now, true);
                match app.exporter.running() {
                    Some(r) => self.export_stable &= self.export_job.as_ref() == Some(&r.job),
                    None => self.finish_export(app, ctx, now),
                }
            }
            Stage::Capturing => {
                let shot = ctx.input(|i| {
                    i.raw.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                let timed_out = now - self.stage_since > Duration::from_secs(5);
                if shot.is_some() || timed_out {
                    // Encoded off the UI thread, like any slow work.
                    let report = app.report.clone();
                    let set = move |result| {
                        report.lock().unwrap_or_else(|e| e.into_inner()).screenshot = Some(result);
                    };
                    match (shot, app.script_screenshot_path()) {
                        (Some(img), Some(path)) => {
                            std::thread::spawn(move || {
                                set(save_png(&path, &img).map(|()| path.display().to_string()));
                            });
                        }
                        (Some(_), None) => set(Ok("(not saved: no --screenshot path)".into())),
                        (None, _) => set(Err("no screenshot event within 5 s".into())),
                    }
                    // One more edit rendered immediately, then close while
                    // it is still in flight.
                    app.draft.push('!');
                    app.submit(now, crate::preview::Quality::Settled);
                    self.to(Stage::Closing, now);
                }
            }
            Stage::Closing => {
                if self.stage_since == now {
                    return;
                }
                let busy = app.worker.in_flight() > 0;
                app.report
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .closed_while_busy = Some(busy);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl Script {
    /// Starts an 8K export into a fresh temporary folder.
    fn start_export(&mut self, app: &mut StudioApp, ctx: &egui::Context, now: Instant) {
        let dir =
            std::env::temp_dir().join(format!("pigment-studio-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let frame = pigment_core::frame::Frame::largest_with_aspect(
            app.doc.recipe().frame.aspect(),
            pigment_io::export::LONG_EDGE_8K,
        );
        let started = std::fs::create_dir_all(&dir).is_ok()
            && frame.is_ok_and(|f| app.start_export_to(f, dir.join("script-8k.png")).is_ok());
        self.export_job = app.exporter.running().map(|r| r.job);
        self.export_dir = Some(dir);
        if started {
            self.to(Stage::Exporting, now);
        } else {
            app.report.lock().unwrap_or_else(|e| e.into_inner()).export =
                Some((false, false, Duration::ZERO));
            app.open_advanced = Some(true);
            self.capture(app, ctx, now);
        }
    }

    /// Checks the exported file, removes the folder, then captures.
    fn finish_export(&mut self, app: &mut StudioApp, ctx: &egui::Context, now: Instant) {
        let elapsed = now - self.stage_since;
        let dir = self.export_dir.clone().unwrap_or_default();
        let decoded = std::fs::File::open(dir.join("script-8k.png"))
            .ok()
            .and_then(|f| {
                png::Decoder::new(std::io::BufReader::new(f))
                    .read_info()
                    .ok()
            })
            .map(|r| (r.info().width, r.info().height));
        let alone = std::fs::read_dir(&dir).is_ok_and(|d| d.count() == 1);
        let _ = std::fs::remove_dir_all(&dir);
        app.report.lock().unwrap_or_else(|e| e.into_inner()).export = Some((
            decoded == Some((7680, 4320)) && alone,
            self.export_stable,
            elapsed,
        ));
        app.open_advanced = Some(true);
        self.capture(app, ctx, now);
    }

    /// Captures the window. Whether a simulated device loss was shown is
    /// judged here, so it holds whichever stage the loss happened in.
    fn capture(&mut self, app: &StudioApp, ctx: &egui::Context, now: Instant) {
        app.report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .device_loss_shown = Some(app.view.device_lost && app.worker.in_flight() == 0);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        self.to(Stage::Capturing, now);
    }
}

impl Default for Script {
    fn default() -> Self {
        Script::new()
    }
}

/// Writes an egui screenshot (sRGB, opaque) as an 8-bit RGB PNG.
pub fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = img.size;
    let rgb: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|c| [c.r(), c.g(), c.b()])
        .collect();
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&rgb).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())
}
