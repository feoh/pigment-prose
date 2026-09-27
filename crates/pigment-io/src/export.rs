//! The export job (task 09): validate the size, render tile by tile into a
//! [`PngSink`], and retry out-of-memory failures with half the GPU budget.
//!
//! The scene is built once by the caller and shared through the request's
//! `Arc<Scene>`; retries reuse it. Tile size and order never change the
//! pixels (tiles are evaluated in whole-image coordinates with a recomputed
//! apron; see `pigment_core::tiles`), so a retry with smaller tiles produces
//! the same image.

use std::path::{Path, PathBuf};

use pigment_core::error::{Problem, RenderError, SinkError, SinkErrorKind, ValidationError};
use pigment_core::frame::{AspectRatio, Frame};
use pigment_core::job::{CancelToken, ProgressSink};
use pigment_core::request::{RenderOutcome, RenderReport, RenderRequest, Renderer, TileSink};
use pigment_core::scene::Scene;
use pigment_core::tiles::{DeviceTileLimits, TilePlan, TilePolicy, apron_pixels};

use crate::png_sink::{PngCompression, PngSink};

/// Long edge of the "4K" preset (3840×2160 at 16:9).
pub const LONG_EDGE_4K: u32 = 3840;
/// Long edge of the "8K" preset (7680×4320 at 16:9).
pub const LONG_EDGE_8K: u32 = 7680;

/// Requested export size. Presets keep the scene's exact aspect ratio;
/// custom sizes must match it (a different aspect ratio is a different
/// scene, chosen by the document, not by the export).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSize {
    /// Largest exact-aspect frame with a long edge of at most 3840 px.
    Uhd4k,
    /// Largest exact-aspect frame with a long edge of at most 7680 px.
    Uhd8k,
    Custom {
        width: u32,
        height: u32,
    },
}

impl ExportSize {
    pub fn frame(self, scene_aspect: AspectRatio) -> Result<Frame, ValidationError> {
        match self {
            ExportSize::Uhd4k => Frame::largest_with_aspect(scene_aspect, LONG_EDGE_4K),
            ExportSize::Uhd8k => Frame::largest_with_aspect(scene_aspect, LONG_EDGE_8K),
            ExportSize::Custom { width, height } => {
                let f = Frame::new(width, height)?;
                check_aspect(f, scene_aspect)?;
                Ok(f)
            }
        }
    }
}

fn check_aspect(f: Frame, scene: AspectRatio) -> Result<(), ValidationError> {
    let got = f.aspect();
    if got != scene {
        return Err(ValidationError {
            field: "frame",
            problem: Problem::AspectMismatch {
                got: (got.width, got.height),
                scene: (scene.width, scene.height),
            },
        });
    }
    Ok(())
}

/// Checks an export size against the frame bounds (`frame::MIN_EDGE`..
/// `MAX_EDGE`, at most 4:1), the scene's exact aspect ratio and the buffer
/// arithmetic of this platform, before anything is allocated.
///
/// The bounds are what has been validated, not what is imaginable: 16384 px
/// per edge is the largest size measured on hardware (task 09 evidence).
/// Memory never limits the size, because tiles and bands are bounded by the
/// budgets in [`TilePolicy::Budget`].
pub fn validate_target(scene: &Scene, width: u32, height: u32) -> Result<Frame, ValidationError> {
    let f = Frame::new(width, height)?;
    check_aspect(f, scene.key().aspect)?;
    // Rows (RGBA and RGB) and one full-height band must be addressable.
    let overflow = ValidationError {
        field: "frame",
        problem: Problem::Overflow,
    };
    (width as usize)
        .checked_mul(4)
        .and_then(|row| row.checked_mul(height as usize))
        .ok_or(overflow)?;
    Ok(f)
}

/// One render attempt of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    pub policy: TilePolicy,
    /// `Some(stage)` if this attempt ran out of GPU memory.
    pub out_of_memory: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    /// The final attempt's report. `plan.gpu_bytes` and `plan.host_band_bytes`
    /// are the cost model's **estimates**, not measurements.
    pub render: RenderReport,
    pub destination: PathBuf,
    /// Size of the written file; 0 when cancelled.
    pub bytes: u64,
    /// Every attempt in order; the last produced `render`.
    pub attempts: Vec<Attempt>,
}

/// Whether some tile edge fits `policy`'s budgets for this request,
/// ignoring device limits (the renderer checks those).
fn budget_can_plan(renderer: &dyn Renderer, request: &RenderRequest, policy: TilePolicy) -> bool {
    let apron = apron_pixels(&request.mapping(), &renderer.supports(&request.appearance));
    let unbounded = DeviceTileLimits {
        max_texture_dimension_2d: u32::MAX,
        max_buffer_size: u64::MAX,
    };
    TilePlan::new(
        request.target.width,
        request.target.height,
        apron,
        policy,
        unbounded,
        renderer.cost_model(),
    )
    .is_ok()
}

/// Render `request` to a PNG at `destination`.
///
/// - The size is validated first ([`validate_target`]); nothing is written
///   for an invalid request, and an unwritable destination fails before any
///   GPU work.
/// - On [`RenderError::OutOfMemory`] under a `Budget` policy, retries with
///   half the GPU budget until no tile edge (down to 256 px) fits, then
///   returns the error. It never lowers the resolution.
/// - Cancellation returns `Ok` with `RenderOutcome::Cancelled`. On
///   cancellation or error the partial file is deleted and an existing file
///   at `destination` is left as it was.
pub fn export_png(
    renderer: &dyn Renderer,
    request: &RenderRequest,
    destination: &Path,
    compression: PngCompression,
    cancel: &CancelToken,
    progress: &mut dyn ProgressSink,
) -> Result<ExportReport, RenderError> {
    validate_target(&request.scene, request.target.width, request.target.height)
        .map_err(RenderError::InvalidRequest)?;
    let mut attempts = Vec::new();
    let mut req = request.clone();
    loop {
        let mut sink = PngSink::with_compression(destination, compression)?;
        let result = renderer.render(&req, cancel, progress, &mut sink);
        let policy = req.target.policy;
        match result {
            Ok(render) => {
                attempts.push(Attempt {
                    policy,
                    out_of_memory: None,
                });
                let bytes = match render.outcome {
                    RenderOutcome::Completed if sink.is_finished() => sink.bytes_written(),
                    RenderOutcome::Completed => {
                        sink.abort();
                        return Err(RenderError::Sink(SinkError {
                            kind: SinkErrorKind::Other,
                            detail: "the renderer completed without finishing the file".into(),
                        }));
                    }
                    RenderOutcome::Cancelled { .. } => {
                        sink.abort();
                        0
                    }
                };
                return Ok(ExportReport {
                    render,
                    destination: destination.to_path_buf(),
                    bytes,
                    attempts,
                });
            }
            Err(RenderError::OutOfMemory { stage }) => {
                // Renderers may fail before touching the sink; clean up here.
                sink.abort();
                attempts.push(Attempt {
                    policy,
                    out_of_memory: Some(stage),
                });
                let TilePolicy::Budget {
                    gpu_bytes,
                    host_bytes,
                } = policy
                else {
                    return Err(RenderError::OutOfMemory { stage });
                };
                let smaller = TilePolicy::Budget {
                    gpu_bytes: gpu_bytes / 2,
                    host_bytes,
                };
                if cancel.is_cancelled() || !budget_can_plan(renderer, &req, smaller) {
                    return Err(RenderError::OutOfMemory { stage });
                }
                req.target.policy = smaller;
            }
            Err(e) => {
                sink.abort();
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    use pigment_core::job::{NoProgress, Phase, Progress};
    use pigment_core::request::{RenderPurpose, RenderTarget, RenderTimings, RequestId};
    use pigment_core::scene::{SceneGenerator, TestCard, diagnostic_seeds};
    use pigment_core::settings::{Appearance, FormSettings};
    use pigment_core::tiles::{
        DEFAULT_HOST_BAND_BUDGET, Support, TileCostModel, TileOrder, TilePlanError,
    };

    use super::*;
    use crate::test_dir::TestDir;

    /// A CPU stand-in for the GPU renderer that follows the `Renderer`
    /// contract: paints a whole-image-coordinate pattern tile by tile, and
    /// simulates allocation failure above a GPU budget, returning before it
    /// touches the sink (as `PaintRenderer` does).
    struct FakeRenderer {
        oom_above: u64,
        cancel_after: Option<u32>,
        calls: RefCell<Vec<TilePolicy>>,
        sink_calls: Cell<u32>,
    }

    impl FakeRenderer {
        fn new(oom_above: u64) -> FakeRenderer {
            FakeRenderer {
                oom_above,
                cancel_after: None,
                calls: RefCell::new(Vec::new()),
                sink_calls: Cell::new(0),
            }
        }
    }

    fn pixel(x: u32, y: u32) -> [u8; 4] {
        [
            (x % 251) as u8,
            (y % 241) as u8,
            ((x * 7 + y * 3) % 256) as u8,
            255,
        ]
    }

    impl Renderer for FakeRenderer {
        fn cost_model(&self) -> TileCostModel {
            TileCostModel {
                extended_bytes_per_px: 8,
                output_bytes_per_px: 4,
                staging_bytes_per_px: 4,
            }
        }

        fn supports(&self, _: &Appearance) -> Vec<Support> {
            vec![Support {
                pass: "fake",
                radius: 0.01,
            }]
        }

        fn render(
            &self,
            req: &RenderRequest,
            cancel: &CancelToken,
            progress: &mut dyn ProgressSink,
            sink: &mut dyn TileSink,
        ) -> Result<RenderReport, RenderError> {
            self.calls.borrow_mut().push(req.target.policy);
            let apron = apron_pixels(&req.mapping(), &self.supports(&req.appearance));
            let limits = DeviceTileLimits {
                max_texture_dimension_2d: 8192,
                max_buffer_size: 256 << 20,
            };
            let plan = TilePlan::new(
                req.target.width,
                req.target.height,
                apron,
                req.target.policy,
                limits,
                self.cost_model(),
            )?;
            if plan.gpu_bytes > self.oom_above {
                return Err(RenderError::OutOfMemory {
                    stage: "tile resource allocation",
                });
            }
            let report = |outcome| RenderReport {
                id: req.id,
                purpose: req.purpose,
                outcome,
                plan,
                timings: RenderTimings::default(),
                renderer_version: 0,
                device: "fake".into(),
                software_adapter: true,
            };
            self.sink_calls.set(self.sink_calls.get() + 1);
            sink.begin(plan.image_w, plan.image_h)?;
            let row = plan.image_w as usize * 4;
            let mut band = vec![0u8; row * plan.tile_h as usize];
            let mut in_band = 0;
            for (i, t) in plan.tiles_in(req.target.order).enumerate() {
                let done = i as u32;
                if cancel.is_cancelled() || self.cancel_after == Some(done) {
                    sink.abort();
                    return Ok(report(RenderOutcome::Cancelled { tiles_done: done }));
                }
                for y in 0..t.h {
                    for x in 0..t.w {
                        let d = y as usize * row + (t.x + x) as usize * 4;
                        band[d..d + 4].copy_from_slice(&pixel(t.x + x, t.y + y));
                    }
                }
                progress.report(Progress {
                    id: req.id,
                    phase: Phase::Tiles,
                    done: done + 1,
                    total: plan.len(),
                });
                in_band += 1;
                if in_band == plan.cols {
                    in_band = 0;
                    if let Err(e) = sink.band(t.y, t.h, &band[..t.h as usize * row]) {
                        sink.abort();
                        return Err(e.into());
                    }
                }
            }
            if let Err(e) = sink.finish() {
                sink.abort();
                return Err(e.into());
            }
            Ok(report(RenderOutcome::Completed))
        }
    }

    fn request(w: u32, h: u32, policy: TilePolicy) -> RenderRequest {
        let seeds = diagnostic_seeds(3);
        let scene = TestCard
            .generate(
                &seeds,
                &FormSettings::default(),
                Frame {
                    width: w,
                    height: h,
                }
                .aspect(),
            )
            .unwrap();
        RenderRequest {
            id: RequestId(1),
            purpose: RenderPurpose::Export,
            scene: Arc::new(scene),
            seeds,
            appearance: Appearance::default(),
            target: RenderTarget {
                width: w,
                height: h,
                policy,
                order: TileOrder::RowMajor,
            },
        }
    }

    fn decode_rgb(path: &Path) -> (u32, u32, Vec<u8>) {
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    }

    #[test]
    fn export_writes_the_whole_image_in_any_tile_order() {
        let dir = TestDir::new("export");
        let mut images = Vec::new();
        for (edge, order) in [
            (256, TileOrder::RowMajor),
            (333, TileOrder::ReverseInBand),
            (4096, TileOrder::RowMajor),
        ] {
            let dest = dir.path().join(format!("t{edge}.png"));
            let mut req = request(1600, 900, TilePolicy::Fixed { edge });
            req.target.order = order;
            let rep = export_png(
                &FakeRenderer::new(u64::MAX),
                &req,
                &dest,
                PngCompression::Fast,
                &CancelToken::new(),
                &mut NoProgress,
            )
            .unwrap();
            assert_eq!(rep.render.outcome, RenderOutcome::Completed);
            assert_eq!(rep.bytes, std::fs::metadata(&dest).unwrap().len());
            let (w, h, rgb) = decode_rgb(&dest);
            assert_eq!((w, h), (1600, 900));
            images.push(rgb);
        }
        assert!(images[0] == images[1] && images[1] == images[2]);
        let (x, y) = (1599u32, 899u32);
        let p = &images[0][((y * 1600 + x) * 3) as usize..][..3];
        assert_eq!(p, &pixel(x, y)[..3], "bottom-right corner");
    }

    #[test]
    fn out_of_memory_retries_with_half_the_budget() {
        let dir = TestDir::new("export-oom");
        let dest = dir.path().join("oom.png");
        // At 4K (22 px apron) 2048 px tiles need ~68 MiB and 1024 px tiles
        // ~17 MiB: fail above 20 MiB. 256 and 128 MiB budgets pick 2048 px
        // and fail; 64 MiB picks 1024 px and succeeds.
        let r = FakeRenderer::new(20 << 20);
        let req = request(3840, 2160, TilePolicy::default_export());
        let rep = export_png(
            &r,
            &req,
            &dest,
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
        .unwrap();
        assert_eq!(rep.render.outcome, RenderOutcome::Completed);
        let budgets: Vec<u64> = rep
            .attempts
            .iter()
            .map(|a| match a.policy {
                TilePolicy::Budget { gpu_bytes, .. } => gpu_bytes >> 20,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(budgets, [256, 128, 64]);
        assert!(rep.attempts[..2].iter().all(|a| a.out_of_memory.is_some()));
        assert_eq!(rep.attempts[2].out_of_memory, None);
        assert_eq!(rep.render.plan.tile_w, 1024, "same size, smaller tiles");
        assert_eq!(decode_rgb(&dest).0, 3840, "never a lower resolution");
        assert_eq!(dir.entries(), vec!["oom.png".to_string()]);
    }

    #[test]
    fn out_of_memory_at_the_floor_is_reported_and_leaves_no_file() {
        let dir = TestDir::new("export-oom-floor");
        let dest = dir.path().join("x.png");
        std::fs::write(&dest, b"old").unwrap();
        let r = FakeRenderer::new(0);
        let req = request(3840, 2160, TilePolicy::default_export());
        let e = export_png(
            &r,
            &req,
            &dest,
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
        .unwrap_err();
        assert!(matches!(e, RenderError::OutOfMemory { .. }), "{e}");
        // 256 MiB halved down to the smallest budget that still plans 256 px.
        let calls = r.calls.borrow();
        assert!(calls.len() > 4, "{calls:?}");
        let last = *calls.last().unwrap();
        let TilePolicy::Budget { gpu_bytes, .. } = last else {
            unreachable!()
        };
        assert!(!budget_can_plan(
            &r,
            &req,
            TilePolicy::Budget {
                gpu_bytes: gpu_bytes / 2,
                host_bytes: DEFAULT_HOST_BAND_BUDGET
            }
        ));
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert_eq!(dir.entries(), vec!["x.png".to_string()]);
        // Fixed policies are not retried.
        let r = FakeRenderer::new(0);
        let req = request(1600, 900, TilePolicy::Fixed { edge: 512 });
        let fixed = export_png(
            &r,
            &req,
            &dest,
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        );
        assert!(matches!(fixed, Err(RenderError::OutOfMemory { .. })));
        assert_eq!(r.calls.borrow().len(), 1);
    }

    #[test]
    fn cancellation_deletes_the_partial_file_and_keeps_the_old_one() {
        let dir = TestDir::new("export-cancel");
        let dest = dir.path().join("keep.png");
        std::fs::write(&dest, b"previous").unwrap();
        let mut r = FakeRenderer::new(u64::MAX);
        r.cancel_after = Some(3);
        let req = request(1600, 900, TilePolicy::Fixed { edge: 256 });
        let rep = export_png(
            &r,
            &req,
            &dest,
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
        .unwrap();
        assert_eq!(
            rep.render.outcome,
            RenderOutcome::Cancelled { tiles_done: 3 }
        );
        assert_eq!(rep.bytes, 0);
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
        assert_eq!(dir.entries(), vec!["keep.png".to_string()]);
    }

    #[test]
    fn invalid_sizes_fail_before_anything_is_written() {
        let dir = TestDir::new("export-invalid");
        let dest = dir.path().join("x.png");
        let r = FakeRenderer::new(u64::MAX);
        let run = |req: &RenderRequest| {
            export_png(
                &r,
                req,
                &dest,
                PngCompression::Fast,
                &CancelToken::new(),
                &mut NoProgress,
            )
        };
        let mut req = request(1600, 900, TilePolicy::default_export());
        for (w, h) in [
            (0, 0),
            (16, 9),
            (32768, 18432),
            (u32::MAX, u32::MAX / 16 * 9),
            (1600, 901),
            (900, 1600),
        ] {
            req.target.width = w;
            req.target.height = h;
            let e = run(&req).unwrap_err();
            assert!(matches!(e, RenderError::InvalidRequest(_)), "{w}x{h}: {e}");
        }
        assert_eq!(r.sink_calls.get(), 0, "the renderer never ran");
        assert!(r.calls.borrow().is_empty());
        assert!(dir.entries().is_empty());
        let msg = validate_target(&req.scene, 900, 1600)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("9:16") && msg.contains("16:9"), "{msg}");
    }

    #[test]
    fn unwritable_destinations_fail_before_rendering() {
        let dir = TestDir::new("export-unwritable");
        let r = FakeRenderer::new(u64::MAX);
        let req = request(1600, 900, TilePolicy::default_export());
        let e = export_png(
            &r,
            &req,
            &dir.path().join("missing/x.png"),
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
        .unwrap_err();
        assert!(matches!(e, RenderError::Sink(_)), "{e}");
        assert!(r.calls.borrow().is_empty());
    }

    #[test]
    fn presets_keep_the_scene_aspect() {
        let a = AspectRatio::of(16, 9);
        assert_eq!(
            ExportSize::Uhd8k.frame(a).unwrap(),
            Frame::new(7680, 4320).unwrap()
        );
        let p = AspectRatio::of(9, 16);
        assert_eq!(
            ExportSize::Uhd4k.frame(p).unwrap(),
            Frame::new(2160, 3840).unwrap()
        );
        assert!(
            ExportSize::Custom {
                width: 3000,
                height: 4000
            }
            .frame(AspectRatio::of(3, 4))
            .is_ok()
        );
        let e = ExportSize::Custom {
            width: 3000,
            height: 4001,
        }
        .frame(AspectRatio::of(3, 4))
        .unwrap_err();
        assert!(matches!(e.problem, Problem::AspectMismatch { .. }));
    }

    #[test]
    fn tile_plan_errors_are_not_retried() {
        let dir = TestDir::new("export-plan");
        let r = FakeRenderer::new(u64::MAX);
        let req = request(15360, 8640, TilePolicy::Single);
        let e = export_png(
            &r,
            &req,
            &dir.path().join("x.png"),
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
        .unwrap_err();
        assert!(matches!(
            e,
            RenderError::TilePlan(TilePlanError::TooLargeForDevice { .. })
        ));
        assert!(dir.entries().is_empty());
    }
}
