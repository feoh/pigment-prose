//! `pigment-prose`: diagnostics CLI. Reads no prose; seeds are numbers.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pigment_core::capability::{AdapterPolicy, AdapterReport};
use pigment_core::frame::Frame;
use pigment_core::job::{CancelToken, NoProgress, Progress};
use pigment_core::request::{
    MemorySink, RenderOutcome, RenderPurpose, RenderReport, RenderRequest, RenderTarget, Renderer,
    RequestIds,
};
use pigment_core::scene::{SceneGenerator, TestCard, diagnostic_seeds};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_core::tiles::TilePolicy;
use pigment_core::version;
use pigment_gpu::{GpuContext, SmokeRenderer, adapter};

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
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("gpu-info") => parse(&args[1..]).and_then(|o| gpu_info(&o)),
        Some("gpu-smoke") => parse(&args[1..]).and_then(|o| gpu_smoke(&o)),
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
