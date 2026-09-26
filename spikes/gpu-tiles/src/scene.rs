//! CPU-side scene description. Everything structural is decided here, in
//! f64, from domain-separated seed streams; the GPU only evaluates the
//! resulting parameters per pixel. This is a spike stand-in for the real seed
//! engine (task 04) and composition generator (task 05).

pub const N_LAYERS: usize = 4;
pub const N_VERTS: usize = 24;
pub const STROKE_PTS: usize = 6;

/// SplitMix64: tiny, well-known, fully specified.
pub struct Stream(u64);

impl Stream {
    /// Domain-separated stream: the same seed yields unrelated sequences for
    /// "terrain" and "paint", so tweaking one never perturbs the other.
    pub fn new(seed: u64, domain: &str) -> Self {
        let mut h = 0xcbf2_9ce4_8422_2325u64; // FNV-1a of the domain label
        for b in domain.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Stream(seed ^ h)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

pub struct Scene {
    /// N_LAYERS * N_VERTS of (x, y, facet_slope, tint) in canvas units.
    pub ridges: Vec<[f32; 4]>,
    /// Per stroke: STROKE_PTS of (x, y, half_width, 0) then (r, g, b, alpha).
    pub strokes: Vec<[f32; 4]>,
    pub n_strokes: u32,
}

/// `w`, `h` are the canvas extents in canvas units (short side == 1).
pub fn build(seed: u64, w: f64, h: f64) -> Scene {
    let mut terrain = Stream::new(seed, "terrain");
    let mut paint = Stream::new(seed, "paint-detail");

    let base = [0.36, 0.45, 0.58, 0.80];
    let amp = [0.16, 0.12, 0.07, 0.06];
    let mut ridges = Vec::with_capacity(N_LAYERS * N_VERTS);
    for l in 0..N_LAYERS {
        // Midpoint-style massing: a few big angular peaks plus jitter.
        let peaks = 2.0 + (terrain.unit() * 2.0).floor();
        let phase = terrain.unit();
        for k in 0..N_VERTS {
            let f = k as f64 / (N_VERTS - 1) as f64;
            let jitter = if k == 0 || k == N_VERTS - 1 {
                0.0
            } else {
                terrain.range(-0.35, 0.35) / (N_VERTS - 1) as f64
            };
            let x = (-0.05 + 1.1 * (f + jitter)) * w;
            let tri = 1.0 - (2.0 * ((f * peaks + phase).fract() - 0.5)).abs();
            let mut y = h * (base[l] - amp[l] * (0.65 * tri + 0.35 * terrain.unit()));
            if l == 3 {
                // Foreground bank rises on one side and leaves room for water.
                y += h * 0.22 * smooth(0.35, 0.8, f);
            }
            let slope = terrain.range(-0.9, 0.9);
            let tint = terrain.unit();
            ridges.push([x as f32, y as f32, slope as f32, tint as f32]);
        }
    }

    let palette: [[f32; 3]; 5] = [
        [0.30, 0.24, 0.18], // umber
        [0.18, 0.26, 0.20], // green-black
        [0.82, 0.68, 0.45], // warm light ochre
        [0.36, 0.40, 0.55], // blue shadow
        [0.93, 0.90, 0.82], // body white
    ];
    let mut strokes = Vec::new();
    let mut n = 0u32;
    let mut push_stroke = |pts: &[[f64; 3]; STROKE_PTS], rgb: [f32; 3], a: f32| {
        for p in pts {
            strokes.push([p[0] as f32, p[1] as f32, p[2] as f32, 0.0]);
        }
        strokes.push([rgb[0], rgb[1], rgb[2], a]);
        n += 1;
    };

    // Foreground marks: loose, mostly horizontal, tapered.
    for _ in 0..18 {
        let x0 = paint.range(0.02, 0.9) * w;
        let y0 = paint.range(0.74, 0.98) * h;
        let len = paint.range(0.05, 0.18);
        let ang = paint.range(-0.35, 0.25);
        let hw = paint.range(0.0025, 0.007);
        let bend = paint.range(-0.03, 0.03);
        let mut pts = [[0.0; 3]; STROKE_PTS];
        for (i, p) in pts.iter_mut().enumerate() {
            let t = i as f64 / (STROKE_PTS - 1) as f64;
            let taper = 1.0 - 0.7 * t;
            p[0] = x0 + len * t * ang.cos();
            p[1] = y0 + len * t * ang.sin() + bend * (t * std::f64::consts::PI).sin();
            p[2] = hw * taper;
        }
        let c = palette[(paint.next_u64() % palette.len() as u64) as usize];
        push_stroke(&pts, c, paint.range(0.8, 0.95) as f32);
    }

    // Midground verticals (tree/reed marks) cross horizontal tile edges.
    for _ in 0..10 {
        let x0 = paint.range(0.05, 0.95) * w;
        let y0 = paint.range(0.55, 0.7) * h;
        let len = paint.range(0.06, 0.14);
        let hw = paint.range(0.002, 0.004);
        let mut pts = [[0.0; 3]; STROKE_PTS];
        for (i, p) in pts.iter_mut().enumerate() {
            let t = i as f64 / (STROKE_PTS - 1) as f64;
            p[0] = x0 + 0.01 * (t * 5.0).sin() * paint.range(-1.0, 1.0);
            p[1] = y0 - len * t;
            p[2] = hw * (1.0 - 0.8 * t);
        }
        push_stroke(&pts, palette[1], 0.9);
    }

    // Diagnostic mark: one long diagonal that crosses every tile row and
    // column so the seam comparison always exercises a mark on a boundary.
    let mut pts = [[0.0; 3]; STROKE_PTS];
    for (i, p) in pts.iter_mut().enumerate() {
        let t = i as f64 / (STROKE_PTS - 1) as f64;
        p[0] = (0.04 + 0.92 * t) * w;
        p[1] = (0.12 + 0.84 * t) * h + 0.01 * (t * 9.0).sin();
        p[2] = 0.004 * (1.0 - 0.5 * t);
    }
    push_stroke(&pts, palette[3], 0.85);

    Scene {
        ridges,
        strokes,
        n_strokes: n,
    }
}

fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
