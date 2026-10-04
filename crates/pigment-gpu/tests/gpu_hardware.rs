//! Hardware GPU suite. Ignored by default so portable CI (no GPU) stays
//! green; run with `scripts/gpu-tests.sh` or
//! `cargo test -p pigment-gpu --test gpu_hardware -- --ignored --test-threads=1`.
//!
//! Without a hardware adapter these tests FAIL with the capability error:
//! a missing GPU is a blocker to report, never a pass.

#![allow(clippy::print_stderr)]

use std::sync::{Arc, OnceLock};

use pigment_core::capability::AdapterPolicy;
use pigment_core::composite;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderRequest, RenderTarget, Renderer, RequestIds,
};
use pigment_core::scene::lakeshore::LakeshoreGenerator;
use pigment_core::scene::{SceneGenerator, TestCard, diagnostic_seeds, raster};
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::PaletteId;
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::{TileOrder, TilePolicy};
use pigment_gpu::{
    CompositeCase, CompositeOp, DebugRenderer, DebugView, GpuContext, PaintRenderer, SmokeRenderer,
};

fn context() -> Arc<GpuContext> {
    static C: OnceLock<Arc<GpuContext>> = OnceLock::new();
    C.get_or_init(|| {
        let ctx = GpuContext::new(&AdapterPolicy::default())
            .unwrap_or_else(|e| panic!("hardware GPU required for this suite: {e}"));
        assert!(
            !ctx.capabilities.adapter.software,
            "software adapter is not GPU evidence"
        );
        eprintln!("GPU suite on {}", ctx.capabilities.label());
        Arc::new(ctx)
    })
    .clone()
}

fn renderer() -> &'static SmokeRenderer {
    static R: OnceLock<SmokeRenderer> = OnceLock::new();
    R.get_or_init(|| SmokeRenderer::new(context()).expect("smoke pipelines"))
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
            order: TileOrder::RowMajor,
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

fn lakeshore_request(text: &str, frame: Frame, policy: TilePolicy) -> RenderRequest {
    let seeds = SeedBundle::derive(TextDigest::from_source(text).unwrap(), Variation(0));
    let scene = LakeshoreGenerator
        .generate(&seeds, &FormSettings::default(), frame.aspect())
        .unwrap();
    RenderRequest {
        id: IDS.next(),
        purpose: RenderPurpose::Preview,
        scene: Arc::new(scene),
        seeds,
        appearance: Appearance::default(),
        target: RenderTarget {
            width: frame.width,
            height: frame.height,
            policy,
            order: TileOrder::RowMajor,
        },
    }
}

fn render_debug(view: DebugView, req: &RenderRequest) -> MemorySink {
    let r = DebugRenderer::new(context(), view).expect("debug pipelines");
    let mut sink = MemorySink::default();
    let rep = r
        .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .expect("render");
    assert_eq!(rep.outcome, RenderOutcome::Completed);
    sink
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn debug_layer_ids_match_the_cpu_rasterizer() {
    // Same coverage rule on both sides; only pixels whose centre lies
    // within float rounding of an edge may differ.
    for (text, frame) in [
        (
            "A pebble rests by the shore.",
            Frame::new(640, 360).unwrap(),
        ),
        ("風が湖を渡る。", Frame::new(360, 640).unwrap()),
        (
            "Clouds drift ☁ above quiet water.",
            Frame::new(500, 500).unwrap(),
        ),
    ] {
        let req = lakeshore_request(text, frame, TilePolicy::Single);
        let gpu = render_debug(DebugView::LayerIds, &req);
        let cpu = raster::front_layers(&req.scene, frame.width as usize, frame.height as usize);
        let mut differ = 0;
        for (i, id) in cpu.iter().enumerate() {
            let want = if *id == raster::NONE {
                0
            } else {
                *id as u32 + 1
            };
            if gpu.rgba8[i * 4] as u32 != want {
                differ += 1;
            }
        }
        let frac = differ as f64 / cpu.len() as f64;
        eprintln!(
            "{text:?} {}x{}: {differ} of {} pixels differ",
            frame.width,
            frame.height,
            cpu.len()
        );
        assert!(frac < 0.002, "{frac} of pixels differ");
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn debug_views_are_identical_tiled_and_single() {
    let frame = Frame::new(1001, 563).unwrap();
    for view in [DebugView::Flat, DebugView::Regions] {
        let single = render_debug(
            view,
            &lakeshore_request("Blue dusk.", frame, TilePolicy::Single),
        );
        for edge in [256, 333] {
            let tiled = render_debug(
                view,
                &lakeshore_request("Blue dusk.", frame, TilePolicy::Fixed { edge }),
            );
            assert!(tiled.rgba8 == single.rgba8, "{view:?} tile {edge}");
        }
    }
}

fn paint_renderer() -> &'static PaintRenderer {
    static R: OnceLock<PaintRenderer> = OnceLock::new();
    R.get_or_init(|| PaintRenderer::new(context()).expect("paint pipelines"))
}

fn render_paint(req: &RenderRequest) -> MemorySink {
    let r = paint_renderer();
    let mut sink = MemorySink::default();
    let rep = r
        .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .expect("render");
    assert_eq!(rep.outcome, RenderOutcome::Completed);
    sink
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn painting_is_identical_tiled_and_single() {
    // The apron covers the loose-edge warp and disc at every looseness.
    // Reflections, rock rims and contacts are evaluated from the scene at
    // offset points (no halo); the sample seeds cover a river valley (15), a
    // lake with rocks under a tower (0) and framing ridges with rocks (8).
    let frame = Frame::new(1001, 563).unwrap();
    for text in [
        "Blue dusk.",
        "sample passage 15",
        "sample passage 0",
        "sample passage 8",
    ] {
        for looseness in [0.0, 0.4, 1.0] {
            let mut single = lakeshore_request(text, frame, TilePolicy::Single);
            single.appearance.painting.edge_looseness = looseness;
            let reference = render_paint(&single);
            for edge in [256, 333] {
                let mut tiled = single.clone();
                tiled.target.policy = TilePolicy::Fixed { edge };
                assert!(
                    render_paint(&tiled).rgba8 == reference.rgba8,
                    "{text:?} looseness {looseness} tile {edge}"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn cloud_volume_ignores_flat_bands_and_responds_to_light() {
    use pigment_core::scene::{LayerRole, LightSide, Scene};

    let frame = Frame::new(960, 540).unwrap();
    let req = lakeshore_request("sample passage 0", frame, TilePolicy::Single);
    let reference = render_paint(&req);
    let mut layers = req.scene.layers().to_vec();
    for layer in &mut layers {
        if layer.role == LayerRole::Cloud {
            layer.shade = 1.0 - layer.shade;
        }
    }
    let mut changed = req.clone();
    changed.scene = Arc::new(
        Scene::new(*req.scene.key(), layers)
            .unwrap()
            .with_light(req.scene.light())
            .with_wind(req.scene.wind()),
    );
    assert_eq!(
        reference.rgba8,
        render_paint(&changed).rgba8,
        "old flat cloud bands must not leak through the participating volume"
    );

    let opposite = match req.scene.light() {
        LightSide::Left => LightSide::Right,
        LightSide::Right => LightSide::Left,
    };
    changed.scene = Arc::new((*req.scene).clone().with_light(opposite));
    let flipped = render_paint(&changed);
    let ids = raster::front_layers(&req.scene, 960, 540);
    let mut sum = 0.0;
    let mut samples = 0;
    for (i, id) in ids.into_iter().enumerate() {
        if id != raster::NONE && req.scene.layers()[id as usize].role == LayerRole::Cloud {
            for ch in 0..3 {
                sum +=
                    (reference.rgba8[i * 4 + ch] as f64 - flipped.rgba8[i * 4 + ch] as f64).abs();
                samples += 1;
            }
        }
    }
    assert!(samples > 1000);
    assert!(
        sum / samples as f64 > 2.0,
        "cloud light must be directional"
    );
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn relief_is_tile_order_independent_in_every_biome_and_orientation() {
    for profile in pigment_core::biome::PROFILES {
        for (width, height) in [(769, 433), (433, 769)] {
            let frame = Frame::new(width, height).unwrap();
            let mut req = lakeshore_request("Relief regression", frame, TilePolicy::Single);
            req.scene = Arc::new(
                pigment_core::scene::generator(profile.id)
                    .generate(&req.seeds, &profile.form, frame.aspect())
                    .unwrap(),
            );
            req.appearance.palette.id = profile.palette;
            req.appearance.season.year = 0.27;
            let reference = render_paint(&req);
            req.target.policy = TilePolicy::Fixed { edge: 223 };
            req.target.order = TileOrder::ReverseInBand;
            assert_eq!(
                reference.rgba8,
                render_paint(&req).rgba8,
                "{:?} {width}x{height}: relief/density must be in whole-image space",
                profile.id
            );
        }
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn painting_is_opaque_repeatable_and_varied() {
    let frame = Frame::new(640, 360).unwrap();
    for text in [
        "A pebble rests by the shore.",
        "風が湖を渡る。",
        "Blue dusk.",
    ] {
        let req = lakeshore_request(text, frame, TilePolicy::Single);
        let a = render_paint(&req);
        let b = render_paint(&req);
        assert!(a.rgba8 == b.rgba8, "{text}: not repeatable");
        let px = a.rgba8.as_chunks::<4>().0;
        assert!(px.iter().all(|p| p[3] == 255), "{text}: not opaque");
        // Not blank or flat: a real spread of values and colors.
        let lum: Vec<f64> = px
            .iter()
            .map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64)
            .collect();
        let mean = lum.iter().sum::<f64>() / lum.len() as f64;
        let sd =
            (lum.iter().map(|l| (l - mean) * (l - mean)).sum::<f64>() / lum.len() as f64).sqrt();
        assert!(sd > 15.0, "{text}: value spread {sd}");
        // "Verdant": a good share of clearly green pixels.
        let green = px
            .iter()
            .filter(|p| p[1] as i32 > p[0] as i32 + 10 && p[1] as i32 > p[2] as i32 + 10)
            .count() as f64
            / px.len() as f64;
        eprintln!("{text}: value sd {sd:.1}, green {:.0}%", green * 100.0);
        assert!(green > 0.05, "{text}: only {green} green");
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn painting_agrees_across_resolutions() {
    // Band-limited textures: a 4x-downsampled 1920 render resembles the 480
    // render of the same scene (value structure, not texture detail).
    let big = render_paint(&lakeshore_request(
        "A pebble rests by the shore.",
        Frame::new(1920, 1080).unwrap(),
        TilePolicy::Single,
    ));
    let small = render_paint(&lakeshore_request(
        "A pebble rests by the shore.",
        Frame::new(480, 270).unwrap(),
        TilePolicy::Single,
    ));
    let mut se = 0.0;
    let mut n = 0.0;
    for y in 0..270usize {
        for x in 0..480usize {
            for ch in 0..3 {
                let mut acc = 0.0;
                for dy in 0..4 {
                    for dx in 0..4 {
                        acc += big.rgba8[((y * 4 + dy) * 1920 + x * 4 + dx) * 4 + ch] as f64;
                    }
                }
                let d = acc / 16.0 - small.rgba8[(y * 480 + x) * 4 + ch] as f64;
                se += d * d;
                n += 1.0;
            }
        }
    }
    let psnr = 10.0 * (255.0f64 * 255.0 / (se / n)).log10();
    eprintln!("480 vs downsampled 1920: PSNR {psnr:.1} dB");
    assert!(psnr > 24.0, "PSNR {psnr}");
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn compositing_matches_the_reference_model() {
    // Opaque fill, partial coverage (edge blending), glazes and washes at
    // the densities the painting uses, over paper and dark colors.
    let colors = [
        [0.905, 0.85, 0.74],
        [0.05, 0.2, 0.07],
        [0.6, 0.3, 0.1],
        [0.2, 0.45, 0.6],
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
    ];
    let mut cases = Vec::new();
    for under in colors {
        for color in colors {
            for amount in [0.0, 0.125, 0.5, 0.8, 1.0, 1.2] {
                for op in [CompositeOp::Glaze, CompositeOp::Over, CompositeOp::Wash] {
                    if op == CompositeOp::Over && amount > 1.0 {
                        continue;
                    }
                    if op == CompositeOp::Wash && under.contains(&0.0) {
                        continue; // paper is never black
                    }
                    cases.push(CompositeCase {
                        op,
                        under,
                        color,
                        amount,
                    });
                }
            }
        }
    }
    let gpu = paint_renderer()
        .evaluate_compositing(&cases)
        .expect("reference");
    assert_eq!(gpu.len(), cases.len());
    let mut worst = 0.0f32;
    for (c, g) in cases.iter().zip(&gpu) {
        let want = match c.op {
            CompositeOp::Glaze => composite::glaze(c.under, c.color, c.amount),
            CompositeOp::Over => composite::over(c.under, c.color, c.amount),
            CompositeOp::Wash => composite::wash(c.under, c.color, c.amount),
        };
        for i in 0..3 {
            assert!(g[i].is_finite(), "{c:?}: {g:?}");
            // pow is not correctly rounded on GPUs; 1e-4 relative is far
            // below one 8-bit output step (about 4e-3).
            let err = (g[i] - want[i]).abs() / (1.0 + want[i].abs());
            worst = worst.max(err);
            assert!(err < 1e-4, "{c:?}: gpu {g:?}, model {want:?}");
        }
    }
    eprintln!(
        "{} compositing cases, worst relative error {worst:e}",
        cases.len()
    );
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn painting_is_valid_at_setting_extremes() {
    // Every combination of paint-setting endpoints, for both palettes: an
    // opaque, varied image with no black (NaN or collapsed) pixels, from the
    // same scene every time.
    let frame = Frame::new(320, 180).unwrap();
    let base = lakeshore_request("A pebble rests by the shore.", frame, TilePolicy::Single);
    let checksum = base.scene.geometry_checksum();
    let mut n = 0;
    for palette in [PaletteId::Lakeshore, PaletteId::GoldenEvening] {
        for bits in 0u32..128 {
            let pick = |k: u32, lo: f64, hi: f64| if bits >> k & 1 == 1 { hi } else { lo };
            let mut req = base.clone();
            let a = &mut req.appearance;
            a.palette.id = palette;
            a.painting.edge_looseness = pick(0, 0.0, 1.0);
            a.painting.wash_gouache = pick(1, 0.0, 1.0);
            a.painting.mark_scale = pick(2, 0.5, 2.0);
            a.painting.granulation = pick(3, 0.0, 1.0);
            a.painting.paper_grain = pick(4, 0.0, 1.0);
            a.palette.intensity = pick(5, 0.0, 1.0);
            a.atmosphere.haze = pick(6, 0.0, 1.0);
            a.validate().unwrap();
            let img = render_paint(&req);
            let px = img.rgba8.as_chunks::<4>().0;
            assert!(
                px.iter().all(|p| p[3] == 255),
                "{palette:?} {bits:07b}: not opaque"
            );
            let black = px.iter().filter(|p| p[..3] == [0, 0, 0]).count();
            assert_eq!(black, 0, "{palette:?} {bits:07b}: {black} black pixels");
            let (lo, hi) = px.iter().fold((255u8, 0u8), |(lo, hi), p| {
                let v = p[1];
                (lo.min(v), hi.max(v))
            });
            assert!(hi - lo > 60, "{palette:?} {bits:07b}: flat ({lo}..{hi})");
            assert_eq!(req.scene.geometry_checksum(), checksum);
            n += 1;
        }
    }
    eprintln!("{n} setting combinations valid");
}

// ---------------------------------------------------------------- seasons (task 16)

fn in_season(req: &RenderRequest, year: f64) -> RenderRequest {
    let mut r = req.clone();
    r.id = IDS.next();
    r.appearance.season.year = year;
    r
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn every_season_is_identical_tiled_and_single_at_high_resolution() {
    // Snow, leaves and ground cover are evaluated from the scene at the
    // pixel (or at offset points), never from neighbouring pixels, so every
    // season tiles exactly, including the snow's edges at 2K.
    let frame = Frame::new(2002, 1126).unwrap();
    for text in ["sample passage 8", "sample passage 15"] {
        let base = lakeshore_request(text, frame, TilePolicy::Single);
        for year in [0.0, 0.25, 0.75, 0.86] {
            let single = in_season(&base, year);
            let reference = render_paint(&single);
            for edge in [333, 512] {
                let mut tiled = in_season(&single, year);
                tiled.target.policy = TilePolicy::Fixed { edge };
                assert!(
                    render_paint(&tiled).rgba8 == reference.rgba8,
                    "{text:?} season {year} tile {edge}"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn midsummer_is_the_default_and_the_year_wraps() {
    let base = lakeshore_request(
        "sample passage 16",
        Frame::new(640, 360).unwrap(),
        TilePolicy::Single,
    );
    let default = render_paint(&base);
    assert!(render_paint(&in_season(&base, 0.5)).rgba8 == default.rgba8);
    // Both ends of the year are the same moment.
    assert!(
        render_paint(&in_season(&base, 0.0)).rgba8 == render_paint(&in_season(&base, 1.0)).rgba8
    );
    // Every season paints the same scene differently.
    for year in [0.0, 0.25, 0.62, 0.75, 0.86] {
        assert!(
            render_paint(&in_season(&base, year)).rgba8 != default.rgba8,
            "{year}"
        );
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn the_sky_and_clouds_stay_the_palettes_in_every_season() {
    // Precedence (docs/seasons-and-biomes.md): the palette owns the sky,
    // clouds and light, the season only what is on the land. Checked on sky
    // and cloud pixels whose whole neighbourhood (the loose edges' reach) is
    // sky or cloud.
    for palette in [PaletteId::Lakeshore, PaletteId::GoldenEvening] {
        for text in ["sample passage 16", "Blue dusk."] {
            let frame = Frame::new(640, 360).unwrap();
            let mut base = lakeshore_request(text, frame, TilePolicy::Single);
            base.appearance.palette.id = palette;
            let ids = render_debug(DebugView::LayerIds, &base);
            let layers = base.scene.layers();
            let (w, h) = (frame.width as i64, frame.height as i64);
            let airy = |x: i64, y: i64| {
                let id = ids.rgba8[((y * w + x) * 4) as usize] as usize;
                id > 0
                    && matches!(
                        layers[id - 1].role,
                        pigment_core::scene::LayerRole::Sky | pigment_core::scene::LayerRole::Cloud
                    )
            };
            let reach = 8;
            let keep: Vec<usize> = (reach..h - reach)
                .flat_map(|y| (reach..w - reach).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    (-reach..=reach).all(|dy| (-reach..=reach).all(|dx| airy(x + dx, y + dy)))
                })
                .map(|(x, y)| (y * w + x) as usize)
                .collect();
            assert!(keep.len() > 10_000, "{text:?}: {} sky pixels", keep.len());
            let summer = render_paint(&base);
            for year in [0.0, 0.25, 0.75, 0.86] {
                let other = render_paint(&in_season(&base, year));
                let changed = keep
                    .iter()
                    .filter(|&&i| summer.rgba8[i * 4..i * 4 + 4] != other.rgba8[i * 4..i * 4 + 4])
                    .count();
                assert_eq!(
                    changed, 0,
                    "{palette:?} {text:?} season {year}: {changed} sky pixels changed"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn snow_only_grows_as_winter_deepens() {
    // From late summer to midwinter the snow channels only rise, and each
    // snow cover is a fixed noise field against a falling threshold, so the
    // snowfall mask only grows. Checked where snow is the only seasonal
    // change: the distant ranges and the massif's rock above the treeline
    // (away from other layers' edges). There a pixel is its midsummer colour
    // blended toward snow by the cover, so its distance from midsummer never
    // shrinks as the snow deepens, lit or in blue shadow.
    use pigment_core::scene::{LayerRole, metrics};
    let frame = Frame::new(640, 360).unwrap();
    let years = [0.62, 0.75, 0.86, 0.14, 0.0];
    for text in ["sample passage 16", "sample passage 8", "Blue dusk."] {
        let base = lakeshore_request(text, frame, TilePolicy::Single);
        let (horizon, summit) = metrics::horizon_and_summit(&base.scene);
        let ids = render_debug(DebugView::LayerIds, &base);
        let layers = base.scene.layers();
        let (w, h) = (frame.width as i64, frame.height as i64);
        let role = |x: i64, y: i64| {
            let id = ids.rgba8[((y * w + x) * 4) as usize] as usize;
            (id > 0).then(|| (id, layers[id - 1].role))
        };
        let reach = 6;
        let keep: Vec<usize> = (reach..h - reach)
            .flat_map(|y| (reach..w - reach).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let Some((_, r)) = role(x, y) else {
                    return false;
                };
                let high = (y as f64 + 0.5) / h as f64 <= summit + 0.4 * (horizon - summit);
                (r == LayerRole::FarRidge || (r == LayerRole::Mountain && high))
                    && (-reach..=reach).all(|dy| {
                        (-reach..=reach).all(|dx| role(x + dx, y + dy).map(|(_, q)| q) == Some(r))
                    })
            })
            .map(|(x, y)| (y * w + x) as usize)
            .collect();
        assert!(keep.len() > 2_000, "{text:?}: {} pixels", keep.len());
        let summer = render_paint(&base).rgba8;
        let away: Vec<Vec<i32>> = years
            .iter()
            .map(|&y| {
                let img = render_paint(&in_season(&base, y)).rgba8;
                keep.iter()
                    .map(|&i| {
                        (0..3)
                            .map(|c| (img[i * 4 + c] as i32 - summer[i * 4 + c] as i32).abs())
                            .sum()
                    })
                    .collect()
            })
            .collect();
        for (k, pair) in away.windows(2).enumerate() {
            let receded = pair[0]
                .iter()
                .zip(&pair[1])
                .filter(|&(a, b)| b + 3 < *a)
                .count();
            let snowier = pair[0].iter().zip(&pair[1]).filter(|&(a, b)| b > a).count();
            eprintln!(
                "{text:?} {} -> {}: of {} pixels {snowier} snowier, {receded} less snowy",
                years[k],
                years[k + 1],
                keep.len()
            );
            assert!(
                receded as f64 <= 0.002 * keep.len() as f64,
                "{text:?}: snow receded"
            );
        }
    }
}
