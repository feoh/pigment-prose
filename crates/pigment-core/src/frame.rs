//! Output frame, aspect ratio and the canvas coordinate system.
//!
//! Conventions (docs/architecture.md, "Coordinates and units"):
//! - **Pixels:** origin at the top-left of the image, +x right, +y down.
//!   Pixel `(i, j)` has its centre at `(i + 0.5, j + 0.5)`.
//! - **Canvas units:** the frame's *short side is 1.0*. Canvas origin is the
//!   top-left of the frame, +y down, so the canvas spans
//!   `[0, extents.width] × [0, extents.height]`. Everything structural
//!   (placement, mark widths, blur radii, texture wavelengths) is in canvas
//!   units, so it covers the same fraction of the painting at any resolution.
//! - The scene depends on the frame's *reduced aspect ratio* only. Changing
//!   pixel size at the same aspect ratio never changes the scene; changing
//!   the aspect ratio recomposes it.

use serde::{Deserialize, Serialize};

use crate::error::{Problem, ValidationError};

/// Smallest accepted edge, in pixels.
pub const MIN_EDGE: u32 = 64;
/// Largest accepted edge, in pixels. Exported and inspected at 16384×9216
/// on the RTX 4070 Ti (task 09, docs/export.md). Raise only with new
/// evidence: the bound is what has been validated, not a memory limit.
pub const MAX_EDGE: u32 = 16384;
/// Most extreme accepted `long / short` ratio. Compositions are authored for
/// ratios up to 4:1; beyond that the templates are untested.
pub const MAX_ASPECT: f64 = 4.0;

/// Pixel dimensions of an export, and of the document's framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
}

pub const UHD_4K: Frame = Frame {
    width: 3840,
    height: 2160,
};
pub const UHD_8K: Frame = Frame {
    width: 7680,
    height: 4320,
};

impl Frame {
    /// Validates and constructs a frame.
    pub fn new(width: u32, height: u32) -> Result<Frame, ValidationError> {
        let f = Frame { width, height };
        f.validate()?;
        Ok(f)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        for (field, v) in [("frame.width", self.width), ("frame.height", self.height)] {
            if v < MIN_EDGE {
                return Err(ValidationError {
                    field,
                    problem: Problem::TooSmall {
                        value: v.into(),
                        min: MIN_EDGE.into(),
                    },
                });
            }
            if v > MAX_EDGE {
                return Err(ValidationError {
                    field,
                    problem: Problem::TooLarge {
                        value: v.into(),
                        max: MAX_EDGE.into(),
                    },
                });
            }
        }
        let ratio = self.long_side() as f64 / self.short_side() as f64;
        if ratio > MAX_ASPECT {
            return Err(ValidationError {
                field: "frame",
                problem: Problem::AspectTooExtreme {
                    long_over_short: ratio,
                    max: MAX_ASPECT,
                },
            });
        }
        Ok(())
    }

    pub fn short_side(&self) -> u32 {
        self.width.min(self.height)
    }

    pub fn long_side(&self) -> u32 {
        self.width.max(self.height)
    }

    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Same pixel count, rotated.
    pub fn rotated(&self) -> Frame {
        Frame {
            width: self.height,
            height: self.width,
        }
    }

    pub fn aspect(&self) -> AspectRatio {
        AspectRatio::of(self.width, self.height)
    }

    /// Largest frame with *exactly* `aspect` whose long side is at most
    /// `max_long`: the biggest whole multiple of the reduced ratio. Export
    /// presets use it ("8K" of a 16:9 scene is 7680×4320, of a 7:5 scene
    /// 7679×5485), so a preset never recomposes the scene.
    pub fn largest_with_aspect(
        aspect: AspectRatio,
        max_long: u32,
    ) -> Result<Frame, ValidationError> {
        let long = aspect.width.max(aspect.height);
        let k = max_long / long;
        let f = Frame {
            width: aspect.width.saturating_mul(k),
            height: aspect.height.saturating_mul(k),
        };
        f.validate()?;
        Ok(f)
    }

    /// Largest size with this frame's aspect ratio whose long side is at
    /// most `max_long` pixels (for previews). Never upscales.
    pub fn fit_within(&self, max_long: u32) -> (u32, u32) {
        let long = self.long_side();
        if long <= max_long {
            return (self.width, self.height);
        }
        let s = max_long as f64 / long as f64;
        let w = ((self.width as f64 * s).round() as u32).max(1);
        let h = ((self.height as f64 * s).round() as u32).max(1);
        (w, h)
    }
}

/// Reduced integer aspect ratio. Part of the scene key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AspectRatio {
    pub width: u32,
    pub height: u32,
}

impl AspectRatio {
    pub fn of(width: u32, height: u32) -> AspectRatio {
        assert!(width > 0 && height > 0, "aspect of an empty frame");
        let g = gcd(width, height);
        AspectRatio {
            width: width / g,
            height: height / g,
        }
    }

    pub fn extents(&self) -> CanvasExtents {
        let (w, h) = (self.width as f64, self.height as f64);
        let short = w.min(h);
        CanvasExtents {
            width: w / short,
            height: h / short,
        }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Canvas size in canvas units. One of the two is exactly 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasExtents {
    pub width: f64,
    pub height: f64,
}

/// Maps a render target's pixels onto the canvas.
///
/// The mapping is per axis, so a preview whose pixel size only approximates
/// the frame's aspect ratio (rounding in [`Frame::fit_within`]) is stretched
/// by under one pixel rather than cropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasMapping {
    pub extents: CanvasExtents,
    pub pixels_w: u32,
    pub pixels_h: u32,
}

impl CanvasMapping {
    pub fn new(extents: CanvasExtents, pixels_w: u32, pixels_h: u32) -> CanvasMapping {
        CanvasMapping {
            extents,
            pixels_w,
            pixels_h,
        }
    }

    /// Canvas position of the centre of whole-image pixel `(px, py)`.
    pub fn pixel_centre(&self, px: i64, py: i64) -> (f64, f64) {
        (
            (px as f64 + 0.5) * self.extents.width / self.pixels_w as f64,
            (py as f64 + 0.5) * self.extents.height / self.pixels_h as f64,
        )
    }

    /// Size of one pixel in canvas units (the larger axis, conservatively).
    /// Used only for antialiasing, band-limiting and support-to-pixel
    /// conversion, never for placing structure.
    pub fn pixel_footprint(&self) -> f64 {
        (self.extents.width / self.pixels_w as f64).max(self.extents.height / self.pixels_h as f64)
    }

    /// Whole pixels needed to cover `radius` canvas units.
    pub fn support_pixels(&self, radius: f64) -> u32 {
        (radius / self.pixel_footprint()).ceil().max(0.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_orientations_validate() {
        for f in [
            UHD_4K,
            UHD_8K,
            UHD_4K.rotated(),
            Frame {
                width: 3000,
                height: 3000,
            },
        ] {
            assert!(f.validate().is_ok(), "{f:?}");
        }
        assert!(Frame::new(5001, 7003).is_ok(), "odd custom size");
    }

    #[test]
    fn bad_frames_are_rejected() {
        assert_eq!(Frame::new(63, 100).unwrap_err().field, "frame.width");
        assert_eq!(
            Frame::new(100, MAX_EDGE + 1).unwrap_err().field,
            "frame.height"
        );
        assert!(matches!(
            Frame::new(4100, 1000).unwrap_err().problem,
            Problem::AspectTooExtreme { .. }
        ));
        assert!(Frame::new(4000, 1000).is_ok(), "exactly 4:1 is allowed");
    }

    #[test]
    fn same_aspect_means_same_scene_key() {
        assert_eq!(UHD_4K.aspect(), UHD_8K.aspect());
        assert_eq!(
            UHD_4K.aspect(),
            AspectRatio {
                width: 16,
                height: 9
            }
        );
        assert_ne!(UHD_4K.aspect(), UHD_4K.rotated().aspect());
        assert_ne!(
            Frame {
                width: 1921,
                height: 1080
            }
            .aspect(),
            UHD_4K.aspect()
        );
    }

    #[test]
    fn extents_put_the_short_side_at_one() {
        let e = UHD_8K.aspect().extents();
        assert_eq!(e.height, 1.0);
        assert!((e.width - 16.0 / 9.0).abs() < 1e-15);
        let p = UHD_4K.rotated().aspect().extents();
        assert_eq!(p.width, 1.0);
    }

    #[test]
    fn mapping_is_resolution_independent() {
        let e = UHD_4K.aspect().extents();
        let small = CanvasMapping::new(e, 960, 540);
        let big = CanvasMapping::new(e, 7680, 4320);
        // The last pixel centre of each sits the same half-pixel from the edge.
        let (sx, _) = small.pixel_centre(959, 0);
        let (bx, _) = big.pixel_centre(7679, 0);
        assert!((e.width - sx - small.pixel_footprint() / 2.0).abs() < 1e-12);
        assert!((e.width - bx - big.pixel_footprint() / 2.0).abs() < 1e-12);
        // 0.012 canvas units: 7 px at 540p, 52 px at 8K (task 02 blur radius).
        assert_eq!(small.support_pixels(0.012), 7);
        assert_eq!(big.support_pixels(0.012), 52);
    }

    #[test]
    fn exact_aspect_presets() {
        let a = |w, h| AspectRatio::of(w, h);
        assert_eq!(Frame::largest_with_aspect(a(16, 9), 7680), Ok(UHD_8K));
        assert_eq!(
            Frame::largest_with_aspect(a(9, 16), 3840),
            Ok(UHD_4K.rotated())
        );
        let f = Frame::largest_with_aspect(a(7, 5), 7680).unwrap();
        assert_eq!((f.width, f.height), (7679, 5485));
        assert_eq!(f.aspect(), a(7, 5));
        let sq = Frame::largest_with_aspect(a(1, 1), 3840).unwrap();
        assert_eq!((sq.width, sq.height), (3840, 3840));
        // A ratio whose smallest exact multiple exceeds the bound is refused.
        assert!(Frame::largest_with_aspect(a(4001, 1000), 3840).is_err());
    }

    #[test]
    fn preview_fit_keeps_aspect_and_never_upscales() {
        assert_eq!(UHD_8K.fit_within(1280), (1280, 720));
        assert_eq!(UHD_4K.rotated().fit_within(960), (540, 960));
        assert_eq!(
            Frame {
                width: 800,
                height: 600
            }
            .fit_within(1280),
            (800, 600)
        );
        let (w, h) = Frame {
            width: 5001,
            height: 7003,
        }
        .fit_within(960);
        assert_eq!(h, 960);
        assert_eq!(w, 686);
    }
}
