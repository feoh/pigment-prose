//! `pigment-prose`: diagnostics CLI. Reads no user prose: `gpu-*` seeds are
//! numbers, and `contact-sheet` reads only the synthetic fixture corpus or
//! generated sample labels. `export --recipe` uses a recipe's digest and
//! settings; a recipe's optional `source_text` is never printed or used.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pigment_core::capability::{AdapterPolicy, AdapterReport};
use pigment_core::frame::AspectRatio;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::recipe::Recipe;
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderReport, RenderRequest, RenderTarget, Renderer,
    RequestIds,
};
use pigment_core::scene::lakeshore::{Composition, LakeshoreGenerator};
use pigment_core::scene::{
    CanvasPoint, LayerRole, MAX_LAYERS, MAX_SCENE_VERTICES, Plant, Scene, SceneGenerator, SceneKey,
    SceneLayer, TestCard, diagnostic_seeds, metrics, raster,
};
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::{TileOrder, TilePolicy};
use pigment_core::version;
use pigment_gpu::{DebugRenderer, DebugView, GpuContext, PaintRenderer, SmokeRenderer, adapter};
use pigment_io::{ExportSize, PngCompression, export_png};

const USAGE: &str = "\
pigment-prose <command> [options]

  gpu-info [--gl] [--allow-software] [--adapter NAME]
      List adapters with type, driver and key limits, and show which one
      would be selected.
  gpu-smoke [--width W] [--height H] [--tile T] [--seed S] [--looseness L]
            [--adapter NAME] [--allow-software] [--gl] [--ppm OUT.ppm]
      Open the device and render the diagnostic test card as one tile and
      tiled; require byte-identical output, test cancellation and time a warm
      preview. Exit status 0 only if every check passes on a hardware GPU.
      Defaults: 1920x1080, tile 512, seed 7, looseness 0.4.
  contact-sheet --out SHEET.png [--aspect W:H] [--view paint|flat|regions]
            [--cell PX] [--cols N] [--variation V] [--first N]
            [--passages FILE | --passage ID --variations N | --sample N
             | --samples I,J,...]
            [--faceting F] [--relief R] [--density D] [--adapter NAME]
            [--palette lakeshore|golden-evening] [--looseness L] [--wash W]
            [--haze H] [--intensity I] [--marks M] [--grain G] [--granulation G]
            [--vary KEY=V1,V2,...] [--cells DIR]
      Render lakeshore scenes (painted, or the structure debug views) into a
      grid PNG, plus SHEET.txt listing each cell's passage id,
      variation, template, geometry checksum and visible coverage by role.
      Cells: the corpus (default fixtures/passages.json) at one variation,
      one passage's variations 0..N-1, N generated sample seeds, or the
      listed sample seeds in the given order (for rating rounds).
      Cells are numbered in their corner from --first (default 1), so
      numbers can run on across the sheets of one review round.
      --vary renders every cell once per value of one setting (KEY is a
      form or paint option name above, e.g. wash=0,1 or relief=0,1), one row
      per cell. --cells also writes each cell as DIR/NN.png with the recipe
      that reproduces it (DIR/NN.recipe.json, no source text); the notes
      end with each image's FNV-1a hash.
      Defaults: 16:9, paint, cell 480 px long side, variation 0, 5 columns,
      form and paint settings at their defaults.
  paint-bench [--sizes 960,1920,3840] [--sample N] [--aspect W:H] [--runs R]
            [--stress] [--adapter NAME]
      Time scene generation and warm single-tile painting (render + readback)
      of N sample seeds at each long-edge size, and report the median and
      worst per-scene medians against the preview budgets. --stress times
      one synthetic coverage worst case (every layer and vertex the scene
      limits allow, every bounding box over the whole frame) instead.
      Defaults: sizes 960,1920,3840, 12 samples, 16:9, 7 runs (first 2 are
      warm-up).
  bench [--runs N] [--sample N] [--adapter NAME] [--allow-software] [--strict]
      Qualification benchmark (task 14) against the provisional targets in
      docs/architecture.md: cold start (device, pipelines, first preview),
      warm interaction (960 px) and settled (1920, 3840 px) previews, a
      prose edit to a settled preview (new scene + render), 4K and 8K
      exports including PNG encoding, and export cancel latency. Prints
      the environment and a table of measured p95 vs target. --strict
      exits non-zero if any target is missed. A software adapter is
      labelled and never counts as a pass. Defaults: 9 runs, 5 seeds.
  export --out FILE.png (--recipe R.recipe.json | --sample N | --passage ID)
            [--passages FILE] [--variation V] [--aspect W:H]
            [--size 4k|8k|WxH] [--tile T | --gpu-budget MIB] [--order reverse]
            [--compression fast|balanced] [--cancel-after TILES]
            [--adapter NAME] [form and paint options as for contact-sheet]
      Paint one scene at full resolution, tile by tile, straight into a PNG
      (bounded GPU and host memory; written beside FILE and renamed into
      place only when complete). A recipe supplies seed, variation, form,
      paint settings and aspect ratio; its frame is the default size. Sizes
      must match the scene's aspect ratio exactly; 4k/8k pick the largest
      exact-aspect frame with a long edge of 3840/7680. Prints the tile plan,
      timings, the cost model's GPU/host estimates and the measured peak
      host memory. --cancel-after cancels once that many tiles are done
      (the partial file must disappear). Defaults: --sample 0, 16:9, 8k,
      the export memory budget, fast compression.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("gpu-info") => parse(&args[1..]).and_then(|o| gpu_info(&o)),
        Some("gpu-smoke") => parse(&args[1..]).and_then(|o| gpu_smoke(&o)),
        Some("contact-sheet") => parse(&args[1..]).and_then(|o| contact_sheet(&o)),
        Some("paint-bench") => parse(&args[1..]).and_then(|o| paint_bench(&o)),
        Some("bench") => parse(&args[1..]).and_then(|o| bench(&o)),
        Some("export") => parse(&args[1..]).and_then(|o| export(&o)),
        Some("--version") => {
            println!("pigment-prose {}", version::APP_VERSION);
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Opts(HashMap<String, String>);

const BOOL_FLAGS: &[&str] = &["gl", "allow-software", "stress", "strict"];

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut m = HashMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let k = a
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {a:?}\n\n{USAGE}"))?;
        if BOOL_FLAGS.contains(&k) {
            m.insert(k.to_string(), String::new());
        } else {
            let v = it.next().ok_or_else(|| format!("--{k} needs a value"))?;
            m.insert(k.to_string(), v.clone());
        }
    }
    Ok(Opts(m))
}

impl Opts {
    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        self.0.get(k).map_or(Ok(default), |v| {
            v.parse().map_err(|_| format!("--{k}: bad number {v:?}"))
        })
    }

    fn policy(&self) -> AdapterPolicy {
        AdapterPolicy {
            name_filter: self.0.get("adapter").cloned(),
            allow_software: self.0.contains_key("allow-software"),
            include_gl: self.0.contains_key("gl"),
        }
    }
}

fn describe(r: &AdapterReport) -> String {
    let l = &r.adapter_limits;
    format!(
        "{name}\n    backend={b:?} type={t:?} software={sw} vendor=0x{v:04x} device=0x{d:04x}\n    \
         driver={drv} ({info})\n    max_texture_dimension_2d={tex} max_buffer_size={mb} \
         max_storage_buffer_binding_size={sb}\n    max_compute_workgroup_size=({wx},{wy}) \
         max_compute_invocations_per_workgroup={wi} max_storage_textures_per_shader_stage={st}",
        name = r.name,
        b = r.backend,
        t = r.kind,
        sw = r.software,
        v = r.vendor_id,
        d = r.device_id,
        drv = r.driver,
        info = r.driver_info,
        tex = l.max_texture_dimension_2d,
        mb = l.max_buffer_size,
        sb = l.max_storage_buffer_binding_size,
        wx = l.max_compute_workgroup_size_x,
        wy = l.max_compute_workgroup_size_y,
        wi = l.max_compute_invocations_per_workgroup,
        st = l.max_storage_textures_per_shader_stage,
    )
}

fn gpu_info(o: &Opts) -> Result<(), String> {
    let policy = o.policy();
    let instance = adapter::instance(policy.include_gl);
    let all = adapter::enumerate(&instance, &policy);
    println!(
        "pigment-prose {} (wgpu {}); {} adapter(s)",
        version::APP_VERSION,
        pigment_gpu::context::WGPU_VERSION,
        all.len()
    );
    for (_, r) in &all {
        println!("- {}", describe(r));
    }
    match adapter::select(&instance, &policy) {
        Ok((_, r)) => println!("selected: {} ({:?})", r.name, r.backend),
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// FNV-1a 64 of the image bytes, for evidence logs.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

fn gpu_smoke(o: &Opts) -> Result<(), String> {
    let width: u32 = o.num("width", 1920)?;
    let height: u32 = o.num("height", 1080)?;
    let tile: u32 = o.num("tile", 512)?;
    let seed: u64 = o.num("seed", 7)?;
    let looseness: f64 = o.num("looseness", 0.4)?;
    let frame = Frame::new(width, height).map_err(|e| e.to_string())?;

    let t_init = Instant::now();
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let renderer = SmokeRenderer::new(ctx.clone()).map_err(|e| e.to_string())?;
    let caps = &ctx.capabilities;
    println!("pigment-prose {} gpu-smoke", version::APP_VERSION);
    println!("wgpu: {}", caps.wgpu_version);
    println!("adapter: {}", describe(&caps.adapter));
    println!(
        "device limits requested: WebGPU defaults (max_texture_dimension_2d={})",
        caps.device_limits.max_texture_dimension_2d
    );
    println!("device init + pipelines: {}", ms(t_init.elapsed()));
    if caps.adapter.software {
        println!("WARNING: software adapter; results are NOT GPU evidence");
    }

    let seeds = diagnostic_seeds(seed);
    let scene = Arc::new(
        TestCard
            .generate(&seeds, &FormSettings::default(), frame.aspect())
            .map_err(|e| e.to_string())?,
    );
    let mut appearance = Appearance::default();
    appearance.painting.edge_looseness = looseness;
    appearance.validate().map_err(|e| e.to_string())?;
    println!(
        "scene: test card v{}, seed {seed}, aspect {}:{}, geometry checksum {:016x}",
        TestCard::VERSION,
        frame.aspect().width,
        frame.aspect().height,
        scene.geometry_checksum()
    );

    let ids = RequestIds::default();
    let request = |policy: TilePolicy, w: u32, h: u32, purpose| RenderRequest {
        id: ids.next(),
        purpose,
        scene: scene.clone(),
        seeds,
        appearance,
        target: RenderTarget {
            width: w,
            height: h,
            policy,
            order: TileOrder::RowMajor,
        },
    };
    let run = |req: &RenderRequest| -> Result<(RenderReport, MemorySink), String> {
        let mut sink = MemorySink::default();
        let rep = renderer
            .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
            .map_err(|e| e.to_string())?;
        Ok((rep, sink))
    };

    let mut failures = Vec::new();
    let (ref_rep, reference) = run(&request(
        TilePolicy::Single,
        width,
        height,
        RenderPurpose::Export,
    ))?;
    println!(
        "single tile {width}x{height}: apron {} px, {} tile, GPU tile bytes {}, render+readback {}, total {}, fnv {:016x}",
        ref_rep.plan.apron,
        ref_rep.plan.len(),
        ref_rep.plan.gpu_bytes,
        ms(ref_rep.timings.render_readback),
        ms(ref_rep.timings.total),
        fnv(&reference.rgba8)
    );
    if !reference.finished
        || reference
            .rgba8
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] != 255)
    {
        failures.push("reference image incomplete or not opaque".to_string());
    }
    let (_, again) = run(&request(
        TilePolicy::Single,
        width,
        height,
        RenderPurpose::Export,
    ))?;
    let repeat_ok = again.rgba8 == reference.rgba8;
    println!(
        "repeat single tile: {}",
        if repeat_ok { "identical" } else { "DIFFERENT" }
    );
    if !repeat_ok {
        failures.push("single-tile render not repeatable".into());
    }

    let policies = [
        TilePolicy::Fixed { edge: tile },
        TilePolicy::Fixed { edge: 333 },
        TilePolicy::default_export(),
    ];
    for policy in policies {
        let (rep, img) = run(&request(policy, width, height, RenderPurpose::Export))?;
        let (mut max, mut n) = (0u8, 0u64);
        for (a, b) in img
            .rgba8
            .as_chunks::<4>()
            .0
            .iter()
            .zip(reference.rgba8.as_chunks::<4>().0.iter())
        {
            let d = a
                .iter()
                .zip(b)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            if d > 0 {
                n += 1;
                max = max.max(d);
            }
        }
        let verdict = if n == 0 {
            "identical".to_string()
        } else {
            format!("DIFFERS: {n} px, max {max}")
        };
        println!(
            "{policy:?}: {}x{} tiles of {}x{}, apron {}, GPU tile bytes {}, render+readback {}, total {} -> {verdict}",
            rep.plan.cols,
            rep.plan.rows,
            rep.plan.tile_w,
            rep.plan.tile_h,
            rep.plan.apron,
            rep.plan.gpu_bytes,
            ms(rep.timings.render_readback),
            ms(rep.timings.total)
        );
        if n != 0 {
            failures.push(format!("{policy:?} differs from single tile"));
        }
    }

    // Cancellation: cancel from the progress callback after the first tile.
    let cancel = CancelToken::new();
    let mut sink = MemorySink::default();
    let req = request(
        TilePolicy::Fixed { edge: 256 },
        width,
        height,
        RenderPurpose::Export,
    );
    let c2 = cancel.clone();
    let mut on_progress = move |p: Progress| {
        if p.done >= 1 {
            c2.cancel();
        }
    };
    let rep = renderer
        .render(&req, &cancel, &mut on_progress, &mut sink)
        .map_err(|e| e.to_string())?;
    let cancel_ok =
        rep.outcome == RenderOutcome::Cancelled { tiles_done: 1 } && sink.aborted && !sink.finished;
    println!(
        "cancellation after tile 1 of {}: {:?}, sink aborted={} -> {}",
        rep.plan.len(),
        rep.outcome,
        sink.aborted,
        if cancel_ok { "ok" } else { "WRONG" }
    );
    if !cancel_ok {
        failures.push("cancellation did not stop at the next tile boundary".into());
    }

    // Warm preview timing at 1280x720 (test card; not painting performance).
    let (pw, ph) = frame.fit_within(1280);
    let mut samples = Vec::new();
    for i in 0..25 {
        let (rep, _) = run(&request(TilePolicy::Single, pw, ph, RenderPurpose::Preview))?;
        if i >= 5 {
            samples.push(rep.timings.total);
        }
    }
    samples.sort();
    println!(
        "preview {pw}x{ph} test card, warm (n={}): median {}, p95 {}",
        samples.len(),
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() * 95 / 100])
    );

    if let Some(path) = o.0.get("ppm") {
        let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
        for p in reference.rgba8.as_chunks::<4>().0.iter() {
            out.extend_from_slice(&p[..3]);
        }
        std::fs::write(path, out).map_err(|e| format!("writing {path}: {e}"))?;
        println!("wrote {path}");
    }

    if caps.adapter.software {
        failures.push("software adapter: not GPU evidence".into());
    }
    if failures.is_empty() {
        println!("RESULT: PASS on {}", caps.label());
        Ok(())
    } else {
        println!("RESULT: FAIL");
        Err(failures.join("; "))
    }
}

struct Cell {
    label: String,
    digest: TextDigest,
    variation: u32,
    seeds: SeedBundle,
    form: FormSettings,
    appearance: Appearance,
}

/// Applies one `contact-sheet` form option (`--faceting` etc.) by name.
fn set_form(f: &mut FormSettings, key: &str, v: &str) -> Result<bool, String> {
    let num =
        || -> Result<f64, String> { v.parse().map_err(|_| format!("--{key}: bad number {v:?}")) };
    match key {
        "faceting" => f.faceting = num()?,
        "relief" => f.relief = num()?,
        "density" => f.woodland_density = num()?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Applies one `contact-sheet` paint option (`--looseness` etc.) by name.
fn set_paint(a: &mut Appearance, key: &str, v: &str) -> Result<(), String> {
    let num =
        || -> Result<f64, String> { v.parse().map_err(|_| format!("--{key}: bad number {v:?}")) };
    match key {
        "looseness" => a.painting.edge_looseness = num()?,
        "wash" => a.painting.wash_gouache = num()?,
        "marks" => a.painting.mark_scale = num()?,
        "grain" => a.painting.paper_grain = num()?,
        "granulation" => a.painting.granulation = num()?,
        "haze" => a.atmosphere.haze = num()?,
        "intensity" => a.palette.intensity = num()?,
        "palette" => {
            a.palette.id = serde_json::from_value(serde_json::Value::String(v.to_string()))
                .map_err(|_| format!("--palette {v:?}: expected lakeshore or golden-evening"))?;
        }
        _ => return Err(format!("unknown paint setting {key:?}")),
    }
    Ok(())
}

const PAINT_KEYS: [&str; 8] = [
    "looseness",
    "wash",
    "marks",
    "grain",
    "granulation",
    "haze",
    "intensity",
    "palette",
];

fn corpus(path: &str) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path}: not JSON ({e})"))?;
    doc.as_array()
        .ok_or_else(|| format!("{path}: expected an array of {{id, text}}"))?
        .iter()
        .map(|p| match (p["id"].as_str(), p["text"].as_str()) {
            (Some(id), Some(t)) => Ok((id.to_string(), t.to_string())),
            _ => Err(format!("{path}: every entry needs string id and text")),
        })
        .collect()
}

fn parse_aspect(s: &str) -> Result<AspectRatio, String> {
    let (w, h) = s
        .split_once(':')
        .ok_or_else(|| format!("--aspect {s:?}: expected W:H"))?;
    let (w, h): (u32, u32) = (
        w.parse()
            .map_err(|_| format!("--aspect {s:?}: bad width"))?,
        h.parse()
            .map_err(|_| format!("--aspect {s:?}: bad height"))?,
    );
    if w == 0 || h == 0 || w.max(h) as f64 / w.min(h) as f64 > pigment_core::frame::MAX_ASPECT {
        return Err(format!("--aspect {s:?}: out of range"));
    }
    Ok(AspectRatio::of(w, h))
}

fn contact_sheet(o: &Opts) -> Result<(), String> {
    let out =
        o.0.get("out")
            .ok_or("contact-sheet needs --out SHEET.png")?;
    let aspect = parse_aspect(o.0.get("aspect").map_or("16:9", String::as_str))?;
    let view = match o.0.get("view").map_or("paint", String::as_str) {
        "paint" => None,
        "flat" => Some(DebugView::Flat),
        "regions" => Some(DebugView::Regions),
        v => return Err(format!("--view {v:?}: expected paint, flat or regions")),
    };
    let mut appearance = Appearance::default();
    for key in PAINT_KEYS {
        if let Some(v) = o.0.get(key) {
            set_paint(&mut appearance, key, v)?;
        }
    }
    appearance.validate().map_err(|e| e.to_string())?;
    let cell: u32 = o.num("cell", 480)?;
    let cols_opt: Option<usize> = o.0.get("cols").map(|_| o.num("cols", 5)).transpose()?;
    let first: usize = o.num("first", 1)?;
    let cells_dir = o.0.get("cells").map(std::path::Path::new);
    if let Some(dir) = cells_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let variation: u32 = o.num("variation", 0)?;
    let form = FormSettings {
        faceting: o.num("faceting", FormSettings::default().faceting)?,
        relief: o.num("relief", FormSettings::default().relief)?,
        woodland_density: o.num("density", FormSettings::default().woodland_density)?,
    };
    form.validate().map_err(|e| e.to_string())?;
    let make = |label: String, text: &str, v: u32| -> Result<Cell, String> {
        let digest = TextDigest::from_source(text).map_err(|e| e.to_string())?;
        Ok(Cell {
            label,
            digest,
            variation: v,
            seeds: SeedBundle::derive(digest, Variation(v)),
            form,
            appearance,
        })
    };
    let passages =
        o.0.get("passages")
            .map_or("fixtures/passages.json", String::as_str);
    let mut cells = Vec::new();
    if let Some(list) = o.0.get("samples") {
        for part in list.split(',') {
            let i: u32 = part
                .trim()
                .parse()
                .map_err(|_| format!("--samples {list:?}: bad number {part:?}"))?;
            let label = format!("sample passage {i}");
            cells.push(make(label.clone(), &label, variation)?);
        }
    } else if let Some(n) = o.0.get("sample") {
        let n: u32 = n
            .parse()
            .map_err(|_| format!("--sample {n:?}: bad number"))?;
        for i in 0..n {
            let label = format!("sample passage {i}");
            cells.push(make(label.clone(), &label, variation)?);
        }
    } else if let Some(id) = o.0.get("passage") {
        let n: u32 = o.num("variations", 10)?;
        let (_, text) = corpus(passages)?
            .into_iter()
            .find(|(pid, _)| pid == id)
            .ok_or_else(|| format!("no passage {id:?} in {passages}"))?;
        for v in 0..n {
            cells.push(make(format!("{id} v{v}"), &text, v)?);
        }
    } else {
        for (id, text) in corpus(passages)? {
            cells.push(make(format!("{id} v{variation}"), &text, variation)?);
        }
    }
    if cells.is_empty() {
        return Err("no cells to render".into());
    }
    // --vary KEY=V1,V2,...: each cell once per value, side by side.
    let mut cols_default = 5;
    if let Some(spec) = o.0.get("vary") {
        let (key, values) = spec
            .split_once('=')
            .ok_or_else(|| format!("--vary {spec:?}: expected KEY=V1,V2,..."))?;
        let values: Vec<&str> = values.split(',').map(str::trim).collect();
        let mut varied = Vec::new();
        for c in &cells {
            for v in &values {
                let (mut appearance, mut form) = (c.appearance, c.form);
                if !set_form(&mut form, key, v)? {
                    set_paint(&mut appearance, key, v)?;
                }
                appearance.validate().map_err(|e| e.to_string())?;
                form.validate().map_err(|e| e.to_string())?;
                varied.push(Cell {
                    label: format!("{} {key}={v}", c.label),
                    digest: c.digest,
                    variation: c.variation,
                    seeds: c.seeds,
                    form,
                    appearance,
                });
            }
        }
        cells = varied;
        cols_default = values.len();
    }

    let ext = aspect.extents();
    let (cw, ch) = if ext.width >= ext.height {
        (cell, ((cell as f64) / ext.width).round() as u32)
    } else {
        (((cell as f64) / ext.height).round() as u32, cell)
    };
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let renderer: Box<dyn Renderer> = match view {
        Some(v) => Box::new(DebugRenderer::new(ctx.clone(), v).map_err(|e| e.to_string())?),
        None => Box::new(PaintRenderer::new(ctx.clone()).map_err(|e| e.to_string())?),
    };
    let cols = cols_opt.unwrap_or(cols_default).clamp(1, cells.len());
    let rows = cells.len().div_ceil(cols);
    const GAP: u32 = 6;
    let (sw, sh) = (
        cols as u32 * cw + (cols as u32 + 1) * GAP,
        rows as u32 * ch + (rows as u32 + 1) * GAP,
    );
    let mut sheet = vec![0x80u8; sw as usize * sh as usize * 3];
    let ids = RequestIds::default();
    let mut notes = format!(
        "pigment-prose {} contact-sheet: lakeshore generator v{}, renderer v{}, view {}, aspect {}:{}, \
         cell {cw}x{ch}, form faceting={} relief={} woodland_density={}\nappearance: {appearance:?}\ndevice: {}\n\
         cell\tlabel\ttemplate\tmirrored\tlayers\tvertices\tchecksum\t{}\tcoverage % ({})\timage fnv\n",
        version::APP_VERSION,
        version::GENERATOR_VERSION,
        version::RENDERER_VERSION,
        view.map_or("paint".to_string(), |v| format!("{v:?}")),
        aspect.width,
        aspect.height,
        form.faceting,
        form.relief,
        form.woodland_density,
        ctx.capabilities.label(),
        metrics::COLUMNS,
        LayerRole::ALL.map(|r| r.name()).join(" "),
    );
    let t0 = Instant::now();
    for (i, c) in cells.iter().enumerate() {
        let scene = Arc::new(
            LakeshoreGenerator
                .generate(&c.seeds, &c.form, aspect)
                .map_err(|e| format!("{}: {e}", c.label))?,
        );
        let comp = Composition::draw(&c.seeds, aspect);
        let req = RenderRequest {
            id: ids.next(),
            purpose: RenderPurpose::Preview,
            scene: scene.clone(),
            seeds: c.seeds,
            appearance: c.appearance,
            target: RenderTarget {
                width: cw,
                height: ch,
                policy: TilePolicy::Single,
                order: TileOrder::RowMajor,
            },
        };
        let mut sink = MemorySink::default();
        renderer
            .render(&req, &CancelToken::new(), &mut NoProgress, &mut sink)
            .map_err(|e| e.to_string())?;
        if let Some(dir) = cells_dir {
            write_cell(dir, first + i, c, cw, ch, &sink.rgba8)?;
        }
        let (col, row) = ((i % cols) as u32, (i / cols) as u32);
        let (ox, oy) = (GAP + col * (cw + GAP), GAP + row * (ch + GAP));
        for y in 0..ch {
            for x in 0..cw {
                let s = ((y * cw + x) * 4) as usize;
                let d = (((oy + y) * sw + ox + x) * 3) as usize;
                sheet[d..d + 3].copy_from_slice(&sink.rgba8[s..s + 3]);
            }
        }
        // Visible cell number (1-based), matching the notes.
        draw_label(
            &mut sheet,
            sw,
            ox + 6,
            oy + 6,
            first + i,
            (ch / 90).clamp(2, 6),
        );
        let cov = raster::role_coverage(&scene, 160, 160);
        let verts: usize = scene.layers().iter().map(|l| l.outline.len()).sum();
        notes.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{verts}\t{:016x}\t{}\t{}\t{:016x}\n",
            first + i,
            c.label,
            comp.template.name(),
            comp.mirrored,
            scene.layers().len(),
            scene.geometry_checksum(),
            metrics::measure(&scene).columns(),
            cov.map(|v| format!("{:.1}", v * 100.0)).join(" "),
            fnv(&sink.rgba8),
        ));
    }
    write_png(std::path::Path::new(out), sw, sh, &sheet)?;
    let txt = std::path::Path::new(out).with_extension("txt");
    std::fs::write(&txt, notes).map_err(|e| format!("writing {}: {e}", txt.display()))?;
    println!(
        "wrote {out} ({sw}x{sh}, {} cells in {}) and {}",
        cells.len(),
        ms(t0.elapsed()),
        txt.display()
    );
    Ok(())
}

fn paint_bench(o: &Opts) -> Result<(), String> {
    let aspect = parse_aspect(o.0.get("aspect").map_or("16:9", String::as_str))?;
    let n: u32 = o.num("sample", 12)?;
    let runs: usize = o.num("runs", 7)?;
    if runs < 3 {
        return Err("--runs must be at least 3".into());
    }
    let sizes: Vec<u32> =
        o.0.get("sizes")
            .map_or("960,1920,3840", String::as_str)
            .split(',')
            .map(|s| {
                s.trim()
                    .parse()
                    .map_err(|_| format!("--sizes: bad number {s:?}"))
            })
            .collect::<Result<_, _>>()?;
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let renderer = PaintRenderer::new(ctx.clone()).map_err(|e| e.to_string())?;
    println!(
        "pigment-prose {} paint-bench: renderer v{}, generator v{}, aspect {}:{}, {n} sample seeds, {runs} runs (2 warm-up)\ndevice: {}",
        version::APP_VERSION,
        version::RENDERER_VERSION,
        version::GENERATOR_VERSION,
        aspect.width,
        aspect.height,
        ctx.capabilities.label()
    );
    if ctx.capabilities.adapter.software {
        println!("WARNING: software adapter; results are NOT GPU evidence");
    }
    let form = FormSettings::default();
    let appearance = Appearance::default();
    let ids = RequestIds::default();
    let mut scenes = Vec::new();
    let mut gen_times = Vec::new();
    for i in 0..n {
        let digest =
            TextDigest::from_source(&format!("sample passage {i}")).map_err(|e| e.to_string())?;
        let seeds = SeedBundle::derive(digest, Variation(0));
        let t = Instant::now();
        let scene = LakeshoreGenerator
            .generate(&seeds, &form, aspect)
            .map_err(|e| e.to_string())?;
        gen_times.push(t.elapsed());
        scenes.push((seeds, Arc::new(scene)));
    }
    if o.0.contains_key("stress") {
        scenes = vec![(diagnostic_seeds(1), Arc::new(stress_scene(aspect)?))];
        println!(
            "stress scene: {} layers, {} vertices",
            MAX_LAYERS, MAX_SCENE_VERTICES
        );
    }
    gen_times.sort();
    println!(
        "scene generation (CPU): median {}, max {}",
        ms(gen_times[gen_times.len() / 2]),
        ms(gen_times[gen_times.len() - 1])
    );
    let frame_of = |long: u32| {
        let ext = aspect.extents();
        if ext.width >= ext.height {
            (long, ((long as f64) / ext.width).round() as u32)
        } else {
            (((long as f64) / ext.height).round() as u32, long)
        }
    };
    for long in sizes {
        let (w, h) = frame_of(long);
        let mut medians = Vec::new();
        for (seeds, scene) in &scenes {
            let req = RenderRequest {
                id: ids.next(),
                purpose: RenderPurpose::Preview,
                scene: scene.clone(),
                seeds: *seeds,
                appearance,
                target: RenderTarget {
                    width: w,
                    height: h,
                    policy: TilePolicy::Single,
                    order: TileOrder::RowMajor,
                },
            };
            let (mut t, mut total) = (Vec::new(), Vec::new());
            for r in 0..runs {
                let mut sink = MemorySink::default();
                let rep = renderer
                    .render(&req, &CancelToken::new(), &mut NoProgress, &mut sink)
                    .map_err(|e| e.to_string())?;
                if r >= 2 {
                    t.push(rep.timings.render_readback);
                    total.push(rep.timings.total);
                }
            }
            t.sort();
            total.sort();
            medians.push((t[t.len() / 2], total[total.len() / 2]));
        }
        medians.sort();
        let worst_total = medians.iter().map(|m| m.1).max().unwrap_or_default();
        println!(
            "{w}x{h} render + readback: median {}, worst scene {}; whole render call (setup, render, sink): worst scene {}",
            ms(medians[medians.len() / 2].0),
            ms(medians[medians.len() - 1].0),
            ms(worst_total)
        );
    }
    Ok(())
}

/// The `q`-quantile of `v` (nearest rank).
fn quantile(v: &[Duration], q: f64) -> Duration {
    let mut v = v.to_vec();
    v.sort();
    v.get(((v.len() as f64 - 1.0) * q).round() as usize)
        .copied()
        .unwrap_or_default()
}

/// The first line of a text file, trimmed (for environment reports).
fn first_line(path: &str, prefix: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().find(|l| l.starts_with(prefix))?;
    Some(line.split(':').nth(1).unwrap_or(line).trim().to_string())
}

/// One benchmark row: what, measured, target, and whether it counts.
struct Row {
    what: String,
    measured: Duration,
    detail: String,
    target: Option<Duration>,
}

fn bench(o: &Opts) -> Result<(), String> {
    let runs: usize = o.num("runs", 9)?;
    let samples: u32 = o.num("sample", 5)?;
    if runs < 3 || samples == 0 {
        return Err("--runs must be at least 3 and --sample at least 1".into());
    }
    let scratch = std::env::temp_dir().join(format!("pigment-bench-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let result = bench_in(o, runs, samples, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn bench_in(o: &Opts, runs: usize, samples: u32, scratch: &std::path::Path) -> Result<(), String> {
    let aspect = AspectRatio::of(16, 9);
    let t = Instant::now();
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let device_time = t.elapsed();
    let t = Instant::now();
    let renderer = PaintRenderer::new(ctx.clone()).map_err(|e| e.to_string())?;
    let pipeline_time = t.elapsed();
    let caps = &ctx.capabilities;
    let sw = caps.adapter.software;
    println!(
        "pigment-prose {} bench: renderer v{}, generator v{}, 16:9, default settings, {samples} seeds, {runs} warm runs each",
        version::APP_VERSION,
        version::RENDERER_VERSION,
        version::GENERATOR_VERSION
    );
    println!("device: {}", caps.label());
    println!(
        "adapter: {:?} {:?}, driver {} ({}), wgpu {}",
        caps.adapter.kind,
        caps.adapter.backend,
        caps.adapter.driver,
        caps.adapter.driver_info,
        caps.wgpu_version
    );
    println!(
        "host: {} {}; cpu: {}; kernel: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        first_line("/proc/cpuinfo", "model name").unwrap_or_else(|| "unknown".into()),
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".into())
    );
    if sw {
        println!("WARNING: software adapter; these are NOT GPU measurements and nothing passes");
    }
    let form = FormSettings::default();
    let appearance = Appearance::default();
    let ids = RequestIds::default();
    let request = |seeds: SeedBundle, scene: Arc<Scene>, w: u32, h: u32, policy| RenderRequest {
        id: ids.next(),
        purpose: RenderPurpose::Preview,
        scene,
        seeds,
        appearance,
        target: RenderTarget {
            width: w,
            height: h,
            policy,
            order: TileOrder::RowMajor,
        },
    };
    let seeds_of = |label: &str| -> Result<SeedBundle, String> {
        let digest = TextDigest::from_source(label).map_err(|e| e.to_string())?;
        Ok(SeedBundle::derive(digest, Variation(0)))
    };
    let render = |req: &RenderRequest| -> Result<RenderReport, String> {
        let mut sink = MemorySink::default();
        renderer
            .render(req, &CancelToken::new(), &mut NoProgress, &mut sink)
            .map_err(|e| e.to_string())
    };
    let mut rows = Vec::new();

    // Cold: the first preview after the renderer exists.
    let t = Instant::now();
    let seeds = seeds_of("bench cold start")?;
    let scene = Arc::new(
        LakeshoreGenerator
            .generate(&seeds, &form, aspect)
            .map_err(|e| e.to_string())?,
    );
    render(&request(seeds, scene, 1920, 1080, TilePolicy::Single))?;
    let first = t.elapsed();
    rows.push(Row {
        what: "cold start: device + pipelines + first 1920×1080 preview".into(),
        measured: device_time + pipeline_time + first,
        detail: format!(
            "device {}, pipelines {}, first preview {}",
            ms(device_time),
            ms(pipeline_time),
            ms(first)
        ),
        target: None,
    });

    // Warm previews.
    let mut scenes = Vec::new();
    for i in 0..samples {
        let seeds = seeds_of(&format!("sample passage {i}"))?;
        let scene = Arc::new(
            LakeshoreGenerator
                .generate(&seeds, &form, aspect)
                .map_err(|e| e.to_string())?,
        );
        scenes.push((seeds, scene));
    }
    for (label, w, h, target) in [
        (
            "interaction preview 960×540 (render + readback)",
            960,
            540,
            33,
        ),
        (
            "settled preview 1920×1080 (render + readback)",
            1920,
            1080,
            150,
        ),
        (
            "settled preview 3840×2160 (render + readback)",
            3840,
            2160,
            150,
        ),
    ] {
        let mut times = Vec::new();
        for (seeds, scene) in &scenes {
            let req = request(*seeds, scene.clone(), w, h, TilePolicy::Single);
            for r in 0..runs + 2 {
                let rep = render(&req)?;
                if r >= 2 {
                    times.push(rep.timings.render_readback);
                }
            }
        }
        rows.push(Row {
            what: label.into(),
            measured: quantile(&times, 0.95),
            detail: format!("median {}, n={}", ms(quantile(&times, 0.5)), times.len()),
            target: Some(Duration::from_millis(target)),
        });
    }

    // A prose edit: new digest, new scene, settled preview.
    let mut edits = Vec::new();
    for r in 0..runs + 2 {
        let t = Instant::now();
        let seeds = seeds_of(&format!("an edited passage, version {r}"))?;
        let scene = Arc::new(
            LakeshoreGenerator
                .generate(&seeds, &form, aspect)
                .map_err(|e| e.to_string())?,
        );
        render(&request(seeds, scene, 1920, 1080, TilePolicy::Single))?;
        if r >= 2 {
            edits.push(t.elapsed());
        }
    }
    rows.push(Row {
        what: "prose edit → settled 1920×1080 (after the 300 ms debounce)".into(),
        measured: quantile(&edits, 0.95),
        detail: format!("median {}, n={}", ms(quantile(&edits, 0.5)), edits.len()),
        target: Some(Duration::from_millis(250)),
    });

    // Exports, including PNG encoding and the atomic rename.
    let (seeds, scene) = scenes[0].clone();
    let export_req = |w: u32, h: u32| RenderRequest {
        purpose: RenderPurpose::Export,
        ..request(seeds, scene.clone(), w, h, TilePolicy::default_export())
    };
    for (label, w, h, target) in [
        ("4K export 3840×2160 incl. PNG", 3840, 2160, 5_000),
        ("8K export 7680×4320 incl. PNG", 7680, 4320, 20_000),
    ] {
        let mut times = Vec::new();
        let mut bytes = 0;
        for _ in 0..3 {
            let dest = scratch.join("bench.png");
            let t = Instant::now();
            let rep = export_png(
                &renderer,
                &export_req(w, h),
                &dest,
                PngCompression::Fast,
                &CancelToken::new(),
                &mut NoProgress,
            )
            .map_err(|e| e.to_string())?;
            times.push(t.elapsed());
            bytes = rep.bytes;
        }
        rows.push(Row {
            what: label.into(),
            measured: quantile(&times, 1.0),
            detail: format!(
                "median {}, {} MiB file, n=3",
                ms(quantile(&times, 0.5)),
                bytes >> 20
            ),
            target: Some(Duration::from_millis(target)),
        });
    }

    // Cancel latency: another thread cancels a 16K export at a moment that
    // does not line up with tiles (like a person pressing Cancel), and the
    // time until the export has stopped and cleaned up is measured.
    let mut latencies = Vec::new();
    for delay in [50u64, 150, 300] {
        let cancel = CancelToken::new();
        let dest = scratch.join("cancelled.png");
        let pressed = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let canceller = {
            let (cancel, pressed) = (cancel.clone(), pressed.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(delay));
                *pressed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                cancel.cancel();
            })
        };
        let rep = export_png(
            &renderer,
            &export_req(16384, 9216),
            &dest,
            PngCompression::Fast,
            &cancel,
            &mut NoProgress,
        )
        .map_err(|e| e.to_string())?;
        let stopped = Instant::now();
        let _ = canceller.join();
        let pressed = pressed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ok_or("the export finished before it could be cancelled")?;
        if !matches!(rep.render.outcome, RenderOutcome::Cancelled { .. }) || dest.exists() {
            return Err(format!(
                "cancel at {delay} ms did not stop the export and remove the partial file"
            ));
        }
        latencies.push(stopped.saturating_duration_since(pressed));
    }
    rows.push(Row {
        what: "export cancel → stopped and cleaned up, 16384×9216, cancelled at 50/150/300 ms"
            .into(),
        measured: quantile(&latencies, 1.0),
        detail: format!(
            "{}; partial file removed each time",
            latencies
                .iter()
                .map(|d| ms(*d))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        target: Some(Duration::from_millis(250)),
    });

    println!();
    println!("| Scenario | Measured (p95, or max for n=3) | Target | Verdict | Detail |");
    println!("| --- | --- | --- | --- | --- |");
    let mut missed = 0;
    for r in &rows {
        let verdict = match (sw, r.target) {
            (true, _) => "not GPU evidence".to_string(),
            (_, None) => "measured".to_string(),
            (_, Some(t)) if r.measured <= t => "meets".to_string(),
            (_, Some(_)) => {
                missed += 1;
                "MISSED".to_string()
            }
        };
        println!(
            "| {} | {} | {} | {verdict} | {} |",
            r.what,
            ms(r.measured),
            r.target.map_or("—".to_string(), |t| format!("≤ {}", ms(t))),
            r.detail
        );
    }
    println!();
    println!(
        "Not measured here: UI frame gaps and request→shown latency (see `pigment-studio --script`)."
    );
    if sw && o.0.contains_key("strict") {
        return Err("software adapter: not a GPU qualification".into());
    }
    if missed > 0 && o.0.contains_key("strict") {
        return Err(format!("{missed} target(s) missed"));
    }
    Ok(())
}

/// The coverage worst case: `MAX_LAYERS` layers at `MAX_SCENE_VERTICES`,
/// each a sawtooth band whose teeth reach the top of the frame, so every
/// layer's bounding box holds every pixel and a sky pixel tests every edge.
fn stress_scene(aspect: AspectRatio) -> Result<Scene, String> {
    let ext = aspect.extents();
    let per = MAX_SCENE_VERTICES / MAX_LAYERS;
    let teeth = per - 2;
    let (w, h) = (ext.width as f32, ext.height as f32);
    let layers = (0..MAX_LAYERS)
        .map(|k| {
            let level = h * (0.3 + 0.6 * k as f32 / MAX_LAYERS as f32);
            let mut outline: Vec<CanvasPoint> = (0..teeth)
                .map(|i| CanvasPoint {
                    x: -0.01 + (w + 0.02) * i as f32 / (teeth - 1) as f32,
                    y: if i % 2 == 1 { -0.01 } else { level },
                })
                .collect();
            outline.push(CanvasPoint {
                x: w + 0.01,
                y: h + 0.01,
            });
            outline.push(CanvasPoint {
                x: -0.01,
                y: h + 0.01,
            });
            SceneLayer {
                role: if k == 0 {
                    LayerRole::Sky
                } else {
                    LayerRole::Mountain
                },
                depth: 1.0 - k as f32 / MAX_LAYERS as f32,
                shade: 0.5,
                plant: Plant::None,
                outline,
            }
        })
        .collect();
    let key = SceneKey::new(0, &diagnostic_seeds(1), FormSettings::default(), aspect);
    Scene::new(key, layers).map_err(|e| e.to_string())
}

/// Writes one sheet cell as `NN.png` plus `NN.recipe.json` (the recipe that
/// reproduces it, without source text: the privacy default).
fn write_cell(
    dir: &std::path::Path,
    n: usize,
    c: &Cell,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> Result<(), String> {
    let png_path = dir.join(format!("{n:02}.png"));
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    write_png(&png_path, w, h, &rgb)?;
    let mut r = Recipe::new(c.digest, Frame::new(w, h).map_err(|e| e.to_string())?);
    r.seed.variation = Variation(c.variation);
    r.form = c.form;
    r.painting = c.appearance.painting;
    r.palette = c.appearance.palette;
    r.atmosphere = c.appearance.atmosphere;
    let json = r.to_canonical_json().map_err(|e| e.to_string())?;
    let path = dir.join(format!("{n:02}.recipe.json"));
    std::fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// An 8-bit sRGB PNG with no text, time or EXIF chunks.
fn write_png(path: &std::path::Path, w: u32, h: u32, rgb: &[u8]) -> Result<(), String> {
    let shown = path.display();
    let file = std::fs::File::create(path).map_err(|e| format!("creating {shown}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = enc.write_header().map_err(|e| format!("{shown}: {e}"))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| format!("{shown}: {e}"))?;
    writer.finish().map_err(|e| format!("{shown}: {e}"))
}

/// 3×5 bitmap digits, one row per `u8` (bit 2 = left column).
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// Draws `n` in white on a dark box at `(x, y)` of an RGB sheet `sw` wide,
/// each font pixel `scale` sheet pixels.
fn draw_label(sheet: &mut [u8], sw: u32, x: u32, y: u32, n: usize, scale: u32) {
    let digits: Vec<usize> = n.to_string().bytes().map(|b| (b - b'0') as usize).collect();
    let pad = scale;
    let (w, h) = (
        digits.len() as u32 * 4 * scale - scale + 2 * pad,
        5 * scale + 2 * pad,
    );
    let mut put = |px: u32, py: u32, rgb: [u8; 3]| {
        let d = ((py * sw + px) * 3) as usize;
        if d + 3 <= sheet.len() {
            sheet[d..d + 3].copy_from_slice(&rgb);
        }
    };
    for py in 0..h {
        for px in 0..w {
            put(x + px, y + py, [24, 24, 24]);
        }
    }
    for (k, &dg) in digits.iter().enumerate() {
        for (row, bits) in DIGITS[dg].iter().enumerate() {
            for colm in 0..3u32 {
                if bits & (4 >> colm) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            put(
                                x + pad + (k as u32 * 4 + colm) * scale + sx,
                                y + pad + row as u32 * scale + sy,
                                [255, 255, 255],
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Peak resident set size of this process, from `/proc/self/status`
/// (`VmHWM`). Linux only; a measurement, unlike the cost model's estimates.
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
}

fn export(o: &Opts) -> Result<(), String> {
    let out = std::path::Path::new(o.0.get("out").ok_or("export needs --out FILE.png")?);
    let variation: u32 = o.num("variation", 0)?;
    // Seeds, form, appearance and aspect: from a recipe, or a synthetic
    // sample/fixture passage with command-line settings.
    let (seeds, mut form, mut appearance, aspect, recipe_frame) =
        if let Some(path) = o.0.get("recipe") {
            let json = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
            let r = Recipe::from_json(&json).map_err(|e| format!("{path}: {e}"))?;
            for n in r.version_notices() {
                eprintln!("note: {n}");
            }
            (
                r.seeds(),
                r.form,
                r.appearance(),
                r.frame.aspect(),
                Some(r.frame),
            )
        } else {
            let text = if let Some(id) = o.0.get("passage") {
                let passages =
                    o.0.get("passages")
                        .map_or("fixtures/passages.json", String::as_str);
                corpus(passages)?
                    .into_iter()
                    .find(|(pid, _)| pid == id)
                    .map(|(_, t)| t)
                    .ok_or_else(|| format!("no passage {id:?} in {passages}"))?
            } else {
                format!("sample passage {}", o.num::<u32>("sample", 0)?)
            };
            let digest = TextDigest::from_source(&text).map_err(|e| e.to_string())?;
            let aspect = parse_aspect(o.0.get("aspect").map_or("16:9", String::as_str))?;
            (
                SeedBundle::derive(digest, Variation(variation)),
                FormSettings::default(),
                Appearance::default(),
                aspect,
                None,
            )
        };
    for key in ["faceting", "relief", "density"] {
        if let Some(v) = o.0.get(key) {
            set_form(&mut form, key, v)?;
        }
    }
    for key in PAINT_KEYS {
        if let Some(v) = o.0.get(key) {
            set_paint(&mut appearance, key, v)?;
        }
    }
    form.validate().map_err(|e| e.to_string())?;
    appearance.validate().map_err(|e| e.to_string())?;
    let size = match o.0.get("size").map(String::as_str) {
        None if recipe_frame.is_some() => None,
        None | Some("8k") => Some(ExportSize::Uhd8k),
        Some("4k") => Some(ExportSize::Uhd4k),
        Some(s) => {
            let (w, h) = s
                .split_once('x')
                .ok_or_else(|| format!("--size {s:?}: expected 4k, 8k or WxH"))?;
            let bad = |_| format!("--size {s:?}: bad number");
            Some(ExportSize::Custom {
                width: w.parse().map_err(bad)?,
                height: h.parse().map_err(bad)?,
            })
        }
    };
    let frame = match (size, recipe_frame) {
        (Some(size), _) => size.frame(aspect).map_err(|e| e.to_string())?,
        (None, Some(f)) => f,
        (None, None) => unreachable!("a size is always chosen without a recipe"),
    };
    let policy = if o.0.contains_key("tile") {
        TilePolicy::Fixed {
            edge: o.num("tile", 2048)?,
        }
    } else if o.0.contains_key("gpu-budget") {
        TilePolicy::Budget {
            gpu_bytes: o.num::<u64>("gpu-budget", 256)? << 20,
            host_bytes: pigment_core::tiles::DEFAULT_HOST_BAND_BUDGET,
        }
    } else {
        TilePolicy::default_export()
    };
    let order = match o.0.get("order").map(String::as_str) {
        None | Some("row-major") => TileOrder::RowMajor,
        Some("reverse") => TileOrder::ReverseInBand,
        Some(v) => return Err(format!("--order {v:?}: expected row-major or reverse")),
    };
    let compression = match o.0.get("compression").map(String::as_str) {
        None | Some("fast") => PngCompression::Fast,
        Some("balanced") => PngCompression::Balanced,
        Some(v) => return Err(format!("--compression {v:?}: expected fast or balanced")),
    };
    let cancel_after: Option<u32> =
        o.0.get("cancel-after")
            .map(|_| o.num("cancel-after", 1))
            .transpose()?;

    let t_ctx = Instant::now();
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let renderer = PaintRenderer::new(ctx.clone()).map_err(|e| e.to_string())?;
    let t_ctx = t_ctx.elapsed();
    println!(
        "pigment-prose {} export: generator v{}, renderer v{}, {}x{} (aspect {}:{}), {policy:?}, {order:?}, {compression:?}\ndevice: {} (wgpu {})",
        version::APP_VERSION,
        version::GENERATOR_VERSION,
        version::RENDERER_VERSION,
        frame.width,
        frame.height,
        aspect.width,
        aspect.height,
        ctx.capabilities.label(),
        pigment_gpu::context::WGPU_VERSION,
    );
    if ctx.capabilities.adapter.software {
        println!("WARNING: software adapter; results are NOT GPU evidence");
    }
    let t_scene = Instant::now();
    let scene = Arc::new(
        LakeshoreGenerator
            .generate(&seeds, &form, aspect)
            .map_err(|e| e.to_string())?,
    );
    let t_scene = t_scene.elapsed();
    let req = RenderRequest {
        id: RequestIds::default().next(),
        purpose: RenderPurpose::Export,
        scene,
        seeds,
        appearance,
        target: RenderTarget {
            width: frame.width,
            height: frame.height,
            policy,
            order,
        },
    };
    let cancel = CancelToken::new();
    let c2 = cancel.clone();
    let mut last_band = Instant::now();
    let mut on_progress = move |p: Progress| {
        if cancel_after.is_some_and(|n| p.done >= n) {
            c2.cancel();
        }
        if p.phase == pigment_core::job::Phase::Tiles
            && last_band.elapsed() > Duration::from_secs(1)
        {
            eprintln!("  {} / {} tiles", p.done, p.total);
            last_band = Instant::now();
        }
    };
    let rep = export_png(&renderer, &req, out, compression, &cancel, &mut on_progress)
        .map_err(|e| e.to_string())?;
    let r = &rep.render;
    let p = &r.plan;
    for a in &rep.attempts {
        if let Some(stage) = a.out_of_memory {
            println!(
                "attempt {:?}: GPU out of memory during {stage}; retried",
                a.policy
            );
        }
    }
    println!(
        "plan: {}x{} tiles of {}x{} px, apron {} px; estimated renderer GPU allocations {} per tile set, host band {} (cost model, not measured)",
        p.cols,
        p.rows,
        p.tile_w,
        p.tile_h,
        p.apron,
        mib(p.gpu_bytes),
        mib(p.host_band_bytes),
    );
    println!(
        "timings: device+pipelines {}, scene {}, setup {}, render+readback {}, PNG encode+write {}, export total {}",
        ms(t_ctx),
        ms(t_scene),
        ms(r.timings.setup),
        ms(r.timings.render_readback),
        ms(r.timings.sink),
        ms(r.timings.total),
    );
    match peak_rss_bytes() {
        Some(b) => println!(
            "measured peak host memory (VmHWM, whole process): {}",
            mib(b)
        ),
        None => println!("measured peak host memory: unavailable on this OS"),
    }
    match r.outcome {
        RenderOutcome::Completed => {
            println!("wrote {} ({})", out.display(), mib(rep.bytes));
            Ok(())
        }
        RenderOutcome::Cancelled { tiles_done } => {
            let left = out.exists();
            println!(
                "cancelled after {tiles_done} of {} tiles; {} {}",
                p.len(),
                out.display(),
                if left {
                    "exists (kept from before)"
                } else {
                    "was not written"
                }
            );
            Ok(())
        }
    }
}
