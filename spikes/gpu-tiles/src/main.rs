//! Pigment Prose architecture spike (task 02). Throwaway code: it exists to
//! measure GPU painting and tiled export on real hardware, not to become the
//! product. Inputs are numeric seeds only; no prose is read or logged.

mod gpu;
mod scene;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

const USAGE: &str = "\
gpu-tiles <command> [options]

  adapters [--gl]
      List every adapter per backend, its type and key limits, and which one
      would be selected.
  render --width W --height H --out FILE.png [--tile T] [--apron A] [--seed S]
         [--adapter NAME] [--allow-software] [--gl] [--tile-local-noise]
         [--default-limits]
      Render tile by tile, streaming rows into a PNG. --tile 0 (default)
      renders one full-image tile. --apron defaults to the blur radius.
  compare A.png B.png [--tile T] [--diff OUT.png]
      Exact per-channel comparison, split into pixels near tile boundaries and
      elsewhere, plus a reference-free seam statistic for each image.
  crop IN.png X Y W H OUT.png [--scale N]
  downsample IN.png FACTOR OUT.png
      Box-filter downsample (averaging sRGB bytes; adequate for structure
      comparison, not colour-accurate resampling).
  bench-preview --width W --height H [--iters N] [--seed S] [--adapter NAME]
      Repeated full render + readback without PNG encoding.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("adapters") => cmd_adapters(&args[1..]),
        Some("render") => cmd_render(&args[1..]),
        Some("compare") => cmd_compare(&args[1..]),
        Some("crop") => cmd_crop(&args[1..]),
        Some("downsample") => cmd_downsample(&args[1..]),
        Some("bench-preview") => cmd_bench_preview(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

struct Opts {
    flags: HashMap<String, String>,
    positional: Vec<String>,
}

const BOOL_FLAGS: &[&str] = &["gl", "allow-software", "tile-local-noise", "default-limits"];

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            if BOOL_FLAGS.contains(&k) {
                flags.insert(k.to_string(), "1".to_string());
            } else {
                let v = it.next().ok_or(format!("--{k} needs a value"))?;
                flags.insert(k.to_string(), v.clone());
            }
        } else {
            positional.push(a.clone());
        }
    }
    Ok(Opts { flags, positional })
}

impl Opts {
    fn num<T: std::str::FromStr>(&self, k: &str, default: Option<T>) -> Result<T, String> {
        match self.flags.get(k) {
            Some(v) => v.parse().map_err(|_| format!("--{k}: bad number {v:?}")),
            None => default.ok_or(format!("--{k} is required")),
        }
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
    fn choice(&self) -> gpu::AdapterChoice {
        gpu::AdapterChoice {
            name_filter: self.flags.get("adapter").cloned(),
            allow_software: self.has("allow-software"),
            include_gl: self.has("gl"),
            default_limits: self.has("default-limits"),
        }
    }
}

fn cmd_adapters(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let choice = o.choice();
    let inst = gpu::instance(choice.include_gl);
    let backends = if choice.include_gl {
        wgpu::Backends::all()
    } else {
        wgpu::Backends::PRIMARY
    };
    let adapters = pollster::block_on(inst.enumerate_adapters(backends));
    println!(
        "wgpu {} ; {} adapter(s)",
        env!("WGPU_VERSION"),
        adapters.len()
    );
    for a in &adapters {
        println!("- {}", gpu::describe(a));
    }
    match gpu::select(&choice) {
        Ok(a) => println!(
            "selected: {} ({:?})",
            a.get_info().name,
            a.get_info().backend
        ),
        Err(e) => println!("selected: none\n{e}"),
    }
    Ok(())
}

fn job_from(o: &Opts) -> Result<gpu::Job, String> {
    let width: u32 = o.num("width", None)?;
    let height: u32 = o.num("height", None)?;
    if width == 0 || height == 0 {
        return Err("width and height must be positive".into());
    }
    Ok(gpu::Job {
        width,
        height,
        tile: o.num("tile", Some(0))?,
        apron: if o.has("apron") {
            Some(o.num("apron", None)?)
        } else {
            None
        },
        seed: o.num("seed", Some(1))?,
        tile_local_noise: o.has("tile-local-noise"),
    })
}

fn open_gpu(o: &Opts) -> Result<(gpu::Gpu, std::time::Duration), String> {
    let t = Instant::now();
    let choice = o.choice();
    let adapter = gpu::select(&choice)?;
    let g = gpu::Gpu::new(adapter, choice.default_limits)?;
    let el = t.elapsed();
    println!(
        "adapter: {} backend={:?} type={:?} driver={} {} limits={}{}",
        g.info.name,
        g.info.backend,
        g.info.device_type,
        g.info.driver,
        g.info.driver_info,
        if choice.default_limits {
            "webgpu-default"
        } else {
            "adapter"
        },
        if g.software {
            " [SOFTWARE - not GPU evidence]"
        } else {
            ""
        }
    );
    Ok((g, el))
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let job = job_from(&o)?;
    let out = o.flags.get("out").ok_or("--out is required")?;
    let (g, init) = open_gpu(&o)?;
    let short = job.short_side() as f64;
    let scene = scene::build(
        job.seed,
        job.width as f64 / short,
        job.height as f64 / short,
    );

    let file = File::create(out).map_err(|e| format!("{out}: {e}"))?;
    let mut enc = png::Encoder::new(BufWriter::new(file), job.width, job.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    // Deliberately no text chunks: nothing identifying, no prose, no
    // attribution. sRGB chunk states the encoding the shader applied.
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut stream = writer
        .stream_writer_with_size(1 << 20)
        .map_err(|e| e.to_string())?;

    let stats = gpu::render(&g, &scene, job, |_, _, rows| {
        stream.write_all(rows).map_err(|e| e.to_string())
    })?;
    let t_fin = Instant::now();
    stream.finish().map_err(|e| e.to_string())?;
    let finish = t_fin.elapsed();
    print_stats(&job, &stats, init);
    println!("png_finish_ms={:.1} out={out}", ms(finish));
    Ok(())
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn print_stats(job: &gpu::Job, s: &gpu::Stats, init: std::time::Duration) {
    println!(
        "size={}x{} tile={}x{} apron={} blur_radius={} tiles={} seed={} tile_local_noise={}",
        job.width,
        job.height,
        s.tile_w,
        s.tile_h,
        s.apron,
        s.blur_radius,
        s.tiles,
        job.seed,
        job.tile_local_noise
    );
    println!(
        "gpu_alloc_mib={:.1} cpu_band_mib={:.1} full_image_rgba8_mib={:.1}",
        s.gpu_bytes as f64 / 1048576.0,
        s.band_bytes as f64 / 1048576.0,
        job.width as f64 * job.height as f64 * 4.0 / 1048576.0
    );
    println!(
        "device_init_ms={:.1} setup_ms={:.1} gpu_readback_ms={:.1} png_stream_ms={:.1} total_ms={:.1}",
        ms(init),
        ms(s.setup),
        ms(s.gpu_and_readback),
        ms(s.sink),
        ms(s.total)
    );
}

fn cmd_bench_preview(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let job = job_from(&o)?;
    let iters: usize = o.num("iters", Some(30))?;
    let (g, init) = open_gpu(&o)?;
    let short = job.short_side() as f64;
    let mut times = Vec::new();
    let mut first = None;
    for i in 0..=iters {
        // Include scene construction: a preview edit rebuilds it.
        let t = Instant::now();
        let scene = scene::build(
            job.seed,
            job.width as f64 / short,
            job.height as f64 / short,
        );
        let s = gpu::render(&g, &scene, job, |_, _, _| Ok(()))?;
        let el = t.elapsed();
        if i == 0 {
            first = Some((el, s));
        } else {
            times.push(ms(el));
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (f, s) = first.unwrap();
    print_stats(&job, &s, init);
    let pct = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    println!(
        "first_frame_ms={:.2} warm_n={} warm_min_ms={:.2} warm_median_ms={:.2} warm_p95_ms={:.2} warm_max_ms={:.2}",
        ms(f),
        times.len(),
        pct(0.0),
        pct(0.5),
        pct(0.95),
        pct(1.0)
    );
    Ok(())
}

struct Image {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

fn load(path: &str) -> Result<Image, String> {
    let f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{path}: expected RGBA8"));
    }
    buf.truncate(info.buffer_size());
    Ok(Image {
        w: info.width,
        h: info.height,
        px: buf,
    })
}

fn save(path: &str, w: u32, h: u32, px: &[u8]) -> Result<(), String> {
    let f = File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(px).map_err(|e| e.to_string())
}

/// Mean absolute RGB step between column x-1 and x (or row y-1 and y).
fn step(img: &Image, vertical_line: bool, at: u32) -> f64 {
    let mut sum = 0u64;
    let n = if vertical_line { img.h } else { img.w };
    for i in 0..n {
        let (a, b) = if vertical_line {
            ((i * img.w + at - 1) as usize, (i * img.w + at) as usize)
        } else {
            (((at - 1) * img.w + i) as usize, (at * img.w + i) as usize)
        };
        for c in 0..3 {
            sum += (img.px[a * 4 + c] as i32 - img.px[b * 4 + c] as i32).unsigned_abs() as u64;
        }
    }
    sum as f64 / (n as f64 * 3.0)
}

/// Reference-free seam statistic: the pixel step across each tile boundary
/// divided by the mean step across nearby non-boundary lines (offsets 3..8
/// on both sides). ~1.0 means the boundary is indistinguishable from its
/// neighbourhood; a seam shows up as a ratio well above 1.
fn seam_ratio(img: &Image, tile: u32) -> (f64, f64) {
    let mut worst = 0.0f64;
    let mut sum = 0.0;
    let mut n = 0;
    for (vertical, extent) in [(true, img.w), (false, img.h)] {
        let mut b = tile;
        while b < extent {
            let at = step(img, vertical, b);
            let mut near = 0.0;
            let mut k = 0;
            for off in 3..=8i64 {
                for s in [-1i64, 1] {
                    let x = b as i64 + s * off;
                    if x >= 1 && x < extent as i64 {
                        near += step(img, vertical, x as u32);
                        k += 1;
                    }
                }
            }
            let ratio = at / (near / k as f64).max(1e-6);
            worst = worst.max(ratio);
            sum += ratio;
            n += 1;
            b += tile;
        }
    }
    (if n > 0 { sum / n as f64 } else { 1.0 }, worst)
}

fn cmd_compare(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let [a, b] = &o.positional[..] else {
        return Err("compare needs two PNG paths".into());
    };
    let tile: u32 = o.num("tile", Some(0))?;
    let (ia, ib) = (load(a)?, load(b)?);
    if (ia.w, ia.h) != (ib.w, ib.h) {
        return Err(format!(
            "size mismatch {}x{} vs {}x{}",
            ia.w, ia.h, ib.w, ib.h
        ));
    }
    let near_boundary = |x: u32, y: u32| {
        tile > 0 && {
            let dx = x % tile;
            let dy = y % tile;
            dx < 2 || dx >= tile - 2 || dy < 2 || dy >= tile - 2
        }
    };
    // Distance to the nearest tile edge, counting image borders as edges: a
    // too-small apron also breaks the frame border, where the reference reads
    // procedural content that lies outside the image.
    let edge_dist = |x: u32, y: u32| -> u32 {
        let axis = |v: u32, extent: u32| {
            let below = v % tile;
            let above = ((v / tile + 1) * tile).min(extent) - v - 1;
            below.min(above)
        };
        axis(x, ia.w).min(axis(y, ia.h))
    };
    let mut max_edge_dist = 0u32;
    let (mut max_d, mut diff_px, mut diff_px_boundary, mut boundary_px) = (0u8, 0u64, 0u64, 0u64);
    let mut sq = 0f64;
    let mut diff_img = if o.has("diff") {
        vec![0u8; ia.px.len()]
    } else {
        Vec::new()
    };
    for y in 0..ia.h {
        for x in 0..ia.w {
            let i = ((y * ia.w + x) * 4) as usize;
            let mut pd = 0u8;
            for c in 0..4 {
                let d = ia.px[i + c].abs_diff(ib.px[i + c]);
                pd = pd.max(d);
                sq += (d as f64) * (d as f64);
            }
            let nb = near_boundary(x, y);
            boundary_px += nb as u64;
            if pd > 0 {
                diff_px += 1;
                diff_px_boundary += nb as u64;
                if tile > 0 {
                    max_edge_dist = max_edge_dist.max(edge_dist(x, y));
                }
            }
            max_d = max_d.max(pd);
            if !diff_img.is_empty() {
                let v = (pd as u32 * 32).min(255) as u8;
                diff_img[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
    let total = ia.w as u64 * ia.h as u64;
    let mse = sq / (total as f64 * 4.0);
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    println!(
        "compare {a} vs {b}\n  size={}x{} tile={tile} identical={} max_channel_diff={max_d} differing_px={diff_px}/{total} psnr_db={psnr:.2}\n  boundary_band_px={boundary_px} differing_in_boundary_band={diff_px_boundary} differing_elsewhere={}",
        ia.w,
        ia.h,
        ia.px == ib.px,
        diff_px - diff_px_boundary
    );
    if tile > 0 && diff_px > 0 {
        println!("  max_distance_of_a_differing_px_from_a_tile_edge={max_edge_dist}");
    }
    if tile > 0 {
        let (ma, wa) = seam_ratio(&ia, tile);
        let (mb, wb) = seam_ratio(&ib, tile);
        println!("  seam_ratio A mean={ma:.3} worst={wa:.3} | B mean={mb:.3} worst={wb:.3}");
    }
    if let Some(p) = o.flags.get("diff") {
        save(p, ia.w, ia.h, &diff_img)?;
        println!("  diff image (x32) -> {p}");
    }
    Ok(())
}

fn cmd_crop(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let [src, x, y, w, h, dst] = &o.positional[..] else {
        return Err("crop IN X Y W H OUT".into());
    };
    let scale: u32 = o.num("scale", Some(1))?;
    let p = |s: &String| s.parse::<u32>().map_err(|_| format!("bad number {s}"));
    let (x, y, w, h) = (p(x)?, p(y)?, p(w)?, p(h)?);
    let img = load(src)?;
    if x + w > img.w || y + h > img.h {
        return Err("crop outside image".into());
    }
    let (ow, oh) = (w * scale, h * scale);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let si = (((y + oy / scale) * img.w + x + ox / scale) * 4) as usize;
            let di = ((oy * ow + ox) * 4) as usize;
            out[di..di + 4].copy_from_slice(&img.px[si..si + 4]);
        }
    }
    save(dst, ow, oh, &out)
}

fn cmd_downsample(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let [src, f, dst] = &o.positional[..] else {
        return Err("downsample IN FACTOR OUT".into());
    };
    let f: u32 = f.parse().map_err(|_| "bad factor".to_string())?;
    let img = load(src)?;
    if f == 0 || img.w % f != 0 || img.h % f != 0 {
        return Err("factor must divide both dimensions".into());
    }
    let (ow, oh) = (img.w / f, img.h / f);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut acc = [0u32; 4];
            for dy in 0..f {
                for dx in 0..f {
                    let si = (((oy * f + dy) * img.w + ox * f + dx) * 4) as usize;
                    for (a, &v) in acc.iter_mut().zip(&img.px[si..si + 4]) {
                        *a += v as u32;
                    }
                }
            }
            let di = ((oy * ow + ox) * 4) as usize;
            for c in 0..4 {
                out[di + c] = ((acc[c] + f * f / 2) / (f * f)) as u8;
            }
        }
    }
    save(dst, ow, oh, &out)
}
