//! Stateless 1D value noise and band-limited fBm for scene profiles.
//!
//! Everything here is integer hashing plus `+ − × ÷` and `floor`, so results
//! are bit-identical on every platform (tier 1). The noise is evaluated at
//! arbitrary canvas positions without consuming RNG state, so a profile does
//! not depend on how many samples were taken before it.

/// SplitMix64 finalizer: a well-mixed 64-bit permutation.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform in `[-1, 1)` for lattice point `i` of the noise `seed`.
pub(crate) fn lattice(seed: u64, i: i64) -> f64 {
    let z = mix(seed ^ mix((i as u64).wrapping_add(0x9e37_79b9_7f4a_7c15)));
    (z >> 11) as f64 * (1.0 / (1u64 << 52) as f64) - 1.0
}

/// Uniform in `[0, 1)` for an integer key, for per-feature constants.
pub(crate) fn unit(seed: u64, i: i64) -> f64 {
    0.5 * (lattice(seed, i) + 1.0)
}

/// Quintic fade `6t⁵ − 15t⁴ + 10t³`: C² continuous at lattice points.
fn quintic(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Value noise at `x` (lattice spacing 1). `angular` = 0 interpolates with
/// the quintic fade (rounded), 1 linearly (straight segments meeting at
/// kinks: facets); values between blend the two weights.
pub(crate) fn value(seed: u64, x: f64, angular: f64) -> f64 {
    let fi = x.floor();
    let t = x - fi;
    let i = fi as i64;
    let a = lattice(seed, i);
    let b = lattice(seed, i + 1);
    let q = quintic(t);
    let w = q + (t - q) * angular;
    a + (b - a) * w
}

/// Band-limited fractional Brownian motion.
///
/// Octave `k` has wavelength `base / 2ᵏ` and amplitude `gain^k`; octaves
/// stop before the wavelength drops below `min_wavelength` (canvas units),
/// so silhouettes never carry detail finer than the preview can show. Each
/// octave has its own lattice seed and a fractional offset, so kinks of
/// different octaves never line up at the origin. The result is normalized
/// by the amplitude sum and lies in `[-1, 1]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fbm {
    pub seed: u64,
    pub base_wavelength: f64,
    pub min_wavelength: f64,
    pub gain: f64,
    pub angular: f64,
}

/// Octave cap independent of the wavelength ratio.
pub(crate) const MAX_OCTAVES: u32 = 8;

impl Fbm {
    pub fn octaves(&self) -> u32 {
        let mut n = 1;
        let mut w = self.base_wavelength * 0.5;
        while n < MAX_OCTAVES && w >= self.min_wavelength {
            n += 1;
            w *= 0.5;
        }
        n
    }

    pub fn eval(&self, x: f64) -> f64 {
        let mut sum = 0.0;
        let mut norm = 0.0;
        let mut amp = 1.0;
        let mut freq = 1.0 / self.base_wavelength;
        for k in 0..self.octaves() {
            let s = mix(self
                .seed
                .wrapping_add(u64::from(k).wrapping_mul(0xa076_1d64_78bd_642f)));
            let offset = unit(s, -1);
            sum += amp * value(s, x * freq + offset, self.angular);
            norm += amp;
            amp *= self.gain;
            freq *= 2.0;
        }
        sum / norm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lattice_is_bounded_and_seeded() {
        for i in -1000..1000 {
            let v = lattice(3, i);
            assert!((-1.0..1.0).contains(&v));
        }
        assert_ne!(lattice(3, 5), lattice(4, 5));
        assert_eq!(lattice(3, 5), lattice(3, 5));
    }

    #[test]
    fn value_noise_hits_lattice_values_and_is_continuous() {
        for angular in [0.0, 0.5, 1.0] {
            assert_eq!(value(9, 4.0, angular), lattice(9, 4));
            let below = value(9, 5.0 - 1e-9, angular);
            assert!((below - lattice(9, 5)).abs() < 1e-6);
        }
        // Linear interpolation is exactly linear between lattice points.
        let (a, b) = (lattice(9, 2), lattice(9, 3));
        assert_eq!(value(9, 2.25, 1.0), a + (b - a) * 0.25);
    }

    #[test]
    fn fbm_is_normalized_and_band_limited() {
        let f = Fbm {
            seed: 1,
            base_wavelength: 0.4,
            min_wavelength: 1.0 / 160.0,
            gain: 0.5,
            angular: 0.3,
        };
        // 0.4, 0.2, 0.1, 0.05, 0.025, 0.0125: the next (0.00625) is exactly 1/160.
        assert_eq!(f.octaves(), 7);
        let coarse = Fbm {
            min_wavelength: 0.05,
            ..f
        };
        assert_eq!(coarse.octaves(), 4);
        for i in 0..4000 {
            let v = f.eval(i as f64 * 0.001 - 2.0);
            assert!((-1.0..=1.0).contains(&v), "{v}");
        }
    }

    #[test]
    fn values_are_frozen() {
        // Exact arithmetic only: these bits hold on every platform.
        let f = Fbm {
            seed: 42,
            base_wavelength: 0.3,
            min_wavelength: 0.01,
            gain: 0.55,
            angular: 0.5,
        };
        let got: Vec<u64> = [0.0, 0.123, -1.7, 3.3].map(|x| f.eval(x).to_bits()).into();
        assert_eq!(got, FROZEN);
    }

    const FROZEN: [u64; 4] = [
        4596363687158826968,
        4600366934366048046,
        13818820389651645585,
        13820019136848052563,
    ];
}
