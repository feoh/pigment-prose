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

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pigment_core::error::RenderError;
use pigment_core::job::{CancelToken, Phase, Progress, ProgressSink};
use pigment_core::palette::palette;
use pigment_core::request::{RenderReport, RenderRequest, Renderer, TileSink};
use pigment_core::scene::{CanvasPoint, LayerRole, LightSide, Scene, metrics};
use pigment_core::seed::Domain;
use pigment_core::settings::Appearance;
use pigment_core::tiles::{Support, Tile, TileCostModel, TilePlan, apron_pixels};
use pigment_core::version::RENDERER_VERSION;

use crate::context::GpuContext;
use crate::coverage::CoverageIndex;
use crate::tiled::{TilePasses, drive, storage_buffer, storage_buffer_u32, texture};

const PARAMS_BYTES: u64 = 144;

// paint.wgsl measures wash/gouache character and texture strength from the
// defaults (DEFAULT_WASH_GOUACHE, DEFAULT_TEXTURE).
const _: () = {
    use pigment_core::settings::{GRANULATION, PAPER_GRAIN, WASH_GOUACHE};
    assert!(WASH_GOUACHE.default == 0.25);
    assert!(PAPER_GRAIN.default == 0.3 && GRANULATION.default == 0.3);
};

/// Loose edges read up to this multiple of the edge-bleed radius from a
/// pixel (the warp plus the softening disc).
const EDGE_REACH: f64 = 1.4;

#[derive(Debug)]
pub struct PaintRenderer {
    ctx: Arc<GpuContext>,
    materials: wgpu::ComputePipeline,
    paint: wgpu::ComputePipeline,
    /// The last scene's uploaded buffers, so renders that change only paint
    /// settings or size (slider drags) skip the coverage index build.
    last_scene: Mutex<Option<(Arc<Scene>, SceneBuffers)>>,
}

#[derive(Debug, Clone)]
struct SceneBuffers {
    layers: wgpu::Buffer,
    verts: wgpu::Buffer,
    bins: wgpu::Buffer,
    entries: wgpu::Buffer,
    /// The water's channel per row (renderer v2): see [`channel_rows`].
    channel: wgpu::Buffer,
    channel_desc: [f32; 4],
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
            last_scene: Mutex::new(None),
        })
    }

    /// The scene's buffers, reused while the same `Arc<Scene>` is rendered.
    /// Another renderer on the same context sharing this one's compiled
    /// pipelines, with its own scene cache: the studio's previews and
    /// exports each get one without compiling the shader twice (task 15).
    pub fn sibling(&self) -> PaintRenderer {
        PaintRenderer {
            ctx: self.ctx.clone(),
            materials: self.materials.clone(),
            paint: self.paint.clone(),
            last_scene: Mutex::new(None),
        }
    }

    fn scene_buffers(&self, scene: &Arc<Scene>) -> SceneBuffers {
        let mut last = self
            .last_scene
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((s, b)) = last.as_ref()
            && Arc::ptr_eq(s, scene)
        {
            return b.clone();
        }
        let b = self.upload_scene(scene);
        *last = Some((scene.clone(), b.clone()));
        b
    }

    /// Layer headers, vertices, coverage bins and bin entries.
    fn upload_scene(&self, scene: &Scene) -> SceneBuffers {
        let index = CoverageIndex::build(scene);
        let mut headers: Vec<f32> = Vec::new();
        let mut verts: Vec<f32> = Vec::new();
        for (l, lb) in scene.layers().iter().zip(&index.layers) {
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
            headers.extend([
                l.role as u8 as f32,
                l.plant as u8 as f32,
                lb.axis as u8 as f32,
                0.0,
            ]);
            headers.extend([lb.base as f32, lb.bins as f32, lb.inv_span, lb.min_v]);
        }
        let bins: Vec<u32> = index.bins.iter().flatten().copied().collect();
        let entries: Vec<u32> = index.entries.iter().flatten().copied().collect();
        let (rows, channel_desc) = channel_rows(scene);
        SceneBuffers {
            layers: storage_buffer(&self.ctx, "layers", &headers),
            verts: storage_buffer(&self.ctx, "verts", &verts),
            bins: storage_buffer_u32(&self.ctx, "coverage bins", &bins),
            entries: storage_buffer_u32(&self.ctx, "coverage entries", &entries),
            channel: storage_buffer(&self.ctx, "water channel", &rows),
            channel_desc,
        }
    }
}

/// A compositing operation of the reference model
/// ([`pigment_core::composite`]), for [`PaintRenderer::evaluate_compositing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositeOp {
    /// `glaze(under, color as transmittance, amount)`.
    Glaze = 0,
    /// `over(under, color, amount)`.
    Over = 1,
    /// `wash(under as paper, color, amount)`.
    Wash = 2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositeCase {
    pub op: CompositeOp,
    pub under: [f32; 3],
    pub color: [f32; 3],
    pub amount: f32,
}

impl PaintRenderer {
    /// Evaluates compositing cases with the painting shader's own functions,
    /// so tests can compare the GPU against the reference model.
    pub fn evaluate_compositing(
        &self,
        cases: &[CompositeCase],
    ) -> Result<Vec<[f32; 3]>, RenderError> {
        let ctx = &self.ctx;
        let dev = &ctx.device;
        ctx.check_alive()?;
        if cases.is_empty() {
            return Ok(Vec::new());
        }
        let input: Vec<f32> = cases
            .iter()
            .flat_map(|c| {
                [
                    c.under[0],
                    c.under[1],
                    c.under[2],
                    c.op as u8 as f32,
                    c.color[0],
                    c.color[1],
                    c.color[2],
                    c.amount,
                ]
            })
            .collect();
        let bytes = cases.len() as u64 * 16;
        let (pipeline, out, staging) = ctx.scoped("compositing reference", || {
            let module = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("paint.wgsl"),
                source: wgpu::ShaderSource::Wgsl(include_str!("paint.wgsl").into()),
            });
            let pipeline = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("composite_reference_main"),
                layout: None,
                module: &module,
                entry_point: Some("composite_reference_main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let buf = |label, usage| {
                dev.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage,
                    mapped_at_creation: false,
                })
            };
            (
                pipeline,
                buf(
                    "reference out",
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                ),
                buf(
                    "reference staging",
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                ),
            )
        })?;
        let input = storage_buffer(ctx, "reference cases", &input);
        let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: out.as_entire_binding(),
                },
            ],
        });
        let mut enc = dev.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&out, 0, &staging, 0, bytes);
        ctx.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        dev.poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Gpu {
                detail: format!("device poll: {e}"),
            })?;
        match rx.recv() {
            Ok(Ok(())) => {}
            _ => {
                return Err(ctx.check_alive().err().unwrap_or(RenderError::Gpu {
                    detail: "reference readback failed".into(),
                }));
            }
        }
        let data = staging.get_mapped_range(..).map_err(|e| RenderError::Gpu {
            detail: format!("mapped range: {e:?}"),
        })?;
        let floats: Vec<f32> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        Ok(floats
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| [v[0], v[1], v[2]])
            .collect())
    }
}

/// Union bounding box of the foreground rocks, `(min x, min y, max x, max
/// y)`, or an empty box (min > max) when there are none. The shader looks
/// for rock contacts and reflections only inside it.
fn rock_bounds(scene: &Scene) -> [f32; 4] {
    scene
        .layers()
        .iter()
        .filter(|l| l.role == LayerRole::ForegroundRock)
        .flat_map(|l| l.outline.iter())
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
            [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]
        })
}

/// Rows sampled down the water's extent.
pub const CHANNEL_ROWS: usize = 256;

/// The spans of `outline` on the row at `y`: pairs of crossings, sorted.
fn spans(outline: &[CanvasPoint], y: f32, out: &mut Vec<(f32, f32)>) {
    let mut xs = Vec::new();
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        if (a.y > y) != (b.y > y) {
            xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
        }
    }
    xs.sort_by(f32::total_cmp);
    out.extend(xs.as_chunks::<2>().0.iter().map(|&[a, b]| (a, b)));
}

/// The water's visible channel, row by row (renderer v2, task 25). A river
/// is the gap the banks leave in a broad water sheet, so for each of
/// [`CHANNEL_ROWS`] rows down the water's bounding box this takes the water
/// layer's spans minus everything drawn in front of it, and keeps the widest
/// visible piece: left bank, right bank, width. The shader runs a river's
/// current along it: a narrow channel is fast, a lake barely moves. A
/// function of the scene only. Returns the rows (`left, right, width, 0`
/// each) and the descriptor `(first row y, row step, row count, 1 if there
/// is water)`.
pub fn channel_rows(scene: &Scene) -> (Vec<f32>, [f32; 4]) {
    let layers = scene.layers();
    let Some(wi) = layers.iter().position(|l| l.role == LayerRole::Water) else {
        return (vec![0.0; 4], [0.0, 1.0, 1.0, 0.0]);
    };
    let o = &layers[wi].outline;
    let (y0, y1) = o
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let step = (y1 - y0) / (CHANNEL_ROWS - 1) as f32;
    let mut rows = Vec::with_capacity(4 * CHANNEL_ROWS);
    let (mut water, mut cover) = (Vec::new(), Vec::new());
    for k in 0..CHANNEL_ROWS {
        // Just inside the extent, so the first and last rows still cross.
        let y = (y0 + step * k as f32).clamp(y0 + 1e-5, y1 - 1e-5);
        water.clear();
        cover.clear();
        spans(o, y, &mut water);
        for l in &layers[wi + 1..] {
            spans(&l.outline, y, &mut cover);
        }
        cover.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Water minus the union of what is in front of it.
        let mut best = (0.0f32, 0.0f32);
        for &(wl, wr) in &water {
            let mut x = wl;
            for &(cl, cr) in &cover {
                if cr <= x || cl >= wr {
                    continue;
                }
                if cl > x && cl - x > best.1 - best.0 {
                    best = (x, cl);
                }
                x = x.max(cr);
                if x >= wr {
                    break;
                }
            }
            if wr > x && wr - x > best.1 - best.0 {
                best = (x, wr);
            }
        }
        rows.extend([best.0, best.1, best.1 - best.0, 0.0]);
    }
    (rows, [y0, step.max(1e-6), CHANNEL_ROWS as f32, 1.0])
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
        let SceneBuffers {
            layers,
            verts,
            bins,
            entries,
            channel,
            channel_desc,
        } = self.scene_buffers(&req.scene);
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
                entry(7, bins.as_entire_binding()),
                entry(8, entries.as_entire_binding()),
                entry(11, channel.as_entire_binding()),
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
        let light = match req.scene.light() {
            LightSide::Left => 0,
            LightSide::Right => 1,
        };
        for v in [apron, req.scene.layers().len() as u32, seed, light] {
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
        for v in rock_bounds(&req.scene) {
            fixed.extend(v.to_le_bytes());
        }
        let wind = req.scene.wind();
        for v in [wind.x, wind.z, wind.strength, 0.0] {
            fixed.extend(v.to_le_bytes());
        }
        for v in channel_desc {
            fixed.extend(v.to_le_bytes());
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
