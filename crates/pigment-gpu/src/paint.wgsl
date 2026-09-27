// Pigment Prose painting renderer (task 06).
//
// Contracts (docs/architecture.md): every sample is a function of the
// WHOLE-IMAGE pixel coordinate; canvas units have the short side = 1;
// neighbourhood reads stay inside the declared apron; working color is
// linear light; sRGB is encoded once at the final store. Noise octaves fade
// to their mean below ~2 output pixels (band-limiting), so previews and
// exports agree in value structure.
//
// Pass 1 (materials_main): the front layer's material color per pixel of
// the extended region, and its layer index in alpha.
// Pass 2 (paint_main): loose edges (a coherent noise warp of the lookup
// plus a small disc average across boundaries, wider with distance and
// looseness), wash pooling at edges, paper grain and granulation.

struct Params {
    ext_origin: vec2<i32>,
    ext_size: vec2<u32>,
    interior_size: vec2<u32>,
    canvas_scale: vec2<f32>, // canvas units per pixel, per axis
    apron: u32,
    n_layers: u32,
    seed: u32,
    _pad: u32,
    // edge_looseness, wash_gouache, mark_scale, granulation
    handling: vec4<f32>,
    // paper_grain, palette intensity, haze, pixel footprint (canvas / px)
    look: vec4<f32>,
    // horizon y, summit y, canvas width, canvas height
    frame: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: Params;
// Per layer: (first, count, depth, shade), (bbox), (role, plant, 0, 0).
@group(0) @binding(1) var<storage, read> layers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> verts: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read> pal: array<vec4<f32>>;
@group(0) @binding(4) var field_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var field_r: texture_2d<f32>;
@group(0) @binding(6) var out_tex: texture_storage_2d<rgba8unorm, write>;

// ---------------------------------------------------------------- palette layout
// (pigment_core::palette::Palette::gpu)
const PAPER = 0u;
const SKY_ZENITH = 1u;
const SKY_HORIZON = 2u;
const SUN = 3u;
const HAZE = 4u;
const CLOUD = 5u;       // light, shadow
const STORM = 7u;
const FAR = 8u;         // light, shadow
const ROCK = 10u;
const SNOW = 12u;
const FOREST = 14u;
const FOLIAGE_WARM = 16u;
const FOLIAGE_COOL = 17u;
const MEADOW = 18u;
const MOSS = 20u;
const WATER_DEEP = 21u;
const WATER_FAR = 22u;
const WATER_SHEEN = 23u;
const SAND = 24u;
const PLANTS = 26u;     // light, shadow per plant (1-based plant ids below)
const BARK = 38u;
const WOOD = 39u;
const BLOSSOM = 40u;    // two colors

// Plants (pigment_core::scene::Plant).
const P_BROADLEAF = 1u;
const P_CONIFER = 2u;
const P_BIRCH = 3u;
const P_SHRUB = 4u;
const P_FLOWERING = 5u;
const P_COPPER = 6u;

// Roles (pigment_core::scene::LayerRole).
const R_SKY = 0u;
const R_FAR_RIDGE = 1u;
const R_MID_RIDGE = 2u;
const R_NEAR_RIDGE = 3u;
const R_WATER = 4u;
const R_SHORE = 5u;
const R_WOODLAND = 6u;
const R_ROCK = 7u;
const R_MOUNTAIN = 8u;
const R_CLOUD = 9u;

fn col(i: u32) -> vec3<f32> {
    return pal[i].xyz;
}

// Light/shadow pair by structural shade.
fn shaded(i: u32, shade: f32) -> vec3<f32> {
    return mix(col(i + 1u), col(i), smoothstep(0.08, 0.92, shade));
}

// ---------------------------------------------------------------- noise

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash3(x: i32, y: i32, s: u32) -> f32 {
    let h = pcg(bitcast<u32>(x) ^ pcg(bitcast<u32>(y) ^ pcg(s ^ P.seed)));
    return f32(h) * (1.0 / 4294967296.0);
}

fn vnoise(p: vec2<f32>, s: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let ix = i32(i.x);
    let iy = i32(i.y);
    return mix(mix(hash3(ix, iy, s), hash3(ix + 1, iy, s), u.x),
               mix(hash3(ix, iy + 1, s), hash3(ix + 1, iy + 1, s), u.x), u.y);
}

// Band-limited fbm in [0, 1]: `wl` is the base wavelength in canvas units.
// Octaves under ~2 px fade to their mean (0.5).
fn fbm(c: vec2<f32>, wl0: f32, octaves: u32, s: u32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amp = 1.0;
    var wl = wl0;
    for (var o = 0u; o < octaves; o++) {
        let wl_px = wl / P.look.w;
        let keep = smoothstep(1.5, 3.0, wl_px);
        sum += amp * mix(0.5, vnoise(c / wl, s + o * 101u), keep);
        norm += amp;
        amp *= 0.55;
        wl *= 0.5;
    }
    return sum / norm;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ---------------------------------------------------------------- coverage

fn inside(l: u32, c: vec2<f32>) -> bool {
    let h = layers[3u * l];
    let bb = layers[3u * l + 1u];
    if (c.x < bb.x || c.x > bb.z || c.y < bb.y || c.y > bb.w) {
        return false;
    }
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

fn front_layer(c: vec2<f32>) -> i32 {
    var front = -1;
    for (var l = 0u; l < P.n_layers; l++) {
        if (inside(l, c)) {
            front = i32(l);
        }
    }
    return front;
}

fn canvas_of_ext(t: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(P.ext_origin + vec2<i32>(t)) + 0.5) * P.canvas_scale;
}

// ---------------------------------------------------------------- materials

// Tree crowns: a cellular field of rounded clumps. Returns (crown, lit):
// crown is 1 at a clump's centre and 0 in the gaps between clumps; lit is
// how much the point faces up (the light), so each clump is sunlit on top
// and shadowed below.
fn crown_field(c: vec2<f32>, size: f32, s: u32) -> vec2<f32> {
    return crown_field_wide(c, size, 1.0, s);
}

// `wide` > 1 flattens crowns into low, wide mounds (shrubs).
fn crown_field_wide(c: vec2<f32>, size: f32, wide: f32, s: u32) -> vec2<f32> {
    let p = vec2<f32>(c.x / (size * wide), c.y / size);
    let i = floor(p);
    var best = 1e9;
    var rel = vec2<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            let pt = vec2<f32>(cell) + vec2<f32>(
                0.15 + 0.7 * hash3(cell.x, cell.y, s),
                0.15 + 0.7 * hash3(cell.x, cell.y, s + 1u)
            );
            let radius = 0.6 + 0.4 * hash3(cell.x, cell.y, s + 2u);
            let q = (p - pt) / radius;
            let d = length(q);
            if (d < best) {
                best = d;
                rel = q;
            }
        }
    }
    let crown = clamp(1.0 - best, 0.0, 1.0);
    let lit = clamp(0.55 - 0.75 * rel.y - 0.2 * rel.x, 0.0, 1.0);
    return vec2<f32>(crown, lit);
}

// Conifer spires: tall pointed crowns, cells twice as tall as wide. The
// front-most (lowest) spire wins where they overlap. Lit on the left and
// toward the tip.
fn spire_field(c: vec2<f32>, size: f32, s: u32) -> vec2<f32> {
    let p = vec2<f32>(c.x / size, c.y / (2.0 * size));
    let i = floor(p);
    var crown = 0.0;
    var lit = 0.0;
    var best_y = -1e9;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            let pt = vec2<f32>(cell) + vec2<f32>(
                0.2 + 0.6 * hash3(cell.x, cell.y, s),
                0.2 + 0.6 * hash3(cell.x, cell.y, s + 1u)
            );
            let d = p - pt;
            let t = (d.y + 0.9) / 1.8; // 0 at the tip, 1 at the base
            if (t < 0.0 || t > 1.0) {
                continue;
            }
            let half = 0.06 + 0.46 * t;
            let v = 1.0 - abs(d.x) / half;
            if (v > 0.0 && pt.y > best_y) {
                best_y = pt.y;
                crown = min(1.0, 2.5 * v);
                lit = clamp(0.5 - 0.8 * d.x / half + 0.3 * (1.0 - t), 0.0, 1.0);
            }
        }
    }
    return vec2<f32>(crown, lit);
}

// Plant ids by natural weights (matches pigment_core::scene::Plant::pick).
fn pick_plant(u: f32) -> u32 {
    if (u < 0.26) { return P_BROADLEAF; }
    if (u < 0.48) { return P_CONIFER; }
    if (u < 0.62) { return P_BIRCH; }
    if (u < 0.80) { return P_SHRUB; }
    if (u < 0.92) { return P_FLOWERING; }
    return P_COPPER;
}

// Stands on forested hills: broadleaf is the matrix and the others form
// patches within it (their share is lower than for woodland stands).
fn pick_hill_plant(u: f32) -> u32 {
    if (u < 0.44) { return P_BROADLEAF; }
    if (u < 0.68) { return P_CONIFER; }
    if (u < 0.79) { return P_BIRCH; }
    if (u < 0.88) { return P_SHRUB; }
    if (u < 0.95) { return P_FLOWERING; }
    return P_COPPER;
}

struct Stands {
    first: u32,
    second: u32,
    // 0 on the boundary between the two nearest stands, 1 well inside.
    inside: f32,
}

// Irregular Voronoi stands (smaller with distance) with the two nearest
// stands, so their colors can blend across a soft, warped boundary.
fn stands(c: vec2<f32>, depth: f32) -> Stands {
    let size = 0.14;
    let w = c + (vec2<f32>(fbm(c, 0.04, 3u, 401u), fbm(c, 0.04, 3u, 402u)) - 0.5) * 0.9 * size;
    let p = w / size;
    let i = floor(p);
    var d1 = 1e9;
    var d2 = 1e9;
    var u1 = 0.0;
    var u2 = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            let pt = vec2<f32>(cell) + vec2<f32>(hash3(cell.x, cell.y, 403u), hash3(cell.x, cell.y, 404u));
            let d = length(p - pt);
            let u = hash3(cell.x, cell.y, 405u);
            if (d < d1) {
                d2 = d1;
                u2 = u1;
                d1 = d;
                u1 = u;
            } else if (d < d2) {
                d2 = d;
                u2 = u;
            }
        }
    }
    return Stands(pick_hill_plant(u1), pick_hill_plant(u2), smoothstep(0.0, 0.35, d2 - d1));
}

fn stand_plant(c: vec2<f32>, depth: f32) -> u32 {
    return stands(c, depth).first;
}

// One plant's painted canopy: its own crown shape, colors and details.
// `bb` is the layer's bounding box, for trunks under near canopies.
// Crowns at the true size for this depth would shear into streaks where
// depth changes continuously down the ground, so crowns are drawn at the two
// nearest power-of-two sizes and blended (as texture mipmaps are).
fn plant_color(plant: u32, c: vec2<f32>, depth: f32, shade: f32, bb: vec4<f32>) -> vec3<f32> {
    let near = 1.0 - depth;
    let size = (0.003 + 0.03 * near * near) * P.handling.z;
    let lvl = log2(size / 0.001);
    let l0 = floor(lvl);
    let s0 = 0.001 * exp2(l0);
    let a = plant_color_at(plant, c, s0, depth, shade, bb);
    let b = plant_color_at(plant, c, 2.0 * s0, depth, shade, bb);
    return mix(a, b, lvl - l0);
}

fn plant_color_at(plant: u32, c: vec2<f32>, size: f32, depth: f32, shade: f32, bb: vec4<f32>) -> vec3<f32> {
    let light = col(PLANTS + 2u * (plant - 1u));
    let shadow = col(PLANTS + 2u * (plant - 1u) + 1u);
    let base = mix(shadow, light, smoothstep(0.08, 0.92, shade)) * (0.72 + 0.3 * depth);
    var cr = vec2<f32>(0.0);
    var gap = mix(shadow, col(FOLIAGE_COOL), 0.3) * 0.7;
    var s = size;
    switch plant {
        case P_CONIFER: {
            s = size * 0.9;
            cr = spire_field(c, s, 17u);
        }
        case P_BIRCH: {
            s = size * 0.6;
            cr = crown_field(c, s, 19u);
            gap = mix(gap, col(BARK), 0.35);
        }
        case P_SHRUB: {
            s = size * 0.7;
            cr = crown_field_wide(c, s, 1.8, 23u);
            gap = mix(col(WOOD), shadow, 0.3) * 0.8;
        }
        default: {
            s = size * 1.25;
            let big = crown_field(c, s, 11u);
            let small = crown_field(c + vec2<f32>(0.37, 0.11), s * 0.45, 13u);
            cr = vec2<f32>(max(big.x, 0.8 * small.x), mix(small.y, big.y, 0.6));
        }
    }
    let sun = mix(base, mix(light, col(FOLIAGE_WARM), 0.35), 0.25 + 0.3 * shade);
    var out = mix(base, sun, smoothstep(0.35, 0.9, cr.y) * (0.4 + 0.6 * shade));
    if (plant == P_FLOWERING) {
        // Blossom on the sunlit tops.
        let fsize = s * 0.22;
        let f = crown_field(c + vec2<f32>(1.3, 0.7), fsize, 27u);
        let cell = floor(c / fsize);
        let kind = hash3(i32(cell.x), i32(cell.y), 28u);
        let bloom = smoothstep(0.4, 0.7, f.x) * smoothstep(0.45, 0.8, cr.y) * step(0.35, kind);
        out = mix(out, select(col(BLOSSOM + 1u), col(BLOSSOM), kind > 0.62), 0.85 * bloom);
    }
    out = mix(gap, out, smoothstep(0.02, 0.35, cr.x));
    // Trunks show under near tall canopies.
    if (depth <= 0.25 && plant != P_SHRUB && bb.w > bb.y) {
        let rel = (bb.w - c.y) / (bb.w - bb.y);
        // Irregular: about half the columns carry a trunk, each offset and
        // with its own height and width.
        let tc = c.x / (s * 1.1);
        let col_i = i32(floor(tc));
        let has = hash3(col_i, 0, 30u) < 0.5;
        let fx = fract(tc) - 0.5 - 0.35 * (hash3(col_i, 0, 31u) - 0.5);
        let tw = 0.05 + 0.05 * hash3(col_i, 0, 29u);
        let reach = 0.18 + 0.14 * hash3(col_i, 0, 32u);
        if (has && rel < reach && abs(fx) < tw && cr.x < 0.5) {
            var trunk = col(WOOD) * 0.55;
            if (plant == P_BIRCH) {
                trunk = col(BARK);
            }
            out = mix(out, trunk, smoothstep(reach, reach - 0.12, rel));
        }
    }
    let mean = mix(base, gap, 0.25);
    return mix(mean, out, smoothstep(2.0, 5.0, s / P.look.w));
}

// Painted forest: clumps of crowns at two scales, warm light on their tops,
// cool dark in the gaps. Crowns shrink with distance and fade to their mean
// color when smaller than a few pixels (band-limiting).
fn foliage(base: vec3<f32>, c: vec2<f32>, depth: f32, shade: f32) -> vec3<f32> {
    let near = 1.0 - depth;
    let size = (0.003 + 0.03 * near * near) * P.handling.z;
    let big = crown_field(c, size, 11u);
    let small = crown_field(c + vec2<f32>(0.37, 0.11), size * 0.45, 13u);
    let crown = max(big.x, 0.8 * small.x);
    let lit = mix(small.y, big.y, 0.6);
    let dark = mix(col(FOLIAGE_COOL), col(FOREST + 1u), 0.6) * 0.8;
    let sun = mix(base, col(FOLIAGE_WARM), 0.2 + 0.3 * shade);
    var out = mix(base, sun, smoothstep(0.35, 0.9, lit) * (0.4 + 0.6 * shade));
    out = mix(dark, out, smoothstep(0.02, 0.35, crown));
    // Mean look for sub-pixel crowns.
    let mean = mix(base, dark, 0.25);
    let resolve = smoothstep(2.0, 5.0, size / P.look.w);
    return mix(mean, out, resolve);
}

// Stands of different trees: dark blue-green conifers, bright deciduous
// crowns, in broad patches; nearer forest is deeper in value so the planes
// step forward.
fn forest_color(c: vec2<f32>, depth: f32, shade: f32) -> vec3<f32> {
    var base = shaded(FOREST, shade);
    // Stands shrink and fade with distance, so far slopes read through
    // their light and shadow planes instead of breaking into blotches.
    let near = 1.0 - depth;
    let stand = fbm(c + vec2<f32>(11.0, 3.0), 0.16 * (0.3 + 0.7 * near), 3u, 201u);
    let conifer = mix(col(FOLIAGE_COOL), col(FOREST + 1u), 0.35);
    let amount = near * near;
    base = mix(base, conifer, 0.7 * amount * smoothstep(0.5, 0.4, stand));
    base = mix(base, col(FOLIAGE_WARM), 0.25 * amount * smoothstep(0.56, 0.66, stand));
    base *= 0.72 + 0.3 * depth;
    return foliage(base, c, depth, shade);
}

// Forested hills: a patchwork of plant stands, fading to the generic
// forest with distance so far slopes read through their planes.
fn mixed_forest(c: vec2<f32>, depth: f32, shade: f32, bb: vec4<f32>) -> vec3<f32> {
    let generic = forest_color(c, depth, shade);
    let amount = smoothstep(0.75, 0.35, depth);
    if (amount <= 0.0) {
        return generic;
    }
    let st = stands(c, depth);
    var rgb = plant_color(st.first, c, depth, shade, vec4<f32>(0.0));
    if (st.second != st.first && st.inside < 1.0) {
        let other = plant_color(st.second, c, depth, shade, vec4<f32>(0.0));
        rgb = mix(other, rgb, 0.5 + 0.5 * st.inside);
    }
    return mix(generic, rgb, amount);
}

// Grass: horizontal strokes of warm and cool green.
fn meadow_color(c: vec2<f32>, depth: f32, shade: f32) -> vec3<f32> {
    let base = shaded(MEADOW, shade);
    let size = (0.004 + 0.02 * (1.0 - depth)) * P.handling.z;
    let strokes = fbm(vec2<f32>(c.x * 0.25, c.y), size, 3u, 21u);
    var out = mix(base, col(FOLIAGE_WARM), 0.35 * smoothstep(0.55, 0.75, strokes) * shade);
    out = mix(out, col(MEADOW + 1u), 0.3 * smoothstep(0.55, 0.3, strokes));
    // Wildflowers in the near meadows: sparse dabs of gold, white and
    // violet, only where they are big enough to read.
    if (depth < 0.35) {
        let fsize = 0.006 * P.handling.z;
        let f = crown_field(c, fsize, 301u);
        let cell = floor(c / fsize);
        let pick = hash3(i32(cell.x), i32(cell.y), 303u);
        let bloom = smoothstep(0.55, 0.75, f.x) * step(0.82, pick)
            * smoothstep(0.55, 0.62, fbm(c, 0.12, 2u, 305u))
            * smoothstep(2.0, 4.0, fsize / P.look.w);
        let kind = hash3(i32(cell.x), i32(cell.y), 304u);
        var flower = col(SUN);
        if (kind > 0.66) {
            flower = col(CLOUD);
        } else if (kind > 0.33) {
            flower = mix(col(FAR + 1u), col(STORM), 0.3) * 1.2;
        }
        out = mix(out, flower, bloom);
    }
    return out;
}

fn material(l: u32, c: vec2<f32>) -> vec3<f32> {
    let h = layers[3u * l];
    let role = u32(layers[3u * l + 2u].x);
    var depth = h.z;
    var shade = h.w;
    let horizon = P.frame.x;
    let summit = P.frame.y;
    // Ground layers can span near and far (river banks): below the horizon
    // their effective depth follows the perspective of the ground itself,
    // so crowns and haze match their true distance.
    // Only for ground in front of the water (depth <= 0.45: banks, spurs,
    // near shore and woods). The inverse of the generator's placement: a
    // spur at nearness t sits at y = horizon + fg * t^1.5 with depth
    // 0.44 - 0.3 t, so each spur's base matches the ground under it.
    if ((role == R_NEAR_RIDGE || role == R_WOODLAND || role == R_SHORE)
        && depth <= 0.45 && c.y > horizon) {
        let f = clamp((c.y - horizon) / max(P.frame.w - horizon, 1e-3), 0.0, 1.0);
        // Nearer of the two: a spur keeps its own depth above its base (its
        // ridge stands out against the bank behind) and takes the ground's
        // below it (no step at its foot).
        depth = min(depth, 0.44 - 0.3 * pow(f, 2.0 / 3.0));
        // A ridge's light fades to neutral toward its foot (the bottom of
        // its bounding box), where it meets the neutral river bank.
        if (role == R_NEAR_RIDGE) {
            let bb = layers[3u * l + 1u];
            let rel = clamp((bb.w - c.y) / max(bb.w - bb.y, 1e-4), 0.0, 1.0);
            shade = mix(0.5, shade, smoothstep(0.05, 0.6, rel));
        }
    }
    let rise = max(horizon - summit, 1e-3);
    var rgb = vec3<f32>(0.0);
    switch role {
        case R_SKY: {
            let t = clamp(c.y / max(horizon, 1e-3), 0.0, 1.0);
            rgb = mix(col(SKY_ZENITH), col(SKY_HORIZON), pow(t, 1.4));
            // Soft variation, like an uneven sky wash.
            rgb *= 0.94 + 0.12 * fbm(c, 0.4, 3u, 31u);
        }
        case R_CLOUD: {
            rgb = shaded(CLOUD, shade);
            if (shade < 0.35) {
                rgb = mix(col(STORM), rgb, shade / 0.35);
            }
            rgb *= 0.92 + 0.16 * fbm(c, 0.08, 4u, 41u);
        }
        case R_FAR_RIDGE: {
            rgb = shaded(FAR, shade);
        }
        case R_MOUNTAIN: {
            // Forest below a ragged treeline, rock above, snow on high peaks.
            let tree_y = horizon - rise * (0.36 + 0.2 * fbm(vec2<f32>(c.x, 0.0), 0.12, 3u, 51u));
            let snow_y = summit + rise * (0.18 + 0.14 * fbm(vec2<f32>(c.x, 1.0), 0.08, 3u, 52u));
            let rock = shaded(ROCK, shade);
            rgb = rock;
            let tree = smoothstep(tree_y - 0.012, tree_y + 0.012, c.y);
            rgb = mix(rgb, forest_color(c, depth, shade), tree);
            if (rise > 0.3 * P.frame.w) {
                let snow = 1.0 - smoothstep(snow_y - 0.01, snow_y + 0.01, c.y);
                rgb = mix(rgb, shaded(SNOW, shade), snow);
            }
        }
        case R_MID_RIDGE: {
            rgb = mixed_forest(c, depth, shade, layers[3u * l + 1u]);
        }
        case R_NEAR_RIDGE: {
            // Mixed forest with a few soft meadow clearings.
            let clearing = smoothstep(0.68, 0.8, fbm(c, 0.07, 3u, 61u));
            rgb = mix(mixed_forest(c, depth, shade, layers[3u * l + 1u]),
                      meadow_color(c, depth, shade), 0.7 * clearing);
        }
        case R_WOODLAND: {
            var plant = u32(layers[3u * l + 2u].y);
            if (plant == 0u) {
                plant = stand_plant(c, depth);
            }
            rgb = plant_color(plant, c, depth, shade, layers[3u * l + 1u]);
        }
        case R_WATER: {
            let t = clamp((c.y - horizon) / max(P.frame.w - horizon, 1e-3), 0.0, 1.0);
            rgb = mix(col(WATER_FAR), col(WATER_DEEP), pow(t, 0.55));
            // Horizontal ripples, finer toward the horizon.
            let ripple = fbm(vec2<f32>(c.x * 0.12, c.y), 0.012 * (0.4 + t), 3u, 71u);
            rgb = mix(rgb, col(WATER_SHEEN), 0.35 * smoothstep(0.62, 0.8, ripple));
            rgb *= 0.8 + 0.4 * shade;
        }
        case R_SHORE: {
            if (depth > 0.45) {
                rgb = shaded(SAND, shade);
            } else {
                let sand = smoothstep(0.7, 0.76, fbm(c, 0.05, 3u, 81u));
                rgb = mix(meadow_color(c, depth, shade), shaded(SAND, shade), sand);
            }
        }
        case R_ROCK: {
            // Stronger plane contrast: rocks are the solid forms up front.
            rgb = shaded(ROCK, clamp((shade - 0.5) * 1.8 + 0.5, 0.0, 1.0));
            // Moss gathers in the shadowed, lower parts of the planes.
            let moss = smoothstep(0.62, 0.72, fbm(c, 0.03 * P.handling.z, 3u, 91u)) * (1.0 - shade);
            rgb = mix(rgb, col(MOSS) * (0.6 + 0.6 * shade), 0.7 * moss);
        }
        default: {
            rgb = shaded(ROCK, shade);
        }
    }

    // Aerial perspective: distance dissolves toward the haze color.
    if (role != R_SKY && role != R_CLOUD) {
        let far = clamp((depth - 0.1) / 0.9, 0.0, 1.0);
        // Distance softens, but the main massif keeps its substance.
        var amt = clamp(P.look.z * far * far * 1.2, 0.0, 0.85);
        if (role == R_MOUNTAIN) {
            amt *= 0.55;
        }
        let haze = mix(col(HAZE), col(SKY_HORIZON), 0.4);
        rgb = mix(rgb, haze, amt);
    }
    // Palette intensity: saturation around luminance, brightness kept.
    let lum = luminance(rgb);
    rgb = max(vec3<f32>(lum) + (rgb - vec3<f32>(lum)) * (0.35 + 1.3 * P.look.y), vec3<f32>(0.0));

    // Wash (transparent glaze over paper) vs gouache (opaque body color).
    // Near forms lean toward gouache, as in the art direction.
    var g = P.handling.y;
    if (role != R_SKY && role != R_CLOUD && depth <= 0.25) {
        g = clamp(g + 0.35, 0.0, 1.0);
    }
    let paper = col(PAPER);
    let density = 0.8 + 0.4 * fbm(c, 0.14, 3u, 101u);
    let wash = paper * pow(clamp(rgb / paper, vec3<f32>(1e-3), vec3<f32>(1.0)), vec3<f32>(density));
    let body = rgb * (0.9 + 0.2 * fbm(c, 0.025 * P.handling.z, 3u, 102u));
    return clamp(mix(wash, body, g), vec3<f32>(0.0), vec3<f32>(1.0));
}

@compute @workgroup_size(8, 8)
fn materials_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.ext_size.x || id.y >= P.ext_size.y) {
        return;
    }
    let c = canvas_of_ext(id.xy);
    let l = front_layer(c);
    var rgb = col(PAPER);
    if (l >= 0) {
        rgb = material(u32(l), c);
    }
    textureStore(field_w, vec2<i32>(id.xy), vec4<f32>(rgb, f32(l)));
}

// ---------------------------------------------------------------- painting

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// 8-tap disc (unit radius), shared by every pixel.
const DISC = array<vec2<f32>, 8>(
    vec2<f32>(0.0, 0.0), vec2<f32>(0.7, 0.1), vec2<f32>(-0.3, 0.65), vec2<f32>(-0.6, -0.4),
    vec2<f32>(0.25, -0.75), vec2<f32>(0.95, -0.3), vec2<f32>(-0.9, 0.3), vec2<f32>(0.4, 0.9)
);

fn load_clamped(t: vec2<i32>) -> vec4<f32> {
    let hi = vec2<i32>(P.ext_size) - vec2<i32>(1);
    return textureLoad(field_r, clamp(t, vec2<i32>(0), hi), 0);
}

@compute @workgroup_size(8, 8)
fn paint_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.interior_size.x || id.y >= P.interior_size.y) {
        return;
    }
    let a = i32(P.apron);
    let t = vec2<i32>(id.xy) + vec2<i32>(a, a);
    let c = canvas_of_ext(vec2<u32>(t));
    let centre = load_clamped(t);
    var depth = 0.5;
    if (centre.a >= 0.0) {
        depth = layers[3u * u32(round(centre.a))].z;
    }
    // Looseness: edge wander and softness grow with distance; near forms
    // stay crisp (selective edges). The radius is in canvas units, so
    // edges look the same at every resolution. 0.02 = MAX_EDGE_BLEED_RADIUS.
    let r_canvas = 0.02 * P.handling.x * (0.15 + 0.85 * depth);
    let r_px = min(r_canvas / P.look.w, f32(a) / 1.4);
    let warp = vec2<f32>(
        fbm(c, 0.035, 3u, 111u) - 0.5,
        fbm(c + vec2<f32>(7.3, 1.9), 0.035, 3u, 112u) - 0.5
    ) * 2.0 * r_px;
    // Offsets are rounded on their own and added to the integer pixel, so
    // the result never depends on the tile's origin (rounding t + warp in
    // f32 would, near half-pixel offsets).
    let base = load_clamped(t + vec2<i32>(round(warp)));
    var sum = vec3<f32>(0.0);
    var differ = 0.0;
    for (var k = 0u; k < 8u; k++) {
        let s = load_clamped(t + vec2<i32>(round(warp + DISC[k] * 0.4 * r_px)));
        sum += s.rgb;
        differ += select(0.0, 1.0, abs(s.a - base.a) > 0.5);
    }
    let edge = differ / 8.0;
    var rgb = mix(base.rgb, sum / 8.0, edge);
    // Watercolor pools pigment at wet edges.
    let pool = edge * (1.0 - edge) * 4.0 * (1.0 - P.handling.y);
    rgb *= 1.0 - 0.12 * pool;

    // Paper tooth and pigment granulation (canvas-space, band-limited).
    let grain = fbm(c, 0.004, 2u, 121u) - 0.5;
    rgb *= 1.0 + 0.14 * P.look.x * grain;
    let gran = fbm(c, 0.0025, 2u, 122u);
    rgb *= 1.0 - 0.18 * P.handling.w * smoothstep(0.55, 0.8, gran) * (1.0 - luminance(rgb));

    let enc = srgb_encode(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(enc, 1.0));
}
