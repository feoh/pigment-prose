// Pigment Prose architecture spike: tiled procedural painting.
//
// Conventions (see docs/architecture-spike.md):
// - Every sample is a function of the WHOLE-IMAGE pixel coordinate
//   `gpx = tile_origin + local`, never of the tile-local coordinate.
// - Canvas units: 1.0 == the image's short side. Placement, mark widths and
//   texture wavelengths are defined in canvas units, so they scale with output
//   resolution. The pixel footprint (1 / short_side) is used only for
//   antialiasing and band-limiting.
// - Color: linear RGB. Watercolor layers are transmittance glazes
//   (Beer-Lambert, `c *= exp(-absorb * density)`); gouache marks are
//   premultiplied "over". sRGB encoding happens once, at the final store.

struct Params {
    image_size: vec2<u32>,
    // Whole-image pixel coordinate of this dispatch's texel (0,0).
    ext_origin: vec2<i32>,
    // Size of the extended (apron-inclusive) region.
    ext_size: vec2<u32>,
    // Offset of the tile interior inside the extended region, and its size.
    interior_offset: vec2<u32>,
    interior_size: vec2<u32>,
    blur_radius: u32,
    seed: u32,
    flags: u32,
    n_strokes: u32,
    short_side: f32,
    aspect: f32,
    _pad0: f32,
    _pad1: f32,
}

const FLAG_TILE_LOCAL_NOISE: u32 = 1u; // negative control: deliberately wrong

const N_LAYERS: u32 = 4u;
const N_VERTS: u32 = 24u;
const STROKE_PTS: u32 = 6u;

@group(0) @binding(0) var<uniform> P: Params;
// Ridge vertices: N_LAYERS * N_VERTS entries of (x, y, facet_slope, tint).
@group(0) @binding(1) var<storage, read> ridges: array<vec4<f32>>;
// Strokes: per stroke, STROKE_PTS points (x, y, half_width, _) followed by
// one color entry (r, g, b, opacity), all in canvas units / linear RGB.
@group(0) @binding(2) var<storage, read> strokes: array<vec4<f32>>;
// Intermediates are written as storage textures by one pass and read as
// sampled textures (textureLoad, no filtering) by the next; each pass gets its
// own bind group so no dispatch uses a texture both ways.
@group(0) @binding(3) var masks_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var masks_r: texture_2d<f32>;
@group(0) @binding(5) var blurh_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var blurh_r: texture_2d<f32>;
@group(0) @binding(7) var out_tex: texture_storage_2d<rgba8unorm, write>;

// ---------------------------------------------------------------- hashing

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash3(x: i32, y: i32, s: u32) -> f32 {
    let h = pcg(bitcast<u32>(x) ^ pcg(bitcast<u32>(y) ^ pcg(s ^ P.seed)));
    return f32(h) * (1.0 / 4294967296.0);
}

// Value noise on an integer lattice in canvas space. Lattice indices come
// from floor() of a whole-image canvas coordinate, so every tile agrees.
fn vnoise(p: vec2<f32>, s: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let ix = i32(i.x);
    let iy = i32(i.y);
    let a = hash3(ix, iy, s);
    let b = hash3(ix + 1, iy, s);
    let c = hash3(ix, iy + 1, s);
    let d = hash3(ix + 1, iy + 1, s);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Band-limited fbm: `base_wavelength` in canvas units. Octaves whose
// wavelength falls below ~2 output pixels fade out, so the texture is
// resolution-aware instead of aliasing into confetti at preview size.
fn fbm(c: vec2<f32>, base_wavelength: f32, octaves: u32, s: u32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amp = 1.0;
    var wl = base_wavelength;
    for (var o = 0u; o < octaves; o++) {
        let wl_px = wl * P.short_side;
        let w = amp * smoothstep(1.5, 3.0, wl_px);
        sum += w * vnoise(c / wl, s + o * 101u);
        norm += amp;
        amp *= 0.55;
        wl *= 0.5;
    }
    // Faded octaves contribute their mean (0.5) rather than vanishing, so
    // the overall value level stays consistent across resolutions.
    var faded = 0.0;
    amp = 1.0;
    wl = base_wavelength;
    for (var o = 0u; o < octaves; o++) {
        let wl_px = wl * P.short_side;
        faded += amp * (1.0 - smoothstep(1.5, 3.0, wl_px)) * 0.5;
        amp *= 0.55;
        wl *= 0.5;
    }
    return (sum + faded) / norm;
}

// ---------------------------------------------------------------- scene

fn global_px(local: vec2<u32>) -> vec2<i32> {
    return P.ext_origin + vec2<i32>(local);
}

// Canvas coordinate of a pixel centre. With the negative-control flag the
// coordinate used for *noise* is deliberately derived from the tile-local
// position, reproducing the classic seam bug.
fn canvas_of(gpx: vec2<i32>) -> vec2<f32> {
    return (vec2<f32>(gpx) + 0.5) / P.short_side;
}

fn noise_canvas(gpx: vec2<i32>) -> vec2<f32> {
    if ((P.flags & FLAG_TILE_LOCAL_NOISE) != 0u) {
        let local = gpx - P.ext_origin - vec2<i32>(P.interior_offset);
        return (vec2<f32>(local) + 0.5) / P.short_side;
    }
    return canvas_of(gpx);
}

struct RidgeHit {
    height: f32,
    seg: u32,
}

// Piecewise-linear (angular) ridge with a small band-limited ragged detail.
fn ridge(layer: u32, c: vec2<f32>, nc: vec2<f32>) -> RidgeHit {
    let base = layer * N_VERTS;
    var seg = 0u;
    for (var k = 0u; k < N_VERTS - 2u; k++) {
        if (c.x >= ridges[base + k + 1u].x) {
            seg = k + 1u;
        }
    }
    let a = ridges[base + seg];
    let b = ridges[base + seg + 1u];
    let t = clamp((c.x - a.x) / max(b.x - a.x, 1e-5), 0.0, 1.0);
    let detail = (fbm(vec2<f32>(nc.x, f32(layer) * 7.0), 0.03, 5u, 11u + layer) - 0.5)
        * (0.012 + 0.006 * f32(layer));
    return RidgeHit(mix(a.y, b.y, t) + detail, seg);
}

// Antialiased coverage of "below the ridge" (y grows downward).
fn below(y: f32, h: f32) -> f32 {
    let px = 1.0 / P.short_side;
    return clamp((y - h) / px + 0.5, 0.0, 1.0);
}

// Cezanne-like color planes: facet boundaries descend from ridge vertices
// at per-vertex slopes; the facet index selects a value/temperature shift.
fn facet_tint(layer: u32, c: vec2<f32>) -> f32 {
    let base = layer * N_VERTS;
    var idx = 0u;
    for (var k = 0u; k < N_VERTS; k++) {
        let v = ridges[base + k];
        let bx = v.x + v.z * (c.y - v.y);
        if (c.x > bx) {
            idx = k;
        }
    }
    return ridges[base + idx].w;
}

// The translucent wash's hard footprint: a low-frequency blob confined to
// the middle band of the picture.
fn wash_mask(c: vec2<f32>, nc: vec2<f32>) -> f32 {
    let n = fbm(nc, 0.35, 4u, 71u);
    let band = smoothstep(0.30, 0.45, c.y) * (1.0 - smoothstep(0.78, 0.9, c.y));
    let px = 1.0 / P.short_side;
    let edge = (n * band - 0.34) / (2.0 * px);
    return clamp(edge + 0.5, 0.0, 1.0);
}

// ---------------------------------------------------------------- pass A

@compute @workgroup_size(8, 8)
fn masks_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.ext_size.x || id.y >= P.ext_size.y) {
        return;
    }
    let gpx = global_px(id.xy);
    let c = canvas_of(gpx);
    let nc = noise_canvas(gpx);
    let r1 = ridge(1u, c, nc);
    let mid_ridge = below(c.y, r1.height);
    let wash = wash_mask(c, nc);
    textureStore(masks_w, vec2<i32>(id.xy), vec4<f32>(mid_ridge, wash, 0.0, 1.0));
}

// ---------------------------------------------------------------- pass B

fn gauss(i: i32, r: u32) -> f32 {
    let sigma = max(f32(r) / 2.5, 0.5);
    let x = f32(i) / sigma;
    return exp(-0.5 * x * x);
}

@compute @workgroup_size(8, 8)
fn blur_h_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.ext_size.x || id.y >= P.ext_size.y) {
        return;
    }
    let r = i32(P.blur_radius);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    let maxx = i32(P.ext_size.x) - 1;
    for (var i = -r; i <= r; i++) {
        // Clamping only affects apron texels, which the interior never reads
        // when apron >= blur radius (the negative control violates this).
        let x = clamp(i32(id.x) + i, 0, maxx);
        let w = gauss(i, P.blur_radius);
        acc += w * textureLoad(masks_r, vec2<i32>(x, i32(id.y)), 0);
        wsum += w;
    }
    textureStore(blurh_w, vec2<i32>(id.xy), acc / wsum);
}

// ---------------------------------------------------------------- pass C

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Absorbance for a pigment that reads as `srgb` when laid at density 1 on
// white paper.
fn absorb(srgb: vec3<f32>) -> vec3<f32> {
    return -log(max(srgb_decode(srgb), vec3<f32>(1e-3)));
}

fn glaze(c: vec3<f32>, pigment_srgb: vec3<f32>, density: f32) -> vec3<f32> {
    return c * exp(-absorb(pigment_srgb) * density);
}

fn over(c: vec3<f32>, color_lin: vec3<f32>, alpha: f32) -> vec3<f32> {
    return color_lin * alpha + c * (1.0 - alpha);
}

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-10), 0.0, 1.0);
    return vec2<f32>(length(p - (a + t * ab)), t);
}

@compute @workgroup_size(8, 8)
fn composite_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.interior_size.x || id.y >= P.interior_size.y) {
        return;
    }
    let e = id.xy + P.interior_offset; // position in extended region
    let gpx = global_px(e);
    let c = canvas_of(gpx);
    let nc = noise_canvas(gpx);
    let px = 1.0 / P.short_side;

    // Vertical half of the separable blur (finite support: needs `r` rows of
    // apron above and below the interior).
    let r = i32(P.blur_radius);
    var soft = vec4<f32>(0.0);
    var wsum = 0.0;
    let maxy = i32(P.ext_size.y) - 1;
    for (var i = -r; i <= r; i++) {
        let y = clamp(i32(e.y) + i, 0, maxy);
        let w = gauss(i, P.blur_radius);
        soft += w * textureLoad(blurh_r, vec2<i32>(i32(e.x), y), 0);
        wsum += w;
    }
    soft /= wsum;
    let hard = textureLoad(masks_r, vec2<i32>(e), 0);

    // Paper: warm off-white with fibre texture in canvas units.
    let grain = fbm(nc, 0.006, 5u, 3u);
    let fibre = fbm(vec2<f32>(nc.x * 0.35, nc.y * 2.5), 0.01, 3u, 5u);
    var col = srgb_decode(vec3<f32>(0.955, 0.935, 0.895)) * (0.965 + 0.05 * grain + 0.02 * fibre);

    // Sky: graded wash with low-frequency blooms, lighter toward the horizon.
    let bloom = fbm(nc, 0.25, 4u, 17u);
    let sky_density = (0.55 - 0.45 * smoothstep(0.0, 0.5, c.y)) * (0.7 + 0.6 * bloom)
        * (0.85 + 0.3 * grain);
    col = glaze(col, vec3<f32>(0.62, 0.74, 0.88), sky_density);

    // Layered angular landforms, back to front.
    let pigments = array<vec3<f32>, 4>(
        vec3<f32>(0.70, 0.72, 0.86), // far ridge: pale violet-blue
        vec3<f32>(0.52, 0.62, 0.70), // mid ridge: blue-grey
        vec3<f32>(0.46, 0.60, 0.38), // near hills: sap green
        vec3<f32>(0.66, 0.50, 0.30), // foreground bank: warm ochre
    );
    let densities = array<f32, 4>(0.45, 0.55, 0.62, 1.0);
    for (var l = 0u; l < N_LAYERS; l++) {
        let rh = ridge(l, c, nc);
        var cov = below(c.y, rh.height);
        if (l == 1u) {
            // Selective edge: crisp on the left, dissolving to soft on the
            // right. The soft variant is the finite-support blurred mask.
            let t = smoothstep(0.25, 0.85, c.x / P.aspect);
            cov = mix(hard.r, soft.r, t);
        }
        // Facet planes matter on the visible face just below the ridge line;
        // fade them with depth so transparent layers don't streak the layers
        // painted over them.
        let face = 1.0 - smoothstep(0.0, 0.3, c.y - rh.height);
        let tint = mix(0.5, facet_tint(l, c), face);
        let granulation = 0.9 + 0.2 * grain;
        let d = densities[l] * (0.7 + 0.6 * tint) * granulation;
        if (l == 3u) {
            // Gouache-like foreground: opaque body colour, broken by paper.
            let body = srgb_decode(pigments[l] * (0.8 + 0.35 * tint));
            col = over(col, body, cov * clamp(0.88 + 0.2 * (grain - 0.5), 0.0, 1.0));
        } else {
            col = glaze(col, pigments[l], d * cov);
        }
    }

    // One translucent shadow wash with pigment pooling at its edge:
    // edge darkening = hard footprint minus its blur (finite support).
    let pool = max(hard.g - soft.g, 0.0);
    let wash_d = soft.g * 0.35 * (0.85 + 0.3 * grain) + pool * 0.6;
    col = glaze(col, vec3<f32>(0.55, 0.52, 0.72), wash_d);

    // Marks: opaque tapered strokes with dry-brush breakup.
    let stride = STROKE_PTS + 1u;
    for (var s = 0u; s < P.n_strokes; s++) {
        let b = s * stride;
        var best = vec3<f32>(1e9, 0.0, 0.0); // (signed distance, t, _)
        for (var k = 0u; k < STROKE_PTS - 1u; k++) {
            let a = strokes[b + k];
            let bb = strokes[b + k + 1u];
            let dt = seg_dist(c, a.xy, bb.xy);
            let hw = mix(a.z, bb.z, dt.y);
            let sd = dt.x - hw;
            if (sd < best.x) {
                best = vec3<f32>(sd, (f32(k) + dt.y) / f32(STROKE_PTS - 1u), 0.0);
            }
        }
        let cov = clamp(-best.x / px + 0.5, 0.0, 1.0);
        if (cov > 0.0) {
            let tone = strokes[b + STROKE_PTS];
            // Drier toward the end of the stroke: paper peaks break through.
            let dryness = smoothstep(0.35, 1.0, best.y);
            let breakup = smoothstep(0.35, 0.65, grain + 0.45 - 0.55 * dryness);
            col = over(col, srgb_decode(tone.rgb), cov * tone.a * breakup);
        }
    }

    let out = srgb_encode(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)));
    textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(out, 1.0));
}
