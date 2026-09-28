//! The studio window: prose, composition and the artistic controls on the
//! left, the painting in the middle, a status line at the bottom. The UI
//! thread only edits the document, submits snapshots to the render worker
//! and uploads finished previews; it never waits on the GPU or on a file
//! dialog.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui::Key;
use pigment_core::capability::GpuCapabilities;
use pigment_core::error::{RenderError, TextError};
use pigment_core::frame::{AspectRatio, Frame};
use pigment_core::request::{RequestId, RequestIds};
use pigment_core::seed::Variation;
use pigment_core::settings::{Channel, ControlSpec, ControlValues, Group};
use pigment_io::Document;
use raw_window_handle::HasWindowHandle;

use crate::controls::{self, Edit};
use crate::files::{self, Choice, Dialogs, Effect, FileFlow, Intent, Parent, Step};
use crate::preview::{PreviewView, Quality, Scheduler, display_size, preview_size};
use crate::script::{Report, Script};
use crate::theme;
use crate::worker::{PreviewJob, PreviewOutcome, PreviewResult, PreviewWorker, WorkerOptions};

/// A synthetic default passage, so the first launch shows a painting.
pub const DEFAULT_PROSE: &str = "Morning light on the lake; wind in the pines below the ridge.";

/// Long edge of a new document's frame (the default export size); the
/// preview only uses its aspect ratio.
const DOCUMENT_LONG_EDGE: u32 = 3840;

/// How long a confirmation ("Saved …") stays up.
const INFO_NOTICE_FOR: Duration = Duration::from_secs(6);

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

    /// The preset with this aspect ratio, if any (recipes may carry others).
    pub fn of(aspect: AspectRatio) -> Option<Shape> {
        Shape::ALL.into_iter().find(|s| s.aspect() == aspect)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Warning,
    Error,
}

/// A message about the last file operation. Names files, never contents.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    pub at: Instant,
}

/// A submitted preview: what it renders and at which quality.
#[derive(Debug, Clone)]
struct Submission {
    id: RequestId,
    job: PreviewJob,
    quality: Quality,
}

pub struct StudioApp {
    pub(crate) doc: Document,
    /// The editor's text. Applied to the document when typing pauses; the
    /// document keeps the last valid prose if this is rejected.
    pub(crate) draft: String,
    pub(crate) prose_error: Option<TextError>,
    pub(crate) worker: PreviewWorker,
    ids: RequestIds,
    pub(crate) view: PreviewView,
    pub(crate) scheduler: Scheduler,
    texture: Option<egui::TextureHandle>,
    caps: GpuCapabilities,
    /// Physical pixels available to the preview.
    pub(crate) area_px: Option<(f32, f32)>,
    last_job: Option<PreviewJob>,
    /// Recent submissions, to know what the image on screen shows.
    recent: VecDeque<Submission>,
    /// The render inputs and quality of the image on screen.
    pub(crate) shown: Option<(PreviewJob, Quality)>,
    show_diagnostics: bool,
    /// The control column is shown (collapse it to give the painting the
    /// window).
    pub(crate) show_controls: bool,
    opts: StudioOptions,
    pub(crate) script: Option<Script>,
    pub(crate) report: Arc<Mutex<Report>>,
    pub(crate) files: FileFlow,
    dialogs: Box<dyn Dialogs>,
    pub(crate) notice: Option<Notice>,
    /// The untouched launch document: closing or replacing it asks nothing.
    pristine: Option<Document>,
    /// Closing is confirmed (or scripted); the next close request goes
    /// through.
    pub(crate) allow_close: bool,
    /// Opens or closes the Advanced section on the next frame.
    pub(crate) open_advanced: Option<bool>,
    /// Keyboard focus last frame, to scroll a newly focused control into
    /// view.
    last_focus: Option<egui::Id>,
    title: String,
}

impl std::fmt::Debug for StudioApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioApp")
            .field("doc", &self.doc)
            .field("view", &self.view)
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}

/// Same scene and paint (ignoring size and when it was asked for).
pub(crate) fn same_inputs(a: &PreviewJob, b: &PreviewJob) -> bool {
    a.seeds == b.seeds && a.form == b.form && a.aspect == b.aspect && a.appearance == b.appearance
}

/// Same render inputs, including size.
fn same_render(a: &PreviewJob, b: &PreviewJob) -> bool {
    same_inputs(a, b) && (a.width, a.height) == (b.width, b.height)
}

impl StudioApp {
    pub fn new(
        worker: PreviewWorker,
        caps: GpuCapabilities,
        opts: StudioOptions,
        report: Arc<Mutex<Report>>,
        dialogs: Box<dyn Dialogs>,
    ) -> StudioApp {
        let doc = Document::from_prose(DEFAULT_PROSE, Shape::Wide.frame())
            .expect("default prose is valid");
        let script = opts.script.then(Script::new);
        StudioApp {
            pristine: Some(doc.clone()),
            doc,
            draft: DEFAULT_PROSE.to_string(),
            prose_error: None,
            worker,
            ids: RequestIds::new(),
            view: PreviewView::default(),
            scheduler: Scheduler::default(),
            texture: None,
            caps,
            area_px: None,
            last_job: None,
            recent: VecDeque::new(),
            shown: None,
            show_diagnostics: false,
            show_controls: true,
            opts,
            script,
            report,
            files: FileFlow::default(),
            dialogs,
            notice: None,
            allow_close: false,
            open_advanced: None,
            last_focus: None,
            title: String::new(),
        }
    }

    /// The document as the controls edit it.
    pub fn values(&self) -> ControlValues {
        ControlValues {
            form: self.doc.recipe().form,
            appearance: self.doc.appearance(),
        }
    }

    /// Whether closing or replacing the document would lose work. The
    /// untouched launch document has nothing worth keeping.
    pub fn unsaved(&self) -> bool {
        // Valid text typed but not yet applied (debounce) counts too.
        let typing = Document::check_prose(&self.draft).is_ok()
            && self.doc.prose() != Some(self.draft.as_str());
        (self.doc.is_dirty() || typing) && (typing || self.pristine.as_ref() != Some(&self.doc))
    }

    /// The painting's inputs as they are now (size aside).
    fn current_job(&self, width: u32, height: u32, now: Instant) -> PreviewJob {
        PreviewJob {
            seeds: self.doc.seeds(),
            form: self.doc.recipe().form,
            aspect: self.doc.recipe().frame.aspect(),
            appearance: self.doc.appearance(),
            width,
            height,
            submitted_at: now,
        }
    }

    /// The image on screen shows the current document.
    pub fn preview_is_current(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|(j, _)| same_inputs(j, &self.current_job(0, 0, Instant::now())))
    }

    /// A source-free recipe is open and the editor is still empty: the
    /// painting comes from the stored seed, not from any text.
    pub fn source_free(&self) -> bool {
        self.doc.prose().is_none() && self.draft.is_empty()
    }

    /// Applies the editor's text to the document.
    fn apply_draft(&mut self) {
        if self.source_free() {
            self.prose_error = None;
            return;
        }
        match self.doc.set_prose(&self.draft) {
            Ok(()) => self.prose_error = None,
            Err(e) => self.prose_error = Some(e),
        }
    }

    /// Sets one control. `interacting`: part of a drag or held key, so a
    /// small preview now and a settled one when the input stops.
    pub fn set_control(&mut self, spec: &ControlSpec, value: f64, now: Instant, interacting: bool) {
        let mut v = self.values();
        spec.set(&mut v, value);
        self.apply_values(
            v,
            Edit {
                changed: true,
                interacting,
            },
            now,
        );
    }

    fn apply_values(&mut self, v: ControlValues, edit: Edit, now: Instant) {
        // The sliders clamp to the specified ranges, so these validate.
        let form = self.doc.set_form(v.form);
        let appearance = self.doc.set_appearance(v.appearance);
        debug_assert!(form.is_ok() && appearance.is_ok());
        if edit.interacting {
            self.scheduler.slider_moved(now);
        } else {
            self.scheduler.now(now);
        }
    }

    /// "Another Composition": the next variation, same prose and settings.
    pub fn another_composition(&mut self, now: Instant) {
        self.doc.next_variation();
        self.scheduler.now(now);
    }

    /// Back one variation (there is none before 0).
    pub fn previous_composition(&mut self, now: Instant) {
        let v = self.doc.recipe().seed.variation.0;
        if v > 0 {
            self.doc.set_variation(Variation(v - 1));
            self.scheduler.now(now);
        }
    }

    fn set_shape(&mut self, aspect: AspectRatio, now: Instant) {
        let f = self.doc.recipe().frame;
        let long = f.width.max(f.height);
        // Keep the document's size class; only the proportions change.
        if let Ok(frame) = Frame::largest_with_aspect(aspect, long)
            && self.doc.set_frame(frame).is_ok()
        {
            self.scheduler.now(now);
        }
    }

    /// Applies the draft and submits a preview if anything visible changed.
    pub(crate) fn submit(&mut self, now: Instant, quality: Quality) {
        if self.view.device_lost {
            return;
        }
        self.apply_draft();
        let Some(area) = self.area_px else { return };
        let aspect = self.doc.recipe().frame.aspect();
        let Some((width, height)) = preview_size(aspect, area, quality.long_edge()) else {
            return;
        };
        let job = self.current_job(width, height, now);
        if self.last_job.as_ref().is_some_and(|j| same_render(j, &job)) {
            return;
        }
        let id = self.ids.next();
        self.view.submitted(id);
        let s = self.worker.submit(id, job.clone());
        self.recent.push_back(Submission {
            id,
            job: job.clone(),
            quality,
        });
        while self.recent.len() > 8 {
            self.recent.pop_front();
        }
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
        let submission = self.recent.iter().find(|s| s.id == id).cloned();
        // Opaque, so premultiplied and unmultiplied are the same bytes; the
        // premultiplied path skips the per-pixel conversion.
        let image = egui::ColorImage::from_rgba_premultiplied(
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
        if rep.last_displayed.is_some_and(|last| last >= id) {
            rep.out_of_order_displays += 1;
        }
        rep.last_displayed = Some(id);
        if submission
            .as_ref()
            .is_some_and(|s| s.quality == Quality::Interaction)
        {
            rep.interaction_shown += 1;
        }
        drop(rep);
        self.shown = submission.map(|s| (s.job, s.quality));
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

    // ---- files ---------------------------------------------------------

    /// Starts a file operation (menu, button or shortcut).
    pub(crate) fn file_intent(
        &mut self,
        intent: Intent,
        ctx: &egui::Context,
        parent: Option<&dyn Parent>,
        now: Instant,
    ) {
        self.apply_draft();
        let unsaved = self.unsaved();
        let effects =
            self.files
                .request(intent, &mut self.doc, unsaved, &mut *self.dialogs, parent);
        self.apply_effects(effects, ctx, now);
    }

    pub(crate) fn file_choice(
        &mut self,
        choice: Choice,
        ctx: &egui::Context,
        parent: Option<&dyn Parent>,
        now: Instant,
    ) {
        let effects = self
            .files
            .choose(choice, &mut self.doc, &mut *self.dialogs, parent);
        self.apply_effects(effects, ctx, now);
    }

    fn poll_files(&mut self, ctx: &egui::Context, parent: Option<&dyn Parent>, now: Instant) {
        let effects = self.files.poll(&mut self.doc, &mut *self.dialogs, parent);
        self.apply_effects(effects, ctx, now);
    }

    fn notify(&mut self, kind: NoticeKind, text: String, now: Instant) {
        self.notice = Some(Notice {
            kind,
            text,
            at: now,
        });
    }

    fn apply_effects(&mut self, effects: Vec<Effect>, ctx: &egui::Context, now: Instant) {
        for e in effects {
            match e {
                Effect::Opened { name, notices } => {
                    self.draft = self.doc.prose().unwrap_or("").to_string();
                    self.prose_error = None;
                    self.pristine = None;
                    self.scheduler.now(now);
                    let mut text = format!("Opened {name}.");
                    let mut kind = NoticeKind::Info;
                    if self.doc.prose().is_none() {
                        text += " It was saved without its prose.";
                    }
                    if !notices.is_empty() {
                        kind = NoticeKind::Warning;
                        let n: Vec<String> = notices.iter().map(|n| n.to_string()).collect();
                        text += &format!(" This recipe was {}.", n.join("; "));
                    }
                    self.notify(kind, text, now);
                }
                Effect::Saved { name } => {
                    self.pristine = None;
                    let with = if self.doc.saves_source_text() {
                        "with the prose"
                    } else {
                        "without the prose"
                    };
                    self.notify(NoticeKind::Info, format!("Saved {name} ({with})."), now);
                }
                Effect::Close => {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Effect::Failed {
                    action,
                    name,
                    error,
                } => {
                    let after = if action == "open" {
                        "The current painting is unchanged."
                    } else {
                        "Any existing file is unchanged, and your changes are still unsaved."
                    };
                    self.notify(
                        NoticeKind::Error,
                        format!("Could not {action} {name}: {error}. {after}"),
                        now,
                    );
                }
            }
        }
    }

    fn document_name(&self) -> String {
        self.doc
            .path()
            .map(files::display_name)
            .unwrap_or_else(|| "Untitled".to_string())
    }

    // ---- layout ----------------------------------------------------------

    fn actions_bar(&mut self, ui: &mut egui::Ui, parent: Option<&dyn Parent>, now: Instant) {
        let ctx = ui.ctx().clone();
        let busy = self.files.busy();
        let cmd = |k: &str| {
            if cfg!(target_os = "macos") {
                format!("⌘{k}")
            } else {
                format!("Ctrl+{k}")
            }
        };
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new("Pigment Prose")
                    .family(theme::semibold())
                    .color(theme::INK),
            );
            ui.add_space(12.0);
            let action = |ui: &mut egui::Ui, text: &str, hint: String| {
                ui.add_enabled(!busy, theme::quiet(text))
                    .on_hover_text(hint)
                    .clicked()
            };
            if action(ui, "Open…", format!("Open a recipe ({})", cmd("O"))) {
                self.file_intent(Intent::Open, &ctx, parent, now);
            }
            if action(ui, "Save", format!("Save the recipe ({})", cmd("S"))) {
                self.file_intent(Intent::Save, &ctx, parent, now);
            }
            if action(
                ui,
                "Save As…",
                format!("Save the recipe to a new file ({})", cmd("Shift+S")),
            ) {
                self.file_intent(Intent::SaveAs, &ctx, parent, now);
            }
            ui.add_enabled(false, theme::quiet("Export PNG…"))
                .on_disabled_hover_text("Exporting images arrives in a later version");
            ui.add_space(12.0);
            let unsaved = self.unsaved();
            if unsaved {
                theme::dot(ui, theme::AMBER);
            }
            ui.label(egui::RichText::new(self.document_name()).color(theme::INK_2))
                .on_hover_text(if unsaved {
                    "Unsaved changes"
                } else {
                    "Everything is saved"
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Button::selectable(self.show_diagnostics, "Diagnostics")
                        .frame_when_inactive(false),
                )
                .on_hover_text(format!(
                    "GPU adapter, backend and preview timings ({})",
                    cmd("D")
                ))
                .clicked()
                .then(|| self.show_diagnostics = !self.show_diagnostics);
                ui.add(egui::Button::selectable(
                    !self.show_controls,
                    "Painting only",
                ))
                .on_hover_text(format!(
                    "Hide the controls so the painting fills the window ({})",
                    cmd("\\")
                ))
                .clicked()
                .then(|| self.show_controls = !self.show_controls);
            });
        });
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
        let Some(n) = &self.notice else { return };
        let (color, word) = match n.kind {
            NoticeKind::Info => (theme::INK_3, "Done"),
            NoticeKind::Warning => (theme::AMBER, "Note"),
            NoticeKind::Error => (theme::CORAL, "Problem"),
        };
        let mut dismiss = false;
        ui.horizontal(|ui| {
            theme::dot(ui, color);
            ui.label(
                egui::RichText::new(word)
                    .family(theme::semibold())
                    .color(color),
            );
            // Leave room for Dismiss; long messages wrap.
            let room = (ui.available_width() - 90.0).max(120.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(egui::RichText::new(&n.text).color(theme::INK)).wrap());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                dismiss = ui
                    .add(theme::quiet("Dismiss"))
                    .on_hover_text("Hide this message")
                    .clicked();
            });
        });
        if dismiss {
            self.notice = None;
        }
    }

    fn prose_section(&mut self, ui: &mut egui::Ui, now: Instant) {
        let label = ui.label(theme::header("Prose"));
        ui.label(
            egui::RichText::new("The text seeds the painting. Its meaning is never read.")
                .small()
                .color(theme::INK_3),
        );
        ui.add_space(2.0);
        let hint = if self.doc.prose().is_none() {
            "Saved without its prose. Typing here starts a new painting."
        } else {
            "Paste or write anything…"
        };
        let edit = egui::TextEdit::multiline(&mut self.draft)
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6))
            .hint_text(hint);
        let resp = ui.add(edit).labelled_by(label.id);
        if resp.changed() {
            self.scheduler.prose_edited(now);
            self.prose_error = if self.source_free() {
                None
            } else {
                Document::check_prose(&self.draft).err()
            };
        }
        if self.source_free() {
            ui.label(
                egui::RichText::new(
                    "This recipe was saved without its prose. The painting is rebuilt from \
                     its stored seed; the words cannot be recovered from it.",
                )
                .small()
                .color(theme::INK_2),
            );
        } else {
            match self.prose_error {
                Some(e) => {
                    ui.horizontal_wrapped(|ui| {
                        theme::dot(ui, theme::AMBER);
                        ui.label(
                            egui::RichText::new(format!(
                                "{e}. The preview keeps the last painting."
                            ))
                            .color(theme::AMBER),
                        );
                    });
                }
                None => {
                    // The seed code: what the painting is actually made
                    // from. One changed character gives a new code.
                    let code = &self.doc.recipe().seed.digest.to_hex()[..8];
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let small = |t: String| egui::RichText::new(t).small().color(theme::INK_3);
                        ui.label(small(format!("{} bytes · seed", self.draft.len())));
                        ui.label(
                            egui::RichText::new(code)
                                .monospace()
                                .size(12.5)
                                .color(theme::INK_2),
                        )
                    })
                    .inner
                    .on_hover_text(
                        "The first digits of the text's digest. The painting is built from \
                         this, never from the words.",
                    );
                }
            }
        }
        ui.add_space(4.0);
        let mut keep = self.doc.keep_source_text();
        let resp = ui
            .checkbox(&mut keep, "Save the prose in the recipe file")
            .on_hover_text(
                "Off: the recipe reproduces the painting, but not your words. \
                 On: anyone with the file can read the prose.",
            );
        if resp.changed() {
            self.doc.set_keep_source_text(keep);
        }
        ui.label(
            egui::RichText::new(if keep {
                "The prose will be saved with the recipe."
            } else {
                "Recipes are saved without the prose."
            })
            .small()
            .color(if keep { theme::AMBER } else { theme::INK_3 }),
        );
    }

    fn composition_section(&mut self, ui: &mut egui::Ui, now: Instant) {
        ui.horizontal(|ui| {
            ui.label(theme::header("Composition"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    egui::RichText::new(self.doc.recipe().seed.variation.0.to_string())
                        .monospace()
                        .color(theme::INK),
                )
                .on_hover_text("Stored in the recipe; reopening it restores this composition");
                ui.label(egui::RichText::new("Variation").small().color(theme::INK_3));
            });
        });
        ui.label(
            egui::RichText::new("Same prose and settings, a new arrangement of the land.")
                .small()
                .color(theme::INK_3),
        );
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let alt = if cfg!(target_os = "macos") {
                "⌥"
            } else {
                "Alt+"
            };
            let first = self.doc.recipe().seed.variation.0 == 0;
            if ui
                .add_enabled(!first, egui::Button::new("Previous"))
                .on_hover_text(format!("Back one composition ({alt}Left)"))
                .on_disabled_hover_text("This is the first composition")
                .clicked()
            {
                self.previous_composition(now);
            }
            let w = ui.available_width();
            let another = ui
                .add_sized(
                    [w, ui.spacing().interact_size.y],
                    theme::primary("Another composition"),
                )
                .on_hover_text(format!("A new arrangement of the land ({alt}Right)"));
            theme::focus_on_accent(ui, &another);
            if another.clicked() {
                self.another_composition(now);
            }
        });
        ui.add_space(8.0);
        let aspect = self.doc.recipe().frame.aspect();
        let current = Shape::of(aspect);
        let label = ui.label(egui::RichText::new("Shape").color(theme::INK));
        let selected = current.map_or_else(
            || format!("Custom {}:{}", aspect.width, aspect.height),
            |s| s.label().to_string(),
        );
        let mut picked = None;
        egui::ComboBox::from_id_salt("shape")
            .selected_text(selected)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for s in Shape::ALL {
                    if ui.selectable_label(current == Some(s), s.label()).clicked() {
                        picked = Some(s);
                    }
                }
            })
            .response
            .labelled_by(label.id);
        if let Some(s) = picked
            && Some(s) != current
        {
            self.set_shape(s.aspect(), now);
        }
        ui.label(
            egui::RichText::new("A new shape recomposes the painting.")
                .small()
                .color(theme::INK_3),
        );
    }

    fn channel_block(
        ui: &mut egui::Ui,
        channel: Channel,
        specs: &[&'static ControlSpec],
        values: &mut ControlValues,
        with_palette: bool,
    ) -> Edit {
        let (title, note) = controls::channel_heading(channel);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(title)
                .family(theme::semibold())
                .color(theme::INK),
        );
        ui.label(egui::RichText::new(note).small().color(theme::INK_3));
        let mut edit = Edit::default();
        for spec in specs {
            ui.add_space(6.0);
            let mut v = spec.get(values);
            let e = controls::slider(ui, spec, &mut v);
            if e.changed {
                spec.set(values, v);
            }
            edit = edit.merge(e);
        }
        if with_palette {
            ui.add_space(6.0);
            edit = edit.merge(controls::palette_picker(
                ui,
                &mut values.appearance.palette.id,
            ));
        }
        edit
    }

    fn painting_section(&mut self, ui: &mut egui::Ui, now: Instant) {
        ui.label(theme::header("Painting"));
        let mut values = self.values();
        let mut edit = Edit::default();
        for (ch, specs) in controls::grouped(Group::Main) {
            edit = edit.merge(Self::channel_block(
                ui,
                ch,
                &specs,
                &mut values,
                ch == Channel::Appearance,
            ));
        }
        ui.add_space(14.0);
        ui.separator();
        let mut header = egui::CollapsingHeader::new(theme::header("Advanced")).id_salt("advanced");
        if let Some(open) = self.open_advanced.take() {
            header = header.open(Some(open));
        }
        header.show_unindented(ui, |ui| {
            for (ch, specs) in controls::grouped(Group::Advanced) {
                edit = edit.merge(Self::channel_block(ui, ch, &specs, &mut values, false));
            }
        });
        ui.add_space(10.0);
        let defaults = ControlValues::default();
        if ui
            .add_enabled(
                values != defaults,
                egui::Button::new("Reset all painting controls"),
            )
            .on_hover_text(
                "Every slider and the palette back to its default. Composition and shape stay.",
            )
            .clicked()
        {
            values = defaults;
            edit = Edit {
                changed: true,
                interacting: false,
            };
        }
        if edit.changed {
            self.apply_values(values, edit, now);
        }
    }

    /// What the painting on screen is, in one word, with its colour.
    fn now_state(&self) -> (&'static str, egui::Color32) {
        if self.view.device_lost {
            ("GPU reset", theme::CORAL)
        } else if self.view.error.is_some() {
            ("Preview failed", theme::CORAL)
        } else if self.view.is_pending() {
            ("Painting…", theme::AMBER)
        } else if self.shown.is_none() {
            ("Starting", theme::INK_3)
        } else if self.preview_is_current() {
            ("Current", theme::INK_3)
        } else {
            ("Earlier settings", theme::AMBER)
        }
    }

    fn status(&self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            let sw = self.caps.adapter.software;
            ui.label(egui::RichText::new(self.caps.label()).small().color(if sw {
                theme::CORAL
            } else {
                theme::INK_2
            }))
            .on_hover_text("The GPU that paints the preview");
            if sw {
                ui.label(
                    egui::RichText::new("SOFTWARE RENDERER, not GPU accelerated")
                        .small()
                        .family(theme::semibold())
                        .color(theme::CORAL),
                );
            }
            ui.separator();
            let figures = match self.view.shown {
                Some(s) => format!(
                    "Preview {}×{} · render {:.1} ms · shown {:.0} ms after request",
                    s.width,
                    s.height,
                    s.render.as_secs_f64() * 1e3,
                    s.latency.as_secs_f64() * 1e3
                ),
                None => "no preview yet".to_string(),
            };
            ui.label(egui::RichText::new(figures).small().color(theme::INK_3));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (word, color) = self.now_state();
                let resp = ui.label(egui::RichText::new(word).small().color(
                    if color == theme::INK_3 {
                        theme::INK_2
                    } else {
                        color
                    },
                ));
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        true,
                        format!("Preview: {word}"),
                    )
                });
                theme::dot(ui, color);
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
                             Save your recipe, then restart Pigment Prose to continue.",
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
        // Drawn 1:1 once settled. A smaller interaction preview, or an old
        // image while a new size is on its way, is scaled to the settled
        // size, never stretched to the window's shape.
        let (w, h) = display_size((shown.width, shown.height), area);
        let size = egui::vec2(w, h) / ppp;
        let img_rect = egui::Rect::from_center_size(rect.center(), size);
        let resp = ui.put(
            img_rect,
            egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size))
                .fit_to_exact_size(size),
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Image, true, "Painting preview")
        });
        // Say plainly when the image is not the current settings.
        let caption = if let Some(e) = &self.view.error {
            let why = match e {
                RenderError::InvalidRequest(v) => {
                    format!("these settings cannot be painted: {v}")
                }
                other => format!("the preview failed: {other}"),
            };
            Some((format!("Showing earlier settings; {why}"), theme::CORAL))
        } else if !self.preview_is_current() && self.shown.is_some() {
            Some((
                "Showing earlier settings · painting the latest".to_string(),
                theme::AMBER,
            ))
        } else {
            None
        };
        if let Some((text, color)) = caption {
            let at = img_rect.left_top() + egui::vec2(10.0, 10.0);
            egui::Area::new(egui::Id::new("preview-caption"))
                .fixed_pos(at)
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(theme::RAISED)
                        .stroke(egui::Stroke::new(1.0, theme::SEAM_STRONG))
                        .corner_radius(theme::RADIUS)
                        .inner_margin(egui::Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.set_max_width(img_rect.width() - 40.0);
                            ui.horizontal(|ui| {
                                theme::dot(ui, color);
                                ui.label(egui::RichText::new(text).small().color(theme::INK));
                            });
                        });
                });
        }
    }

    fn confirm_modal(&mut self, ctx: &egui::Context, parent: Option<&dyn Parent>, now: Instant) {
        let Step::Confirm { then } = self.files.step else {
            return;
        };
        let what = match then {
            Intent::Quit => "before closing",
            _ => "before opening another recipe",
        };
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("unsaved"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(
                    egui::RichText::new("Save changes to this painting?")
                        .family(theme::semibold())
                        .size(17.0)
                        .color(theme::INK),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} has unsaved changes. Save them {what}, or they will be lost.",
                        self.document_name()
                    ))
                    .color(theme::INK_2),
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    // The destructive choice stands apart, outlined.
                    let discard = ui.add(
                        egui::Button::new(egui::RichText::new("Don't save").color(theme::CORAL))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0, theme::SEAM_STRONG)),
                    );
                    if discard.clicked() {
                        choice = Some(Choice::Discard);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save = ui.add(theme::primary("Save…"));
                        if ui.memory(|m| m.focused().is_none()) {
                            save.request_focus();
                        }
                        theme::focus_on_accent(ui, &save);
                        if save.clicked() {
                            choice = Some(Choice::Save);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(Choice::Cancel);
                        }
                    });
                });
            });
        if choice.is_none() && modal.should_close() {
            choice = Some(Choice::Cancel);
        }
        if let Some(c) = choice {
            self.file_choice(c, ctx, parent, now);
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

    /// Global shortcuts, taken before any widget sees the keys.
    fn shortcuts(&mut self, ctx: &egui::Context, parent: Option<&dyn Parent>, now: Instant) {
        use egui::Modifiers as M;
        let take = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if take(M::COMMAND, Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if take(M::COMMAND, Key::D) {
            self.show_diagnostics = !self.show_diagnostics;
        }
        if take(M::COMMAND, Key::Backslash) {
            self.show_controls = !self.show_controls;
        }
        if self.files.busy() {
            return;
        }
        // Most specific first: Shift+S before S.
        if take(M::COMMAND | M::SHIFT, Key::S) {
            self.file_intent(Intent::SaveAs, ctx, parent, now);
        } else if take(M::COMMAND, Key::S) {
            self.file_intent(Intent::Save, ctx, parent, now);
        }
        if take(M::COMMAND, Key::O) {
            self.file_intent(Intent::Open, ctx, parent, now);
        }
        if take(M::ALT, Key::ArrowRight) {
            self.another_composition(now);
        }
        if take(M::ALT, Key::ArrowLeft) {
            self.previous_composition(now);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!(
            "{}{} — Pigment Prose",
            if self.unsaved() { "*" } else { "" },
            self.document_name()
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let parent: Option<&dyn Parent> = frame.window_handle().is_ok().then_some(&*frame as _);
        self.collect(&ctx, now);
        self.poll_files(&ctx, parent, now);
        self.shortcuts(&ctx, parent, now);
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            self.apply_draft();
            if self.unsaved() || self.files.busy() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.file_intent(Intent::Quit, &ctx, parent, now);
            }
        }
        if let Some(mut script) = self.script.take() {
            script.step(self, &ctx, now);
            self.script = Some(script);
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|n| n.kind == NoticeKind::Info && now - n.at > INFO_NOTICE_FOR)
        {
            self.notice = None;
        }

        egui::Panel::top("actions")
            .frame(theme::panel_frame(egui::Margin::symmetric(14, 0)))
            .exact_size(38.0)
            .show(ui, |ui| self.actions_bar(ui, parent, now));
        if self.notice.is_some() {
            egui::Panel::top("notice")
                .frame(theme::panel_frame(egui::Margin::symmetric(14, 6)))
                .show(ui, |ui| self.notice_bar(ui));
        }
        egui::Panel::bottom("status")
            .frame(theme::panel_frame(egui::Margin::symmetric(14, 0)))
            .exact_size(28.0)
            .show(ui, |ui| self.status(ui));
        if self.show_controls {
            egui::Panel::left("controls")
                .frame(theme::panel_frame(egui::Margin::ZERO))
                .resizable(true)
                .default_size(360.0)
                .size_range(300.0..=600.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    left: 16,
                                    right: 18,
                                    top: 14,
                                    bottom: 18,
                                })
                                .show(ui, |ui| {
                                    self.prose_section(ui, now);
                                    ui.add_space(14.0);
                                    ui.separator();
                                    ui.add_space(10.0);
                                    self.composition_section(ui, now);
                                    ui.add_space(14.0);
                                    ui.separator();
                                    ui.add_space(10.0);
                                    self.painting_section(ui, now);
                                });
                            // egui does not follow keyboard focus in a scroll
                            // area; Tab must never land on a hidden control.
                            let focus = ui.memory(|m| m.focused());
                            if focus != self.last_focus {
                                if let Some(r) = focus.and_then(|id| ui.ctx().read_response(id)) {
                                    r.scroll_to_me(Some(egui::Align::Center));
                                }
                                self.last_focus = focus;
                            }
                        });
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::SURROUND)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| self.preview(ui, now));
        if self.show_diagnostics {
            self.diagnostics(&ctx);
        }
        self.confirm_modal(&ctx, parent, now);
        self.update_title(&ctx);

        if let Some(q) = self.scheduler.take_due(now) {
            self.submit(now, q);
        }
        if let Some(wait) = self.scheduler.wait(now) {
            ctx.request_repaint_after(wait);
        }
        if let Some(n) = &self.notice
            && n.kind == NoticeKind::Info
        {
            ctx.request_repaint_after(INFO_NOTICE_FOR.saturating_sub(now - n.at));
        }
    }

    fn on_exit(&mut self) {
        let t = Instant::now();
        let joined = self.worker.shutdown(Duration::from_secs(2));
        let mut r = self.report.lock().unwrap_or_else(|e| e.into_inner());
        r.shutdown = Some((t.elapsed(), joined));
    }
}
