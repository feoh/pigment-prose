//! CPU reference rasterizer and polygon checks for scenes.
//!
//! Used by tests (visible coverage per role, topology), by the contact-sheet
//! statistics and as the reference the GPU debug view is compared against.
//! Coverage uses the same rule as the GPU shaders: even-odd crossings of a
//! horizontal ray to the right of the sample, with an edge counted when
//! exactly one endpoint lies strictly below the sample's y.

use super::{CanvasPoint, LayerRole, Scene};

/// Sentinel for "no layer covers this sample".
pub const NONE: u16 = u16::MAX;

/// Index of the front-most layer covering each sample of a `w × h` grid of
/// sample centres over the frame, row-major. Scanline even-odd fill: cost is
/// `O(h × vertices + w × h)`.
pub fn front_layers(scene: &Scene, w: usize, h: usize) -> Vec<u16> {
    let ext = scene.extents();
    let mut out = vec![NONE; w * h];
    let mut xs: Vec<f64> = Vec::new();
    for (li, layer) in scene.layers().iter().enumerate() {
        let pts = &layer.outline;
        let (mut miny, mut maxy) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in pts {
            miny = miny.min(p.y as f64);
            maxy = maxy.max(p.y as f64);
        }
        for row in 0..h {
            let cy = (row as f64 + 0.5) * ext.height / h as f64;
            if cy < miny || cy > maxy {
                continue;
            }
            xs.clear();
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[j]);
                let (ay, by) = (a.y as f64, b.y as f64);
                if (ay > cy) != (by > cy) {
                    let (ax, bx) = (a.x as f64, b.x as f64);
                    xs.push((bx - ax) * (cy - ay) / (by - ay) + ax);
                }
                j = i;
            }
            xs.sort_by(f64::total_cmp);
            // Inside iff an odd number of crossings lie to the right, i.e.
            // cx falls in [xs[2k], xs[2k+1]).
            for pair in xs.as_chunks::<2>().0 {
                let col = |x: f64| {
                    ((x * w as f64 / ext.width) - 0.5)
                        .ceil()
                        .clamp(0.0, w as f64)
                };
                let (first, last) = (col(pair[0]) as usize, col(pair[1]) as usize);
                let row_out = &mut out[row * w..(row + 1) * w];
                for v in &mut row_out[first..last.max(first)] {
                    *v = li as u16;
                }
            }
        }
    }
    out
}

/// Visible fraction of the frame covered by each role (front-most layer
/// only), from a `w × h` sample grid, in [`LayerRole::ALL`] order.
pub fn role_coverage(scene: &Scene, w: usize, h: usize) -> [f64; 9] {
    let mut counts = [0usize; 9];
    for id in front_layers(scene, w, h) {
        if id != NONE {
            let role = scene.layers()[id as usize].role;
            counts[LayerRole::ALL.iter().position(|r| *r == role).unwrap()] += 1;
        }
    }
    counts.map(|c| c as f64 / (w * h) as f64)
}

/// Fraction of samples covered by no layer at all.
pub fn empty_fraction(scene: &Scene, w: usize, h: usize) -> f64 {
    let ids = front_layers(scene, w, h);
    ids.iter().filter(|&&id| id == NONE).count() as f64 / ids.len() as f64
}

fn orient(a: CanvasPoint, b: CanvasPoint, c: CanvasPoint) -> f64 {
    let (ax, ay, bx, by, cx, cy) = (
        a.x as f64, a.y as f64, b.x as f64, b.y as f64, c.x as f64, c.y as f64,
    );
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

fn on_segment(a: CanvasPoint, b: CanvasPoint, p: CanvasPoint) -> bool {
    p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
}

/// Whether closed segments `ab` and `cd` share any point.
fn segments_touch(a: CanvasPoint, b: CanvasPoint, c: CanvasPoint, d: CanvasPoint) -> bool {
    let (d1, d2) = (orient(c, d, a), orient(c, d, b));
    let (d3, d4) = (orient(a, b, c), orient(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(c, d, a))
        || (d2 == 0.0 && on_segment(c, d, b))
        || (d3 == 0.0 && on_segment(a, b, c))
        || (d4 == 0.0 && on_segment(a, b, d))
}

/// Whether the closed polygon is simple: no repeated consecutive vertices,
/// and no two non-adjacent edges touch. Sweep over edges sorted by their
/// left end, so typical scene layers cost far less than `O(n²)`.
pub fn is_simple(outline: &[CanvasPoint]) -> bool {
    let n = outline.len();
    if n < 3 {
        return false;
    }
    let edge = |i: usize| (outline[i], outline[(i + 1) % n]);
    if (0..n).any(|i| {
        let (a, b) = edge(i);
        a == b
    }) {
        return false;
    }
    let mut order: Vec<usize> = (0..n).collect();
    let minx = |i: usize| {
        let (a, b) = edge(i);
        a.x.min(b.x)
    };
    order.sort_by(|&i, &j| minx(i).total_cmp(&minx(j)));
    for (k, &i) in order.iter().enumerate() {
        let (a, b) = edge(i);
        let maxx = a.x.max(b.x);
        for &j in &order[k + 1..] {
            if minx(j) > maxx {
                break;
            }
            let adjacent = (i + 1) % n == j || (j + 1) % n == i;
            if adjacent {
                // Adjacent edges share one endpoint; they must not fold back
                // onto each other.
                let (c, d) = edge(j);
                let shared = if b == c { b } else { a };
                let (p, q) = if b == c { (a, d) } else { (c, b) };
                if orient(p, shared, q) == 0.0
                    && on_segment(shared, p, q)
                    && on_segment(shared, q, p)
                {
                    return false;
                }
                continue;
            }
            let (c, d) = edge(j);
            if segments_touch(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::AspectRatio;
    use crate::scene::{SceneKey, SceneLayer, diagnostic_seeds};
    use crate::settings::FormSettings;

    fn p(x: f32, y: f32) -> CanvasPoint {
        CanvasPoint { x, y }
    }

    #[test]
    fn simple_and_non_simple_polygons() {
        assert!(is_simple(&[
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0)
        ]));
        // Bow tie.
        assert!(!is_simple(&[
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(1.0, 0.0),
            p(0.0, 1.0)
        ]));
        // Repeated vertex.
        assert!(!is_simple(&[
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(1.0, 0.0),
            p(0.0, 1.0)
        ]));
        // A vertex touching a non-adjacent edge.
        assert!(!is_simple(&[
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(1.0, 1.0),
            p(1.0, 0.0),
            p(0.5, 1.0)
        ]));
        // Spike folding back along its own edge.
        assert!(!is_simple(&[
            p(0.0, 0.0),
            p(2.0, 0.0),
            p(1.0, 0.0),
            p(0.0, 1.0)
        ]));
    }

    #[test]
    fn rasterizer_fills_front_most_layer() {
        let aspect = AspectRatio::of(2, 1);
        let key = SceneKey::new(0, &diagnostic_seeds(1), FormSettings::default(), aspect);
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32, depth: f32| SceneLayer {
            role: LayerRole::Sky,
            depth,
            shade: 0.5,
            outline: vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)],
        };
        let scene = Scene::new(
            key,
            vec![rect(0.0, 0.0, 1.0, 1.0, 1.0), rect(0.5, 0.5, 2.5, 2.0, 0.5)],
        )
        .unwrap();
        // 8×4 samples over a 2×1 canvas: centres at x = 0.125, 0.375, …
        let ids = front_layers(&scene, 8, 4);
        let row = |r: usize| ids[r * 8..(r + 1) * 8].to_vec();
        assert_eq!(row(0), [0, 0, 0, 0, NONE, NONE, NONE, NONE]);
        assert_eq!(row(3), [0, 0, 1, 1, 1, 1, 1, 1]);
        assert_eq!(empty_fraction(&scene, 8, 4), 8.0 / 32.0);
    }
}
