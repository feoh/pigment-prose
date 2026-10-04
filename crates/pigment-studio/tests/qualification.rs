//! Real-hardware qualification: the integrated MVP from an approved recipe to
//! tiled PNG, tile seams, brush/texture scale, repeated previews/exports, and
//! biome-specific stress cases. Ignored by default; `scripts/gpu-tests.sh`
//! runs it (see docs/qualification.md).
//!
//! Tolerances follow docs/architecture.md, "Reproducibility tiers": exact
//! where the tier promises exactness (the same device), bounded metrics
//! where it does not (different resolutions). Image metrics are printed so
//! the qualification log records the measured values, not just a pass.

#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pigment_core::capability::AdapterPolicy;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress};
use pigment_core::recipe::{Component, VersionNotice};
use pigment_core::request::{
    MemorySink, RenderPurpose, RenderRequest, RenderTarget, Renderer, RequestId,
};
use pigment_core::tiles::{TileOrder, TilePolicy};
use pigment_gpu::{GpuContext, PaintRenderer};
use pigment_io::Document;
use pigment_studio::export::{ExportJob, ExportOutcome, Exporter};
use pigment_studio::worker::{PreviewJob, PreviewOutcome, PreviewWorker, WorkerOptions};

fn ctx() -> Arc<GpuContext> {
    static CTX: OnceLock<Arc<GpuContext>> = OnceLock::new();
    CTX.get_or_init(|| {
        let ctx = GpuContext::new(&AdapterPolicy::default()).expect("a hardware GPU");
        assert!(
            !ctx.capabilities.adapter.software,
            "software adapters do not qualify"
        );
        eprintln!("qualification on {}", ctx.capabilities.label());
        Arc::new(ctx)
    })
    .clone()
}

fn baseline() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/visual-review/baseline-16")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("pigment-qual-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An approved recipe and its frozen geometry checksum.
fn approved(sheet: &str, cell: usize) -> (Document, String) {
    let notes = std::fs::read_to_string(baseline().join(format!("{sheet}.txt"))).unwrap();
    let mut lines = notes.lines().skip_while(|l| !l.starts_with("cell\t"));
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = header.iter().position(|h| *h == "checksum").unwrap();
    let row = lines
        .find(|l| l.split('\t').next() == Some(&cell.to_string()))
        .expect("cell");
    let checksum = row.split('\t').nth(col).unwrap().to_string();
    let (doc, notices) = Document::open(
        &baseline()
            .join(sheet)
            .join(format!("{cell:02}.recipe.json")),
    )
    .unwrap();
    assert_eq!(
        notices,
        [VersionNotice {
            component: Component::Renderer,
            recorded: 3,
            current: pigment_core::version::RENDERER_VERSION,
        }],
        "baseline Alpine geometry and pixels are unchanged; only the renderer version advanced for jungle"
    );
    (doc, checksum)
}

fn request(doc: &Document, w: u32, h: u32, policy: TilePolicy) -> RenderRequest {
    let r = doc.recipe();
    let scene = pigment_core::scene::generator(r.biome)
        .generate(&doc.seeds(), &r.form, r.frame.aspect())
        .unwrap();
    RenderRequest {
        id: RequestId(1),
        purpose: RenderPurpose::Export,
        scene: Arc::new(scene),
        seeds: doc.seeds(),
        appearance: doc.appearance(),
        target: RenderTarget {
            width: w,
            height: h,
            policy,
            order: TileOrder::RowMajor,
        },
    }
}

/// RGB bytes of a render.
fn render_rgb(renderer: &PaintRenderer, req: &RenderRequest) -> Vec<u8> {
    let mut sink = MemorySink::default();
    renderer
        .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .unwrap();
    sink.rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn decode_rgb(path: &Path) -> (u32, u32, Vec<u8>) {
    let d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// Box-downsamples RGB `w`×`h` by `k` (sRGB values averaged).
fn downsample(rgb: &[u8], w: usize, h: usize, k: usize) -> Vec<f64> {
    let (ow, oh) = (w / k, h / k);
    let mut out = vec![0.0; ow * oh * 3];
    for y in 0..oh * k {
        for x in 0..ow * k {
            for c in 0..3 {
                out[((y / k) * ow + x / k) * 3 + c] += rgb[(y * w + x) * 3 + c] as f64;
            }
        }
    }
    let n = (k * k) as f64;
    out.iter_mut().for_each(|v| *v /= n);
    out
}

fn psnr(a: &[f64], b: &[f64]) -> f64 {
    let se: f64 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
    10.0 * (255.0f64 * 255.0 / (se / a.len() as f64)).log10()
}

fn to_f64(rgb: &[u8]) -> Vec<f64> {
    rgb.iter().map(|&v| v as f64).collect()
}

/// Mean absolute luminance Laplacian: how much fine texture there is.
fn texture_energy(rgb: &[f64], w: usize, h: usize) -> f64 {
    let lum = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        0.2126 * rgb[i] + 0.7152 * rgb[i + 1] + 0.0722 * rgb[i + 2]
    };
    let mut sum = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let l = 4.0 * lum(x, y) - lum(x - 1, y) - lum(x + 1, y) - lum(x, y - 1) - lum(x, y + 1);
            sum += l.abs();
        }
    }
    sum / ((w - 2) * (h - 2)) as f64
}

fn wait_preview(w: &PreviewWorker) -> (u32, u32, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for r in w.drain() {
            if let PreviewOutcome::Image {
                width,
                height,
                rgba8,
            } = r.outcome
            {
                return (width, height, rgba8);
            }
        }
        assert!(Instant::now() < deadline, "no preview");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_export(e: &Exporter) -> ExportOutcome {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(o) = e.take_finished() {
            return o;
        }
        assert!(Instant::now() < deadline, "export did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn export_job(doc: &Document, frame: Frame, dest: PathBuf) -> ExportJob {
    ExportJob {
        biome: doc.recipe().biome,
        seeds: doc.seeds(),
        form: doc.recipe().form,
        aspect: doc.recipe().frame.aspect(),
        appearance: doc.appearance(),
        frame,
        destination: dest,
    }
}

fn rss() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib: u64 = s
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kib * 1024)
}

/// Recipe → scene → preview → tiled PNG, for approved recipes in all three
/// shapes, through the studio's own preview worker and export worker.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn approved_recipes_go_from_scene_to_preview_to_a_tiled_png() {
    let ctx = ctx();
    let renderer = PaintRenderer::new(ctx.clone()).unwrap();
    let preview = PreviewWorker::spawn(
        PaintRenderer::new(ctx.clone()).unwrap(),
        WorkerOptions::default(),
        || {},
    );
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx.clone()).unwrap(), || {});
    let dir = Scratch::new("e2e");
    for (sheet, cell) in [("corpus-16x9", 1), ("corpus-9x16", 3), ("corpus-1x1", 5)] {
        let (doc, checksum) = approved(sheet, cell);
        let r = doc.recipe();
        // 1. Scene: exactly the approved geometry.
        let scene = pigment_core::scene::generator(r.biome)
            .generate(&doc.seeds(), &r.form, r.frame.aspect())
            .unwrap();
        assert_eq!(
            format!("{:016x}", scene.geometry_checksum()),
            checksum,
            "{sheet} {cell}"
        );
        // 2. Preview at 960 px through the worker (the approved cells were
        // saved at 400 px; the scene depends only on the aspect ratio).
        let pf = Frame::largest_with_aspect(r.frame.aspect(), 960).unwrap();
        let (pw, ph) = (pf.width, pf.height);
        preview.submit(
            RequestId(cell as u64),
            PreviewJob {
                biome: r.biome,
                seeds: doc.seeds(),
                form: r.form,
                aspect: r.frame.aspect(),
                appearance: doc.appearance(),
                width: pw,
                height: ph,
                submitted_at: Instant::now(),
            },
        );
        let (w, h, rgba) = wait_preview(&preview);
        assert_eq!((w, h), (pw, ph));
        let small: Vec<f64> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
            .collect();
        // 3. Tiled PNG at 4× the preview through the export worker (tiles
        // of 2048 px: several per image).
        let frame = Frame::new(pw * 4, ph * 4).unwrap();
        let dest = dir.0.join(format!("{sheet}-{cell}.png"));
        exporter
            .start(export_job(&doc, frame, dest.clone()))
            .unwrap();
        match wait_export(&exporter) {
            ExportOutcome::Written { tiles, .. } => assert!(tiles > 1, "{sheet}: {tiles} tile"),
            o => panic!("{sheet} {cell}: {o:?}"),
        }
        let (ew, eh, png) = decode_rgb(&dest);
        assert_eq!((ew, eh), (frame.width, frame.height));
        // 4. Tiled PNG == one-tile render, byte for byte (tier 2).
        let single = render_rgb(
            &renderer,
            &request(&doc, frame.width, frame.height, TilePolicy::Single),
        );
        assert!(
            png == single,
            "{sheet} {cell}: tiled PNG differs from the single tile"
        );
        // 5. The export keeps the preview's composition.
        let down = downsample(&png, ew as usize, eh as usize, 4);
        let p = psnr(&down, &small);
        eprintln!("{sheet} {cell}: {ew}×{eh} export, 4× downsampled vs 960 preview PSNR {p:.1} dB");
        assert!(p > 24.0, "{sheet} {cell}: PSNR {p}");
    }
}

/// A desert recipe survives save/reopen and exercises the shared GPU
/// painter through a multi-tile export, bit-identical to one full render.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn desert_recipe_reopens_and_tiled_export_matches_single() {
    let ctx = ctx();
    let renderer = PaintRenderer::new(ctx.clone()).unwrap();
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx).unwrap(), || {});
    let dir = Scratch::new("desert-e2e");
    let recipe_path = dir.0.join("desert.recipe.json");
    let mut doc = Document::from_prose(
        "A dry wash winds below the layered red mesas.",
        Frame::new(1600, 900).unwrap(),
    )
    .unwrap();
    doc.set_biome(pigment_core::biome::BiomeId::Desert).unwrap();
    let mut appearance = doc.appearance();
    appearance.season.year = 0.68;
    doc.set_appearance(appearance).unwrap();
    doc.save_as(&recipe_path).unwrap();
    let (doc, notices) = Document::open(&recipe_path).unwrap();
    assert!(notices.is_empty());
    assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Desert);

    let frame = Frame::new(3840, 2160).unwrap();
    let dest = dir.0.join("desert-4k.png");
    exporter
        .start(export_job(&doc, frame, dest.clone()))
        .unwrap();
    match wait_export(&exporter) {
        ExportOutcome::Written { tiles, .. } => assert!(tiles > 1, "expected tiled export"),
        outcome => panic!("desert export failed: {outcome:?}"),
    }
    let (width, height, tiled) = decode_rgb(&dest);
    assert_eq!((width, height), (frame.width, frame.height));
    let single = render_rgb(
        &renderer,
        &request(&doc, frame.width, frame.height, TilePolicy::Single),
    );
    assert_eq!(tiled, single, "desert tile boundaries must be invisible");
}

/// A tundra recipe survives save/reopen and exercises the shared GPU painter
/// through a winter 4K tiled export, bit-identical to one full render.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn tundra_recipe_reopens_and_winter_tiled_export_matches_single() {
    let ctx = ctx();
    let renderer = PaintRenderer::new(ctx.clone()).unwrap();
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx).unwrap(), || {});
    let dir = Scratch::new("tundra-e2e");
    let recipe_path = dir.0.join("tundra.recipe.json");
    let mut doc = Document::from_prose(
        "A brief midsummer grazes the low rolling tundra.",
        Frame::new(1600, 900).unwrap(),
    )
    .unwrap();
    doc.set_biome(pigment_core::biome::BiomeId::Tundra).unwrap();
    let mut appearance = doc.appearance();
    appearance.season.year = 0.15;
    doc.set_appearance(appearance).unwrap();
    doc.save_as(&recipe_path).unwrap();
    let (doc, notices) = Document::open(&recipe_path).unwrap();
    assert!(notices.is_empty());
    assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Tundra);

    let frame = Frame::new(3840, 2160).unwrap();
    let dest = dir.0.join("tundra-winter-4k.png");
    exporter
        .start(export_job(&doc, frame, dest.clone()))
        .unwrap();
    match wait_export(&exporter) {
        ExportOutcome::Written { tiles, .. } => assert!(tiles > 1, "expected tiled export"),
        outcome => panic!("tundra export failed: {outcome:?}"),
    }
    let (width, height, tiled) = decode_rgb(&dest);
    assert_eq!((width, height), (frame.width, frame.height));
    let single = render_rgb(
        &renderer,
        &request(&doc, frame.width, frame.height, TilePolicy::Single),
    );
    assert_eq!(tiled, single, "tundra tile boundaries must be invisible");
}

/// An owner-approved jungle recipe is saved without its source prose,
/// reopened, then exported at 4K through multiple tiles and compared with the
/// one-tile painter. The understory fans and canopy shapes cross tile edges.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn jungle_recipe_reopens_source_free_and_tiled_export_matches_single() {
    let ctx = ctx();
    let renderer = PaintRenderer::new(ctx.clone()).unwrap();
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx).unwrap(), || {});
    let dir = Scratch::new("jungle-e2e");
    let recipe_path = dir.0.join("jungle.recipe.json");
    let mut doc = Document::from_prose(
        "A narrow clearing beneath a high canopy.",
        Frame::new(1600, 900).unwrap(),
    )
    .unwrap();
    doc.set_biome(pigment_core::biome::BiomeId::Jungle).unwrap();
    doc.set_keep_source_text(false);
    let mut appearance = doc.appearance();
    appearance.season.year = 0.82;
    doc.set_appearance(appearance).unwrap();
    doc.save_as(&recipe_path).unwrap();
    let recipe_bytes = std::fs::read(&recipe_path).unwrap();
    assert!(!String::from_utf8_lossy(&recipe_bytes).contains("narrow clearing"));
    let (doc, notices) = Document::open(&recipe_path).unwrap();
    assert!(notices.is_empty());
    assert_eq!(doc.recipe().biome, pigment_core::biome::BiomeId::Jungle);
    assert!(doc.prose().is_none());

    let frame = Frame::new(3840, 2160).unwrap();
    let dest = dir.0.join("jungle-4k.png");
    exporter
        .start(export_job(&doc, frame, dest.clone()))
        .unwrap();
    match wait_export(&exporter) {
        ExportOutcome::Written { tiles, .. } => assert!(tiles > 1, "expected tiled export"),
        outcome => panic!("jungle export failed: {outcome:?}"),
    }
    let (width, height, tiled) = decode_rgb(&dest);
    assert_eq!((width, height), (frame.width, frame.height));
    let single = render_rgb(
        &renderer,
        &request(&doc, frame.width, frame.height, TilePolicy::Single),
    );
    assert_eq!(tiled, single, "jungle tile boundaries must be invisible");
}

/// Record real preview and 8K export costs for the densest reviewed jungle
/// scene. The tiled exporter must stay within the established bounded-memory
/// policy; report RSS and time as qualification evidence.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn jungle_preview_and_8k_export_performance_is_recorded() {
    let ctx = ctx();
    let renderer = PaintRenderer::new(ctx.clone()).unwrap();
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx).unwrap(), || {});
    let dir = Scratch::new("jungle-8k");
    let mut doc = Document::from_prose(
        "A narrow clearing beneath a high canopy.",
        Frame::new(1600, 900).unwrap(),
    )
    .unwrap();
    doc.set_biome(pigment_core::biome::BiomeId::Jungle).unwrap();
    doc.set_variation(pigment_core::seed::Variation(7));
    let preview_start = Instant::now();
    let preview = render_rgb(&renderer, &request(&doc, 960, 540, TilePolicy::Single));
    let preview_elapsed = preview_start.elapsed();
    assert_eq!(preview.len(), 960 * 540 * 3);

    let frame = Frame::new(7680, 4320).unwrap();
    let dest = dir.0.join("jungle-8k.png");
    let before = rss();
    let started = Instant::now();
    exporter
        .start(export_job(&doc, frame, dest.clone()))
        .unwrap();
    let outcome = wait_export(&exporter);
    let elapsed = started.elapsed();
    let after = rss();
    let tiles = match outcome {
        ExportOutcome::Written { tiles, .. } => tiles,
        outcome => panic!("jungle 8K export failed: {outcome:?}"),
    };
    let bytes = std::fs::metadata(&dest).unwrap().len();
    eprintln!(
        "jungle preview 960×540: {:.2}s; 8K tiled export: {:.2}s, {tiles} tiles, {:.1} MiB PNG, RSS delta {} MiB",
        preview_elapsed.as_secs_f64(),
        elapsed.as_secs_f64(),
        bytes as f64 / (1u64 << 20) as f64,
        before.zip(after).map_or_else(
            || "unavailable".to_string(),
            |(b, a)| format!("{:.1}", (a as i64 - b as i64) as f64 / (1u64 << 20) as f64)
        ),
    );
    assert!(tiles > 1, "8K must use bounded tiled export");
}

/// Seams: a painting tiled with 333 px tiles (a non-divisor, so boundaries
/// fall everywhere) against the one-tile render, measured in 4 px bands
/// either side of every tile boundary and in the interior, plus the tiled
/// image's own step across each boundary against its steps elsewhere.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn tile_boundaries_show_no_seams() {
    let renderer = PaintRenderer::new(ctx()).unwrap();
    let (mut doc, _) = approved("corpus-16x9", 2);
    // Loose edges have the widest support, the hardest case for aprons.
    let mut a = doc.appearance();
    a.painting.edge_looseness = 1.0;
    doc.set_appearance(a).unwrap();
    let (w, h, tile) = (1998usize, 1124usize, 333usize);
    let single = render_rgb(
        &renderer,
        &request(&doc, w as u32, h as u32, TilePolicy::Single),
    );
    let tiled = render_rgb(
        &renderer,
        &request(
            &doc,
            w as u32,
            h as u32,
            TilePolicy::Fixed { edge: tile as u32 },
        ),
    );
    let near = |v: usize| {
        let m = v % tile;
        v >= tile && (m < 4 || m >= tile - 4)
    };
    let (mut band_max, mut interior_max, mut band_n) = (0u8, 0u8, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let d = (0..3)
                .map(|c| single[i + c].abs_diff(tiled[i + c]))
                .max()
                .unwrap();
            if near(x) || near(y) {
                band_max = band_max.max(d);
                band_n += 1;
            } else {
                interior_max = interior_max.max(d);
            }
        }
    }
    // The tiled image's own luminance step across boundary columns vs all
    // columns: a seam would stand out even without a reference.
    let lum = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        0.2126 * tiled[i] as f64 + 0.7152 * tiled[i + 1] as f64 + 0.0722 * tiled[i + 2] as f64
    };
    let step = |cols: &mut dyn Iterator<Item = usize>| {
        let (mut s, mut n) = (0.0, 0.0);
        for x in cols {
            for y in 0..h {
                s += (lum(x, y) - lum(x - 1, y)).abs();
                n += 1.0;
            }
        }
        s / n
    };
    let at_boundaries = step(&mut (1..w).filter(|x| x % tile == 0));
    let everywhere = step(&mut (1..w));
    let ratio = at_boundaries / everywhere;
    eprintln!(
        "seams: max |tiled − single| {band_max}/255 in {band_n} boundary-band pixels, \
         {interior_max}/255 inside; boundary step {at_boundaries:.2} vs {everywhere:.2} everywhere (×{ratio:.2})"
    );
    assert_eq!(
        (band_max, interior_max),
        (0, 0),
        "tier 2: identical on one device"
    );
    assert!(ratio < 1.5, "boundary columns stand out: ×{ratio:.2}");
}

/// Brush and texture scale are set in canvas units, so the same painting
/// at 1920 and 3840 px has the same composition and the same amount of
/// texture once the larger one is downsampled.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn brush_and_texture_scale_hold_at_two_resolutions() {
    let renderer = PaintRenderer::new(ctx()).unwrap();
    for (sheet, cell) in [("corpus-16x9", 4), ("corpus-1x1", 7)] {
        let (doc, _) = approved(sheet, cell);
        let aspect = doc.recipe().frame.aspect();
        let lo = Frame::largest_with_aspect(aspect, 1920).unwrap();
        let hi = Frame::new(lo.width * 2, lo.height * 2).unwrap();
        let small = to_f64(&render_rgb(
            &renderer,
            &request(&doc, lo.width, lo.height, TilePolicy::Single),
        ));
        let big = render_rgb(
            &renderer,
            &request(&doc, hi.width, hi.height, TilePolicy::Single),
        );
        let big_down = downsample(&big, hi.width as usize, hi.height as usize, 2);
        let (w, h) = (lo.width as usize, lo.height as usize);
        let p = psnr(&big_down, &small);
        // Composition at a coarse scale: 16× smaller.
        let coarse = |v: &[f64]| {
            let u8s: Vec<u8> = v.iter().map(|&x| x.round() as u8).collect();
            downsample(&u8s, w, h, 16)
        };
        let pc = psnr(&coarse(&big_down), &coarse(&small));
        let (e_small, e_big) = (
            texture_energy(&small, w, h),
            texture_energy(&big_down, w, h),
        );
        let ratio = e_big / e_small;
        eprintln!(
            "{sheet} {cell}: {}×{} vs 2×: PSNR {p:.1} dB, coarse {pc:.1} dB, texture energy {e_small:.2} vs {e_big:.2} (×{ratio:.2})",
            lo.width, lo.height
        );
        assert!(p > 24.0, "{sheet} {cell}: PSNR {p}");
        assert!(pc > 32.0, "{sheet} {cell}: composition PSNR {pc}");
        assert!(
            (0.7..1.4).contains(&ratio),
            "{sheet} {cell}: texture ×{ratio}"
        );
    }
}

/// Twenty-five exports back to back with previews running alongside:
/// every export completes, only finished files remain, previews end on the
/// newest request, and process memory stops growing after warm-up.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn repeated_exports_with_previews_stay_bounded() {
    let ctx = ctx();
    let preview = PreviewWorker::spawn(
        PaintRenderer::new(ctx.clone()).unwrap(),
        WorkerOptions::default(),
        || {},
    );
    let mut exporter = Exporter::spawn(PaintRenderer::new(ctx.clone()).unwrap(), || {});
    let dir = Scratch::new("repeat");
    let (doc, _) = approved("corpus-16x9", 6);
    let r = doc.recipe().clone();
    let mut id = 0u64;
    let mut newest = None;
    let mut after_warmup = None;
    for n in 0..25 {
        let mut d = doc.clone();
        let mut a = d.appearance();
        a.painting.wash_gouache = (n % 10) as f64 / 10.0;
        d.set_appearance(a).unwrap();
        let dest = dir.0.join(format!("{:02}.png", n % 5));
        exporter
            .start(export_job(&d, Frame::new(1920, 1080).unwrap(), dest))
            .unwrap();
        // Previews while the export runs, faster than they can finish.
        while exporter.is_running() {
            id += 1;
            let mut ap = d.appearance();
            ap.atmosphere.haze = (id % 20) as f64 / 20.0;
            preview.submit(
                RequestId(id),
                PreviewJob {
                    biome: d.recipe().biome,
                    seeds: d.seeds(),
                    form: r.form,
                    aspect: r.frame.aspect(),
                    appearance: ap,
                    width: 960,
                    height: 540,
                    submitted_at: Instant::now(),
                },
            );
            assert!(preview.in_flight() <= 2);
            for res in preview.drain() {
                newest = Some(res.id);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        match wait_export(&exporter) {
            ExportOutcome::Written { .. } => {}
            o => panic!("export {n}: {o:?}"),
        }
        if n == 4 {
            after_warmup = rss();
        }
    }
    let end = rss();
    // Only the five finished files: no temporaries.
    let mut names: Vec<String> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["00.png", "01.png", "02.png", "03.png", "04.png"]);
    // The newest preview arrives last.
    let deadline = Instant::now() + Duration::from_secs(10);
    // Completion can set in_flight to zero just AFTER the preceding drain.
    // Observe the result identity, not a racy queue/activity snapshot. The
    // final result may also have been drained while an export was running.
    while Instant::now() < deadline && newest != Some(RequestId(id)) {
        for res in preview.drain() {
            newest = Some(res.id);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        newest,
        Some(RequestId(id)),
        "the newest preview is the last one delivered"
    );
    if let (Some(a), Some(b)) = (after_warmup, end) {
        let growth = b as i64 - a as i64;
        eprintln!(
            "25 exports and {id} preview requests: resident memory {:+.1} MiB after warm-up",
            growth as f64 / (1u64 << 20) as f64
        );
        assert!(growth < 64 << 20, "resident memory grew by {growth} bytes");
    }
}
