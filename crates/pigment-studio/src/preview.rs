//! Preview scheduling and display rules (docs/architecture.md, "Preview
//! lifecycle"), kept free of egui so they can be tested directly.
//!
//! - **Debounce:** prose edits wait 300 ms after the last keystroke;
//!   preview-area resizes wait 100 ms; shape (aspect ratio) changes, new
//!   compositions and the first frame render at once.
//! - **Sliders:** every slider value renders at once as an *interaction
//!   preview* (long edge 960 px), not debounced; latest-wins coalescing in
//!   the worker absorbs the rate. 150 ms after the last slider input the
//!   preview is rendered again at the settled size.
//! - **Size:** the painting keeps the document's aspect ratio and is fitted
//!   inside the preview area at its physical pixel size, capped at a long
//!   edge of 3840 px (the settled preview). It is never stretched to the
//!   window's shape.
//! - **Stale results:** a result is shown only if it is newer than the one
//!   on screen ([`PreviewState::accept`]).

use std::time::{Duration, Instant};

use pigment_core::error::RenderError;
use pigment_core::frame::AspectRatio;
use pigment_core::job::PreviewState;
use pigment_core::request::RequestId;

use crate::worker::{PreviewOutcome, PreviewResult};

pub const PROSE_DEBOUNCE: Duration = Duration::from_millis(300);
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(100);
/// Quiet time after the last slider input before the settled preview.
pub const SETTLE_AFTER: Duration = Duration::from_millis(150);
/// Long-edge cap of the settled preview. Raised from 1920 px in task 12 so
/// the painting fills the preview area on high-DPI displays: 3840×2160
/// renders in 9–15 ms on the RTX 4070 Ti (`paint-bench`, 2026-09-28).
/// Slower GPUs lower it at run time ([`adapt_settled_cap`]).
pub const SETTLED_LONG_EDGE: u32 = 3840;
/// Settled previews should render within this (docs/architecture.md,
/// "Responsiveness targets").
pub const SETTLED_BUDGET: Duration = Duration::from_millis(150);
/// The settled cap is never lowered below this: 1920×1080 renders in
/// 83 ms p95 on the Intel UHD iGPU measured in task 14.
pub const SETTLED_FLOOR: u32 = 1920;

/// The settled cap after a settled preview with a long edge of `rendered`
/// px took `render`. Unchanged while renders stay inside the budget. Past
/// it, the cap drops to the size predicted to take two thirds of the
/// budget (render time grows with the pixel count, so with the square of
/// the edge), never below [`SETTLED_FLOOR`], and it never rises again.
pub fn adapt_settled_cap(cap: u32, rendered: u32, render: Duration) -> u32 {
    if render <= SETTLED_BUDGET || rendered <= SETTLED_FLOOR {
        return cap;
    }
    let scale = (SETTLED_BUDGET.as_secs_f64() * 2.0 / 3.0 / render.as_secs_f64()).sqrt();
    let target = (rendered as f64 * scale) as u32;
    target.clamp(SETTLED_FLOOR, cap)
}
/// Long-edge cap while a slider is moving.
pub const INTERACTION_LONG_EDGE: u32 = 960;
/// Smallest preview edge rendered; below this the area is too small to use.
pub const MIN_PREVIEW_EDGE: u32 = 16;

/// The largest `aspect` rectangle inside `avail` physical pixels, capped at
/// `max_long` on its long edge. `None` if the area is too small.
pub fn preview_size(aspect: AspectRatio, avail: (f32, f32), max_long: u32) -> Option<(u32, u32)> {
    let (aw, ah) = (aspect.width as f64, aspect.height as f64);
    let (w, h) = (avail.0.max(0.0) as f64, avail.1.max(0.0) as f64);
    let scale = (w / aw).min(h / ah).min(max_long as f64 / aw.max(ah));
    let (pw, ph) = ((aw * scale).floor() as u32, (ah * scale).floor() as u32);
    (pw >= MIN_PREVIEW_EDGE && ph >= MIN_PREVIEW_EDGE).then_some((pw, ph))
}

/// Size, in physical pixels, at which an image of `image` pixels is drawn
/// inside `avail`: fitted with its own aspect ratio, capped at the settled
/// long edge. A smaller interaction preview is scaled up to that size, so
/// the painting does not jump while a slider moves; an image within a pixel
/// and a half of it is drawn 1:1.
pub fn display_size(image: (u32, u32), avail: (f32, f32)) -> (f32, f32) {
    let (w, h) = (image.0.max(1) as f32, image.1.max(1) as f32);
    let cap = SETTLED_LONG_EDGE as f32 / w.max(h);
    let scale = (avail.0 / w).min(avail.1 / h).min(cap).max(0.0);
    let fit = (w * scale, h * scale);
    if (fit.0 - w).abs() <= 1.5 && (fit.1 - h).abs() <= 1.5 {
        (w, h)
    } else {
        fit
    }
}

/// What the next preview is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// A slider is moving: small and fast.
    Interaction,
    /// Input has settled: full preview size.
    Settled,
}

impl Quality {
    /// Long-edge cap for this quality, given the current settled cap.
    pub fn long_edge(self, settled_cap: u32) -> u32 {
        match self {
            Quality::Interaction => INTERACTION_LONG_EDGE.min(settled_cap),
            Quality::Settled => settled_cap,
        }
    }
}

/// When to submit the next preview, and at which quality.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scheduler {
    due: Option<Instant>,
    /// Pending settled re-render after slider input.
    settle: Option<Instant>,
}

impl Scheduler {
    /// Render as soon as possible (first frame, shape change, new
    /// composition, opened recipe).
    pub fn now(&mut self, now: Instant) {
        self.due = Some(now);
    }

    /// A keystroke: wait until typing pauses.
    pub fn prose_edited(&mut self, now: Instant) {
        self.due = Some(now + PROSE_DEBOUNCE);
    }

    /// The preview area changed size. Does not delay an earlier deadline
    /// by more than the resize debounce.
    pub fn resized(&mut self, now: Instant) {
        let d = now + RESIZE_DEBOUNCE;
        self.due = Some(self.due.map_or(d, |old| old.max(d)));
    }

    /// A slider moved (drag or key): render now at interaction quality and
    /// again at the settled size once the input stops.
    pub fn slider_moved(&mut self, now: Instant) {
        self.due = Some(now);
        self.settle = Some(now + SETTLE_AFTER);
    }

    /// Whether a render is due, and at which quality. Clears what it
    /// returns.
    pub fn take_due(&mut self, now: Instant) -> Option<Quality> {
        let settling = self.settle.is_some_and(|s| now < s);
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            return Some(if settling {
                Quality::Interaction
            } else {
                self.settle = None;
                Quality::Settled
            });
        }
        if self.settle.is_some_and(|s| now >= s) {
            self.settle = None;
            return Some(Quality::Settled);
        }
        None
    }

    /// Time until the next deadline, for `request_repaint_after`.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        [self.due, self.settle]
            .into_iter()
            .flatten()
            .min()
            .map(|d| d.saturating_duration_since(now))
    }
}

/// What is on screen, and how it got there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub id: RequestId,
    pub width: u32,
    pub height: u32,
    /// Decision to render → result collected by the UI.
    pub latency: Duration,
    /// Render + readback inside the renderer.
    pub render: Duration,
    pub scene_reused: bool,
}

/// The UI's view of preview results.
#[derive(Debug, Default)]
pub struct PreviewView {
    state: PreviewState,
    pub shown: Option<Shown>,
    /// The newest failure, cleared by the next image shown.
    pub error: Option<RenderError>,
    pub device_lost: bool,
    /// Results that arrived after a newer one was already on screen.
    pub stale_dropped: u32,
    pub cancelled: u32,
}

/// An image to upload as the preview texture.
#[derive(Debug)]
pub struct Accepted {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

impl PreviewView {
    pub fn submitted(&mut self, id: RequestId) {
        self.state.submitted(id);
    }

    /// The screen shows an older recipe than the newest request.
    pub fn is_pending(&self) -> bool {
        self.state.is_pending()
    }

    pub fn requested(&self) -> Option<RequestId> {
        self.state.requested
    }

    /// Applies one result. Returns the image to show, if any.
    pub fn receive(&mut self, r: PreviewResult, collected: Instant) -> Option<Accepted> {
        match r.outcome {
            PreviewOutcome::Image {
                width,
                height,
                rgba8,
            } => {
                if !self.state.accept(r.id) {
                    self.stale_dropped += 1;
                    return None;
                }
                self.error = None;
                self.shown = Some(Shown {
                    id: r.id,
                    width,
                    height,
                    latency: collected.saturating_duration_since(r.submitted_at),
                    render: r.timings.map_or(Duration::ZERO, |t| t.render_readback),
                    scene_reused: r.scene_reused,
                });
                Some(Accepted {
                    width,
                    height,
                    rgba8,
                })
            }
            PreviewOutcome::Cancelled => {
                self.cancelled += 1;
                None
            }
            PreviewOutcome::Failed(e) => {
                // An old failure must not mask a newer success.
                if self.shown.is_some_and(|s| s.id > r.id) {
                    self.stale_dropped += 1;
                    return None;
                }
                if matches!(e, RenderError::DeviceLost { .. }) {
                    self.device_lost = true;
                }
                // Stop "rendering…" for a request that will never arrive.
                self.state.accept(r.id);
                self.error = Some(e);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: u64, at: Instant) -> PreviewResult {
        PreviewResult {
            id: RequestId(id),
            outcome: PreviewOutcome::Image {
                width: 2,
                height: 1,
                rgba8: vec![id as u8; 8],
            },
            scene_reused: false,
            scene_time: Duration::ZERO,
            timings: None,
            submitted_at: at,
        }
    }

    #[test]
    fn sizes_keep_the_aspect_and_fit() {
        let a = AspectRatio::of(16, 9);
        assert_eq!(preview_size(a, (1600.0, 1200.0), 1920), Some((1600, 900)));
        assert_eq!(preview_size(a, (1000.0, 300.0), 1920), Some((533, 300)));
        assert_eq!(preview_size(a, (5000.0, 5000.0), 1920), Some((1920, 1080)));
        let p = AspectRatio::of(9, 16);
        assert_eq!(preview_size(p, (5000.0, 5000.0), 1920), Some((1080, 1920)));
        assert_eq!(preview_size(p, (5000.0, 5000.0), 960), Some((540, 960)));
        let sq = AspectRatio::of(1, 1);
        assert_eq!(preview_size(sq, (800.5, 600.9), 1920), Some((600, 600)));
        assert_eq!(preview_size(a, (10.0, 10.0), 1920), None);
        assert_eq!(preview_size(a, (-5.0, 100.0), 1920), None);
        // Always within one pixel of the exact aspect ratio.
        for (w, h) in [(1234.0, 777.0), (333.0, 999.0), (1920.0, 1081.0)] {
            let (pw, ph) = preview_size(a, (w, h), 1920).unwrap();
            assert!(pw as f32 <= w && ph as f32 <= h);
            assert!((pw as f64 / 16.0 * 9.0 - ph as f64).abs() < 1.0);
        }
    }

    #[test]
    fn typing_is_debounced_and_resizes_do_not_starve_it() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        s.prose_edited(t0);
        assert!(s.take_due(t0 + Duration::from_millis(299)).is_none());
        s.prose_edited(t0 + Duration::from_millis(200)); // still typing
        assert!(s.take_due(t0 + Duration::from_millis(450)).is_none());
        assert_eq!(
            s.take_due(t0 + Duration::from_millis(500)),
            Some(Quality::Settled)
        );
        assert!(
            s.take_due(t0 + Duration::from_millis(501)).is_none(),
            "cleared"
        );
        s.resized(t0);
        assert_eq!(s.wait(t0), Some(RESIZE_DEBOUNCE));
        assert!(s.take_due(t0 + RESIZE_DEBOUNCE).is_some());
        s.now(t0);
        assert_eq!(s.take_due(t0), Some(Quality::Settled));
    }

    #[test]
    fn slider_input_renders_small_at_once_then_settles() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut s = Scheduler::default();
        // A drag: one value per frame, each rendered at once, small.
        for f in 0..10 {
            s.slider_moved(ms(f * 16));
            assert_eq!(s.take_due(ms(f * 16)), Some(Quality::Interaction));
            assert_eq!(s.take_due(ms(f * 16 + 1)), None);
        }
        // Released at 144 ms: nothing until 150 ms of quiet, then settled.
        assert_eq!(s.wait(ms(200)), Some(Duration::from_millis(94)));
        assert_eq!(s.take_due(ms(293)), None);
        assert_eq!(s.take_due(ms(294)), Some(Quality::Settled));
        assert_eq!(s.take_due(ms(400)), None);
        assert_eq!(s.wait(ms(400)), None);
        // A resize during a drag stays small; the settle still follows.
        s.slider_moved(ms(1000));
        s.take_due(ms(1000));
        s.resized(ms(1010));
        assert_eq!(s.take_due(ms(1110)), Some(Quality::Interaction));
        assert_eq!(s.take_due(ms(1150)), Some(Quality::Settled));
        // Prose typed after a drag has settled renders settled.
        s.prose_edited(ms(2000));
        assert_eq!(s.take_due(ms(2300)), Some(Quality::Settled));
    }

    #[test]
    fn a_slow_gpu_lowers_the_settled_cap_to_fit_the_budget() {
        let ms = Duration::from_millis;
        // Fast enough: unchanged.
        assert_eq!(adapt_settled_cap(3840, 3840, ms(14)), 3840);
        assert_eq!(adapt_settled_cap(3840, 3840, ms(150)), 3840);
        // The Intel iGPU's 3840×2160 at ~317 ms: about 2150 px.
        let c = adapt_settled_cap(3840, 3840, ms(317));
        assert!((2000..2300).contains(&c), "{c}");
        // Never below the floor, never raised, never from a floor-sized render.
        assert_eq!(adapt_settled_cap(3840, 3840, ms(5000)), SETTLED_FLOOR);
        assert_eq!(adapt_settled_cap(2400, 2400, ms(20)), 2400);
        assert_eq!(adapt_settled_cap(3840, 1920, ms(400)), 3840);
        assert!(adapt_settled_cap(c, c, ms(151)) <= c);
    }

    #[test]
    fn interaction_previews_are_drawn_at_the_settled_size() {
        let avail = (1848.0, 1039.0);
        let a = AspectRatio::of(16, 9);
        let settled = preview_size(a, avail, SETTLED_LONG_EDGE).unwrap();
        let small = preview_size(a, avail, INTERACTION_LONG_EDGE).unwrap();
        assert_eq!(small, (960, 540));
        let d = display_size(small, avail);
        assert!((d.0 - settled.0 as f32).abs() < 2.0 && (d.1 - settled.1 as f32).abs() < 2.0);
        // The settled preview itself is drawn 1:1.
        assert_eq!(
            display_size(settled, avail),
            (settled.0 as f32, settled.1 as f32)
        );
        // Never above the settled cap, never stretched.
        let big = display_size((960, 540), (9000.0, 9000.0));
        let cap = SETTLED_LONG_EDGE as f32;
        assert_eq!(big, (cap, cap * 9.0 / 16.0));
        let tall = display_size((540, 960), (1000.0, 500.0));
        assert!((tall.0 / tall.1 - 540.0 / 960.0).abs() < 1e-4 && tall.1 <= 500.0);
    }

    #[test]
    fn an_out_of_order_old_result_never_replaces_the_newest() {
        let t = Instant::now();
        let mut v = PreviewView::default();
        for i in 1..=3 {
            v.submitted(RequestId(i));
        }
        assert!(v.receive(image(3, t), t).is_some());
        assert!(v.receive(image(1, t), t).is_none(), "late old result");
        assert!(v.receive(image(2, t), t).is_none());
        assert_eq!(v.shown.unwrap().id, RequestId(3));
        assert_eq!(v.stale_dropped, 2);
        assert!(!v.is_pending());
        // An old failure does not replace a newer image either.
        let fail = PreviewResult {
            outcome: PreviewOutcome::Failed(RenderError::Gpu { detail: "x".into() }),
            ..image(2, t)
        };
        assert!(v.receive(fail, t).is_none());
        assert!(v.error.is_none());
    }

    #[test]
    fn failures_and_device_loss_end_the_pending_state() {
        let t = Instant::now();
        let mut v = PreviewView::default();
        v.submitted(RequestId(1));
        let lost = PreviewResult {
            outcome: PreviewOutcome::Failed(RenderError::DeviceLost {
                detail: "reset".into(),
            }),
            ..image(1, t)
        };
        v.receive(lost, t);
        assert!(v.device_lost);
        assert!(!v.is_pending());
        assert!(v.error.is_some());
    }
}
