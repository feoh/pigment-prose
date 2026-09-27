//! The studio window: prose on the left, the painting in the middle, a
//! status line at the bottom. The UI thread only edits the document, submits
//! snapshots to the render worker and uploads finished previews; it never
//! waits on the GPU.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use pigment_core::capability::GpuCapabilities;
use pigment_core::error::{RenderError, TextError};
use pigment_core::frame::{AspectRatio, Frame};
use pigment_core::request::RequestIds;
use pigment_io::Document;

use crate::preview::{PreviewView, SETTLED_LONG_EDGE, Scheduler, preview_size};
use crate::script::{Report, Script};
use crate::worker::{PreviewJob, PreviewOutcome, PreviewResult, PreviewWorker, WorkerOptions};

/// A synthetic default passage, so the first launch shows a painting.
pub const DEFAULT_PROSE: &str = "Morning light on the lake; wind in the pines below the ridge.";

/// Long edge of the document frame for each shape (the default export
/// size); the preview only uses its aspect ratio.
const DOCUMENT_LONG_EDGE: u32 = 3840;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Wide,
    Landscape,
    Square,
    Portrait,
    Tall,
}

impl Shape {
    pub const ALL: [Shape; 5] = [
        Shape::Wide,
        Shape::Landscape,
        Shape::Square,
        Shape::Portrait,
        Shape::Tall,
    ];

    pub fn aspect(self) -> AspectRatio {
        let (w, h) = match self {
            Shape::Wide => (16, 9),
            Shape::Landscape => (3, 2),
            Shape::Square => (1, 1),
            Shape::Portrait => (4, 5),
            Shape::Tall => (9, 16),
        };
        AspectRatio::of(w, h)
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Wide => "Wide landscape 16:9",
            Shape::Landscape => "Landscape 3:2",
            Shape::Square => "Square 1:1",
            Shape::Portrait => "Portrait 4:5",
            Shape::Tall => "Tall portrait 9:16",
        }
    }

    fn frame(self) -> Frame {
        Frame::largest_with_aspect(self.aspect(), DOCUMENT_LONG_EDGE)
            .expect("preset shapes are valid")
    }
}

#[derive(Debug, Clone, Default)]
pub struct StudioOptions {
    /// Print one line per displayed preview (id, size, scene, timings).
    pub log_timings: bool,
    /// Run the scripted responsiveness check, then close.
    pub script: bool,
    /// Where the script saves its capture of the window.
    pub screenshot: Option<PathBuf>,
    pub worker: WorkerOptions,
}

pub struct StudioApp {
    pub(crate) doc: Document,
    /// The editor's text. Applied to the document when typing pauses; the
    /// document keeps the last valid prose if this is rejected.
    pub(crate) draft: String,
    pub(crate) prose_error: Option<TextError>,
    shape: Shape,
    pub(crate) worker: PreviewWorker,
    ids: RequestIds,
    pub(crate) view: PreviewView,
    pub(crate) scheduler: Scheduler,
    texture: Option<egui::TextureHandle>,
    caps: GpuCapabilities,
    /// Physical pixels available to the preview.
    area_px: Option<(f32, f32)>,
    last_job: Option<PreviewJob>,
    show_diagnostics: bool,
    opts: StudioOptions,
    pub(crate) script: Option<Script>,
    pub(crate) report: Arc<Mutex<Report>>,
}

impl std::fmt::Debug for StudioApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioApp")
            .field("doc", &self.doc)
            .field("shape", &self.shape)
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

/// Same render inputs (ignoring when it was asked for).
fn same_render(a: &PreviewJob, b: &PreviewJob) -> bool {
    a.seeds == b.seeds
        && a.form == b.form
        && a.aspect == b.aspect
        && a.appearance == b.appearance
        && (a.width, a.height) == (b.width, b.height)
}

impl StudioApp {
    pub fn new(
        worker: PreviewWorker,
        caps: GpuCapabilities,
        opts: StudioOptions,
        report: Arc<Mutex<Report>>,
    ) -> StudioApp {
        let shape = Shape::Wide;
        let doc =
            Document::from_prose(DEFAULT_PROSE, shape.frame()).expect("default prose is valid");
        let script = opts.script.then(Script::new);
        StudioApp {
            doc,
            draft: DEFAULT_PROSE.to_string(),
            prose_error: None,
            shape,
            worker,
            ids: RequestIds::new(),
            view: PreviewView::default(),
            scheduler: Scheduler::default(),
            texture: None,
            caps,
            area_px: None,
            last_job: None,
            show_diagnostics: false,
            opts,
            script,
            report,
        }
    }

    /// Applies the draft and submits a preview if anything visible changed.
    pub(crate) fn submit(&mut self, now: Instant) {
        if self.view.device_lost {
            return;
        }
        match self.doc.set_prose(&self.draft) {
            Ok(()) => self.prose_error = None,
            Err(e) => self.prose_error = Some(e),
        }
        let Some(area) = self.area_px else { return };
        let aspect = self.doc.recipe().frame.aspect();
        let Some((width, height)) = preview_size(aspect, area, SETTLED_LONG_EDGE) else {
            return;
        };
        let job = PreviewJob {
            seeds: self.doc.seeds(),
            form: self.doc.recipe().form,
            aspect,
            appearance: self.doc.appearance(),
            width,
            height,
            submitted_at: now,
        };
        if self.last_job.as_ref().is_some_and(|j| same_render(j, &job)) {
            return;
        }
        let id = self.ids.next();
        self.view.submitted(id);
        let s = self.worker.submit(id, job.clone());
        self.last_job = Some(job);
        let mut r = self.report.lock().unwrap_or_else(|e| e.into_inner());
        r.submitted += 1;
        r.superseded += s.superseded.is_some() as u32;
        r.max_in_flight = r.max_in_flight.max(self.worker.in_flight());
    }

    /// Takes finished results from the worker and uploads the newest.
    fn collect(&mut self, ctx: &egui::Context, now: Instant) {
        for r in self.worker.drain() {
            self.accept(ctx, r, now);
        }
    }

    pub(crate) fn accept(&mut self, ctx: &egui::Context, r: PreviewResult, now: Instant) -> bool {
        let (id, scene_reused, scene_time) = (r.id, r.scene_reused, r.scene_time);
        let failed = matches!(r.outcome, PreviewOutcome::Failed(_));
        let Some(img) = self.view.receive(r, now) else {
            if failed && let Some(e) = &self.view.error {
                eprintln!("preview {}: {e}", id.0);
            }
            return false;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba8,
        );
        match &mut self.texture {
            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
            }
        }
        let shown = self.view.shown.expect("just accepted");
        let mut rep = self.report.lock().unwrap_or_else(|e| e.into_inner());
        rep.displayed += 1;
        rep.latencies.push(shown.latency);
        rep.renders.push(shown.render);
        drop(rep);
        if self.opts.log_timings {
            println!(
                "preview id={} {}x{} scene={} ({:.2} ms) render+readback {:.2} ms, request→shown {:.1} ms",
                id.0,
                img.width,
                img.height,
                if scene_reused { "reused" } else { "built" },
                scene_time.as_secs_f64() * 1e3,
                shown.render.as_secs_f64() * 1e3,
                shown.latency.as_secs_f64() * 1e3,
            );
        }
        true
    }

    fn actions_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Pigment Prose");
            ui.separator();
            ui.add_enabled(false, egui::Button::new("Save recipe…"))
                .on_disabled_hover_text("Saving recipes is not available yet");
            ui.add_enabled(false, egui::Button::new("Export PNG…"))
                .on_disabled_hover_text("Exporting images is not available yet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_diagnostics, "Diagnostics")
                    .on_hover_text("GPU adapter, backend and preview timings (Ctrl+D)");
            });
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui, now: Instant) {
        ui.add_space(4.0);
        let label = ui.heading("Prose");
        ui.label(
            egui::RichText::new(
                "Paste or write anything. The text seeds the painting; its meaning is not read.",
            )
            .weak(),
        );
        let edit = egui::TextEdit::multiline(&mut self.draft)
            .desired_rows(14)
            .desired_width(f32::INFINITY)
            .hint_text("Paste some prose…");
        let resp = ui.add(edit).labelled_by(label.id);
        if resp.changed() {
            self.scheduler.prose_edited(now);
            self.prose_error = Document::check_prose(&self.draft).err();
        }
        match self.prose_error {
            Some(e) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("{e}. The preview keeps the last painting."),
                );
            }
            None => {
                ui.label(
                    egui::RichText::new(format!("{} bytes", self.draft.len()))
                        .weak()
                        .small(),
                );
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.heading("Painting");
        let before = self.shape;
        egui::ComboBox::from_label("Shape")
            .selected_text(self.shape.label())
            .show_ui(ui, |ui| {
                for s in Shape::ALL {
                    ui.selectable_value(&mut self.shape, s, s.label());
                }
            });
        if self.shape != before {
            // Validated preset; a new aspect ratio recomposes the scene.
            let _ = self.doc.set_frame(self.shape.frame());
            self.scheduler.now(now);
        }
        ui.label(
            egui::RichText::new("A new shape recomposes the painting.")
                .weak()
                .small(),
        );
    }

    fn status(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let sw = self.caps.adapter.software;
            let device = egui::RichText::new(self.caps.label());
            ui.label(if sw {
                device.color(ui.visuals().warn_fg_color)
            } else {
                device
            })
            .on_hover_text("GPU used for painting");
            if sw {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "SOFTWARE RENDERER, not GPU accelerated",
                );
            }
            ui.separator();
            match self.view.shown {
                Some(s) => {
                    ui.label(format!(
                        "Preview {}×{} · render {:.1} ms · shown {:.0} ms after request",
                        s.width,
                        s.height,
                        s.render.as_secs_f64() * 1e3,
                        s.latency.as_secs_f64() * 1e3
                    ));
                }
                None => {
                    ui.label("No preview yet");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.view.is_pending() {
                    ui.add(egui::Spinner::new());
                    ui.label("Painting…");
                }
            });
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, now: Instant) {
        let rect = ui.available_rect_before_wrap();
        let ppp = ui.ctx().pixels_per_point();
        let area = (rect.width() * ppp, rect.height() * ppp);
        let rounded = |a: (f32, f32)| (a.0.round() as i64, a.1.round() as i64);
        match self.area_px {
            None => self.scheduler.now(now),
            Some(old) if rounded(old) != rounded(area) => self.scheduler.resized(now),
            _ => {}
        }
        self.area_px = Some(area);

        if self.view.device_lost {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "The GPU was reset (device lost), so the preview has stopped.\n\
                             Restart Pigment Prose to continue. Your prose is still in the editor.",
                        )
                        .color(ui.visuals().error_fg_color),
                    );
                });
            });
            return;
        }
        let (Some(tex), Some(shown)) = (&self.texture, self.view.shown) else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Spinner::new().size(32.0));
                });
            });
            return;
        };
        // Shown at 1:1 physical pixels once settled; while a new size is on
        // its way the old image is scaled to fit, never stretched.
        let natural = egui::vec2(shown.width as f32, shown.height as f32) / ppp;
        let scale = (rect.width() / natural.x)
            .min(rect.height() / natural.y)
            .min(1.0);
        let size = natural * scale;
        let img_rect = egui::Rect::from_center_size(rect.center(), size);
        let resp = ui.put(
            img_rect,
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size))
                .fit_to_exact_size(size),
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Image, true, "Painting preview")
        });
        if let Some(e) = &self.view.error {
            let msg = match e {
                RenderError::InvalidRequest(v) => {
                    format!("This combination cannot be painted: {v}")
                }
                other => format!("Preview failed: {other}"),
            };
            ui.put(
                egui::Rect::from_min_size(
                    img_rect.left_bottom() - egui::vec2(0.0, 28.0),
                    egui::vec2(img_rect.width(), 24.0),
                ),
                egui::Label::new(egui::RichText::new(msg).color(ui.visuals().error_fg_color)),
            );
        }
    }

    fn diagnostics(&mut self, ctx: &egui::Context) {
        let caps = &self.caps;
        let a = &caps.adapter;
        let l = &caps.device_limits;
        let stats = self.worker.stats();
        let mut open = self.show_diagnostics;
        egui::Window::new("Diagnostics")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::Grid::new("diag")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        let mut row = |k: &str, v: String| {
                            ui.label(k);
                            ui.label(v);
                            ui.end_row();
                        };
                        row("Adapter", a.name.clone());
                        row("Backend", format!("{:?}", a.backend));
                        row("Device type", format!("{:?}", a.kind));
                        row(
                            "Hardware acceleration",
                            if a.software {
                                "no (software rasterizer)".into()
                            } else {
                                "yes".into()
                            },
                        );
                        row("Driver", format!("{} ({})", a.driver, a.driver_info));
                        row("wgpu", caps.wgpu_version.to_string());
                        row("Max 2D texture", l.max_texture_dimension_2d.to_string());
                        row("Max buffer", format!("{} MiB", l.max_buffer_size >> 20));
                        row(
                            "Preview jobs",
                            format!(
                                "{} started, {} painted, {} cancelled, {} failed; {} in flight",
                                stats.started,
                                stats.rendered,
                                stats.cancelled,
                                stats.failed,
                                self.worker.in_flight()
                            ),
                        );
                        row(
                            "Scenes",
                            format!(
                                "{} built, {} reused",
                                stats.scenes_built, stats.scenes_reused
                            ),
                        );
                        row("Stale results dropped", self.view.stale_dropped.to_string());
                        if !self.opts.worker.delay.is_zero() {
                            row(
                                "Simulated render delay",
                                format!("{} ms", self.opts.worker.delay.as_millis()),
                            );
                        }
                    });
            });
        self.show_diagnostics = open;
    }

    pub(crate) fn script_screenshot_path(&self) -> Option<PathBuf> {
        self.opts.screenshot.clone()
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        self.collect(&ctx, now);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
            self.show_diagnostics = !self.show_diagnostics;
        }
        if let Some(mut script) = self.script.take() {
            script.step(self, &ctx, now);
            self.script = Some(script);
        }

        egui::Panel::top("actions").show(ui, |ui| self.actions_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status(ui));
        egui::Panel::left("controls")
            .resizable(true)
            .default_size(340.0)
            .size_range(240.0..=600.0)
            .show(ui, |ui| self.controls(ui, now));
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(ui.visuals().extreme_bg_color))
            .show(ui, |ui| self.preview(ui, now));
        if self.show_diagnostics {
            self.diagnostics(&ctx);
        }

        if self.scheduler.take_due(now) {
            self.submit(now);
        }
        if let Some(wait) = self.scheduler.wait(now) {
            ctx.request_repaint_after(wait);
        }
    }

    fn on_exit(&mut self) {
        let t = Instant::now();
        let joined = self.worker.shutdown(Duration::from_secs(2));
        let mut r = self.report.lock().unwrap_or_else(|e| e.into_inner());
        r.shutdown = Some((t.elapsed(), joined));
    }
}
