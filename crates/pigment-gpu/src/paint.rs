//! The painting renderer (task 06): color planes, washes and loose edges.
//!
//! Two compute passes per tile over the shared tile loop ([`crate::tiled`]):
//! materials (palette colors by role and structural shade, foliage, water,
//! treeline and snowline, aerial perspective, saturation, wash/gouache), then
//! painting (loose edges, pooling, paper and granulation). See `paint.wgsl`
//! and `docs/painting.md`.
//!
//! Only `Domain::PaintDetail` and the appearance settings are read here; the
//! scene is never modified, so paint settings cannot move geometry.

use std::sync::Arc;
use std::time::Instant;

use pigment_core::error::RenderError;
use pigment_core::job::{CancelToken, Phase, Progress, ProgressSink};
use pigment_core::palette::palette;
use pigment_core::request::{RenderReport, RenderRequest, Renderer, TileSink};
use pigment_core::scene::{Scene, metrics};
use pigment_core::seed::Domain;
use pigment_core::settings::Appearance;
use pigment_core::tiles::{Support, Tile, TileCostModel, TilePlan, apron_pixels};
use pigment_core::version::RENDERER_VERSION;

use crate::context::GpuContext;
use crate::tiled::{TilePasses, drive, storage_buffer, texture};

const PARAMS_BYTES: u64 = 96;

/// Loose edges read up to this multiple of the edge-bleed radius from a
/// pixel (the warp plus the softening disc).
const EDGE_REACH: f64 = 1.4;

#[derive(Debug)]
pub struct PaintRenderer {
    ctx: Arc<GpuContext>,
    materials: wgpu::ComputePipeline,
    paint: wgpu::ComputePipeline,
}

impl PaintRenderer {
    pub fn new(ctx: Arc<GpuContext>) -> Result<PaintRenderer, RenderError> {
        let (materials, paint) = ctx.scoped("shader compilation", || {
            let module = ctx
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("paint.wgsl"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("paint.wgsl").into()),
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
            (make("materials_main"), make("paint_main"))
        })?;
        Ok(PaintRenderer {
            ctx,
            materials,
            paint,
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

struct PaintTiles<'a> {
    renderer: &'a PaintRenderer,
    out: wgpu::Texture,
    params: wgpu::Buffer,
    bg_materials: wgpu::BindGroup,
    bg_paint: wgpu::BindGroup,
    /// Everything but the tile origin and sizes, already encoded.
    fixed: Vec<u8>,
}

impl TilePasses for PaintTiles<'_> {
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
        words.extend(&self.fixed);
        self.renderer
            .ctx
            .queue
            .write_buffer(&self.params, 0, &words);

        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.renderer.materials);
        pass.set_bind_group(0, &self.bg_materials, &[]);
        pass.dispatch_workgroups(tile.ext_w.div_ceil(8), tile.ext_h.div_ceil(8), 1);
        pass.set_pipeline(&self.renderer.paint);
        pass.set_bind_group(0, &self.bg_paint, &[]);
        pass.dispatch_workgroups(tile.w.div_ceil(8), tile.h.div_ceil(8), 1);
    }
}

impl Renderer for PaintRenderer {
    fn cost_model(&self) -> TileCostModel {
        TileCostModel {
            extended_bytes_per_px: 8, // one rgba16float material field
            output_bytes_per_px: 4,
            staging_bytes_per_px: 4,
        }
    }

    /// Loose edges: the warp and softening disc reach `EDGE_REACH` × the
    /// edge-bleed radius, plus one pixel of rounding.
    fn supports(&self, appearance: &Appearance) -> Vec<Support> {
        let r = appearance.painting.edge_bleed_radius();
        if r == 0.0 {
            return Vec::new();
        }
        vec![
            Support {
                pass: "loose-edges",
                radius: r * EDGE_REACH,
            },
            Support {
                pass: "rounding",
                radius: f64::MIN_POSITIVE,
            },
        ]
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
                "materials",
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
        let pal: Vec<f32> = palette(req.appearance.palette.id)
            .gpu()
            .into_iter()
            .flatten()
            .collect();
        let pal = storage_buffer(ctx, "palette", &pal);
        let field_v = field.create_view(&Default::default());
        let out_v = out.create_view(&Default::default());
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let bind = |p: &wgpu::ComputePipeline, entries: &[wgpu::BindGroupEntry]| {
            dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &p.get_bind_group_layout(0),
                entries,
            })
        };
        let bg_materials = bind(
            &self.materials,
            &[
                entry(0, params.as_entire_binding()),
                entry(1, layers.as_entire_binding()),
                entry(2, verts.as_entire_binding()),
                entry(3, pal.as_entire_binding()),
                entry(4, wgpu::BindingResource::TextureView(&field_v)),
            ],
        );
        let bg_paint = bind(
            &self.paint,
            &[
                entry(0, params.as_entire_binding()),
                entry(1, layers.as_entire_binding()),
                entry(5, wgpu::BindingResource::TextureView(&field_v)),
                entry(6, wgpu::BindingResource::TextureView(&out_v)),
            ],
        );

        let a = &req.appearance;
        let seed = {
            let s = req.seeds.stream(Domain::PaintDetail).0;
            (s ^ (s >> 32)) as u32
        };
        let (horizon, summit) = metrics::horizon_and_summit(&req.scene);
        let ext = req.scene.extents();
        let mut fixed = Vec::new();
        for v in [
            (mapping.extents.width / mapping.pixels_w as f64) as f32,
            (mapping.extents.height / mapping.pixels_h as f64) as f32,
        ] {
            fixed.extend(v.to_le_bytes());
        }
        for v in [apron, req.scene.layers().len() as u32, seed, 0] {
            fixed.extend(v.to_le_bytes());
        }
        for v in [
            a.painting.edge_looseness,
            a.painting.wash_gouache,
            a.painting.mark_scale,
            a.painting.granulation,
            a.painting.paper_grain,
            a.palette.intensity,
            a.atmosphere.haze,
            mapping.pixel_footprint(),
            horizon,
            summit,
            ext.width,
            ext.height,
        ] {
            fixed.extend((v as f32).to_le_bytes());
        }
        let tiles = PaintTiles {
            renderer: self,
            out,
            params,
            bg_materials,
            bg_paint,
            fixed,
        };
        drive(
            ctx,
            req,
            plan,
            RENDERER_VERSION,
            &tiles,
            t0,
            cancel,
            progress,
            sink,
        )
    }
}
