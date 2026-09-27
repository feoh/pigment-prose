//! Render requests, results, output sinks and the renderer interface.
//!
//! A request is an immutable snapshot: an `Arc<Scene>`, the seeds, a copy of
//! the appearance settings and a target. Editing controls after submission
//! can never change a request in flight (task 13's snapshot rule).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::{RenderError, SinkError};
use crate::frame::CanvasMapping;
use crate::job::{CancelToken, ProgressSink};
use crate::scene::Scene;
use crate::seed::SeedBundle;
use crate::settings::Appearance;
use crate::tiles::{Support, TileCostModel, TilePlan, TilePolicy};

/// Monotonically increasing per process. Larger = newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

/// Issues request ids. One per application; `Sync`.
#[derive(Debug, Default)]
pub struct RequestIds(AtomicU64);

impl RequestIds {
    pub const fn new() -> RequestIds {
        RequestIds(AtomicU64::new(0))
    }

    pub fn next(&self) -> RequestId {
        RequestId(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPurpose {
    /// Interactive; latest-wins, may be superseded or cancelled freely.
    Preview,
    /// Final file; one at a time, cancelled only by the user.
    Export,
}

/// Pixel size of this render and how to tile it. Previews use a smaller
/// pixel size with the *same* scene; see `Frame::fit_within`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderTarget {
    pub width: u32,
    pub height: u32,
    pub policy: TilePolicy,
}

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub id: RequestId,
    pub purpose: RenderPurpose,
    pub scene: Arc<Scene>,
    /// The renderer reads only `Domain::PaintDetail` from this.
    pub seeds: SeedBundle,
    pub appearance: Appearance,
    pub target: RenderTarget,
}

impl RenderRequest {
    pub fn mapping(&self) -> CanvasMapping {
        CanvasMapping::new(self.scene.extents(), self.target.width, self.target.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    Completed,
    /// Stopped at a tile boundary; the sink was told to `abort`.
    Cancelled {
        tiles_done: u32,
    },
}

/// Wall-clock timings. GPU timestamp queries are not used yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderTimings {
    /// Resource creation and uploads before the first tile.
    pub setup: Duration,
    /// Submit → mapped rows copied out, summed over tiles.
    pub render_readback: Duration,
    /// Time inside the sink (PNG encoding, preview upload), summed.
    pub sink: Duration,
    pub total: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderReport {
    pub id: RequestId,
    pub purpose: RenderPurpose,
    pub outcome: RenderOutcome,
    pub plan: TilePlan,
    pub timings: RenderTimings,
    /// `version::RENDERER_VERSION` of the renderer that produced this.
    pub renderer_version: u32,
    /// Adapter/backend label, e.g. `"NVIDIA GeForce RTX 4070 Ti (Vulkan)"`.
    pub device: String,
    pub software_adapter: bool,
}

/// Receives finished rows, top to bottom.
///
/// Rows are tightly packed RGBA8 (stride `width * 4`), sRGB-encoded, alpha
/// always 255 (the painting is opaque paper). Bands arrive in order and
/// cover the image exactly once.
pub trait TileSink {
    fn begin(&mut self, width: u32, height: u32) -> Result<(), SinkError>;
    fn band(&mut self, first_row: u32, rows: u32, rgba8: &[u8]) -> Result<(), SinkError>;
    /// All rows delivered successfully. Export sinks finalize the file here.
    fn finish(&mut self) -> Result<(), SinkError>;
    /// Cancelled or failed: discard partial output (task 09 deletes the
    /// temporary file and keeps any existing destination).
    fn abort(&mut self);
}

/// Collects the whole image in memory. For previews and tests only.
#[derive(Debug, Default)]
pub struct MemorySink {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
    pub finished: bool,
    pub aborted: bool,
}

impl TileSink for MemorySink {
    fn begin(&mut self, width: u32, height: u32) -> Result<(), SinkError> {
        self.width = width;
        self.height = height;
        self.rgba8 = vec![0; width as usize * height as usize * 4];
        Ok(())
    }

    fn band(&mut self, first_row: u32, rows: u32, rgba8: &[u8]) -> Result<(), SinkError> {
        let stride = self.width as usize * 4;
        let start = first_row as usize * stride;
        let len = rows as usize * stride;
        if rgba8.len() != len || start + len > self.rgba8.len() {
            return Err(SinkError {
                kind: crate::error::SinkErrorKind::Other,
                detail: format!(
                    "band of {} bytes at row {first_row} does not fit",
                    rgba8.len()
                ),
            });
        }
        self.rgba8[start..start + len].copy_from_slice(rgba8);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        self.finished = true;
        Ok(())
    }

    fn abort(&mut self) {
        self.aborted = true;
    }
}

/// A GPU (or, after task 23, possibly CPU) painting implementation.
pub trait Renderer {
    /// Per-tile allocation model, for [`TilePlan`].
    fn cost_model(&self) -> TileCostModel;

    /// Supports of the chained neighbourhood passes for these settings. The
    /// apron is their pixel sum at the request's mapping.
    fn supports(&self, appearance: &Appearance) -> Vec<Support>;

    /// Render tile by tile into `sink`. Checks `cancel` before every tile and
    /// reports progress after every tile. On cancellation or error it calls
    /// `sink.abort()`; on success `sink.finish()`.
    fn render(
        &self,
        request: &RenderRequest,
        cancel: &CancelToken,
        progress: &mut dyn ProgressSink,
        sink: &mut dyn TileSink,
    ) -> Result<RenderReport, RenderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_increase() {
        let ids = RequestIds::default();
        let a = ids.next();
        let b = ids.next();
        assert!(b > a);
        assert_eq!(a, RequestId(1));
    }

    #[test]
    fn memory_sink_rejects_misplaced_bands() {
        let mut s = MemorySink::default();
        s.begin(2, 2).unwrap();
        s.band(0, 1, &[1; 8]).unwrap();
        s.band(1, 1, &[2; 8]).unwrap();
        assert_eq!(&s.rgba8[8..], &[2; 8]);
        assert!(s.band(2, 1, &[0; 8]).is_err());
        assert!(s.band(0, 1, &[0; 4]).is_err());
    }
}
