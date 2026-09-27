//! `pigment-prose`: diagnostics CLI. Reads no user prose: `gpu-*` seeds are
//! numbers, and `contact-sheet` reads only the synthetic fixture corpus or
//! generated sample labels.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pigment_core::capability::{AdapterPolicy, AdapterReport};
use pigment_core::frame::AspectRatio;
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderReport, RenderRequest, RenderTarget, Renderer,
    RequestIds,
};
use pigment_core::scene::lakeshore::{Composition, LakeshoreGenerator};
use pigment_core::scene::{LayerRole, SceneGenerator, TestCard, diagnostic_seeds, raster};
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::TilePolicy;
use pigment_core::version;
use pigment_gpu::{DebugRenderer, DebugView, GpuContext, SmokeRenderer, adapter};

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
  contact-sheet --out SHEET.png [--aspect W:H] [--view flat|regions]
            [--cell PX] [--cols N] [--variation V]
            [--passages FILE | --passage ID --variations N | --sample N]
            [--faceting F] [--relief R] [--density D] [--adapter NAME]
      Render lakeshore scenes (task 05 geometry, debug views, no painting)
      into a grid PNG, plus SHEET.txt listing each cell's passage id,
      variation, template, geometry checksum and visible coverage by role.
      Cells: the corpus (default fixtures/passages.json) at one variation,
      one passage's variations 0..N-1, or N generated sample seeds.
      Defaults: 16:9, regions, cell 480 px long side, variation 0, 5 columns,
      form settings at their defaults.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("gpu-info") => parse(&args[1..]).and_then(|o| gpu_info(&o)),
        Some("gpu-smoke") => parse(&args[1..]).and_then(|o| gpu_smoke(&o)),
        Some("contact-sheet") => parse(&args[1..]).and_then(|o| contact_sheet(&o)),
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

const BOOL_FLAGS: &[&str] = &["gl", "allow-software"];

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
    seeds: SeedBundle,
}

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
    let view = match o.0.get("view").map_or("regions", String::as_str) {
        "flat" => DebugView::Flat,
        "regions" => DebugView::Regions,
        v => return Err(format!("--view {v:?}: expected flat or regions")),
    };
    let cell: u32 = o.num("cell", 480)?;
    let cols: usize = o.num("cols", 5)?;
    let variation: u32 = o.num("variation", 0)?;
    let form = FormSettings {
        faceting: o.num("faceting", FormSettings::default().faceting)?,
        relief: o.num("relief", FormSettings::default().relief)?,
        woodland_density: o.num("density", FormSettings::default().woodland_density)?,
    };
    form.validate().map_err(|e| e.to_string())?;
    let derive = |text: &str, v: u32| -> Result<SeedBundle, String> {
        let digest = TextDigest::from_source(text).map_err(|e| e.to_string())?;
        Ok(SeedBundle::derive(digest, Variation(v)))
    };
    let passages =
        o.0.get("passages")
            .map_or("fixtures/passages.json", String::as_str);
    let mut cells = Vec::new();
    if let Some(n) = o.0.get("sample") {
        let n: u32 = n
            .parse()
            .map_err(|_| format!("--sample {n:?}: bad number"))?;
        for i in 0..n {
            let label = format!("sample passage {i}");
            cells.push(Cell {
                seeds: derive(&label, variation)?,
                label,
            });
        }
    } else if let Some(id) = o.0.get("passage") {
        let n: u32 = o.num("variations", 10)?;
        let (_, text) = corpus(passages)?
            .into_iter()
            .find(|(pid, _)| pid == id)
            .ok_or_else(|| format!("no passage {id:?} in {passages}"))?;
        for v in 0..n {
            cells.push(Cell {
                label: format!("{id} v{v}"),
                seeds: derive(&text, v)?,
            });
        }
    } else {
        for (id, text) in corpus(passages)? {
            cells.push(Cell {
                label: format!("{id} v{variation}"),
                seeds: derive(&text, variation)?,
            });
        }
    }
    if cells.is_empty() {
        return Err("no cells to render".into());
    }

    let ext = aspect.extents();
    let (cw, ch) = if ext.width >= ext.height {
        (cell, ((cell as f64) / ext.width).round() as u32)
    } else {
        (((cell as f64) / ext.height).round() as u32, cell)
    };
    let ctx = Arc::new(GpuContext::new(&o.policy()).map_err(|e| e.to_string())?);
    let renderer = DebugRenderer::new(ctx.clone(), view).map_err(|e| e.to_string())?;
    let cols = cols.clamp(1, cells.len());
    let rows = cells.len().div_ceil(cols);
    const GAP: u32 = 6;
    let (sw, sh) = (
        cols as u32 * cw + (cols as u32 + 1) * GAP,
        rows as u32 * ch + (rows as u32 + 1) * GAP,
    );
    let mut sheet = vec![0x80u8; sw as usize * sh as usize * 3];
    let ids = RequestIds::default();
    let mut notes = format!(
        "pigment-prose {} contact-sheet: lakeshore generator v{}, debug view {view:?}, aspect {}:{}, \
         cell {cw}x{ch}, form faceting={} relief={} woodland_density={}\ndevice: {}\n\
         cell\tlabel\ttemplate\tmirrored\tlayers\tvertices\tchecksum\tcoverage % ({})\n",
        version::APP_VERSION,
        version::GENERATOR_VERSION,
        aspect.width,
        aspect.height,
        form.faceting,
        form.relief,
        form.woodland_density,
        ctx.capabilities.label(),
        LayerRole::ALL.map(|r| r.name()).join(" "),
    );
    let t0 = Instant::now();
    for (i, c) in cells.iter().enumerate() {
        let scene = Arc::new(
            LakeshoreGenerator
                .generate(&c.seeds, &form, aspect)
                .map_err(|e| format!("{}: {e}", c.label))?,
        );
        let comp = Composition::draw(&c.seeds, aspect);
        let req = RenderRequest {
            id: ids.next(),
            purpose: RenderPurpose::Preview,
            scene: scene.clone(),
            seeds: c.seeds,
            appearance: Appearance::default(),
            target: RenderTarget {
                width: cw,
                height: ch,
                policy: TilePolicy::Single,
            },
        };
        let mut sink = MemorySink::default();
        renderer
            .render(&req, &CancelToken::new(), &mut NoProgress, &mut sink)
            .map_err(|e| e.to_string())?;
        let (col, row) = ((i % cols) as u32, (i / cols) as u32);
        let (ox, oy) = (GAP + col * (cw + GAP), GAP + row * (ch + GAP));
        for y in 0..ch {
            for x in 0..cw {
                let s = ((y * cw + x) * 4) as usize;
                let d = (((oy + y) * sw + ox + x) * 3) as usize;
                sheet[d..d + 3].copy_from_slice(&sink.rgba8[s..s + 3]);
            }
        }
        let cov = raster::role_coverage(&scene, 160, 160);
        let verts: usize = scene.layers().iter().map(|l| l.outline.len()).sum();
        notes.push_str(&format!(
            "{i}\t{}\t{}\t{}\t{}\t{verts}\t{:016x}\t{}\n",
            c.label,
            comp.template.name(),
            comp.mirrored,
            scene.layers().len(),
            scene.geometry_checksum(),
            cov.map(|v| format!("{:.1}", v * 100.0)).join(" "),
        ));
    }
    let file = std::fs::File::create(out).map_err(|e| format!("creating {out}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), sw, sh);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = enc.write_header().map_err(|e| format!("{out}: {e}"))?;
    writer
        .write_image_data(&sheet)
        .map_err(|e| format!("{out}: {e}"))?;
    writer.finish().map_err(|e| format!("{out}: {e}"))?;
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
