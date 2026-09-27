// GPU capability smoke renderer: flat-value test card.
//
// Not the painting renderer (tasks 06-07). It exercises the contracts every
// painting pass must follow (docs/architecture.md):
// - samples are functions of WHOLE-IMAGE pixel coordinates, never tile-local;
// - canvas units: short side = 1; `canvas_scale` maps pixels to canvas per axis;
// - neighbourhood passes read only within the declared apron;
// - linear-light working values, sRGB encoded once at the final store.
// WebGPU core only: rgba16float/rgba8unorm write-only storage, textureLoad.

struct Params {
    ext_origin: vec2<i32>,   // whole-image pixel of extended texel (0,0)
    ext_size: vec2<u32>,     // extended (apron-inclusive) region size
    interior_size: vec2<u32>,
    canvas_scale: vec2<f32>, // canvas units per pixel, per axis
    apron: u32,
    seed: u32,
    n_layers: u32,
    paper_grain: f32,
}

@group(0) @binding(0) var<uniform> P: Params;
// Per layer: (first vertex, vertex count, depth, flat value).
@group(0) @binding(1) var<storage, read> layers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> verts: array<vec2<f32>>;
@group(0) @binding(3) var field_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var field_r: texture_2d<f32>;
@group(0) @binding(5) var out_tex: texture_storage_2d<rgba8unorm, write>;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash2(x: i32, y: i32) -> f32 {
    let h = pcg(bitcast<u32>(x) ^ pcg(bitcast<u32>(y) ^ pcg(P.seed)));
    return f32(h) * (1.0 / 4294967296.0);
}

// Value noise on a canvas-space lattice: identical in every tile.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let ix = i32(i.x);
    let iy = i32(i.y);
    return mix(mix(hash2(ix, iy), hash2(ix + 1, iy), u.x),
               mix(hash2(ix, iy + 1), hash2(ix + 1, iy + 1), u.x), u.y);
}

// Crossing-number point-in-polygon test for layer `l`.
fn inside(l: u32, c: vec2<f32>) -> bool {
    let h = layers[l];
    let first = u32(h.x);
    let n = u32(h.y);
    var in_poly = false;
    var j = first + n - 1u;
    for (var i = first; i < first + n; i++) {
        let a = verts[i];
        let b = verts[j];
        if ((a.y > c.y) != (b.y > c.y)) {
            let x = (b.x - a.x) * (c.y - a.y) / (b.y - a.y) + a.x;
            if (c.x < x) {
                in_poly = !in_poly;
            }
        }
        j = i;
    }
    return in_poly;
}

fn canvas_of(ext_texel: vec2<u32>) -> vec2<f32> {
    let gpx = P.ext_origin + vec2<i32>(ext_texel);
    return (vec2<f32>(gpx) + 0.5) * P.canvas_scale;
}

// Pass 1: flat linear value of the front-most covering layer, over the
// extended region (apron included, even beyond the frame).
@compute @workgroup_size(8, 8)
fn field_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.ext_size.x || id.y >= P.ext_size.y) {
        return;
    }
    let c = canvas_of(id.xy);
    var v = 0.0;
    for (var l = 0u; l < P.n_layers; l++) {
        if (inside(l, c)) {
            v = layers[l].w;
        }
    }
    textureStore(field_w, vec2<i32>(id.xy), vec4<f32>(v, 0.0, 0.0, 1.0));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Pass 2: box average over the apron radius (the finite-support "edge
// bleed"), paper grain, sRGB encode. Runs over the interior only.
@compute @workgroup_size(8, 8)
fn composite_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.interior_size.x || id.y >= P.interior_size.y) {
        return;
    }
    let r = i32(P.apron);
    let centre = vec2<i32>(id.xy) + vec2<i32>(r, r);
    var sum = 0.0;
    for (var dy = -r; dy <= r; dy++) {
        for (var dx = -r; dx <= r; dx++) {
            sum += textureLoad(field_r, centre + vec2<i32>(dx, dy), 0).r;
        }
    }
    let side = f32(2 * r + 1);
    var v = sum / (side * side);
    // Paper tooth at a fixed canvas wavelength (1/160 of the short side).
    let c = canvas_of(vec2<u32>(centre));
    v = v * (1.0 - 0.12 * P.paper_grain * vnoise(c * 160.0));
    let rgb = srgb_encode(clamp(vec3<f32>(v, v * 0.97, v * 0.9), vec3<f32>(0.0), vec3<f32>(1.0)));
    textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(rgb, 1.0));
}
