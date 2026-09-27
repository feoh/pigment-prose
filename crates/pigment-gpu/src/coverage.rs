//! Coverage index for the painting renderer's point-in-polygon test.
//!
//! The parity test casts a ray from the pixel toward +x and counts the
//! outline edges it crosses. An edge can be crossed only if the pixel's `y`
//! lies in the edge's `y` span and the edge reaches right of the pixel. So
//! each layer's bounding box is cut into horizontal bins, each bin lists the
//! edges whose `y` span overlaps it, sorted by their right-most `x`
//! (descending), and the shader stops at the first edge that ends left of
//! the pixel. Parity does not depend on the order edges are counted in, and
//! every skipped edge provably cannot be crossed, so the result is exactly
//! the brute-force result (`indexed_matches_brute_force`).
//!
//! The index is a function of the scene alone: never of tiles, pixels or
//! paint settings. Its size is bounded: at most [`MAX_BINS`] bins per layer,
//! and the bin count is halved until a layer's entries fit in
//! [`ENTRIES_PER_VERTEX`] × its vertex count.

use pigment_core::scene::Scene;

/// Bins per layer, at most.
pub const MAX_BINS: u32 = 256;
/// A layer's bin entries are kept under this multiple of its vertex count.
pub const ENTRIES_PER_VERTEX: usize = 32;
/// The shader stops at an edge whose right-most `x` is this far (canvas
/// units) left of the pixel. Far larger than the rounding of the crossing
/// `x` (about 1e-7 here), far smaller than a pixel at 16K (6e-5).
pub const BREAK_MARGIN: f32 = 1e-5;

/// Per layer: where its bins start, how many it has, and the factor that
/// maps `y − bbox.min_y` to a bin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerBins {
    pub base: u32,
    pub bins: u32,
    pub inv_height: f32,
    pub min_y: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverageIndex {
    pub layers: Vec<LayerBins>,
    /// Per bin: (first entry, entry count).
    pub bins: Vec<[u32; 2]>,
    /// Per entry: (global index of the edge's first vertex, right-most `x`
    /// as `f32` bits). The edge runs from that vertex to the previous one
    /// (the layer's last vertex for its first).
    pub entries: Vec<[u32; 2]>,
}

/// The bin of `y`, exactly as the shader computes it.
fn bin_of(y: f32, lb: &LayerBins) -> u32 {
    let k = ((y - lb.min_y) * lb.inv_height).floor();
    k.clamp(0.0, (lb.bins - 1) as f32) as u32
}

impl CoverageIndex {
    pub fn build(scene: &Scene) -> CoverageIndex {
        let mut ix = CoverageIndex::default();
        let mut first = 0u32;
        for l in scene.layers() {
            let pts = &l.outline;
            let n = pts.len();
            let (min_y, max_y) = pts
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                    (a.min(p.y), b.max(p.y))
                });
            // (vertex, lo, hi, max x) for every edge that can be crossed.
            let edges: Vec<(u32, f32, f32, f32)> = (0..n)
                .filter_map(|i| {
                    let a = pts[i];
                    let b = pts[(i + n - 1) % n];
                    (a.y != b.y)
                        .then(|| (first + i as u32, a.y.min(b.y), a.y.max(b.y), a.x.max(b.x)))
                })
                .collect();
            let mut bins = (n as u32 / 2).next_power_of_two().clamp(1, MAX_BINS);
            let layout = |bins: u32| {
                let lb = LayerBins {
                    base: ix.bins.len() as u32,
                    bins,
                    inv_height: if max_y > min_y {
                        bins as f32 / (max_y - min_y)
                    } else {
                        0.0
                    },
                    min_y,
                };
                // One bin of margin on each side absorbs any difference
                // between this and the shader's rounding of the bin.
                let spans: Vec<(u32, u32)> = edges
                    .iter()
                    .map(|&(_, lo, hi, _)| {
                        (
                            bin_of(lo, &lb).saturating_sub(1),
                            (bin_of(hi, &lb) + 1).min(bins - 1),
                        )
                    })
                    .collect();
                (lb, spans)
            };
            let (lb, spans) = loop {
                let (lb, spans) = layout(bins);
                let total: usize = spans.iter().map(|&(a, b)| (b - a + 1) as usize).sum();
                if bins == 1 || total <= ENTRIES_PER_VERTEX * n {
                    break (lb, spans);
                }
                bins /= 2;
            };
            let mut lists: Vec<Vec<(u32, f32)>> = vec![Vec::new(); bins as usize];
            for (&(v, _, _, max_x), &(a, b)) in edges.iter().zip(&spans) {
                for k in a..=b {
                    lists[k as usize].push((v, max_x));
                }
            }
            for list in lists {
                let mut list = list;
                list.sort_by(|p, q| q.1.total_cmp(&p.1).then(p.0.cmp(&q.0)));
                ix.bins.push([ix.entries.len() as u32, list.len() as u32]);
                ix.entries
                    .extend(list.into_iter().map(|(v, x)| [v, x.to_bits()]));
            }
            ix.layers.push(lb);
            first += n as u32;
        }
        ix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pigment_core::frame::AspectRatio;
    use pigment_core::scene::lakeshore::LakeshoreGenerator;
    use pigment_core::scene::{CanvasPoint, SceneGenerator};
    use pigment_core::seed::{SeedBundle, TextDigest, Variation};
    use pigment_core::settings::FormSettings;

    fn crosses(a: CanvasPoint, b: CanvasPoint, c: CanvasPoint) -> bool {
        if (a.y > c.y) != (b.y > c.y) {
            let x = (b.x - a.x) * (c.y - a.y) / (b.y - a.y) + a.x;
            return c.x < x;
        }
        false
    }

    fn brute(pts: &[CanvasPoint], c: CanvasPoint) -> bool {
        let n = pts.len();
        (0..n).fold(false, |acc, i| {
            acc ^ crosses(pts[i], pts[(i + n - 1) % n], c)
        })
    }

    /// The shader's lookup, on the CPU.
    fn indexed(
        ix: &CoverageIndex,
        verts: &[CanvasPoint],
        l: usize,
        first: usize,
        n: usize,
        c: CanvasPoint,
    ) -> bool {
        let lb = &ix.layers[l];
        let [off, count] = ix.bins[(lb.base + bin_of(c.y, lb)) as usize];
        let mut inside = false;
        for &[v, x] in &ix.entries[off as usize..(off + count) as usize] {
            if f32::from_bits(x) < c.x - BREAK_MARGIN {
                break;
            }
            let v = v as usize;
            let prev = if v == first { first + n - 1 } else { v - 1 };
            inside ^= crosses(verts[v], verts[prev], c);
        }
        inside
    }

    fn scene(i: u32, aspect: AspectRatio) -> Scene {
        let digest = TextDigest::from_source(&format!("sample passage {i}")).unwrap();
        LakeshoreGenerator
            .generate(
                &SeedBundle::derive(digest, Variation(0)),
                &FormSettings::default(),
                aspect,
            )
            .unwrap()
    }

    #[test]
    fn indexed_matches_brute_force() {
        for (i, aspect) in [
            (0, AspectRatio::of(16, 9)),
            (3, AspectRatio::of(16, 9)),
            (8, AspectRatio::of(9, 16)),
            (5, AspectRatio::of(1, 1)),
        ] {
            let scene = scene(i, aspect);
            let ix = CoverageIndex::build(&scene);
            let verts: Vec<CanvasPoint> = scene
                .layers()
                .iter()
                .flat_map(|l| l.outline.iter().copied())
                .collect();
            let ext = scene.extents();
            let (w, h) = (ext.width as f32, ext.height as f32);
            let mut first = 0;
            let mut hits = 0;
            for (l, layer) in scene.layers().iter().enumerate() {
                let n = layer.outline.len();
                // A pixel grid, plus every vertex's own y (the edge ends,
                // where the half-open span matters).
                let mut probes: Vec<CanvasPoint> = (0..91)
                    .flat_map(|py| {
                        (0..161).map(move |px| CanvasPoint {
                            x: (px as f32 + 0.5) * w / 161.0,
                            y: (py as f32 + 0.5) * h / 91.0,
                        })
                    })
                    .collect();
                probes.extend(layer.outline.iter().map(|p| CanvasPoint {
                    x: p.x - 0.003,
                    y: p.y,
                }));
                for c in probes {
                    let b = brute(&layer.outline, c);
                    assert_eq!(
                        b,
                        indexed(&ix, &verts, l, first, n, c),
                        "layer {l} at {c:?}"
                    );
                    hits += b as usize;
                }
                first += n;
            }
            assert!(hits > 1000, "the probes must land inside layers");
        }
    }

    #[test]
    fn index_size_is_bounded() {
        for i in 0..8 {
            let scene = scene(i, AspectRatio::of(16, 9));
            let ix = CoverageIndex::build(&scene);
            let verts: usize = scene.layers().iter().map(|l| l.outline.len()).sum();
            assert!(ix.entries.len() <= ENTRIES_PER_VERTEX * verts);
            assert!(ix.layers.iter().all(|l| l.bins >= 1 && l.bins <= MAX_BINS));
            assert_eq!(
                ix.bins.len(),
                ix.layers.iter().map(|l| l.bins as usize).sum::<usize>()
            );
        }
    }
}
