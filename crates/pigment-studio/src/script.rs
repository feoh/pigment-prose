//! `pigment-studio --script`: a scripted run of the real window, used as the
//! task 11 acceptance check. It types into the prose editor, resizes the
//! window twice, waits for the preview to settle, feeds the view a
//! deliberately out-of-order old result, captures the window, then starts
//! one more (delayed) render and closes the window while it runs.
//!
//! Frames are requested continuously, so the gap between frames measures
//! whether the UI thread was ever blocked, in particular while the worker
//! runs a render slowed by `--preview-delay-ms`.

use std::path::Path;
use std::time::{Duration, Instant};

use eframe::egui;
use pigment_core::request::RequestId;

use crate::app::StudioApp;
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
        }
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

#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Typing,
    Settling,
    Capturing,
    Closing,
}

#[derive(Debug)]
pub struct Script {
    start: Option<Instant>,
    last_frame: Option<Instant>,
    typed: usize,
    stage: Stage,
    stage_since: Instant,
}

impl Script {
    pub fn new() -> Script {
        Script {
            start: None,
            last_frame: None,
            typed: 0,
            stage: Stage::Typing,
            stage_since: Instant::now(),
        }
    }

    fn to(&mut self, stage: Stage, now: Instant) {
        self.stage = stage;
        self.stage_since = now;
    }

    pub fn step(&mut self, app: &mut StudioApp, ctx: &egui::Context, now: Instant) {
        ctx.request_repaint(); // keep frames flowing so gaps are measurable
        let start = *self.start.get_or_insert(now);
        let t = now - start;
        let busy = app.worker.in_flight() > 0;
        {
            let mut r = app.report.lock().unwrap_or_else(|e| e.into_inner());
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
                    r.device_loss_shown = Some(app.view.device_lost && app.worker.in_flight() == 0);
                    r.stale_rejected = Some(!replaced && app.view.shown.map(|s| s.id) == shown);
                    drop(r);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    self.to(Stage::Capturing, now);
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
                    app.submit(now);
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
