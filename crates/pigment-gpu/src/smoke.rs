//! Tiled test-card renderer: the GPU capability smoke path.
//!
//! Implements [`Renderer`] end to end (tile plan → per-tile compute →
//! readback → ordered bands → sink, with cancellation, progress and error
//! scopes) using a flat-value test card instead of painting. The painting
//! renderer (tasks 06–07) lives beside it in `paint.rs` and follows the
//! same loop.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use pigment_core::error::{RenderError, SinkError};
use pigment_core::job::{CancelToken, Phase, Progress, ProgressSink};
use pigment_core::request::{
    RenderOutcome, RenderReport, RenderRequest, RenderTimings, Renderer, TileSink,
};
use pigment_core::seed::Domain;
use pigment_core::settings::Appearance;
use pigment_core::tiles::{Support, TileCostModel, TilePlan, apron_pixels};

use crate::context::GpuContext;

/// Version of the test card's *pixels*; not `RENDERER_VERSION`.
pub const SMOKE_RENDERER_VERSION: u32 = 0;

const PARAMS_BYTES: u64 = 48;

#[derive(Debug)]
pub struct SmokeRenderer {
    ctx: Arc<GpuContext>,
    field: wgpu::ComputePipeline,
    composite: wgpu::ComputePipeline,
}

impl SmokeRenderer {
    pub fn new(ctx: Arc<GpuContext>) -> Result<SmokeRenderer, RenderError> {
        let (field, composite) = ctx.scoped("shader compilation", || {
            let module = ctx
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("smoke.wgsl"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("smoke.wgsl").into()),
                });
            let make = |entry: &str| {
                ctx.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    })
            };
            (make("field_main"), make("composite_main"))
        })?;
        Ok(SmokeRenderer {
            ctx,
            field,
            composite,
        })
    }

    fn upload_scene(&self, req: &RenderRequest) -> (wgpu::Buffer, wgpu::Buffer, u32) {
        let mut headers: Vec<f32> = Vec::new();
        let mut verts: Vec<f32> = Vec::new();
        for l in req.scene.layers() {
            let first = (verts.len() / 2) as f32;
            // Flat value by depth: nearer is darker (linear light).
            let value = 0.08 + 0.72 * l.depth;
            headers.extend([first, l.outline.len() as f32, l.depth, value]);
            for p in &l.outline {
                verts.extend([p.x, p.y]);
            }
        }
        let buf = |label: &str, data: &[f32]| {
            let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
            let b = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (bytes.len() as u64).max(16),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.ctx.queue.write_buffer(&b, 0, &bytes);
            b
        };
        (
            buf("layers", &headers),
            buf("verts", &verts),
            req.scene.layers().len() as u32,
        )
    }
}

impl Renderer for SmokeRenderer {
    fn cost_model(&self) -> TileCostModel {
        TileCostModel {
            extended_bytes_per_px: 8, // one rgba16float field
            output_bytes_per_px: 4,
            staging_bytes_per_px: 4,
        }
    }

    fn supports(&self, appearance: &Appearance) -> Vec<Support> {
        vec![Support {
            pass: "edge-bleed",
            radius: appearance.painting.edge_bleed_radius(),
        }]
    }

    fn render(
        &self,
        req: &RenderRequest,
        cancel: &CancelToken,
        progress: &mut dyn ProgressSink,
        sink: &mut dyn TileSink,
    ) -> Result<RenderReport, RenderError> {
        let t0 = Instant::now();
        let ctx = &self.ctx;
        let dev = &ctx.device;
        ctx.check_alive()?;
        req.appearance.validate()?;
        let mapping = req.mapping();
        let apron = apron_pixels(&mapping, &self.supports(&req.appearance));
        let plan = TilePlan::new(
            req.target.width,
            req.target.height,
            apron,
            req.target.policy,
            ctx.capabilities.device_limits.tile_limits(),
            self.cost_model(),
        )?;
        let report = |outcome, timings| RenderReport {
            id: req.id,
            purpose: req.purpose,
            outcome,
            plan,
            timings,
            renderer_version: SMOKE_RENDERER_VERSION,
            device: ctx.capabilities.label(),
            software_adapter: ctx.capabilities.adapter.software,
        };
        progress.report(Progress {
            id: req.id,
            phase: Phase::Setup,
            done: 0,
            total: plan.len(),
        });

        let (ew, eh) = (plan.tile_w + 2 * apron, plan.tile_h + 2 * apron);
        let padded_bpr = (plan.tile_w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let res = ctx.scoped("tile resource allocation", || {
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
            let field = tex(
                "field",
                ew,
                eh,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let out = tex(
                "out",
                plan.tile_w,
                plan.tile_h,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            );
            let staging = dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("staging"),
                size: padded_bpr as u64 * plan.tile_h as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let params = dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("params"),
                size: PARAMS_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (field, out, staging, params)
        })?;
        let (field, out, staging, params) = res;
        let (layers, verts, n_layers) = self.upload_scene(req);
        let field_v = field.create_view(&Default::default());
        let out_v = out.create_view(&Default::default());
        let bind = |p: &wgpu::ComputePipeline, entries: &[wgpu::BindGroupEntry]| {
            dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &p.get_bind_group_layout(0),
                entries,
            })
        };
        let bg_field = bind(
            &self.field,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: layers.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: verts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&field_v),
                },
            ],
        );
        let bg_comp = bind(
            &self.composite,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&field_v),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&out_v),
                },
            ],
        );

        let seed = {
            let s = req.seeds.stream(Domain::PaintDetail).0;
            (s ^ (s >> 32)) as u32
        };
        let scale = [
            (mapping.extents.width / mapping.pixels_w as f64) as f32,
            (mapping.extents.height / mapping.pixels_h as f64) as f32,
        ];
        let mut timings = RenderTimings {
            setup: t0.elapsed(),
            ..Default::default()
        };
        let fail = |sink: &mut dyn TileSink, e: RenderError| {
            sink.abort();
            Err(e)
        };
        if let Err(e) = sink.begin(plan.image_w, plan.image_h) {
            return fail(sink, e.into());
        }
        let row_bytes = plan.image_w as usize * 4;
        let mut band = vec![0u8; row_bytes * plan.tile_h as usize];
        let mut done = 0u32;

        for tile in plan.tiles() {
            if cancel.is_cancelled() {
                sink.abort();
                timings.total = t0.elapsed();
                return Ok(report(
                    RenderOutcome::Cancelled { tiles_done: done },
                    timings,
                ));
            }
            if let Err(e) = ctx.check_alive() {
                return fail(sink, e);
            }
            let t_tile = Instant::now();
            let mut words = Vec::with_capacity(PARAMS_BYTES as usize);
            for v in [tile.ext_x as i32, tile.ext_y as i32] {
                words.extend(v.to_le_bytes());
            }
            for v in [tile.ext_w, tile.ext_h, tile.w, tile.h] {
                words.extend(v.to_le_bytes());
            }
            for v in scale {
                words.extend(v.to_le_bytes());
            }
            for v in [apron, seed, n_layers] {
                words.extend(v.to_le_bytes());
            }
            words.extend((req.appearance.painting.paper_grain as f32).to_le_bytes());
            ctx.queue.write_buffer(&params, 0, &words);

            let mut enc = dev.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.field);
                pass.set_bind_group(0, &bg_field, &[]);
                pass.dispatch_workgroups(tile.ext_w.div_ceil(8), tile.ext_h.div_ceil(8), 1);
                pass.set_pipeline(&self.composite);
                pass.set_bind_group(0, &bg_comp, &[]);
                pass.dispatch_workgroups(tile.w.div_ceil(8), tile.h.div_ceil(8), 1);
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
                        rows_per_image: Some(plan.tile_h),
                    },
                },
                wgpu::Extent3d {
                    width: tile.w,
                    height: tile.h,
                    depth_or_array_layers: 1,
                },
            );
            ctx.queue.submit([enc.finish()]);
            let used = padded_bpr as u64 * tile.h as u64;
            let (tx, rx) = mpsc::channel();
            staging.map_async(wgpu::MapMode::Read, 0..used, move |r| {
                let _ = tx.send(r);
            });
            if let Err(e) = dev.poll(wgpu::PollType::wait_indefinitely()) {
                return fail(
                    sink,
                    RenderError::Gpu {
                        detail: format!("device poll: {e}"),
                    },
                );
            }
            match rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let e = ctx.check_alive().err().unwrap_or(RenderError::Gpu {
                        detail: format!("readback map failed: {e}"),
                    });
                    return fail(sink, e);
                }
                Err(_) => {
                    return fail(
                        sink,
                        RenderError::Gpu {
                            detail: "readback callback dropped".into(),
                        },
                    );
                }
            }
            {
                let data = match staging.get_mapped_range(0..used) {
                    Ok(d) => d,
                    Err(e) => {
                        return fail(
                            sink,
                            RenderError::Gpu {
                                detail: format!("mapped range: {e:?}"),
                            },
                        );
                    }
                };
                for row in 0..tile.h as usize {
                    let src = &data[row * padded_bpr as usize..][..tile.w as usize * 4];
                    let dst = row * row_bytes + tile.x as usize * 4;
                    band[dst..dst + src.len()].copy_from_slice(src);
                }
            }
            staging.unmap();
            timings.render_readback += t_tile.elapsed();
            done += 1;
            progress.report(Progress {
                id: req.id,
                phase: Phase::Tiles,
                done,
                total: plan.len(),
            });

            if tile.x + tile.w == plan.image_w {
                let t_sink = Instant::now();
                let rows = tile.h as usize;
                if let Err(e) = sink.band(tile.y, tile.h, &band[..rows * row_bytes]) {
                    return fail(sink, e.into());
                }
                timings.sink += t_sink.elapsed();
            }
        }
        if let Err(e) = ctx.check_alive() {
            return fail(sink, e);
        }
        progress.report(Progress {
            id: req.id,
            phase: Phase::Finalize,
            done,
            total: plan.len(),
        });
        if let Err(e) = sink.finish() {
            let e: SinkError = e;
            return fail(sink, e.into());
        }
        timings.total = t0.elapsed();
        Ok(report(RenderOutcome::Completed, timings))
    }
}
