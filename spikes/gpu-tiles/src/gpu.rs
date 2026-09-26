//! Adapter selection, diagnostics and the tiled compute renderer.

use std::time::{Duration, Instant};

use crate::scene::Scene;

pub struct AdapterChoice {
    pub name_filter: Option<String>,
    pub allow_software: bool,
    pub include_gl: bool,
    /// Request WebGPU's portable default limits instead of the adapter's.
    pub default_limits: bool,
}

fn backends(include_gl: bool) -> wgpu::Backends {
    if include_gl {
        wgpu::Backends::all()
    } else {
        wgpu::Backends::PRIMARY
    }
}

pub fn instance(include_gl: bool) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends(include_gl);
    wgpu::Instance::new(desc)
}

pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    // DeviceType::Cpu covers lavapipe/llvmpipe/WARP/SwiftShader. Some GL
    // drivers report Other for llvmpipe, so check the name as well.
    let name = info.name.to_lowercase();
    info.device_type == wgpu::DeviceType::Cpu
        || name.contains("llvmpipe")
        || name.contains("lavapipe")
        || name.contains("swiftshader")
        || name.contains("warp")
}

fn rank(info: &wgpu::AdapterInfo) -> u32 {
    if is_software(info) {
        return 0;
    }
    match info.device_type {
        wgpu::DeviceType::DiscreteGpu => 4,
        wgpu::DeviceType::IntegratedGpu => 3,
        wgpu::DeviceType::VirtualGpu => 2,
        _ => 1,
    }
}

pub fn describe(a: &wgpu::Adapter) -> String {
    let i = a.get_info();
    let l = a.limits();
    format!(
        "{name}\n    backend={backend:?} type={ty:?} software={sw} vendor=0x{vendor:04x} device=0x{device:04x}\n    driver={driver} ({info})\n    max_texture_dimension_2d={tex} max_storage_buffer_binding_size={sb} max_buffer_size={mb}\n    max_compute_workgroup_size=({wx},{wy},{wz}) max_compute_invocations_per_workgroup={wi} max_storage_textures_per_shader_stage={st}",
        name = i.name,
        backend = i.backend,
        ty = i.device_type,
        sw = is_software(&i),
        vendor = i.vendor,
        device = i.device,
        driver = i.driver,
        info = i.driver_info,
        tex = l.max_texture_dimension_2d,
        sb = l.max_storage_buffer_binding_size,
        mb = l.max_buffer_size,
        wx = l.max_compute_workgroup_size_x,
        wy = l.max_compute_workgroup_size_y,
        wz = l.max_compute_workgroup_size_z,
        wi = l.max_compute_invocations_per_workgroup,
        st = l.max_storage_textures_per_shader_stage,
    )
}

pub fn select(choice: &AdapterChoice) -> Result<wgpu::Adapter, String> {
    let inst = instance(choice.include_gl);
    let mut adapters = pollster::block_on(inst.enumerate_adapters(backends(choice.include_gl)));
    if adapters.is_empty() {
        return Err(no_adapter_help("no adapters were reported by any backend"));
    }
    if let Some(f) = &choice.name_filter {
        let f = f.to_lowercase();
        adapters.retain(|a| a.get_info().name.to_lowercase().contains(&f));
        if adapters.is_empty() {
            return Err(format!(
                "no adapter name contains {f:?}; run `gpu-tiles adapters` to list candidates"
            ));
        }
    }
    adapters.sort_by_key(|a| std::cmp::Reverse(rank(&a.get_info())));
    let best = adapters.remove(0);
    if is_software(&best.get_info()) && !choice.allow_software {
        return Err(no_adapter_help(&format!(
            "only a software rasterizer is available ({}); it is not accepted as GPU \
             evidence. Pass --allow-software to run anyway (results are labelled software)",
            best.get_info().name
        )));
    }
    Ok(best)
}

fn no_adapter_help(reason: &str) -> String {
    format!(
        "GPU initialisation failed: {reason}.\n\
         Check that a hardware driver is installed and visible:\n  \
         Linux: install the vendor Vulkan ICD (e.g. nvidia-utils / mesa-vulkan-drivers) and run `vulkaninfo --summary`\n  \
         Windows: update the GPU driver (Direct3D 12 or Vulkan)\n  \
         macOS: Metal is built in; check the machine supports Metal 2+\n\
         Set WGPU_BACKEND=vulkan|dx12|metal|gl to force a backend when diagnosing."
    )
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub limits: wgpu::Limits,
    pub software: bool,
    masks: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    composite: wgpu::ComputePipeline,
}

impl Gpu {
    pub fn new(adapter: wgpu::Adapter, default_limits: bool) -> Result<Self, String> {
        let info = adapter.get_info();
        let limits = if default_limits {
            wgpu::Limits::default()
        } else {
            adapter.limits()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gpu-tiles"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: Default::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: Default::default(),
        }))
        .map_err(|e| format!("request_device failed on {}: {e}", info.name))?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("paint.wgsl").into()),
        });
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let masks = make("masks_main");
        let blur = make("blur_h_main");
        let composite = make("composite_main");
        let software = is_software(&info);
        Ok(Gpu {
            device,
            queue,
            info,
            limits,
            software,
            masks,
            blur,
            composite,
        })
    }
}

#[derive(Clone, Copy)]
pub struct Job {
    pub width: u32,
    pub height: u32,
    /// Tile edge in pixels; 0 renders the whole image as a single tile.
    pub tile: u32,
    /// Apron in pixels; None = exactly the blur radius (the finite support).
    pub apron: Option<u32>,
    pub seed: u64,
    pub tile_local_noise: bool,
}

impl Job {
    pub fn short_side(&self) -> u32 {
        self.width.min(self.height)
    }
    /// Soft-edge / pooling radius: 0.012 canvas units, so it scales with
    /// output resolution like every other mark.
    pub fn blur_radius(&self) -> u32 {
        ((self.short_side() as f64) * 0.012).round().max(1.0) as u32
    }
}

#[derive(Default, Debug)]
pub struct Stats {
    pub tiles: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    pub apron: u32,
    pub blur_radius: u32,
    pub gpu_bytes: u64,
    pub band_bytes: u64,
    pub setup: Duration,
    pub gpu_and_readback: Duration,
    pub sink: Duration,
    pub total: Duration,
}

/// Renders `job` tile by tile and hands each completed horizontal band of
/// RGBA8 rows to `sink(first_row, rows, bytes)`. GPU memory is bounded by the
/// tile size; CPU memory by one band.
pub fn render(
    gpu: &Gpu,
    scene: &Scene,
    job: Job,
    mut sink: impl FnMut(u32, u32, &[u8]) -> Result<(), String>,
) -> Result<Stats, String> {
    let t_start = Instant::now();
    let dev = &gpu.device;
    let radius = job.blur_radius();
    let apron = job.apron.unwrap_or(radius);
    let (tw, th) = if job.tile == 0 {
        (job.width, job.height)
    } else {
        (job.tile.min(job.width), job.tile.min(job.height))
    };
    let (ew, eh) = (tw + 2 * apron, th + 2 * apron);
    let maxdim = gpu.limits.max_texture_dimension_2d;
    if ew > maxdim || eh > maxdim {
        return Err(format!(
            "tile {tw}x{th} plus apron {apron} needs a {ew}x{eh} texture, above this \
             adapter's max_texture_dimension_2d={maxdim}; use a smaller --tile"
        ));
    }

    let tex = |label: &str, w: u32, h: u32, format, usage| {
        dev.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let inter = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
    let masks = tex("masks", ew, eh, wgpu::TextureFormat::Rgba16Float, inter);
    let blurh = tex("blur_h", ew, eh, wgpu::TextureFormat::Rgba16Float, inter);
    let out = tex(
        "out",
        tw,
        th,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let padded_bpr =
        (tw * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging_size = padded_bpr as u64 * th as u64;
    let staging = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: staging_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let buf = |label: &str, data: &[[f32; 4]], usage| {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let b = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len().max(16) as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&b, 0, &bytes);
        b
    };
    let ridges = buf("ridges", &scene.ridges, wgpu::BufferUsages::STORAGE);
    let strokes = buf("strokes", &scene.strokes, wgpu::BufferUsages::STORAGE);
    let params = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: 72,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let view = |t: &wgpu::Texture| t.create_view(&Default::default());
    let (masks_v, blurh_v, out_v) = (view(&masks), view(&blurh), view(&out));
    let bg = |p: &wgpu::ComputePipeline, entries: &[(u32, wgpu::BindingResource)]| {
        let entries: Vec<_> = entries
            .iter()
            .map(|(b, r)| wgpu::BindGroupEntry {
                binding: *b,
                resource: r.clone(),
            })
            .collect();
        dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &p.get_bind_group_layout(0),
            entries: &entries,
        })
    };
    let bg_masks = bg(
        &gpu.masks,
        &[
            (0, params.as_entire_binding()),
            (1, ridges.as_entire_binding()),
            (3, wgpu::BindingResource::TextureView(&masks_v)),
        ],
    );
    let bg_blur = bg(
        &gpu.blur,
        &[
            (0, params.as_entire_binding()),
            (4, wgpu::BindingResource::TextureView(&masks_v)),
            (5, wgpu::BindingResource::TextureView(&blurh_v)),
        ],
    );
    let bg_comp = bg(
        &gpu.composite,
        &[
            (0, params.as_entire_binding()),
            (1, ridges.as_entire_binding()),
            (2, strokes.as_entire_binding()),
            (4, wgpu::BindingResource::TextureView(&masks_v)),
            (6, wgpu::BindingResource::TextureView(&blurh_v)),
            (7, wgpu::BindingResource::TextureView(&out_v)),
        ],
    );

    let short = job.short_side() as f32;
    let mut stats = Stats {
        tile_w: tw,
        tile_h: th,
        apron,
        blur_radius: radius,
        gpu_bytes: 2 * (ew as u64 * eh as u64 * 8) + tw as u64 * th as u64 * 4 + staging_size,
        band_bytes: job.width as u64 * th as u64 * 4,
        setup: t_start.elapsed(),
        ..Default::default()
    };
    let mut band = vec![0u8; stats.band_bytes as usize];

    for y0 in (0..job.height).step_by(th as usize) {
        let h = th.min(job.height - y0);
        for x0 in (0..job.width).step_by(tw as usize) {
            let w = tw.min(job.width - x0);
            let t_tile = Instant::now();
            let (xw, xh) = (w + 2 * apron, h + 2 * apron);
            let words: [u32; 18] = [
                job.width,
                job.height,
                (x0 as i32 - apron as i32) as u32,
                (y0 as i32 - apron as i32) as u32,
                xw,
                xh,
                apron,
                apron,
                w,
                h,
                radius,
                // The shader hashes 32 bits of seed; fold the 64-bit value.
                (job.seed ^ (job.seed >> 32)) as u32,
                job.tile_local_noise as u32,
                scene.n_strokes,
                short.to_bits(),
                (job.width as f32 / short).to_bits(),
                0,
                0,
            ];
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            gpu.queue.write_buffer(&params, 0, &bytes);

            let mut enc = dev.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&gpu.masks);
                pass.set_bind_group(0, &bg_masks, &[]);
                pass.dispatch_workgroups(xw.div_ceil(8), xh.div_ceil(8), 1);
                pass.set_pipeline(&gpu.blur);
                pass.set_bind_group(0, &bg_blur, &[]);
                pass.dispatch_workgroups(xw.div_ceil(8), xh.div_ceil(8), 1);
                pass.set_pipeline(&gpu.composite);
                pass.set_bind_group(0, &bg_comp, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &out,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr),
                        rows_per_image: Some(th),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit([enc.finish()]);
            let used = padded_bpr as u64 * h as u64;
            staging.map_async(wgpu::MapMode::Read, 0..used, |r| {
                r.expect("staging map failed");
            });
            dev.poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| format!("device poll failed: {e}"))?;
            {
                let data = staging
                    .get_mapped_range(0..used)
                    .map_err(|e| format!("mapped range: {e:?}"))?;
                for row in 0..h as usize {
                    let src = &data[row * padded_bpr as usize..][..w as usize * 4];
                    let dst_off = (row * job.width as usize + x0 as usize) * 4;
                    band[dst_off..dst_off + w as usize * 4].copy_from_slice(src);
                }
            }
            staging.unmap();
            stats.gpu_and_readback += t_tile.elapsed();
            stats.tiles += 1;
        }
        let t_sink = Instant::now();
        sink(y0, h, &band[..job.width as usize * h as usize * 4])?;
        stats.sink += t_sink.elapsed();
    }
    stats.total = t_start.elapsed();
    Ok(stats)
}
