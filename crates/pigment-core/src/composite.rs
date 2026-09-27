//! The compositing model (task 06): the reference the painting shader
//! implements, in linear light.
//!
//! - **Glaze** (a watercolor wash): light passes through pigment, so the
//!   color under it is multiplied by a transmittance, `under · T^density`,
//!   which is `under · exp(−A · density)` with absorbance `A = −ln T`. A glaze
//!   never adds light, so paper shows through and washes stay luminous.
//!   Overlapping glazes multiply: their order does not matter, and where two
//!   washes overlap the result is darker than either (the watercolor
//!   overlap band).
//! - **Wash:** a glaze over paper whose pigment reads as `pigment` at
//!   density 1: `T = pigment / paper`.
//! - **Over** (gouache, opaque marks): premultiplied "over" of an opaque
//!   color at coverage `a`: `color · a + under · (1 − a)`.
//! - **Edge blend:** a soft edge averages opaque samples. Each sample is an
//!   opaque color at coverage `1/n`, so the average is premultiplied over
//!   applied `n` times, and it stays inside the samples' range per channel
//!   (no dark fringes). Storing a straight (non-premultiplied) color with its
//!   coverage and multiplying by coverage again is the classic fringe; the
//!   tests show it as a negative control.
//!
//! `paint.wgsl` uses the same operations (`glaze`, `wash`, `over`), and the
//! hardware suite checks that the GPU evaluates them like this module
//! (`compositing_matches_the_reference_model`).

pub type Rgb = [f32; 3];

/// Transmittance is kept in `[MIN_TRANSMITTANCE, 1]`: no glaze is fully
/// opaque (black) or adds light, and `ln T` stays finite.
pub const MIN_TRANSMITTANCE: f32 = 1e-3;

fn map(a: Rgb, f: impl Fn(usize, f32) -> f32) -> Rgb {
    [f(0, a[0]), f(1, a[1]), f(2, a[2])]
}

/// A glaze of transmittance `t` (per channel) at `density` over `under`.
pub fn glaze(under: Rgb, t: Rgb, density: f32) -> Rgb {
    map(under, |i, u| {
        u * t[i].clamp(MIN_TRANSMITTANCE, 1.0).powf(density)
    })
}

/// A wash of `pigment` over `paper`: the pigment's color at density 1.
pub fn wash(paper: Rgb, pigment: Rgb, density: f32) -> Rgb {
    glaze(paper, map(pigment, |i, p| p / paper[i]), density)
}

/// Premultiplied "over" of an opaque `color` at coverage `a`.
pub fn over(under: Rgb, color: Rgb, a: f32) -> Rgb {
    map(under, |i, u| color[i] * a + u * (1.0 - a))
}

/// A soft edge: the mean of opaque samples.
pub fn edge_blend(samples: &[Rgb]) -> Rgb {
    let n = samples.len() as f32;
    let sum = samples
        .iter()
        .fold([0.0; 3], |s, c| map(s, |i, v| v + c[i]));
    map(sum, |_, v| v / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAPER: Rgb = [0.905, 0.85, 0.74];
    const COLORS: [Rgb; 5] = [
        [0.05, 0.2, 0.07],
        [0.6, 0.3, 0.1],
        [0.2, 0.45, 0.6],
        [0.9, 0.85, 0.7],
        [0.0, 0.0, 0.0],
    ];

    fn close(a: Rgb, b: Rgb, tol: f32) -> bool {
        a.iter()
            .zip(&b)
            .all(|(x, y)| (x - y).abs() <= tol * (1.0 + y.abs()))
    }

    #[test]
    fn opaque_fill_hides_what_is_under() {
        for u in COLORS {
            for c in COLORS {
                assert_eq!(over(u, c, 1.0), c);
                assert_eq!(over(u, c, 0.0), u);
            }
        }
    }

    #[test]
    fn paper_survives_a_paper_colored_wash() {
        for d in [0.0, 0.8, 1.0, 1.2, 3.0] {
            assert!(close(wash(PAPER, PAPER, d), PAPER, 1e-6), "density {d}");
        }
        // Density 0 is no pigment at all.
        for c in COLORS {
            assert_eq!(wash(PAPER, c, 0.0), PAPER);
        }
    }

    #[test]
    fn a_wash_at_density_one_is_its_pigment() {
        for c in &COLORS[..4] {
            assert!(close(wash(PAPER, *c, 1.0), *c, 1e-6), "{c:?}");
        }
    }

    #[test]
    fn glazes_only_darken_and_stay_finite() {
        for u in COLORS {
            for t in COLORS {
                for d in [0.0, 0.8, 1.2, 4.0] {
                    let g = glaze(u, t, d);
                    for i in 0..3 {
                        assert!(
                            g[i].is_finite() && g[i] >= 0.0 && g[i] <= u[i],
                            "{u:?} {t:?} {d}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn overlapping_washes_multiply() {
        let t1 = [0.4, 0.7, 0.5];
        let t2 = [0.8, 0.5, 0.6];
        for d in [0.8, 1.0, 1.2] {
            let ab = glaze(glaze(PAPER, t1, d), t2, d);
            let ba = glaze(glaze(PAPER, t2, d), t1, d);
            assert!(close(ab, ba, 1e-6), "order must not matter");
            let product = map(PAPER, |i, p| p * t1[i].powf(d) * t2[i].powf(d));
            assert!(close(ab, product, 1e-6));
            // The overlap band is darker than either wash alone.
            let (a, b) = (glaze(PAPER, t1, d), glaze(PAPER, t2, d));
            assert!((0..3).all(|i| ab[i] < a[i] && ab[i] < b[i]));
        }
        // Density behaves like stacked layers of the same wash.
        let twice = glaze(glaze(PAPER, t1, 1.0), t1, 1.0);
        assert!(close(glaze(PAPER, t1, 2.0), twice, 1e-6));
    }

    #[test]
    fn edge_blend_has_no_dark_fringe() {
        let (dark, light) = (COLORS[0], COLORS[3]);
        for k in 0..=8 {
            let a = k as f32 / 8.0;
            let mut samples = vec![light; 8 - k];
            samples.extend(std::iter::repeat_n(dark, k));
            let blended = edge_blend(&samples);
            assert!(close(blended, over(light, dark, a), 1e-6));
            for i in 0..3 {
                let (lo, hi) = (dark[i].min(light[i]), dark[i].max(light[i]));
                assert!(blended[i] >= lo - 1e-6 && blended[i] <= hi + 1e-6);
            }
        }
        // Negative control: a straight color filtered against empty (black)
        // texels picks up `color · a`, and straight "over" multiplies by `a`
        // again. The edge comes out darker than the correct blend.
        let a = 0.5;
        let fringe = map(light, |i, u| dark[i] * a * a + u * (1.0 - a));
        let correct = over(light, dark, a);
        assert!((0..3).all(|i| fringe[i] < correct[i] - 0.01));
    }
}
