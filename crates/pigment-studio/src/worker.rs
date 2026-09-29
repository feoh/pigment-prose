//! The render worker (docs/architecture.md, "Preview lifecycle").
//!
//! One thread owns the renderer and loops on a latest-wins
//! [`Mailbox`]: a new submission replaces the pending job and cancels the
//! running one, so at most one job runs and one waits however fast the user
//! types or drags. Scene generation runs here too, and the last scene is
//! reused while the structural inputs are unchanged, so paint-only changes
//! and preview resizes never rebuild it.
//!
//! Results go back over a bounded channel and the UI is woken with the
//! supplied callback (`egui::Context::request_repaint`). The UI thread never
//! waits on this thread except in [`PreviewWorker::shutdown`], which is
//! bounded by a timeout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pigment_core::error::RenderError;
use pigment_core::frame::AspectRatio;
use pigment_core::job::{Mailbox, NoProgress, Submitted};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderRequest, RenderTarget, RenderTimings, Renderer,
    RequestId,
};
use pigment_core::scene::{Scene, SceneKey};
use pigment_core::seed::SeedBundle;
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::{TileOrder, TilePolicy};

/// Everything one preview render needs: a snapshot, so later edits in the
/// UI cannot reach a job in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewJob {
    pub biome: pigment_core::biome::BiomeId,
    pub seeds: SeedBundle,
    pub form: FormSettings,
    pub aspect: AspectRatio,
    pub appearance: Appearance,
    pub width: u32,
    pub height: u32,
    /// When the UI decided to render (for latency measurement).
    pub submitted_at: Instant,
}

#[derive(Debug)]
pub enum PreviewOutcome {
    /// RGBA8, sRGB-encoded, opaque, `width × height`.
    Image {
        width: u32,
        height: u32,
        rgba8: Vec<u8>,
    },
    /// Superseded before it started, or cancelled at a tile boundary.
    Cancelled,
    Failed(RenderError),
}

#[derive(Debug)]
pub struct PreviewResult {
    pub id: RequestId,
    pub outcome: PreviewOutcome,
    /// The scene came from the cache (no regeneration).
    pub scene_reused: bool,
    pub scene_time: Duration,
    pub timings: Option<RenderTimings>,
    pub submitted_at: Instant,
}

/// Test and diagnostic knobs. Defaults change nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkerOptions {
    /// Simulated slow GPU: wait this long before each render, like one long
    /// dispatch. Cancellation is checked only before and after it; shutdown
    /// interrupts it.
    pub delay: Duration,
    /// Simulated device loss: after this many successful renders every job
    /// fails with `RenderError::DeviceLost`.
    pub lose_device_after: Option<u32>,
}

/// Counters for tests and the diagnostics panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkerStats {
    pub started: u32,
    pub rendered: u32,
    pub cancelled: u32,
    pub failed: u32,
    pub scenes_built: u32,
    pub scenes_reused: u32,
}

#[derive(Debug)]
pub struct PreviewWorker {
    mailbox: Arc<Mailbox<PreviewJob>>,
    results: Option<Receiver<PreviewResult>>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<Mutex<WorkerStats>>,
    thread: Option<JoinHandle<()>>,
}

/// Results the UI has not collected yet. The worker blocks when the channel
/// is full, which bounds memory if the UI stops draining (window hidden).
const RESULT_CAPACITY: usize = 2;

impl PreviewWorker {
    pub fn spawn<R: Renderer + Send + 'static>(
        renderer: R,
        options: WorkerOptions,
        wake: impl Fn() + Send + 'static,
    ) -> PreviewWorker {
        let mailbox = Arc::new(Mailbox::new());
        let (tx, rx) = mpsc::sync_channel(RESULT_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(WorkerStats::default()));
        let thread = {
            let (mailbox, shutdown, stats) = (mailbox.clone(), shutdown.clone(), stats.clone());
            std::thread::Builder::new()
                .name("pigment-render".into())
                .spawn(move || run(renderer, options, &mailbox, &tx, &shutdown, &stats, &wake))
                .expect("spawn the render thread")
        };
        PreviewWorker {
            mailbox,
            results: Some(rx),
            shutdown,
            stats,
            thread: Some(thread),
        }
    }

    /// Never blocks on rendering.
    pub fn submit(&self, id: RequestId, job: PreviewJob) -> Submitted {
        self.mailbox.submit(id, job)
    }

    /// Finished results, oldest first. Never blocks.
    pub fn drain(&self) -> Vec<PreviewResult> {
        let mut out = Vec::new();
        if let Some(rx) = &self.results {
            while let Ok(r) = rx.try_recv() {
                out.push(r);
            }
        }
        out
    }

    /// Jobs pending or running: 0, 1 or 2.
    pub fn in_flight(&self) -> usize {
        self.mailbox.in_flight()
    }

    pub fn stats(&self) -> WorkerStats {
        *self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops the worker: drops pending work, cancels running work,
    /// interrupts a simulated delay, and waits up to `timeout` for the
    /// thread. Returns whether it finished in time (a real GPU dispatch
    /// cannot be interrupted, but previews are single short tiles). A thread
    /// still running is detached and ends with the process.
    pub fn shutdown(&mut self, timeout: Duration) -> bool {
        self.shutdown.store(true, Ordering::Release);
        self.mailbox.close();
        // Unblock a worker waiting to send a result.
        self.results = None;
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        thread.join().is_ok()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(2));
    }
}

fn run<R: Renderer>(
    renderer: R,
    options: WorkerOptions,
    mailbox: &Mailbox<PreviewJob>,
    tx: &SyncSender<PreviewResult>,
    shutdown: &AtomicBool,
    stats: &Mutex<WorkerStats>,
    wake: &dyn Fn(),
) {
    let bump = |f: fn(&mut WorkerStats)| f(&mut stats.lock().unwrap_or_else(|e| e.into_inner()));
    let mut last_scene: Option<(SceneKey, Arc<Scene>)> = None;
    let mut device_lost = false;
    while let Some(job) = mailbox.next() {
        let (id, cancel, p) = (job.id, job.cancel, job.payload);
        bump(|s| s.started += 1);
        let finish = |outcome, scene_reused, scene_time, timings| PreviewResult {
            id,
            outcome,
            scene_reused,
            scene_time,
            timings,
            submitted_at: p.submitted_at,
        };

        // Scene: reused while seeds, form and aspect ratio are unchanged.
        let t_scene = Instant::now();
        let generator = pigment_core::scene::generator(p.biome);
        let key = SceneKey::new(generator.version(), &p.seeds, p.biome, p.form, p.aspect);
        let (scene, reused) = match &last_scene {
            Some((k, s)) if *k == key => (Ok(s.clone()), true),
            _ => (
                generator
                    .generate(&p.seeds, &p.form, p.aspect)
                    .map(Arc::new),
                false,
            ),
        };
        let scene_time = t_scene.elapsed();
        let result = match scene {
            Err(e) => {
                bump(|s| s.failed += 1);
                finish(
                    PreviewOutcome::Failed(RenderError::InvalidRequest(e)),
                    false,
                    scene_time,
                    None,
                )
            }
            Ok(scene) => {
                if reused {
                    bump(|s| s.scenes_reused += 1);
                } else {
                    bump(|s| s.scenes_built += 1);
                    last_scene = Some((key, scene.clone()));
                }
                // Simulated slow render (tests, --preview-delay-ms).
                let until = Instant::now() + options.delay;
                while Instant::now() < until && !shutdown.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5).min(until - Instant::now()));
                }
                if let Some(n) = options.lose_device_after {
                    device_lost |= stats.lock().unwrap_or_else(|e| e.into_inner()).rendered >= n;
                }
                let req = RenderRequest {
                    id,
                    purpose: RenderPurpose::Preview,
                    scene,
                    seeds: p.seeds,
                    appearance: p.appearance,
                    target: RenderTarget {
                        width: p.width,
                        height: p.height,
                        policy: TilePolicy::Single,
                        order: TileOrder::RowMajor,
                    },
                };
                let mut sink = MemorySink::default();
                let rendered = if device_lost {
                    Err(RenderError::DeviceLost {
                        detail: "simulated device loss".into(),
                    })
                } else {
                    renderer.render(&req, &cancel, &mut NoProgress, &mut sink)
                };
                match rendered {
                    Ok(rep) if rep.outcome == RenderOutcome::Completed => {
                        bump(|s| s.rendered += 1);
                        finish(
                            PreviewOutcome::Image {
                                width: sink.width,
                                height: sink.height,
                                rgba8: sink.rgba8,
                            },
                            reused,
                            scene_time,
                            Some(rep.timings),
                        )
                    }
                    Ok(rep) => {
                        bump(|s| s.cancelled += 1);
                        finish(
                            PreviewOutcome::Cancelled,
                            reused,
                            scene_time,
                            Some(rep.timings),
                        )
                    }
                    Err(e) => {
                        bump(|s| s.failed += 1);
                        finish(PreviewOutcome::Failed(e), reused, scene_time, None)
                    }
                }
            }
        };
        mailbox.done(id);
        if tx.send(result).is_err() {
            break; // the UI has gone
        }
        wake();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::AtomicU32;

    use pigment_core::job::{CancelToken, ProgressSink};
    use pigment_core::request::{RenderReport, TileSink};
    use pigment_core::seed::{TextDigest, Variation};
    use pigment_core::tiles::{DeviceTileLimits, Support, TileCostModel, TilePlan};

    use super::*;

    /// CPU stand-in: a flat image whose red channel is the request id.
    #[derive(Debug, Default)]
    pub(crate) struct FlatRenderer {
        pub(crate) calls: Arc<AtomicU32>,
    }

    impl Renderer for FlatRenderer {
        fn cost_model(&self) -> TileCostModel {
            TileCostModel {
                extended_bytes_per_px: 8,
                output_bytes_per_px: 4,
                staging_bytes_per_px: 4,
            }
        }

        fn supports(&self, _: &Appearance) -> Vec<Support> {
            Vec::new()
        }

        fn render(
            &self,
            req: &RenderRequest,
            cancel: &CancelToken,
            _: &mut dyn ProgressSink,
            sink: &mut dyn TileSink,
        ) -> Result<RenderReport, RenderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let plan = TilePlan::new(
                req.target.width,
                req.target.height,
                0,
                req.target.policy,
                DeviceTileLimits {
                    max_texture_dimension_2d: 8192,
                    max_buffer_size: 1 << 28,
                },
                self.cost_model(),
            )?;
            let report = |outcome| RenderReport {
                id: req.id,
                purpose: req.purpose,
                outcome,
                plan,
                timings: RenderTimings::default(),
                renderer_version: 0,
                device: "flat".into(),
                software_adapter: true,
            };
            if cancel.is_cancelled() {
                sink.abort();
                return Ok(report(RenderOutcome::Cancelled { tiles_done: 0 }));
            }
            sink.begin(plan.image_w, plan.image_h)?;
            let px = [req.id.0 as u8, 0, 0, 255];
            let band: Vec<u8> = px.repeat((plan.image_w * plan.image_h) as usize);
            sink.band(0, plan.image_h, &band)?;
            sink.finish()?;
            Ok(report(RenderOutcome::Completed))
        }
    }

    pub(crate) fn job(text: &str, w: u32, h: u32) -> PreviewJob {
        PreviewJob {
            biome: pigment_core::biome::BiomeId::Alpine,
            seeds: SeedBundle::derive(TextDigest::from_source(text).unwrap(), Variation(0)),
            form: FormSettings::default(),
            aspect: AspectRatio::of(16, 9),
            appearance: Appearance::default(),
            width: w,
            height: h,
            submitted_at: Instant::now(),
        }
    }

    fn wait_for(worker: &PreviewWorker, n: usize, timeout: Duration) -> Vec<PreviewResult> {
        let deadline = Instant::now() + timeout;
        let mut got = Vec::new();
        while got.len() < n && Instant::now() < deadline {
            got.extend(worker.drain());
            std::thread::sleep(Duration::from_millis(2));
        }
        got
    }

    #[test]
    fn renders_and_reuses_the_scene_for_paint_and_size_changes() {
        let w = PreviewWorker::spawn(FlatRenderer::default(), WorkerOptions::default(), || {});
        let mut j = job("Blue dusk.", 160, 90);
        w.submit(RequestId(1), j.clone());
        let r1 = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
        assert!(!r1.scene_reused);
        assert!(matches!(
            r1.outcome,
            PreviewOutcome::Image {
                width: 160,
                height: 90,
                ..
            }
        ));
        j.width = 320;
        j.height = 180;
        j.appearance.atmosphere.haze = 0.9;
        w.submit(RequestId(2), j.clone());
        let r2 = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
        assert!(r2.scene_reused, "paint settings and size keep the scene");
        j.form.relief = 0.9;
        w.submit(RequestId(3), j.clone());
        let r3 = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
        assert!(!r3.scene_reused, "form rebuilds it");
        j.biome = pigment_core::biome::BiomeId::Desert;
        w.submit(RequestId(4), j);
        let r4 = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
        assert!(!r4.scene_reused, "biome is a structural cache input");
        let s = w.stats();
        assert_eq!((s.scenes_built, s.scenes_reused), (3, 1));
    }

    #[test]
    fn a_burst_while_rendering_slowly_stays_bounded_and_ends_on_the_newest() {
        let calls = Arc::new(AtomicU32::new(0));
        let r = FlatRenderer {
            calls: calls.clone(),
        };
        let opts = WorkerOptions {
            delay: Duration::from_millis(150),
            ..Default::default()
        };
        let w = PreviewWorker::spawn(r, opts, || {});
        let mut submits = Vec::new();
        for i in 1..=200u64 {
            let payload = job("Typing fast", 64, 36);
            let t = Instant::now();
            w.submit(RequestId(i), payload);
            submits.push(t.elapsed());
            assert!(w.in_flight() <= 2);
            std::thread::sleep(Duration::from_millis(1));
        }
        // A submit that waited on the 150 ms render would make most of the
        // burst slow. One or two long ones are the test machine descheduling
        // this thread (seen once on a loaded macOS CI runner: 140 ms).
        submits.sort();
        let p95 = submits[submits.len() * 95 / 100];
        let slow = submits
            .iter()
            .filter(|d| **d > Duration::from_millis(50))
            .count();
        assert!(
            p95 < Duration::from_millis(20) && slow <= 2,
            "submit waited on the render: p95 {p95:?}, {slow} over 50 ms, worst {:?}",
            submits.last()
        );
        let mut results = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            results.extend(w.drain());
            if results.iter().any(|r| r.id == RequestId(200)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let last = results.iter().find(|r| r.id == RequestId(200)).unwrap();
        assert!(matches!(last.outcome, PreviewOutcome::Image { .. }));
        // 200 submissions, but only a handful of jobs ever started.
        let started = w.stats().started;
        assert!(started <= 5, "{started} jobs started");
        assert!(calls.load(Ordering::Relaxed) <= started);
    }

    #[test]
    fn shutdown_interrupts_a_slow_render_promptly() {
        let opts = WorkerOptions {
            delay: Duration::from_secs(30),
            ..Default::default()
        };
        let mut w = PreviewWorker::spawn(FlatRenderer::default(), opts, || {});
        w.submit(RequestId(1), job("slow", 64, 36));
        std::thread::sleep(Duration::from_millis(50));
        let t = Instant::now();
        assert!(w.shutdown(Duration::from_secs(2)));
        assert!(
            t.elapsed() < Duration::from_millis(500),
            "{:?}",
            t.elapsed()
        );
        assert!(w.drain().is_empty());
    }

    #[test]
    fn shutdown_does_not_hang_when_results_are_not_collected() {
        let mut w = PreviewWorker::spawn(FlatRenderer::default(), WorkerOptions::default(), || {});
        for i in 1..=20 {
            w.submit(RequestId(i), job("nobody drains", 64, 36));
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(w.shutdown(Duration::from_secs(2)));
    }

    #[test]
    fn simulated_device_loss_is_reported_for_every_later_job() {
        let opts = WorkerOptions {
            lose_device_after: Some(1),
            ..Default::default()
        };
        let w = PreviewWorker::spawn(FlatRenderer::default(), opts, || {});
        for i in 1..=3 {
            w.submit(RequestId(i), job("gpu reset", 64, 36));
            let r = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
            match (i, r.outcome) {
                (1, PreviewOutcome::Image { .. }) => {}
                (_, PreviewOutcome::Failed(RenderError::DeviceLost { .. })) if i > 1 => {}
                (_, o) => panic!("job {i}: {o:?}"),
            }
        }
    }

    #[test]
    fn scene_errors_are_failures_not_panics() {
        let w = PreviewWorker::spawn(FlatRenderer::default(), WorkerOptions::default(), || {});
        let mut j = job("bad form", 64, 36);
        j.form.relief = f64::NAN;
        w.submit(RequestId(1), j);
        let r = wait_for(&w, 1, Duration::from_secs(5)).pop().unwrap();
        assert!(matches!(
            r.outcome,
            PreviewOutcome::Failed(RenderError::InvalidRequest(_))
        ));
    }

    #[test]
    fn the_ui_is_woken_for_every_result() {
        let wakes = Arc::new(AtomicU32::new(0));
        let w = {
            let wakes = wakes.clone();
            PreviewWorker::spawn(
                FlatRenderer::default(),
                WorkerOptions::default(),
                move || {
                    wakes.fetch_add(1, Ordering::Relaxed);
                },
            )
        };
        w.submit(RequestId(1), job("wake", 64, 36));
        wait_for(&w, 1, Duration::from_secs(5));
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }
}
