//! The tile loop shared by every renderer: plan → per-tile encode → readback
//! → ordered bands → sink, with cancellation between tiles, progress after
//! each tile, device-loss checks and `sink.abort()` on every failure path.
//!
//! A renderer supplies a [`TilePasses`]: the resources it allocated for this
//! render and a function that records one tile's compute passes, ending with
//! the tile's pixels in [`TilePasses::output`] (`rgba8unorm`, at least
//! `plan.tile_w × plan.tile_h`).

use std::sync::mpsc;
use std::time::Instant;

use pigment_core::error::{RenderError, SinkError};
use pigment_core::job::{CancelToken, Phase, Progress, ProgressSink};
use pigment_core::request::{RenderOutcome, RenderReport, RenderRequest, RenderTimings, TileSink};
use pigment_core::tiles::{Tile, TilePlan};

use crate::context::GpuContext;

pub(crate) trait TilePasses {
    /// The `rgba8unorm` texture a tile's pixels end up in.
    fn output(&self) -> &wgpu::Texture;
    /// Upload per-tile parameters and record this tile's passes.
    fn encode(&self, tile: &Tile, encoder: &mut wgpu::CommandEncoder);
}

/// Runs `passes` over every tile of `plan`. `t0` is when the render started
/// (setup time is measured from it to the first tile).
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive(
    ctx: &GpuContext,
    req: &RenderRequest,
    plan: TilePlan,
    renderer_version: u32,
    passes: &dyn TilePasses,
    t0: Instant,
    cancel: &CancelToken,
    progress: &mut dyn ProgressSink,
    sink: &mut dyn TileSink,
) -> Result<RenderReport, RenderError> {
    let dev = &ctx.device;
    let report = |outcome, timings| RenderReport {
        id: req.id,
        purpose: req.purpose,
        outcome,
        plan,
        timings,
        renderer_version,
        device: ctx.capabilities.label(),
        software_adapter: ctx.capabilities.adapter.software,
    };
    let padded_bpr = (plan.tile_w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let staging = ctx.scoped("readback allocation", || {
        dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: padded_bpr as u64 * plan.tile_h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    })?;
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
    // Tiles of the current band read back so far; the band goes to the sink
    // when all `plan.cols` are in, whatever order they arrived in.
    let mut in_band = 0u32;

    for tile in plan.tiles_in(req.target.order) {
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
        let mut enc = dev.create_command_encoder(&Default::default());
        passes.encode(&tile, &mut enc);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: passes.output(),
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

        in_band += 1;
        if in_band == plan.cols {
            in_band = 0;
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

/// A texture for one tile (or its apron-extended region).
pub(crate) fn texture(
    ctx: &GpuContext,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
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
}

/// A read-only storage buffer holding `data` (at least 16 bytes).
pub(crate) fn storage_buffer(ctx: &GpuContext, label: &str, data: &[f32]) -> wgpu::Buffer {
    storage_buffer_bytes(
        ctx,
        label,
        data.iter().flat_map(|f| f.to_le_bytes()).collect(),
    )
}

/// [`storage_buffer`] for `u32` data.
pub(crate) fn storage_buffer_u32(ctx: &GpuContext, label: &str, data: &[u32]) -> wgpu::Buffer {
    storage_buffer_bytes(
        ctx,
        label,
        data.iter().flat_map(|v| v.to_le_bytes()).collect(),
    )
}

fn storage_buffer_bytes(ctx: &GpuContext, label: &str, bytes: Vec<u8>) -> wgpu::Buffer {
    let b = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (bytes.len() as u64).max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    ctx.queue.write_buffer(&b, 0, &bytes);
    b
}
