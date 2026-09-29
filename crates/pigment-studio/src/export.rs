//! Exporting the painting from the studio (task 13): the size choice, the
//! immutable job snapshot and the export worker.
//!
//! - **One export at a time**, on its own thread with its own renderer on
//!   the shared device, so previews keep working and are never queued
//!   behind an export (docs/architecture.md, "Preview lifecycle").
//! - **Snapshot:** a job carries copies of the seeds, form, aspect ratio,
//!   appearance and frame taken when the user pressed Export. Later edits
//!   in the window cannot reach it. The prose is not part of it at all.
//! - **Backend:** `pigment_io::export_png` (task 09): tiles, bounded memory,
//!   an atomic rename into place, and on cancel or error the partial file
//!   is removed and an existing file is kept.
//! - **Progress** is counts reported by the renderer (tiles done of total,
//!   and the phase). No percentages of time and no ETA are invented.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pigment_core::error::{Problem, RenderError, SinkErrorKind, ValidationError};
use pigment_core::frame::{AspectRatio, Frame, MAX_EDGE, MIN_EDGE};
use pigment_core::job::{CancelToken, Phase, Progress};
use pigment_core::request::{
    RenderOutcome, RenderPurpose, RenderRequest, RenderTarget, Renderer, RequestId,
};
use pigment_core::seed::SeedBundle;
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::{TileOrder, TilePolicy};
use pigment_io::export::{LONG_EDGE_4K, LONG_EDGE_8K};
use pigment_io::{PngCompression, export_png};

/// The size the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeChoice {
    Uhd4k,
    Uhd8k,
    /// A custom long edge; the short edge follows the painting's exact
    /// proportions.
    Custom,
}

/// The export dialog's size settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeForm {
    pub choice: SizeChoice,
    /// Custom width and height as typed; [`SizeForm::frame`] snaps them to
    /// the exact aspect ratio.
    pub width: u32,
    pub height: u32,
}

impl Default for SizeForm {
    fn default() -> Self {
        SizeForm {
            choice: SizeChoice::Uhd8k,
            width: 5120,
            height: 2880,
        }
    }
}

impl SizeForm {
    /// Width typed: the height follows, snapped to the exact ratio.
    pub fn set_width(&mut self, aspect: AspectRatio, width: u32) {
        let k = (width / aspect.width).max(1);
        self.width = aspect.width.saturating_mul(k);
        self.height = aspect.height.saturating_mul(k);
    }

    /// Height typed: the width follows.
    pub fn set_height(&mut self, aspect: AspectRatio, height: u32) {
        let k = (height / aspect.height).max(1);
        self.width = aspect.width.saturating_mul(k);
        self.height = aspect.height.saturating_mul(k);
    }

    /// The exact pixel frame this choice exports for a painting of `aspect`,
    /// or why it cannot.
    pub fn frame(&self, aspect: AspectRatio) -> Result<Frame, ValidationError> {
        match self.choice {
            SizeChoice::Uhd4k => Frame::largest_with_aspect(aspect, LONG_EDGE_4K),
            SizeChoice::Uhd8k => Frame::largest_with_aspect(aspect, LONG_EDGE_8K),
            SizeChoice::Custom => {
                let f = Frame::new(self.width, self.height)?;
                if f.aspect() != aspect {
                    return Err(ValidationError {
                        field: "frame",
                        problem: Problem::AspectMismatch {
                            got: (f.aspect().width, f.aspect().height),
                            scene: (aspect.width, aspect.height),
                        },
                    });
                }
                Ok(f)
            }
        }
    }
}

/// A plain-language reason a size cannot be exported.
pub fn size_problem(e: &ValidationError) -> String {
    match e.problem {
        Problem::TooSmall { .. } => format!("Each side must be at least {MIN_EDGE} px."),
        Problem::TooLarge { .. } => format!(
            "Each side can be at most {MAX_EDGE} px (the largest size verified on hardware)."
        ),
        Problem::AspectTooExtreme { .. } => {
            "The long side can be at most 4 times the short side.".to_string()
        }
        Problem::AspectMismatch { scene, .. } => format!(
            "The size must keep the painting's {}:{} proportions.",
            scene.0, scene.1
        ),
        _ => e.to_string(),
    }
}

/// A file size for people: KB below a megabyte, MB above.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1_000_000 {
        format!("{} KB", bytes.div_ceil(1000))
    } else {
        format!("{:.1} MB", bytes as f64 / 1e6)
    }
}

/// File name suggested for an export: never derived from the prose.
pub fn suggested_name(recipe_path: Option<&Path>, frame: Frame) -> String {
    let stem = recipe_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .map(|n| {
            n.trim_end_matches(".json")
                .trim_end_matches(".recipe")
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "painting".to_string());
    format!("{stem}-{}x{}.png", frame.width, frame.height)
}

/// Everything one export needs, copied when the user pressed Export.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportJob {
    pub biome: pigment_core::biome::BiomeId,
    pub seeds: SeedBundle,
    pub form: FormSettings,
    pub aspect: AspectRatio,
    pub appearance: Appearance,
    pub frame: Frame,
    pub destination: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportOutcome {
    Written {
        destination: PathBuf,
        bytes: u64,
        width: u32,
        height: u32,
        tiles: u32,
        elapsed: Duration,
    },
    /// Cancelled; any partial file was removed and an existing file kept.
    Cancelled,
    Failed {
        destination: PathBuf,
        error: RenderError,
    },
}

/// What the window shows while an export runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub job: ExportJob,
    pub started: Instant,
    /// The newest progress report, if any yet.
    pub progress: Option<Progress>,
    pub cancelling: bool,
}

#[derive(Debug, Default)]
struct Shared {
    running: Option<Running>,
    finished: Option<ExportOutcome>,
}

/// The export already running refused a second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// The export thread. One job at a time.
#[derive(Debug)]
pub struct Exporter {
    jobs: Option<Sender<(RequestId, ExportJob, CancelToken)>>,
    shared: Arc<Mutex<Shared>>,
    cancel: Option<CancelToken>,
    next: u64,
    thread: Option<JoinHandle<()>>,
}

fn lock(m: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Exporter {
    /// `wake` is called on every progress report and when a job ends.
    pub fn spawn<R: Renderer + Send + 'static>(
        renderer: R,
        wake: impl Fn() + Send + 'static,
    ) -> Exporter {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("pigment-export".into())
                .spawn(move || run(renderer, &rx, &shared, &wake))
                .expect("spawn the export thread")
        };
        Exporter {
            jobs: Some(tx),
            shared,
            cancel: None,
            next: 1,
            thread: Some(thread),
        }
    }

    /// Starts `job`, or refuses while another export runs.
    pub fn start(&mut self, job: ExportJob) -> Result<(), Busy> {
        let mut s = lock(&self.shared);
        if s.running.is_some() {
            return Err(Busy);
        }
        let cancel = CancelToken::new();
        s.running = Some(Running {
            job: job.clone(),
            started: Instant::now(),
            progress: None,
            cancelling: false,
        });
        s.finished = None;
        drop(s);
        let id = RequestId(self.next);
        self.next += 1;
        self.cancel = Some(cancel.clone());
        if let Some(tx) = &self.jobs
            && tx.send((id, job, cancel)).is_ok()
        {
            return Ok(());
        }
        // The thread has gone: report it rather than hang.
        let mut s = lock(&self.shared);
        s.running = None;
        s.finished = Some(ExportOutcome::Failed {
            destination: PathBuf::new(),
            error: RenderError::Gpu {
                detail: "the export thread is not running".into(),
            },
        });
        Ok(())
    }

    /// The running export, if any.
    pub fn running(&self) -> Option<Running> {
        lock(&self.shared).running.clone()
    }

    pub fn is_running(&self) -> bool {
        lock(&self.shared).running.is_some()
    }

    /// Asks the running export to stop at the next tile.
    pub fn cancel(&self) {
        if let Some(c) = &self.cancel {
            c.cancel();
        }
        if let Some(r) = &mut lock(&self.shared).running {
            r.cancelling = true;
        }
    }

    /// The outcome of the last export, once.
    pub fn take_finished(&self) -> Option<ExportOutcome> {
        lock(&self.shared).finished.take()
    }

    /// Cancels any running export and waits up to `timeout` for the thread,
    /// so a partial file is cleaned up before the process exits.
    pub fn shutdown(&mut self, timeout: Duration) -> bool {
        self.cancel();
        self.jobs = None;
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

impl Drop for Exporter {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(5));
    }
}

fn run<R: Renderer>(
    renderer: R,
    jobs: &Receiver<(RequestId, ExportJob, CancelToken)>,
    shared: &Mutex<Shared>,
    wake: &dyn Fn(),
) {
    while let Ok((id, job, cancel)) = jobs.recv() {
        let started = Instant::now();
        let outcome = export(&renderer, id, &job, &cancel, &mut |p: Progress| {
            if let Some(r) = &mut lock(shared).running {
                r.progress = Some(p);
            }
            wake();
        });
        let outcome = match outcome {
            Ok((bytes, tiles, RenderOutcome::Completed)) => ExportOutcome::Written {
                destination: job.destination.clone(),
                bytes,
                width: job.frame.width,
                height: job.frame.height,
                tiles,
                elapsed: started.elapsed(),
            },
            Ok((_, _, RenderOutcome::Cancelled { .. })) => ExportOutcome::Cancelled,
            Err(error) => ExportOutcome::Failed {
                destination: job.destination.clone(),
                error,
            },
        };
        let mut s = lock(shared);
        s.running = None;
        s.finished = Some(outcome);
        drop(s);
        wake();
    }
}

/// Builds the scene from the snapshot and renders it to the destination.
fn export(
    renderer: &dyn Renderer,
    id: RequestId,
    job: &ExportJob,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(Progress),
) -> Result<(u64, u32, RenderOutcome), RenderError> {
    progress(Progress {
        id,
        phase: Phase::Scene,
        done: 0,
        total: 1,
    });
    let scene = pigment_core::scene::generator(job.biome)
        .generate(&job.seeds, &job.form, job.aspect)
        .map_err(RenderError::InvalidRequest)?;
    let request = RenderRequest {
        id,
        purpose: RenderPurpose::Export,
        scene: Arc::new(scene),
        seeds: job.seeds,
        appearance: job.appearance,
        target: RenderTarget {
            width: job.frame.width,
            height: job.frame.height,
            policy: TilePolicy::default_export(),
            order: TileOrder::RowMajor,
        },
    };
    if cancel.is_cancelled() {
        return Ok((0, 0, RenderOutcome::Cancelled { tiles_done: 0 }));
    }
    let report = export_png(
        renderer,
        &request,
        &job.destination,
        PngCompression::Fast,
        cancel,
        &mut |p: Progress| progress(p),
    )?;
    let tiles = report.render.plan.tiles().count() as u32;
    Ok((report.bytes, tiles, report.render.outcome))
}

/// What to tell the user when an export fails: the problem and what to do.
pub fn failure_message(e: &RenderError, destination: &Path) -> String {
    let name = crate::files::display_name(destination);
    match e {
        RenderError::Sink(s) => match s.kind {
            SinkErrorKind::DiskFull => format!(
                "Could not write {name}: the disk is full. Free some space or choose another \
                 folder. Any existing file is unchanged."
            ),
            _ => format!(
                "Could not write {name}: {}. Choose another folder or check its permissions. \
                 Any existing file is unchanged.",
                s.detail
            ),
        },
        RenderError::OutOfMemory { .. } => format!(
            "The GPU ran out of memory for {name}, even with the smallest tiles. Close other \
             programs that use the GPU, or export a smaller size."
        ),
        RenderError::InvalidRequest(v) => format!("{name} cannot be exported: {}", size_problem(v)),
        RenderError::DeviceLost { .. } => format!(
            "The GPU was reset during the export of {name}. Save your recipe and restart \
             Pigment Prose."
        ),
        other => format!("The export of {name} failed: {other}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Condvar;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pigment_core::error::SinkError;
    use pigment_core::job::ProgressSink;
    use pigment_core::request::{RenderReport, RenderTimings, TileSink};
    use pigment_core::seed::{TextDigest, Variation};
    use pigment_core::tiles::{DeviceTileLimits, Support, TileCostModel, TilePlan};

    use super::*;

    /// A CPU renderer that paints tile by tile and can be held at a tile
    /// until the test releases it, or made to fail. It records the requests
    /// it was given.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct GateRenderer {
        pub(crate) gate: Arc<(Mutex<u32>, Condvar)>,
        /// Tiles allowed to run before blocking; `u32::MAX` = never block.
        pub(crate) seen: Arc<Mutex<Vec<RenderRequest>>>,
        pub(crate) tiles_rendered: Arc<AtomicU32>,
        pub(crate) fail_with: Option<RenderError>,
    }

    impl GateRenderer {
        pub(crate) fn open() -> GateRenderer {
            let g = GateRenderer::default();
            *g.gate.0.lock().unwrap() = u32::MAX;
            g
        }
        /// Lets `n` more tiles through.
        pub(crate) fn release(&self, n: u32) {
            let (m, cv) = &*self.gate;
            let mut left = m.lock().unwrap();
            *left = left.saturating_add(n);
            cv.notify_all();
        }
    }

    impl Renderer for GateRenderer {
        fn cost_model(&self) -> TileCostModel {
            // Big enough that a modest export needs several tiles.
            TileCostModel {
                extended_bytes_per_px: 64,
                output_bytes_per_px: 32,
                staging_bytes_per_px: 32,
            }
        }

        fn supports(&self, _: &Appearance) -> Vec<Support> {
            Vec::new()
        }

        fn render(
            &self,
            req: &RenderRequest,
            cancel: &CancelToken,
            progress: &mut dyn ProgressSink,
            sink: &mut dyn TileSink,
        ) -> Result<RenderReport, RenderError> {
            self.seen.lock().unwrap().push(req.clone());
            if let Some(e) = &self.fail_with {
                return Err(e.clone());
            }
            let policy = match req.target.policy {
                // Small tiles so there are several to cancel between.
                TilePolicy::Budget { host_bytes, .. } => TilePolicy::Budget {
                    gpu_bytes: 256 * 256 * 128,
                    host_bytes,
                },
                p => p,
            };
            let plan = TilePlan::new(
                req.target.width,
                req.target.height,
                0,
                policy,
                DeviceTileLimits {
                    max_texture_dimension_2d: 8192,
                    max_buffer_size: 1 << 30,
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
                device: "gate".into(),
                software_adapter: true,
            };
            sink.begin(plan.image_w, plan.image_h)?;
            let total = plan.tiles().count() as u32;
            let w = plan.image_w as usize;
            let mut band_y = 0;
            let mut band: Vec<u8> = Vec::new();
            for (i, tile) in plan.tiles().enumerate() {
                {
                    let (m, cv) = &*self.gate;
                    let mut left = m.lock().unwrap();
                    while *left == 0 && !cancel.is_cancelled() {
                        left = cv.wait_timeout(left, Duration::from_millis(5)).unwrap().0;
                    }
                    if *left != u32::MAX && *left > 0 {
                        *left -= 1;
                    }
                }
                if cancel.is_cancelled() {
                    sink.abort();
                    return Ok(report(RenderOutcome::Cancelled {
                        tiles_done: i as u32,
                    }));
                }
                if tile.y != band_y || band.is_empty() {
                    band_y = tile.y;
                    band = vec![0; w * tile.h as usize * 4];
                }
                for y in 0..tile.h as usize {
                    for x in tile.x as usize..(tile.x + tile.w) as usize {
                        let i = (y * w + x) * 4;
                        let v = (req.appearance.painting.edge_looseness * 255.0) as u8;
                        band[i..i + 4].copy_from_slice(&[v, (x % 256) as u8, (y % 256) as u8, 255]);
                    }
                }
                let done = i as u32 + 1;
                self.tiles_rendered.fetch_add(1, Ordering::Relaxed);
                progress.report(Progress {
                    id: req.id,
                    phase: Phase::Tiles,
                    done,
                    total,
                });
                if tile.x + tile.w == plan.image_w {
                    sink.band(band_y, tile.h, &band)?;
                    band.clear();
                }
            }
            progress.report(Progress {
                id: req.id,
                phase: Phase::Finalize,
                done: 0,
                total: 1,
            });
            sink.finish()?;
            Ok(report(RenderOutcome::Completed))
        }
    }

    pub(crate) fn job(dest: &Path, frame: Frame) -> ExportJob {
        ExportJob {
            biome: pigment_core::biome::BiomeId::Alpine,
            seeds: SeedBundle::derive(
                TextDigest::from_source("Export test.").unwrap(),
                Variation(0),
            ),
            form: FormSettings::default(),
            aspect: frame.aspect(),
            appearance: Appearance::default(),
            frame,
            destination: dest.to_path_buf(),
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pigment-export-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait(e: &Exporter) -> ExportOutcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(o) = e.take_finished() {
                return o;
            }
            assert!(Instant::now() < deadline, "export did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn sizes_keep_the_paintings_exact_proportions() {
        let wide = AspectRatio::of(16, 9);
        let mut f = SizeForm {
            choice: SizeChoice::Uhd4k,
            ..Default::default()
        };
        assert_eq!(
            f.frame(wide).unwrap(),
            Frame {
                width: 3840,
                height: 2160
            }
        );
        f.choice = SizeChoice::Uhd8k;
        assert_eq!(
            f.frame(AspectRatio::of(4, 5)).unwrap(),
            Frame {
                width: 6144,
                height: 7680
            }
        );
        assert_eq!(
            f.frame(AspectRatio::of(1, 1)).unwrap(),
            Frame {
                width: 7680,
                height: 7680
            }
        );
        f.choice = SizeChoice::Custom;
        f.set_width(wide, 5000);
        assert_eq!((f.width, f.height), (4992, 2808), "snapped to 16:9");
        assert!(f.frame(wide).is_ok());
        f.set_height(AspectRatio::of(9, 16), 3000);
        assert_eq!((f.width, f.height), (1683, 2992));
        // Out of bounds, and a hand-made mismatch, are explained.
        f.set_width(wide, 16);
        let e = f.frame(wide).unwrap_err();
        assert!(size_problem(&e).contains("at least 64"));
        f.set_width(wide, 20000);
        assert!(size_problem(&f.frame(wide).unwrap_err()).contains("at most 16384"));
        f.width = 1000;
        f.height = 1000;
        assert!(size_problem(&f.frame(wide).unwrap_err()).contains("16:9"));
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(format_bytes(0), "0 KB");
        assert_eq!(format_bytes(1), "1 KB");
        assert_eq!(format_bytes(412_345), "413 KB");
        assert_eq!(format_bytes(22_100_000), "22.1 MB");
    }

    #[test]
    fn suggested_names_never_come_from_the_prose() {
        let frame = Frame {
            width: 7680,
            height: 4320,
        };
        assert_eq!(suggested_name(None, frame), "painting-7680x4320.png");
        assert_eq!(
            suggested_name(Some(Path::new("/a/lake.recipe.json")), frame),
            "lake-7680x4320.png"
        );
        assert_eq!(
            suggested_name(Some(Path::new("/a/x.json")), frame),
            "x-7680x4320.png"
        );
    }

    #[test]
    fn exports_a_snapshot_that_later_edits_cannot_reach() {
        let dir = temp("snapshot");
        let dest = dir.join("out.png");
        let r = GateRenderer::default();
        let mut e = Exporter::spawn(r.clone(), || {});
        let frame = Frame {
            width: 1600,
            height: 900,
        };
        let mut j = job(&dest, frame);
        j.appearance.painting.edge_looseness = 0.2;
        e.start(j.clone()).unwrap();
        // One export at a time.
        assert_eq!(e.start(job(&dir.join("second.png"), frame)), Err(Busy));
        // The caller's copy changes while the export runs; the job does not.
        j.appearance.painting.edge_looseness = 0.9;
        r.release(u32::MAX);
        match wait(&e) {
            ExportOutcome::Written {
                bytes,
                width,
                height,
                tiles,
                ..
            } => {
                assert!(bytes > 0 && tiles > 1);
                assert_eq!((width, height), (1600, 900));
            }
            o => panic!("{o:?}"),
        }
        let seen = r.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].appearance.painting.edge_looseness, 0.2);
        assert_eq!((seen[0].target.width, seen[0].target.height), (1600, 900));
        assert_eq!(seen[0].purpose, RenderPurpose::Export);
        drop(seen);
        // The file decodes at the exported size, with red = looseness 0.2.
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&dest).unwrap()));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (1600, 900));
        assert_eq!(buf[0], (0.2f64 * 255.0) as u8);
        assert_eq!(files_in(&dir), ["out.png"], "no temporary file left");
        // And the next export may start.
        assert!(e.start(job(&dir.join("next.png"), frame)).is_ok());
        r.release(u32::MAX);
        wait(&e);
    }

    #[test]
    fn cancelling_mid_render_removes_the_partial_and_keeps_the_old_file() {
        let dir = temp("cancel");
        let dest = dir.join("keep.png");
        std::fs::write(&dest, b"the old export").unwrap();
        let r = GateRenderer::default();
        let mut e = Exporter::spawn(r.clone(), || {});
        e.start(job(
            &dest,
            Frame {
                width: 1600,
                height: 900,
            },
        ))
        .unwrap();
        r.release(2);
        let deadline = Instant::now() + Duration::from_secs(5);
        while r.tiles_rendered.load(Ordering::Relaxed) < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let running = e.running().unwrap();
        assert_eq!(
            running.progress.map(|p| (p.phase, p.done)),
            Some((Phase::Tiles, 2))
        );
        e.cancel();
        assert!(e.running().unwrap().cancelling);
        assert_eq!(wait(&e), ExportOutcome::Cancelled);
        assert_eq!(std::fs::read(&dest).unwrap(), b"the old export");
        assert_eq!(files_in(&dir), ["keep.png"]);
        assert!(!e.is_running());
    }

    #[test]
    fn failures_are_reported_with_what_to_do_and_leave_nothing_behind() {
        let dir = temp("fail");
        // A destination directory that does not exist: fails before render.
        let mut e = Exporter::spawn(GateRenderer::open(), || {});
        let frame = Frame {
            width: 640,
            height: 360,
        };
        let missing = dir.join("no/such/dir/x.png");
        e.start(job(&missing, frame)).unwrap();
        match wait(&e) {
            ExportOutcome::Failed {
                error: err,
                destination,
            } => {
                assert_eq!(destination, missing);
                let m = failure_message(&err, &missing);
                assert!(m.contains("x.png") && m.contains("another folder"), "{m}");
            }
            o => panic!("{o:?}"),
        }
        // A full disk while writing.
        let full = GateRenderer {
            fail_with: Some(RenderError::Sink(SinkError {
                kind: SinkErrorKind::DiskFull,
                detail: "no space".into(),
            })),
            ..GateRenderer::open()
        };
        let mut e = Exporter::spawn(full, || {});
        let dest = dir.join("full.png");
        std::fs::write(&dest, b"old").unwrap();
        e.start(job(&dest, frame)).unwrap();
        match wait(&e) {
            ExportOutcome::Failed { error: err, .. } => {
                assert!(failure_message(&err, &dest).contains("disk is full"));
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert_eq!(files_in(&dir), ["full.png"]);
        // An invalid size never reaches the renderer.
        let r = GateRenderer::open();
        let mut e = Exporter::spawn(r.clone(), || {});
        let mut bad = job(&dir.join("bad.png"), frame);
        bad.frame = Frame {
            width: 641,
            height: 360,
        };
        e.start(bad).unwrap();
        assert!(matches!(
            wait(&e),
            ExportOutcome::Failed {
                error: RenderError::InvalidRequest(_),
                ..
            }
        ));
        assert!(r.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn shutdown_cancels_a_running_export_and_cleans_up() {
        let dir = temp("shutdown");
        let dest = dir.join("closing.png");
        let r = GateRenderer::default();
        let mut e = Exporter::spawn(r.clone(), || {});
        e.start(job(
            &dest,
            Frame {
                width: 1600,
                height: 900,
            },
        ))
        .unwrap();
        r.release(1);
        std::thread::sleep(Duration::from_millis(30));
        let t = Instant::now();
        assert!(e.shutdown(Duration::from_secs(5)));
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
    }

    #[test]
    fn the_worker_wakes_the_ui_for_progress_and_the_end() {
        let wakes = Arc::new(AtomicU32::new(0));
        let w = wakes.clone();
        let mut e = Exporter::spawn(GateRenderer::open(), move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        let dir = temp("wake");
        e.start(job(
            &dir.join("w.png"),
            Frame {
                width: 640,
                height: 360,
            },
        ))
        .unwrap();
        wait(&e);
        assert!(wakes.load(Ordering::Relaxed) >= 3, "scene, tiles, end");
    }
}
