//! Hardware GPU suite. Ignored by default so portable CI (no GPU) stays
//! green; run with `scripts/gpu-tests.sh` or
//! `cargo test -p pigment-gpu --test gpu_hardware -- --ignored --test-threads=1`.
//!
//! Without a hardware adapter these tests FAIL with the capability error:
//! a missing GPU is a blocker to report, never a pass.

#![allow(clippy::print_stderr)]

use std::sync::{Arc, OnceLock};

use pigment_core::capability::AdapterPolicy;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderRequest, RenderTarget, Renderer, RequestIds,
};
use pigment_core::scene::{SceneGenerator, TestCard, diagnostic_seeds};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::TilePolicy;
use pigment_gpu::{GpuContext, SmokeRenderer};

fn renderer() -> &'static SmokeRenderer {
    static R: OnceLock<SmokeRenderer> = OnceLock::new();
    R.get_or_init(|| {
        let ctx = GpuContext::new(&AdapterPolicy::default())
            .unwrap_or_else(|e| panic!("hardware GPU required for this suite: {e}"));
        assert!(
            !ctx.capabilities.adapter.software,
            "software adapter is not GPU evidence"
        );
        eprintln!("GPU suite on {}", ctx.capabilities.label());
        SmokeRenderer::new(Arc::new(ctx)).expect("smoke pipelines")
    })
}

static IDS: RequestIds = RequestIds::new();

fn request(frame: Frame, seed: u64, looseness: f64, policy: TilePolicy) -> RenderRequest {
    let seeds = diagnostic_seeds(seed);
    let scene = TestCard
        .generate(&seeds, &FormSettings::default(), frame.aspect())
        .unwrap();
    let mut appearance = Appearance::default();
    appearance.painting.edge_looseness = looseness;
    RenderRequest {
        id: IDS.next(),
        purpose: RenderPurpose::Export,
        scene: Arc::new(scene),
        seeds,
        appearance,
        target: RenderTarget {
            width: frame.width,
            height: frame.height,
            policy,
        },
    }
}

fn render(req: &RenderRequest) -> MemorySink {
    let mut sink = MemorySink::default();
    let rep = renderer()
        .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .expect("render");
    assert_eq!(rep.outcome, RenderOutcome::Completed);
    assert!(sink.finished && !sink.aborted);
    sink
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn tiled_output_is_byte_identical_to_single_tile() {
    let cases = [
        (Frame::new(1920, 1080).unwrap(), 512),
        (Frame::new(1080, 1920).unwrap(), 333),
        (Frame::new(1500, 1500).unwrap(), 700),
        (Frame::new(1001, 703).unwrap(), 256),
    ];
    for (frame, edge) in cases {
        for looseness in [0.0, 0.4, 1.0] {
            let single = render(&request(frame, 11, looseness, TilePolicy::Single));
            let tiled = render(&request(frame, 11, looseness, TilePolicy::Fixed { edge }));
            assert!(
                single.rgba8 == tiled.rgba8,
                "{frame:?} tile {edge} looseness {looseness}: tiled output differs"
            );
        }
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn output_is_opaque_and_repeatable() {
    let req = request(Frame::new(640, 480).unwrap(), 3, 0.4, TilePolicy::Single);
    let a = render(&req);
    let b = render(&req);
    assert!(a.rgba8 == b.rgba8);
    assert!(a.rgba8.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let lo = a
        .rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[0])
        .min()
        .unwrap();
    let hi = a
        .rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[0])
        .max()
        .unwrap();
    assert!(
        hi - lo > 64,
        "test card should span several values ({lo}..{hi})"
    );
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn paint_settings_change_pixels_but_not_the_scene() {
    let frame = Frame::new(640, 360).unwrap();
    let a = request(frame, 5, 0.1, TilePolicy::Single);
    let b = request(frame, 5, 0.9, TilePolicy::Single);
    assert_eq!(a.scene.geometry_checksum(), b.scene.geometry_checksum());
    assert!(
        render(&a).rgba8 != render(&b).rgba8,
        "looseness must change the painting"
    );
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn cancellation_stops_at_the_next_tile_and_aborts_the_sink() {
    let req = request(
        Frame::new(1024, 1024).unwrap(),
        1,
        0.2,
        TilePolicy::Fixed { edge: 256 },
    );
    let cancel = CancelToken::new();
    let c = cancel.clone();
    let mut seen = Vec::new();
    let mut progress = |p: Progress| {
        seen.push(p.done);
        if p.done == 3 {
            c.cancel();
        }
    };
    let mut sink = MemorySink::default();
    let rep = renderer()
        .render(&req, &cancel, &mut progress, &mut sink)
        .unwrap();
    assert_eq!(rep.outcome, RenderOutcome::Cancelled { tiles_done: 3 });
    assert!(sink.aborted && !sink.finished);
    assert_eq!(rep.plan.len(), 16);
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn impossible_tile_plan_is_an_error_not_a_crash() {
    // WebGPU default limits cap textures at 8192; one 16K tile cannot fit.
    let req = request(Frame::new(16384, 8192).unwrap(), 1, 0.0, TilePolicy::Single);
    let mut sink = MemorySink::default();
    let err = renderer()
        .render(&req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .unwrap_err();
    assert!(err.to_string().contains("texture limit"), "{err}");
}
