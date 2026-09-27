//! Tile planning with halo (apron) accounting and a memory budget.
//!
//! Every neighbourhood effect declares a finite [`Support`] radius in canvas
//! units. The apron is the sum of the pixel supports of the chained passes.
//! Each tile is evaluated over interior + apron in whole-image coordinates,
//! and only the interior is written out. Aprons are recomputed per tile,
//! never exchanged, so tile size and order cannot change the image (task 02
//! verified bit-identical output on one device).

use std::fmt;

use crate::frame::CanvasMapping;

/// Finite support of one pass, in canvas units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Support {
    pub pass: &'static str,
    pub radius: f64,
}

/// Apron in pixels for passes applied one after another.
pub fn apron_pixels(mapping: &CanvasMapping, chained: &[Support]) -> u32 {
    chained
        .iter()
        .map(|s| mapping.support_pixels(s.radius))
        .sum()
}

/// Tile edges tried by [`TilePolicy::Budget`], largest first. Task 02: 1024–2048
/// px keeps apron overdraw near 1.2×; smaller tiles are only for tight budgets.
pub const TILE_EDGES: [u32; 4] = [2048, 1024, 512, 256];

/// Default cap on renderer-owned GPU allocations for one export job.
pub const DEFAULT_EXPORT_GPU_BUDGET: u64 = 256 << 20;
/// Default cap on the host band buffer (one row of tiles) for one export job.
pub const DEFAULT_HOST_BAND_BUDGET: u64 = 256 << 20;

/// wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`.
pub const COPY_ROW_ALIGNMENT: u64 = 256;

/// Bytes a renderer allocates per tile, by region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileCostModel {
    /// Intermediate textures covering interior + apron, per pixel.
    pub extended_bytes_per_px: u64,
    /// Output texture covering the interior, per pixel.
    pub output_bytes_per_px: u64,
    /// Readback staging per interior pixel before row alignment.
    pub staging_bytes_per_px: u64,
}

/// Device facts the planner needs (a subset of `GpuCapabilities`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceTileLimits {
    pub max_texture_dimension_2d: u32,
    pub max_buffer_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilePolicy {
    /// One tile covering the whole target. Previews use this.
    Single,
    /// Largest [`TILE_EDGES`] entry that fits both budgets. Exports use this.
    Budget { gpu_bytes: u64, host_bytes: u64 },
    /// Exact square edge. Regression and seam tests use this.
    Fixed { edge: u32 },
}

/// Order in which a render visits the tiles of each band. Bands always
/// complete top to bottom (a PNG writer streams them), but tile content must
/// not depend on the order within a band; the seam tests render both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileOrder {
    #[default]
    RowMajor,
    /// Right to left within each band.
    ReverseInBand,
}

impl TilePolicy {
    pub fn default_export() -> TilePolicy {
        TilePolicy::Budget {
            gpu_bytes: DEFAULT_EXPORT_GPU_BUDGET,
            host_bytes: DEFAULT_HOST_BAND_BUDGET,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilePlanError {
    EmptyImage,
    /// The tile plus apron exceeds the device's texture limit.
    TooLargeForDevice {
        ext_w: u64,
        ext_h: u64,
        max: u32,
    },
    /// Even the smallest tile edge exceeds a budget.
    BudgetTooSmall {
        gpu_needed: u64,
        host_needed: u64,
    },
}

impl fmt::Display for TilePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TilePlanError::EmptyImage => f.write_str("the image has no pixels"),
            TilePlanError::TooLargeForDevice { ext_w, ext_h, max } => write!(
                f,
                "a {ext_w}x{ext_h} tile (with apron) exceeds this GPU's 2D texture limit of \
                 {max}; use a tiled policy or smaller tiles"
            ),
            TilePlanError::BudgetTooSmall {
                gpu_needed,
                host_needed,
            } => write!(
                f,
                "the smallest tile needs {gpu_needed} GPU bytes and {host_needed} host bytes, \
                 more than the memory budget"
            ),
        }
    }
}

impl std::error::Error for TilePlanError {}

/// A validated tiling of one render target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePlan {
    pub image_w: u32,
    pub image_h: u32,
    /// Nominal interior size; edge tiles may be smaller.
    pub tile_w: u32,
    pub tile_h: u32,
    pub apron: u32,
    pub cols: u32,
    pub rows: u32,
    /// Renderer-owned GPU bytes for one tile's resources (reused per tile).
    pub gpu_bytes: u64,
    /// Host bytes for one band of rows (`image_w × tile_h × 4`).
    pub host_band_bytes: u64,
}

/// One tile, in whole-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    /// Row-major index, `0..plan.len()`.
    pub index: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Origin of the apron-inclusive region; negative at image borders
    /// (the canvas continues beyond the frame).
    pub ext_x: i64,
    pub ext_y: i64,
    pub ext_w: u32,
    pub ext_h: u32,
}

impl TilePlan {
    pub fn new(
        image_w: u32,
        image_h: u32,
        apron: u32,
        policy: TilePolicy,
        limits: DeviceTileLimits,
        cost: TileCostModel,
    ) -> Result<TilePlan, TilePlanError> {
        if image_w == 0 || image_h == 0 {
            return Err(TilePlanError::EmptyImage);
        }
        let plan = |edge_w: u32, edge_h: u32| -> Result<TilePlan, TilePlanError> {
            let (tw, th) = (edge_w.min(image_w), edge_h.min(image_h));
            let ext_w = tw as u64 + 2 * apron as u64;
            let ext_h = th as u64 + 2 * apron as u64;
            let max = limits.max_texture_dimension_2d;
            if ext_w > max as u64 || ext_h > max as u64 {
                return Err(TilePlanError::TooLargeForDevice { ext_w, ext_h, max });
            }
            let row = (tw as u64 * cost.staging_bytes_per_px).div_ceil(COPY_ROW_ALIGNMENT)
                * COPY_ROW_ALIGNMENT;
            let staging = row * th as u64;
            let gpu = ext_w * ext_h * cost.extended_bytes_per_px
                + tw as u64 * th as u64 * cost.output_bytes_per_px
                + staging;
            if staging > limits.max_buffer_size {
                return Err(TilePlanError::BudgetTooSmall {
                    gpu_needed: gpu,
                    host_needed: 0,
                });
            }
            Ok(TilePlan {
                image_w,
                image_h,
                tile_w: tw,
                tile_h: th,
                apron,
                cols: image_w.div_ceil(tw),
                rows: image_h.div_ceil(th),
                gpu_bytes: gpu,
                host_band_bytes: image_w as u64 * th as u64 * 4,
            })
        };
        match policy {
            TilePolicy::Single => plan(image_w, image_h),
            TilePolicy::Fixed { edge } => plan(edge.max(1), edge.max(1)),
            TilePolicy::Budget {
                gpu_bytes,
                host_bytes,
            } => {
                let mut last = TilePlanError::EmptyImage;
                for edge in TILE_EDGES {
                    match plan(edge, edge) {
                        Ok(p) if p.gpu_bytes <= gpu_bytes && p.host_band_bytes <= host_bytes => {
                            return Ok(p);
                        }
                        Ok(p) => {
                            last = TilePlanError::BudgetTooSmall {
                                gpu_needed: p.gpu_bytes,
                                host_needed: p.host_band_bytes,
                            }
                        }
                        Err(e) => last = e,
                    }
                }
                Err(last)
            }
        }
    }

    pub fn len(&self) -> u32 {
        self.cols * self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Tiles in row-major order. Bands of `tile_h` rows complete in order,
    /// which lets a PNG writer stream them.
    pub fn tiles(&self) -> impl Iterator<Item = Tile> + '_ {
        self.tiles_in(TileOrder::RowMajor)
    }

    /// Tiles band by band, top to bottom, visiting each band in `order`.
    /// `Tile::index` stays the row-major index.
    pub fn tiles_in(&self, order: TileOrder) -> impl Iterator<Item = Tile> + '_ {
        (0..self.len()).map(move |n| {
            let r = n / self.cols;
            let c = match order {
                TileOrder::RowMajor => n % self.cols,
                TileOrder::ReverseInBand => self.cols - 1 - n % self.cols,
            };
            let i = r * self.cols + c;
            let (x, y) = (c * self.tile_w, r * self.tile_h);
            let (w, h) = (
                self.tile_w.min(self.image_w - x),
                self.tile_h.min(self.image_h - y),
            );
            Tile {
                index: i,
                x,
                y,
                w,
                h,
                ext_x: x as i64 - self.apron as i64,
                ext_y: y as i64 - self.apron as i64,
                ext_w: w + 2 * self.apron,
                ext_h: h + 2 * self.apron,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{UHD_4K, UHD_8K};

    /// Task 02's pipeline: two rgba16float intermediates, rgba8 output + staging.
    const SPIKE_COST: TileCostModel = TileCostModel {
        extended_bytes_per_px: 16,
        output_bytes_per_px: 4,
        staging_bytes_per_px: 4,
    };
    const WEBGPU_DEFAULT: DeviceTileLimits = DeviceTileLimits {
        max_texture_dimension_2d: 8192,
        max_buffer_size: 256 << 20,
    };

    #[test]
    fn tiles_cover_every_pixel_exactly_once() {
        for (w, h, edge) in [
            (3840, 2160, 512),
            (5001, 7003, 1000),
            (3840, 2160, 333),
            (64, 64, 2048),
        ] {
            let p = TilePlan::new(
                w,
                h,
                26,
                TilePolicy::Fixed { edge },
                WEBGPU_DEFAULT,
                SPIKE_COST,
            )
            .unwrap();
            let mut covered = vec![0u8; (w * h) as usize];
            for t in p.tiles() {
                assert_eq!(t.ext_w, t.w + 52);
                for y in t.y..t.y + t.h {
                    for x in t.x..t.x + t.w {
                        covered[(y * w + x) as usize] += 1;
                    }
                }
            }
            assert!(covered.iter().all(|&c| c == 1), "{w}x{h} tile {edge}");
            assert_eq!(p.tiles().count() as u32, p.len());
        }
    }

    #[test]
    fn reverse_order_visits_the_same_tiles_band_by_band() {
        let p = TilePlan::new(
            1000,
            700,
            10,
            TilePolicy::Fixed { edge: 300 },
            WEBGPU_DEFAULT,
            SPIKE_COST,
        )
        .unwrap();
        let fwd: Vec<Tile> = p.tiles().collect();
        let rev: Vec<Tile> = p.tiles_in(TileOrder::ReverseInBand).collect();
        assert_eq!(rev.len(), fwd.len());
        // Same band sequence, reversed within each band.
        for (band_f, band_r) in fwd.chunks(p.cols as usize).zip(rev.chunks(p.cols as usize)) {
            let mut back = band_r.to_vec();
            back.reverse();
            assert_eq!(band_f, &back[..]);
        }
        assert_eq!(rev[0].x, 900);
        assert_eq!(rev[0].index, 3);
    }

    #[test]
    fn border_tiles_extend_beyond_the_frame() {
        let p = TilePlan::new(
            1000,
            1000,
            10,
            TilePolicy::Fixed { edge: 512 },
            WEBGPU_DEFAULT,
            SPIKE_COST,
        )
        .unwrap();
        let first = p.tiles().next().unwrap();
        assert_eq!((first.ext_x, first.ext_y), (-10, -10));
        let last = p.tiles().last().unwrap();
        assert_eq!((last.x, last.w, last.ext_w), (512, 488, 508));
    }

    #[test]
    fn budget_policy_matches_the_task_02_8k_measurement() {
        // 8K at 1024 tiles with a 52 px apron allocated 27.4 MiB in task 02.
        let p = TilePlan::new(
            7680,
            4320,
            52,
            TilePolicy::default_export(),
            WEBGPU_DEFAULT,
            SPIKE_COST,
        )
        .unwrap();
        assert_eq!(
            (p.tile_w, p.tile_h),
            (2048, 2048),
            "largest edge within 256 MiB"
        );
        let tight = TilePolicy::Budget {
            gpu_bytes: 28 << 20,
            host_bytes: 256 << 20,
        };
        let p = TilePlan::new(7680, 4320, 52, tight, WEBGPU_DEFAULT, SPIKE_COST).unwrap();
        assert_eq!(p.tile_w, 1024);
        assert_eq!(p.len(), 40);
        let mib = p.gpu_bytes as f64 / (1 << 20) as f64;
        assert!((mib - 27.4).abs() < 0.1, "{mib}");
    }

    #[test]
    fn host_budget_limits_band_height() {
        let host = TilePolicy::Budget {
            gpu_bytes: u64::MAX,
            host_bytes: 16 << 20,
        };
        let p = TilePlan::new(
            UHD_8K.width,
            UHD_8K.height,
            52,
            host,
            WEBGPU_DEFAULT,
            SPIKE_COST,
        )
        .unwrap();
        assert!(p.host_band_bytes <= 16 << 20);
        assert_eq!(p.tile_h, 512);
    }

    #[test]
    fn impossible_plans_fail_with_reasons() {
        let tiny = TilePolicy::Budget {
            gpu_bytes: 1 << 20,
            host_bytes: 1 << 30,
        };
        assert!(matches!(
            TilePlan::new(
                UHD_4K.width,
                UHD_4K.height,
                26,
                tiny,
                WEBGPU_DEFAULT,
                SPIKE_COST
            ),
            Err(TilePlanError::BudgetTooSmall { .. })
        ));
        // A 16K single tile does not fit WebGPU's default 8192 limit.
        assert!(matches!(
            TilePlan::new(
                15360,
                8640,
                0,
                TilePolicy::Single,
                WEBGPU_DEFAULT,
                SPIKE_COST
            ),
            Err(TilePlanError::TooLargeForDevice { max: 8192, .. })
        ));
        assert_eq!(
            TilePlan::new(0, 10, 0, TilePolicy::Single, WEBGPU_DEFAULT, SPIKE_COST),
            Err(TilePlanError::EmptyImage)
        );
    }

    #[test]
    fn apron_sums_chained_supports() {
        let m = CanvasMapping::new(UHD_8K.aspect().extents(), 7680, 4320);
        let passes = [
            Support {
                pass: "blur_h",
                radius: 0.012,
            },
            Support {
                pass: "bleed",
                radius: 0.004,
            },
        ];
        assert_eq!(apron_pixels(&m, &passes), 52 + 18);
    }
}
