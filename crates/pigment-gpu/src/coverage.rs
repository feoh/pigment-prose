//! Coverage index for the painting renderer's point-in-polygon test.
//!
//! The parity test casts a ray from the pixel and counts the outline edges
//! it crosses. Each layer chooses its ray: toward +x (**rows**) or toward
//! +y (**columns**). Call the ray's direction `u` and the other axis `v`.
//! An edge can be crossed only if the pixel's `v` lies in the edge's `v`
//! span and the edge reaches past the pixel along `u`. So the layer's
//! bounding box is cut into bins along `v`, each bin lists the edges whose
//! `v` span overlaps it, sorted by their largest `u` (descending), and the
//! shader stops at the first edge that ends before the pixel. Parity does
//! not depend on the order edges are counted in, and every skipped edge
//! provably cannot be crossed, so the result is exactly the brute-force
//! parity along the layer's ray (`indexed_matches_brute_force`).
//!
//! The axis matters because a long, nearly straight chain along a bin (a
//! shoreline in row bins) puts hundreds of edges in one bin, exactly where
//! reflections and edges probe most. Each layer takes the axis with the
//! smaller edge-weighted mean list length, `Σ len² / Σ len`. The two rays
//! disagree only for points within float rounding of an edge
//! (`axes_agree_away_from_edges`).
//!
//! The index is a function of the scene alone: never of tiles, pixels or
//! paint settings. Its size is bounded: at most [`MAX_BINS`] bins per layer,
//! and the bin count is halved until a layer's entries fit in
//! [`ENTRIES_PER_VERTEX`] × its vertex count.

use pigment_core::scene::{CanvasPoint, Scene};

/// Bins per layer, at most.
pub const MAX_BINS: u32 = 256;
/// A layer's bin entries are kept under this multiple of its vertex count.
pub const ENTRIES_PER_VERTEX: usize = 32;
/// The shader stops at an edge whose largest `u` is this far (canvas units)
/// before the pixel. Far larger than the rounding of the crossing (about
/// 1e-7 here), far smaller than a pixel at 16K (6e-5).
pub const BREAK_MARGIN: f32 = 1e-5;

/// The direction of a layer's parity ray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Ray toward +x; bins along y.
    Rows = 0,
    /// Ray toward +y; bins along x.
    Columns = 1,
}

impl Axis {
    /// `(u, v)` of a point: `u` along the ray, `v` across it.
    pub fn uv(self, p: CanvasPoint) -> (f32, f32) {
        match self {
            Axis::Rows => (p.x, p.y),
            Axis::Columns => (p.y, p.x),
        }
    }
}

/// Per layer: its ray, where its bins start, how many it has, and the
/// factor that maps `v − bbox.min_v` to a bin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerBins {
    pub axis: Axis,
    pub base: u32,
    pub bins: u32,
    pub inv_span: f32,
    pub min_v: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverageIndex {
    pub layers: Vec<LayerBins>,
    /// Per bin: (first entry, entry count).
    pub bins: Vec<[u32; 2]>,
    /// Per entry: (global index of the edge's first vertex, largest `u` as
    /// `f32` bits). The edge runs from that vertex to the previous one (the
    /// layer's last vertex for its first).
    pub entries: Vec<[u32; 2]>,
}

/// The bin of `v`, exactly as the shader computes it.
fn bin_of(v: f32, lb: &LayerBins) -> u32 {
    let k = ((v - lb.min_v) * lb.inv_span).floor();
    k.clamp(0.0, (lb.bins - 1) as f32) as u32
}

/// One layer's bins along `axis`: its layout and sorted lists.
fn layout(
    pts: &[CanvasPoint],
    first: u32,
    base: u32,
    axis: Axis,
) -> (LayerBins, Vec<Vec<(u32, f32)>>) {
    let n = pts.len();
    let uv: Vec<(f32, f32)> = pts.iter().map(|&p| axis.uv(p)).collect();
    let (min_v, max_v) = uv
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
            (a.min(p.1), b.max(p.1))
        });
    // (vertex, lo v, hi v, max u) for every edge that can be crossed.
    let edges: Vec<(u32, f32, f32, f32)> = (0..n)
        .filter_map(|i| {
            let a = uv[i];
            let b = uv[(i + n - 1) % n];
            (a.1 != b.1).then(|| (first + i as u32, a.1.min(b.1), a.1.max(b.1), a.0.max(b.0)))
        })
        .collect();
    let mut bins = (n as u32 / 2).next_power_of_two().clamp(1, MAX_BINS);
    loop {
        let lb = LayerBins {
            axis,
            base,
            bins,
            inv_span: if max_v > min_v {
                bins as f32 / (max_v - min_v)
            } else {
                0.0
            },
            min_v,
        };
        // One bin of margin on each side absorbs any difference between
        // this and the shader's rounding of the bin.
        let spans: Vec<(u32, u32)> = edges
            .iter()
            .map(|&(_, lo, hi, _)| {
                (
                    bin_of(lo, &lb).saturating_sub(1),
                    (bin_of(hi, &lb) + 1).min(bins - 1),
                )
            })
            .collect();
        let total: usize = spans.iter().map(|&(a, b)| (b - a + 1) as usize).sum();
        if bins == 1 || total <= ENTRIES_PER_VERTEX * n {
            let mut lists: Vec<Vec<(u32, f32)>> = vec![Vec::new(); bins as usize];
            for (&(v, _, _, max_u), &(a, b)) in edges.iter().zip(&spans) {
                for k in a..=b {
                    lists[k as usize].push((v, max_u));
                }
            }
            for list in &mut lists {
                list.sort_by(|p, q| q.1.total_cmp(&p.1).then(p.0.cmp(&q.0)));
            }
            return (lb, lists);
        }
        bins /= 2;
    }
}

/// Edge-weighted mean list length: what a probe near the outline scans.
fn cost(lists: &[Vec<(u32, f32)>]) -> f64 {
    let (sum, sq) = lists.iter().fold((0.0, 0.0), |(s, q), l| {
        let n = l.len() as f64;
        (s + n, q + n * n)
    });
    if sum == 0.0 { 0.0 } else { sq / sum }
}

impl CoverageIndex {
    pub fn build(scene: &Scene) -> CoverageIndex {
        let mut ix = CoverageIndex::default();
        let mut first = 0u32;
        for l in scene.layers() {
            let base = ix.bins.len() as u32;
            let rows = layout(&l.outline, first, base, Axis::Rows);
            let cols = layout(&l.outline, first, base, Axis::Columns);
            let (lb, lists) = if cost(&cols.1) < cost(&rows.1) {
                cols
            } else {
                rows
            };
            for list in lists {
                ix.bins.push([ix.entries.len() as u32, list.len() as u32]);
                ix.entries
                    .extend(list.into_iter().map(|(v, u)| [v, u.to_bits()]));
            }
            ix.layers.push(lb);
            first += l.outline.len() as u32;
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

    /// Whether the ray from `c` along +u crosses edge `ab` (the shader's test).
    fn crosses(axis: Axis, a: CanvasPoint, b: CanvasPoint, c: CanvasPoint) -> bool {
        let ((au, av), (bu, bv), (cu, cv)) = (axis.uv(a), axis.uv(b), axis.uv(c));
        if (av > cv) != (bv > cv) {
            let u = (bu - au) * (cv - av) / (bv - av) + au;
            return cu < u;
        }
        false
    }

    fn brute(axis: Axis, pts: &[CanvasPoint], c: CanvasPoint) -> bool {
        let n = pts.len();
        (0..n).fold(false, |acc, i| {
            acc ^ crosses(axis, pts[i], pts[(i + n - 1) % n], c)
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
        let (cu, cv) = lb.axis.uv(c);
        let [off, count] = ix.bins[(lb.base + bin_of(cv, lb)) as usize];
        let mut inside = false;
        for &[v, u] in &ix.entries[off as usize..(off + count) as usize] {
            if f32::from_bits(u) < cu - BREAK_MARGIN {
                break;
            }
            let v = v as usize;
            let prev = if v == first { first + n - 1 } else { v - 1 };
            inside ^= crosses(lb.axis, verts[v], verts[prev], c);
        }
        inside
    }

    fn distance_to_outline(pts: &[CanvasPoint], c: CanvasPoint) -> f32 {
        let n = pts.len();
        (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + n - 1) % n]);
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let len2 = dx * dx + dy * dy;
                let t = if len2 > 0.0 {
                    (((c.x - a.x) * dx + (c.y - a.y) * dy) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (px, py) = (a.x + t * dx - c.x, a.y + t * dy - c.y);
                (px * px + py * py).sqrt()
            })
            .fold(f32::INFINITY, f32::min)
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

    fn probes(scene: &Scene, layer: &[CanvasPoint]) -> Vec<CanvasPoint> {
        let ext = scene.extents();
        let (w, h) = (ext.width as f32, ext.height as f32);
        // A pixel grid, plus points level with and beside every vertex (the
        // edge ends, where the half-open spans matter), on both axes.
        let mut probes: Vec<CanvasPoint> = (0..61)
            .flat_map(|py| {
                (0..109).map(move |px| CanvasPoint {
                    x: (px as f32 + 0.5) * w / 109.0,
                    y: (py as f32 + 0.5) * h / 61.0,
                })
            })
            .collect();
        probes.extend(layer.iter().map(|p| CanvasPoint {
            x: p.x - 0.003,
            y: p.y,
        }));
        probes.extend(layer.iter().map(|p| CanvasPoint {
            x: p.x,
            y: p.y - 0.003,
        }));
        probes
    }

    const CASES: [(u32, (u32, u32)); 4] = [(0, (16, 9)), (3, (16, 9)), (8, (9, 16)), (5, (1, 1))];

    #[test]
    fn indexed_matches_brute_force() {
        let mut axes = [0usize; 2];
        for (i, (aw, ah)) in CASES {
            let scene = scene(i, AspectRatio::of(aw, ah));
            let ix = CoverageIndex::build(&scene);
            let verts: Vec<CanvasPoint> = scene
                .layers()
                .iter()
                .flat_map(|l| l.outline.iter().copied())
                .collect();
            let mut first = 0;
            let mut hits = 0;
            for (l, layer) in scene.layers().iter().enumerate() {
                let n = layer.outline.len();
                let axis = ix.layers[l].axis;
                axes[axis as usize] += 1;
                for c in probes(&scene, &layer.outline) {
                    let b = brute(axis, &layer.outline, c);
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
        // Both rays are in use (wide water and near-horizontal chains take
        // columns; tall banks and planes take rows).
        assert!(axes[0] > 0 && axes[1] > 0, "{axes:?}");
    }

    #[test]
    fn axes_agree_away_from_edges() {
        for (i, (aw, ah)) in CASES {
            let scene = scene(i, AspectRatio::of(aw, ah));
            for layer in scene.layers() {
                for c in probes(&scene, &layer.outline) {
                    let (r, k) = (
                        brute(Axis::Rows, &layer.outline, c),
                        brute(Axis::Columns, &layer.outline, c),
                    );
                    if r != k {
                        let d = distance_to_outline(&layer.outline, c);
                        assert!(
                            d < 1e-5,
                            "rows and columns disagree {d} from the outline at {c:?}"
                        );
                    }
                }
            }
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
