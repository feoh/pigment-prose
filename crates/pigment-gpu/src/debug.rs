//! Scene debug renderer (task 05): shows structure, not painting.
//!
//! - [`DebugView::Flat`]: neutral gray values per layer from its role and
//!   structural `shade`, lightened with depth. Checks value grouping and
//!   depth planes before any paint exists.
//! - [`DebugView::Regions`]: a fixed color per role, modulated by `shade`,
//!   with a 1-pixel outline wherever the front-most layer changes. Shows the
//!   region topology later passes select by (woodland, water, rocks).
//! - [`DebugView::LayerIds`]: the front-most layer index + 1 in the red
//!   channel, unencoded (0 = uncovered). For comparing against the CPU
//!   reference rasterizer in tests.
//!
//! Coverage follows the CPU reference rasterizer's rule exactly
//! (`pigment_core::scene::raster`), so the two can be compared pixel by
//! pixel. Uses the shared tile loop, so tiled output equals single-tile.

use std::sync::Arc;
use std::time::Instant;

use pigment_core::error::RenderError;
use pigment_core::job::{CancelToken, Phase, Progress, ProgressSink};
use pigment_core::request::{RenderReport, RenderRequest, Renderer, TileSink};
use pigment_core::scene::Scene;
use pigment_core::settings::Appearance;
use pigment_core::tiles::{Support, Tile, TileCostModel, TilePlan, apron_pixels};

use crate::context::GpuContext;
use crate::tiled::{TilePasses, drive, storage_buffer, texture};

/// Version of the debug views' pixels; not `RENDERER_VERSION`.
pub const DEBUG_RENDERER_VERSION: u32 = 0;

const PARAMS_BYTES: u64 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView {
    Flat,
    Regions,
    LayerIds,
}

#[derive(Debug)]
pub struct DebugRenderer {
    ctx: Arc<GpuContext>,
    view: DebugView,
    field: wgpu::ComputePipeline,
    composite: wgpu::ComputePipeline,
}

impl DebugRenderer {
    pub fn new(ctx: Arc<GpuContext>, view: DebugView) -> Result<DebugRenderer, RenderError> {
        let (field, composite) = ctx.scoped("shader compilation", || {
            let module = ctx
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("debug.wgsl"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("debug.wgsl").into()),
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
        Ok(DebugRenderer {
            ctx,
            view,
            field,
            composite,
        })
    }

    fn upload_scene(&self, scene: &Scene) -> (wgpu::Buffer, wgpu::Buffer) {
        let mut headers: Vec<f32> = Vec::new();
        let mut verts: Vec<f32> = Vec::new();
        for l in scene.layers() {
            let first = (verts.len() / 2) as f32;
            let (mut minx, mut miny) = (f32::INFINITY, f32::INFINITY);
            let (mut maxx, mut maxy) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            for p in &l.outline {
                verts.extend([p.x, p.y]);
                minx = minx.min(p.x);
                miny = miny.min(p.y);
                maxx = maxx.max(p.x);
                maxy = maxy.max(p.y);
            }
            headers.extend([first, l.outline.len() as f32, l.depth, l.shade]);
            headers.extend([minx, miny, maxx, maxy]);
            headers.extend([l.role as u8 as f32, 0.0, 0.0, 0.0]);
        }
        (
            storage_buffer(&self.ctx, "layers", &headers),
            storage_buffer(&self.ctx, "verts", &verts),
        )
    }
}

struct DebugTiles<'a> {
    renderer: &'a DebugRenderer,
    out: wgpu::Texture,
    params: wgpu::Buffer,
    bg_field: wgpu::BindGroup,
    bg_comp: wgpu::BindGroup,
    apron: u32,
    n_layers: u32,
    scale: [f32; 2],
}

impl TilePasses for DebugTiles<'_> {
    fn output(&self) -> &wgpu::Texture {
        &self.out
    }

    fn encode(&self, tile: &Tile, enc: &mut wgpu::CommandEncoder) {
        let mut words = Vec::with_capacity(PARAMS_BYTES as usize);
        for v in [tile.ext_x as i32, tile.ext_y as i32] {
            words.extend(v.to_le_bytes());
        }
        for v in [tile.ext_w, tile.ext_h, tile.w, tile.h] {
            words.extend(v.to_le_bytes());
        }
        for v in self.scale {
            words.extend(v.to_le_bytes());
        }
        let view = match self.renderer.view {
            DebugView::Flat => 0u32,
            DebugView::Regions => 1,
            DebugView::LayerIds => 2,
        };
        for v in [self.apron, self.n_layers, view, 0] {
            words.extend(v.to_le_bytes());
        }
        self.renderer
            .ctx
            .queue
            .write_buffer(&self.params, 0, &words);

        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.renderer.field);
        pass.set_bind_group(0, &self.bg_field, &[]);
        pass.dispatch_workgroups(tile.ext_w.div_ceil(8), tile.ext_h.div_ceil(8), 1);
        pass.set_pipeline(&self.renderer.composite);
        pass.set_bind_group(0, &self.bg_comp, &[]);
        pass.dispatch_workgroups(tile.w.div_ceil(8), tile.h.div_ceil(8), 1);
    }
}

impl Renderer for DebugRenderer {
    fn cost_model(&self) -> TileCostModel {
        TileCostModel {
            extended_bytes_per_px: 8, // one rgba16float layer-index field
            output_bytes_per_px: 4,
            staging_bytes_per_px: 4,
        }
    }

    /// Regions: one pixel of neighbourhood for the outlines, whatever the
    /// resolution (a tiny radius rounds up to one pixel). Flat: none.
    fn supports(&self, _appearance: &Appearance) -> Vec<Support> {
        match self.view {
            DebugView::Flat | DebugView::LayerIds => Vec::new(),
            DebugView::Regions => vec![Support {
                pass: "region-outline",
                radius: f64::MIN_POSITIVE,
            }],
        }
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
        progress.report(Progress {
            id: req.id,
            phase: Phase::Setup,
            done: 0,
            total: plan.len(),
        });
        let (ew, eh) = (plan.tile_w + 2 * apron, plan.tile_h + 2 * apron);
        let (field, out, params) = ctx.scoped("tile resource allocation", || {
            let field = texture(
                ctx,
                "field",
                ew,
                eh,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let out = texture(
                ctx,
                "out",
                plan.tile_w,
                plan.tile_h,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            );
            let params = dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("params"),
                size: PARAMS_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (field, out, params)
        })?;
        let (layers, verts) = self.upload_scene(&req.scene);
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
                    binding: 1,
                    resource: layers.as_entire_binding(),
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
        let tiles = DebugTiles {
            renderer: self,
            out,
            params,
            bg_field,
            bg_comp,
            apron,
            n_layers: req.scene.layers().len() as u32,
            scale: [
                (mapping.extents.width / mapping.pixels_w as f64) as f32,
                (mapping.extents.height / mapping.pixels_h as f64) as f32,
            ],
        };
        drive(
            ctx,
            req,
            plan,
            DEBUG_RENDERER_VERSION,
            &tiles,
            t0,
            cancel,
            progress,
            sink,
        )
    }
}
