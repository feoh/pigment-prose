//! Preparing the painter off the UI thread (task 15).
//!
//! Compiling the painting shader is fast once the driver has cached it, but
//! the first time (after installing, or after a driver update) it is not:
//! measured on this machine with the driver's cache empty, 3.6 s on the
//! NVIDIA driver and 34 s (with a 4.2 GB peak) on Mesa's Intel driver.
//! Built in the window's start-up, that kept the window from appearing at
//! all. Now [`Preparation::start`] compiles once on its own thread while
//! the window opens and says so, and every [`Deferred`] renderer (previews,
//! exports) takes its own renderer sharing those pipelines the first time
//! it is asked to paint.

use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use pigment_core::error::RenderError;
use pigment_core::job::{CancelToken, ProgressSink};
use pigment_core::request::{RenderReport, RenderRequest, Renderer, TileSink};
use pigment_core::settings::Appearance;
use pigment_core::tiles::{Support, TileCostModel};
use pigment_gpu::PaintRenderer;

/// A renderer that can hand out another one sharing its compiled pipelines.
pub trait Share: Renderer + Send + 'static {
    fn share(&self) -> Self;
}

impl Share for PaintRenderer {
    fn share(&self) -> PaintRenderer {
        self.sibling()
    }
}

/// Where preparation stands, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Readiness {
    Preparing,
    Ready,
    Failed(String),
}

#[derive(Debug)]
pub struct Preparation<R> {
    slot: Mutex<Option<Result<R, RenderError>>>,
    done: Condvar,
}

impl<R: Share> Preparation<R> {
    /// Runs `make` on a new thread and calls `wake` when it is done.
    pub fn start(
        make: impl FnOnce() -> Result<R, RenderError> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Arc<Preparation<R>> {
        let prep = Arc::new(Preparation {
            slot: Mutex::new(None),
            done: Condvar::new(),
        });
        let p = prep.clone();
        std::thread::Builder::new()
            .name("pigment-prepare".into())
            .spawn(move || {
                let t0 = Instant::now();
                let made = make();
                match &made {
                    Ok(_) => eprintln!(
                        "pigment-studio: painter ready in {:.0} ms",
                        t0.elapsed().as_secs_f64() * 1e3
                    ),
                    Err(e) => eprintln!("pigment-studio: the painter could not be prepared: {e}"),
                }
                *p.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(made);
                p.done.notify_all();
                wake();
            })
            .expect("spawn the preparation thread");
        prep
    }

    pub fn readiness(&self) -> Readiness {
        match &*self.slot.lock().unwrap_or_else(|e| e.into_inner()) {
            None => Readiness::Preparing,
            Some(Ok(_)) => Readiness::Ready,
            Some(Err(e)) => Readiness::Failed(e.to_string()),
        }
    }

    /// A renderer of its own, once preparation is done. Blocks until then.
    fn wait(&self) -> Result<R, RenderError> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        while slot.is_none() {
            slot = self.done.wait(slot).unwrap_or_else(|e| e.into_inner());
        }
        match slot.as_ref() {
            Some(Ok(r)) => Ok(r.share()),
            Some(Err(e)) => Err(e.clone()),
            None => unreachable!("waited until set"),
        }
    }

    /// A renderer for one worker thread. It waits for preparation the first
    /// time it is used (on that thread, never the UI's).
    pub fn renderer(self: &Arc<Self>) -> Deferred<R> {
        Deferred {
            prep: self.clone(),
            own: OnceLock::new(),
        }
    }
}

#[derive(Debug)]
pub struct Deferred<R> {
    prep: Arc<Preparation<R>>,
    own: OnceLock<Result<R, RenderError>>,
}

impl<R: Share> Deferred<R> {
    fn get(&self) -> &Result<R, RenderError> {
        self.own.get_or_init(|| self.prep.wait())
    }
}

impl<R: Share> Renderer for Deferred<R> {
    fn cost_model(&self) -> TileCostModel {
        match self.get() {
            Ok(r) => r.cost_model(),
            // Only reached before a render that fails with the same error.
            Err(_) => TileCostModel {
                extended_bytes_per_px: 8,
                output_bytes_per_px: 4,
                staging_bytes_per_px: 4,
            },
        }
    }

    fn supports(&self, appearance: &Appearance) -> Vec<Support> {
        match self.get() {
            Ok(r) => r.supports(appearance),
            Err(_) => Vec::new(),
        }
    }

    fn render(
        &self,
        request: &RenderRequest,
        cancel: &CancelToken,
        progress: &mut dyn ProgressSink,
        sink: &mut dyn TileSink,
    ) -> Result<RenderReport, RenderError> {
        match self.get() {
            Ok(r) => r.render(request, cancel, progress, sink),
            Err(e) => {
                sink.abort();
                Err(e.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use pigment_core::request::RequestIds;

    use super::*;
    use crate::worker::tests::{FlatRenderer, job};
    use crate::worker::{PreviewOutcome, PreviewWorker, WorkerOptions};

    impl Share for FlatRenderer {
        fn share(&self) -> FlatRenderer {
            FlatRenderer {
                calls: self.calls.clone(),
            }
        }
    }

    fn wait_for(worker: &PreviewWorker) -> PreviewOutcome {
        for _ in 0..500 {
            if let Some(r) = worker.drain().pop() {
                return r.outcome;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no result");
    }

    #[test]
    fn workers_start_before_the_painter_is_ready_and_share_one_build() {
        let (release, gate) = mpsc::channel::<()>();
        let builds = Arc::new(AtomicU32::new(0));
        let b = builds.clone();
        let prep = Preparation::start(
            move || {
                gate.recv().unwrap();
                b.fetch_add(1, Ordering::Relaxed);
                Ok(FlatRenderer::default())
            },
            || {},
        );
        // The window's side: nothing blocks while it prepares.
        assert_eq!(prep.readiness(), Readiness::Preparing);
        let previews = PreviewWorker::spawn(prep.renderer(), WorkerOptions::default(), || {});
        let exports = prep.renderer();
        let ids = RequestIds::default();
        previews.submit(ids.next(), job("waiting", 64, 36));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            previews.drain().is_empty(),
            "painted before the painter was ready"
        );
        assert_eq!(prep.readiness(), Readiness::Preparing);

        release.send(()).unwrap();
        assert!(matches!(wait_for(&previews), PreviewOutcome::Image { .. }));
        assert_eq!(prep.readiness(), Readiness::Ready);
        // A second worker's renderer comes from the same build.
        assert!(exports.get().is_ok());
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_failed_preparation_fails_every_render_and_says_why() {
        let prep = Preparation::<FlatRenderer>::start(
            || {
                Err(RenderError::Gpu {
                    detail: "shader compilation failed".into(),
                })
            },
            || {},
        );
        let previews = PreviewWorker::spawn(prep.renderer(), WorkerOptions::default(), || {});
        previews.submit(RequestIds::default().next(), job("fails", 64, 36));
        match wait_for(&previews) {
            PreviewOutcome::Failed(RenderError::Gpu { detail }) => {
                assert!(detail.contains("shader compilation"));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(prep.readiness(), Readiness::Failed(m) if m.contains("shader compilation"))
        );
    }
}
