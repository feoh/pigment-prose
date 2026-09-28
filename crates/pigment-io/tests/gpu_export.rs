//! Hardware export suite (task 09). Ignored by default so portable CI (no
//! GPU) stays green; run with `scripts/gpu-tests.sh` or
//! `cargo test --release -p pigment-io --test gpu_export -- --ignored --test-threads=1`.
//!
//! Tolerance: on one device, driver and backend, tiled export equals the
//! single-tile render **byte for byte** (reproducibility tier 2). The
//! full-resolution vs preview comparisons check composition, not texture,
//! with a PSNR floor, because band-limited paint texture legitimately gains
//! detail at higher resolution.

#![allow(clippy::print_stderr)]

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use pigment_core::capability::AdapterPolicy;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderRequest, RenderTarget, Renderer, RequestIds,
};
use pigment_core::scene::SceneGenerator;
use pigment_core::scene::lakeshore::LakeshoreGenerator;
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::{TileOrder, TilePolicy};
use pigment_gpu::{GpuContext, PaintRenderer};
use pigment_io::{ExportSize, PngCompression, export_png};

fn renderer() -> &'static PaintRenderer {
    static R: OnceLock<PaintRenderer> = OnceLock::new();
    R.get_or_init(|| {
        let ctx = GpuContext::new(&AdapterPolicy::default())
            .unwrap_or_else(|e| panic!("hardware GPU required for this suite: {e}"));
        assert!(
            !ctx.capabilities.adapter.software,
            "software adapter is not GPU evidence"
        );
        eprintln!("GPU export suite on {}", ctx.capabilities.label());
        PaintRenderer::new(Arc::new(ctx)).expect("paint pipelines")
    })
}

static IDS: RequestIds = RequestIds::new();

fn request(text: &str, frame: Frame, policy: TilePolicy) -> RenderRequest {
    let seeds = SeedBundle::derive(TextDigest::from_source(text).unwrap(), Variation(0));
    let scene = LakeshoreGenerator
        .generate(&seeds, &FormSettings::default(), frame.aspect())
        .unwrap();
    RenderRequest {
        id: IDS.next(),
        purpose: RenderPurpose::Export,
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

fn render_memory(req: &RenderRequest) -> Vec<u8> {
    let mut sink = MemorySink::default();
    let rep = renderer()
        .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .expect("render");
    assert_eq!(rep.outcome, RenderOutcome::Completed);
    sink.rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let p =
            std::env::temp_dir().join(format!("pigment-gpu-export-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }

    fn entries(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn export(req: &RenderRequest, dest: &Path) -> pigment_io::ExportReport {
    let t = Instant::now();
    let rep = export_png(
        renderer(),
        req,
        dest,
        PngCompression::Fast,
        &CancelToken::new(),
        &mut NoProgress,
    )
    .expect("export");
    assert_eq!(rep.render.outcome, RenderOutcome::Completed);
    eprintln!(
        "{}x{} export, {}x{} tiles of {} px, apron {}: {:.0} ms ({:.0} ms PNG), {:.1} MiB",
        req.target.width,
        req.target.height,
        rep.render.plan.cols,
        rep.render.plan.rows,
        rep.render.plan.tile_w,
        rep.render.plan.apron,
        t.elapsed().as_secs_f64() * 1e3,
        rep.render.timings.sink.as_secs_f64() * 1e3,
        rep.bytes as f64 / (1 << 20) as f64
    );
    rep
}

/// Chunk types of a PNG file, in order.
fn chunk_types(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let (mut i, mut out) = (8, Vec::new());
    while i < bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&bytes[i + 4..i + 8]).into_owned());
        i += 12 + len;
    }
    out
}

fn assert_clean_metadata(path: &Path) {
    let chunks = chunk_types(path);
    let mut kinds: Vec<&str> = chunks.iter().map(String::as_str).collect();
    kinds.dedup();
    assert_eq!(kinds, ["IHDR", "sRGB", "IDAT", "IEND"], "{path:?}");
}

/// Decodes `path` row by row, box-averaging `k`×`k` blocks (sRGB values),
/// so an 8K file is checked without holding it in memory.
fn decode_downsampled(path: &Path, k: u32) -> (u32, u32, Vec<f64>) {
    let dec = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
    let mut r = dec.read_info().unwrap();
    let (w, h) = (r.info().width, r.info().height);
    assert_eq!(r.info().color_type, png::ColorType::Rgb);
    assert_eq!(r.info().bit_depth, png::BitDepth::Eight);
    assert!(w % k == 0 && h % k == 0);
    let (ow, oh) = (w / k, h / k);
    let mut acc = vec![0.0; (ow * oh * 3) as usize];
    let mut row = vec![0; r.output_line_size(w).unwrap()];
    for y in 0..h {
        r.read_row(&mut row).unwrap().expect("row");
        let base = ((y / k) * ow * 3) as usize;
        for x in 0..w as usize {
            for c in 0..3 {
                acc[base + (x / k as usize) * 3 + c] += row[x * 3 + c] as f64;
            }
        }
    }
    let n = (k * k) as f64;
    acc.iter_mut().for_each(|v| *v /= n);
    (w, h, acc)
}

fn psnr(a: &[f64], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let se: f64 = a.iter().zip(b).map(|(x, y)| (x - *y as f64).powi(2)).sum();
    10.0 * (255.0f64 * 255.0 / (se / a.len() as f64)).log10()
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn tiled_png_export_equals_the_single_tile_render() {
    // Wide washes (sky, water), tile-crossing trees and rocks, and corners:
    // river valley (15), lake with rocks (0), framing ridges with rocks (8).
    let dir = Scratch::new("tiled");
    let frame = Frame::new(1001, 563).unwrap();
    for text in ["sample passage 15", "sample passage 0", "sample passage 8"] {
        for looseness in [0.0, 0.4, 1.0] {
            let mut single = request(text, frame, TilePolicy::Single);
            single.appearance.painting.edge_looseness = looseness;
            let reference = render_memory(&single);
            for (policy, order) in [
                (TilePolicy::Fixed { edge: 256 }, TileOrder::RowMajor),
                (TilePolicy::Fixed { edge: 333 }, TileOrder::ReverseInBand),
                (TilePolicy::default_export(), TileOrder::ReverseInBand),
            ] {
                let mut tiled = single.clone();
                tiled.target.policy = policy;
                tiled.target.order = order;
                let dest = dir.0.join("t.png");
                export(&tiled, &dest);
                assert_clean_metadata(&dest);
                let (w, h, rgb) = decode_downsampled(&dest, 1);
                assert_eq!((w, h), (1001, 563));
                let exact = rgb.iter().zip(&reference).all(|(a, b)| *a == *b as f64);
                assert!(exact, "{text:?} looseness {looseness} {policy:?} {order:?}");
            }
        }
    }
    assert_eq!(dir.entries(), vec!["t.png".to_string()]);
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn eight_k_export_keeps_the_preview_composition() {
    let dir = Scratch::new("8k");
    let text = "sample passage 15";
    let probe = request(text, Frame::new(960, 540).unwrap(), TilePolicy::Single);
    let frame = ExportSize::Uhd8k.frame(probe.scene.key().aspect).unwrap();
    assert_eq!((frame.width, frame.height), (7680, 4320));
    let mut req = probe.clone();
    req.target = RenderTarget {
        width: frame.width,
        height: frame.height,
        policy: TilePolicy::default_export(),
        order: TileOrder::RowMajor,
    };
    let dest = dir.0.join("8k.png");
    let rep = export(&req, &dest);
    assert!(rep.render.plan.len() > 1, "8K must be tiled");
    assert_clean_metadata(&dest);
    let (w, h, down) = decode_downsampled(&dest, 8);
    assert_eq!((w, h), (7680, 4320));
    let preview = render_memory(&probe);
    let p = psnr(&down, &preview);
    eprintln!("960x540 preview vs 8K downsampled 8x: PSNR {p:.1} dB");
    assert!(p > 24.0, "PSNR {p}");
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn custom_portrait_export_keeps_the_preview_composition() {
    let dir = Scratch::new("portrait");
    let text = "sample passage 0";
    let probe = request(text, Frame::new(600, 800).unwrap(), TilePolicy::Single);
    let frame = ExportSize::Custom {
        width: 3000,
        height: 4000,
    }
    .frame(probe.scene.key().aspect)
    .unwrap();
    let mut req = probe.clone();
    req.target = RenderTarget {
        width: frame.width,
        height: frame.height,
        policy: TilePolicy::Fixed { edge: 1000 },
        order: TileOrder::ReverseInBand,
    };
    let dest = dir.0.join("portrait.png");
    export(&req, &dest);
    assert_clean_metadata(&dest);
    let (w, h, down) = decode_downsampled(&dest, 5);
    assert_eq!((w, h), (3000, 4000));
    let p = psnr(&down, &render_memory(&probe));
    eprintln!("600x800 preview vs 3000x4000 downsampled 5x: PSNR {p:.1} dB");
    assert!(p > 24.0, "PSNR {p}");
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn cancelled_export_leaves_the_old_file_and_no_partial() {
    let dir = Scratch::new("cancel");
    let dest = dir.0.join("keep.png");
    std::fs::write(&dest, b"previous export").unwrap();
    let req = request(
        "sample passage 8",
        Frame::new(3840, 2160).unwrap(),
        TilePolicy::Fixed { edge: 512 },
    );
    let cancel = CancelToken::new();
    let c2 = cancel.clone();
    let mut on_progress = move |p: Progress| {
        if p.done >= 2 {
            c2.cancel();
        }
    };
    let rep = export_png(
        renderer(),
        &req,
        &dest,
        PngCompression::Fast,
        &cancel,
        &mut on_progress,
    )
    .unwrap();
    assert_eq!(
        rep.render.outcome,
        RenderOutcome::Cancelled { tiles_done: 2 }
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"previous export");
    assert_eq!(dir.entries(), vec!["keep.png".to_string()]);
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn impossible_budgets_and_bad_destinations_are_errors() {
    let dir = Scratch::new("errors");
    let mut req = request(
        "sample passage 8",
        Frame::new(3840, 2160).unwrap(),
        TilePolicy::Budget {
            gpu_bytes: 1 << 20,
            host_bytes: 256 << 20,
        },
    );
    let run = |req: &RenderRequest, dest: &Path| {
        export_png(
            renderer(),
            req,
            dest,
            PngCompression::Fast,
            &CancelToken::new(),
            &mut NoProgress,
        )
    };
    let e = run(&req, &dir.0.join("x.png")).unwrap_err();
    assert!(e.to_string().contains("memory budget"), "{e}");
    req.target.policy = TilePolicy::default_export();
    let e = run(&req, &dir.0.join("missing/x.png")).unwrap_err();
    assert!(e.to_string().contains("cannot write"), "{e}");
    req.target.height = 2161;
    let e = run(&req, &dir.0.join("x.png")).unwrap_err();
    assert!(e.to_string().contains("aspect ratio"), "{e}");
    assert!(dir.entries().is_empty());
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A preview-size request for `doc`, as the contact sheet renders cells.
fn document_request(doc: &pigment_io::Document, policy: TilePolicy) -> RenderRequest {
    let r = doc.recipe();
    let scene = LakeshoreGenerator
        .generate(&doc.seeds(), &r.form, r.frame.aspect())
        .unwrap();
    RenderRequest {
        id: IDS.next(),
        purpose: RenderPurpose::Export,
        scene: Arc::new(scene),
        seeds: doc.seeds(),
        appearance: doc.appearance(),
        target: RenderTarget {
            width: r.frame.width,
            height: r.frame.height,
            policy,
            order: TileOrder::RowMajor,
        },
    }
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn approved_recipes_repaint_identically_after_save_and_load() {
    // Every approved baseline recipe (task 25), saved and reopened, paints the same
    // pixels as the original file. On the baseline device (RTX 4070 Ti,
    // Vulkan) they must also match the approved image hashes.
    let dir = Scratch::new("baseline-paint");
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/visual-review/baseline-25");
    let label = renderer_label();
    let baseline_device = label == "NVIDIA GeForce RTX 4070 Ti (Vulkan)";
    let mut checked = 0;
    for sheet in [
        "corpus-16x9",
        "corpus-9x16",
        "corpus-1x1",
        "round-06-scenes",
    ] {
        let notes = std::fs::read_to_string(base.join(format!("{sheet}.txt"))).unwrap();
        let mut lines = notes.lines().skip_while(|l| !l.starts_with("cell\t"));
        let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
        let fnv_col = header.iter().position(|h| *h == "image fnv").unwrap();
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split('\t').collect();
            let cell: usize = f[0].parse().unwrap();
            let path = base.join(sheet).join(format!("{cell:02}.recipe.json"));
            let (original, _) = pigment_io::Document::open(&path).unwrap();
            let copy = dir.0.join("copy.recipe.json");
            original.clone().save_as(&copy).unwrap();
            let (reopened, _) = pigment_io::Document::open(&copy).unwrap();
            let a = render_memory(&document_request(&original, TilePolicy::Single));
            let b = render_memory(&document_request(
                &reopened,
                TilePolicy::Fixed { edge: 128 },
            ));
            assert!(a == b, "{sheet} {cell}: reopened recipe paints differently");
            if baseline_device {
                let mut sink = MemorySink::default();
                renderer()
                    .render(
                        &document_request(&reopened, TilePolicy::Single),
                        &CancelToken::new(),
                        &mut NoProgress,
                        &mut sink,
                    )
                    .unwrap();
                assert_eq!(
                    format!("{:016x}", fnv(&sink.rgba8)),
                    f[fnv_col],
                    "{sheet} {cell}: differs from the approved baseline"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 47);
    if !baseline_device {
        eprintln!(
            "{label} is not the baseline device: checked reopened == original only, \
             not the approved image hashes"
        );
    }
}

fn renderer_label() -> String {
    // The renderer's report carries the device label.
    let req = request(
        "label probe",
        Frame::new(64, 64).unwrap(),
        TilePolicy::Single,
    );
    let mut sink = MemorySink::default();
    renderer()
        .render(&req, &CancelToken::new(), &mut NoProgress, &mut sink)
        .unwrap()
        .device
}

#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn kept_prose_never_reaches_the_exported_png() {
    const PROSE: &str = "Zebra lantern marigolds drift past the quiet jetty at dusk.";
    let dir = Scratch::new("privacy");
    let mut doc = pigment_io::Document::from_prose(PROSE, Frame::new(1600, 900).unwrap()).unwrap();
    doc.set_keep_source_text(true);
    let recipe_path = dir.0.join("kept.recipe.json");
    doc.save_as(&recipe_path).unwrap();
    assert!(
        std::fs::read_to_string(&recipe_path)
            .unwrap()
            .contains(PROSE)
    );
    let (doc, _) = pigment_io::Document::open(&recipe_path).unwrap();
    assert_eq!(doc.prose(), Some(PROSE));
    let png = dir.0.join("out.png");
    export(&document_request(&doc, TilePolicy::default_export()), &png);
    assert_clean_metadata(&png);
    let bytes = std::fs::read(&png).unwrap();
    for needle in [
        "Zebra",
        "zebra",
        "marigold",
        "jetty",
        "source_text",
        "recipe",
    ] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle} in the PNG"
        );
    }
}
