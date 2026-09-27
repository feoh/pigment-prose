//! Preview scheduling and display rules (docs/architecture.md, "Preview
//! lifecycle"), kept free of egui so they can be tested directly.
//!
//! - **Debounce:** prose edits wait 300 ms after the last keystroke;
//!   preview-area resizes wait 100 ms; shape (aspect ratio) changes and the
//!   first frame render at once.
//! - **Size:** the painting keeps the document's aspect ratio and is fitted
//!   inside the preview area at its physical pixel size, capped at a long
//!   edge of 1920 px (the settled preview). It is never stretched to the
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
/// Long-edge cap of the settled preview.
pub const SETTLED_LONG_EDGE: u32 = 1920;
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

/// When to submit the next preview.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scheduler {
    due: Option<Instant>,
}

impl Scheduler {
    /// Render as soon as possible (first frame, shape change).
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

    /// Whether a render is due; clears the deadline when it is.
    pub fn take_due(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            return true;
        }
        false
    }

    /// Time until the next deadline, for `request_repaint_after`.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.due.map(|d| d.saturating_duration_since(now))
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
        assert!(!s.take_due(t0 + Duration::from_millis(299)));
        s.prose_edited(t0 + Duration::from_millis(200)); // still typing
        assert!(!s.take_due(t0 + Duration::from_millis(450)));
        assert!(s.take_due(t0 + Duration::from_millis(500)));
        assert!(!s.take_due(t0 + Duration::from_millis(501)), "cleared");
        s.resized(t0);
        assert_eq!(s.wait(t0), Some(RESIZE_DEBOUNCE));
        assert!(s.take_due(t0 + RESIZE_DEBOUNCE));
        s.now(t0);
        assert!(s.take_due(t0));
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
