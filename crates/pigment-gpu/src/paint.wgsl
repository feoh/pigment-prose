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
    light: u32, // 0: light from the left, 1: from the right

    // edge_looseness, wash_gouache, mark_scale, granulation
    handling: vec4<f32>,
    // paper_grain, palette intensity, haze, pixel footprint (canvas / px)
    look: vec4<f32>,
    // horizon y, summit y, canvas width, canvas height
    frame: vec4<f32>,
    // union bounding box of the foreground rocks (min > max if none)
    rocks: vec4<f32>,
    // wind over the water (renderer v2): direction x (across), z (into the
    // distance), strength 0–1, unused
    wind: vec4<f32>,
    // water channel rows (channel_rows in paint.rs): first row y, row step,
    // row count, 1 if the scene has water
    channel: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: Params;
// Per layer: (first, count, depth, shade), (bbox), (role, plant, coverage
// axis, 0), (coverage bin base, bin count, 1 / bin span, bbox min v).
@group(0) @binding(1) var<storage, read> layers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> verts: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read> pal: array<vec4<f32>>;
@group(0) @binding(4) var field_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var field_r: texture_2d<f32>;
@group(0) @binding(6) var out_tex: texture_storage_2d<rgba8unorm, write>;
// Coverage index (crates/pigment-gpu/src/coverage.rs): per bin (first
// entry, count); per entry (edge's vertex, right-most x as f32 bits),
// sorted by that x, descending.
@group(0) @binding(7) var<storage, read> cov_bins: array<vec2<u32>>;
@group(0) @binding(8) var<storage, read> cov_entries: array<vec2<u32>>;
// Compositing reference cases (composite_reference_main only).
@group(0) @binding(9) var<storage, read> ref_cases: array<vec4<f32>>;
@group(0) @binding(10) var<storage, read_write> ref_out: array<vec4<f32>>;
// The water's channel per row: left bank, right bank, width, unused.
@group(0) @binding(11) var<storage, read> channel_rows: array<vec4<f32>>;

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

// ---------------------------------------------------------------- compositing
// The reference model is pigment_core::composite (linear light).

const MIN_TRANSMITTANCE = 1e-3;

// A glaze of transmittance t at `density` over `under`: under * t^density.
fn glaze(under: vec3<f32>, t: vec3<f32>, density: f32) -> vec3<f32> {
    return under * pow(clamp(t, vec3<f32>(MIN_TRANSMITTANCE), vec3<f32>(1.0)), vec3<f32>(density));
}

// A wash of `pigment` over `paper` (the pigment's color at density 1).
fn wash(paper: vec3<f32>, pigment: vec3<f32>, density: f32) -> vec3<f32> {
    return glaze(paper, pigment / paper, density);
}

// Premultiplied "over" of an opaque color at coverage a.
fn over(under: vec3<f32>, color: vec3<f32>, a: f32) -> vec3<f32> {
    return mix(under, color, a);
}

// Wash/gouache character relative to the default balance
// (settings::WASH_GOUACHE.default, 0.25; asserted in paint.rs): 0 at the
// default, rising to 1 at pure wash (thinness) or pure gouache (thickness).
const DEFAULT_WASH_GOUACHE = 0.25;
// settings::PAPER_GRAIN.default and GRANULATION.default (asserted in paint.rs).
const DEFAULT_TEXTURE = 0.3;

fn thinness() -> f32 {
    return max(0.0, DEFAULT_WASH_GOUACHE - P.handling.y) / DEFAULT_WASH_GOUACHE;
}

fn thickness() -> f32 {
    return max(0.0, P.handling.y - DEFAULT_WASH_GOUACHE) / (1.0 - DEFAULT_WASH_GOUACHE);
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

// x direction toward the light: -1 when it comes from the left.
fn light_x() -> f32 {
    return select(-1.0, 1.0, P.light == 1u);
}

// Image-plane direction toward the light (up and to its side), as used for
// the planes' shade (lakeshore::face_shade).
fn to_light() -> vec2<f32> {
    return normalize(vec2<f32>(0.66 * light_x(), -0.75));
}

// ---------------------------------------------------------------- coverage

// Parity of the ray's crossings, over only the edges in the pixel's
// coverage bin that reach past it (coverage.rs proves this equals the test
// over every edge). The layer's ray runs along u: +x for row bins, +y for
// column bins (u and v swapped).
fn inside(l: u32, c: vec2<f32>) -> bool {
    let h = layers[4u * l];
    let bb = layers[4u * l + 1u];
    if (c.x < bb.x || c.x > bb.z || c.y < bb.y || c.y > bb.w) {
        return false;
    }
    let first = u32(h.x);
    let last = first + u32(h.y) - 1u;
    let columns = layers[4u * l + 2u].z > 0.5;
    let q = select(c, c.yx, columns);
    let cb = layers[4u * l + 3u];
    let k = u32(clamp(floor((q.y - cb.w) * cb.z), 0.0, cb.y - 1.0));
    let bin = cov_bins[u32(cb.x) + k];
    let stop = q.x - 1e-5; // coverage::BREAK_MARGIN
    var in_poly = false;
    for (var e = bin.x; e < bin.x + bin.y; e++) {
        let ent = cov_entries[e];
        if (bitcast<f32>(ent.y) < stop) {
            break;
        }
        let a = select(verts[ent.x], verts[ent.x].yx, columns);
        let pb = verts[select(ent.x - 1u, last, ent.x == first)];
        let b = select(pb, pb.yx, columns);
        if ((a.y > q.y) != (b.y > q.y)) {
            let u = (b.x - a.x) * (q.y - a.y) / (b.y - a.y) + a.x;
            if (q.x < u) {
                in_poly = !in_poly;
            }
        }
    }
    return in_poly;
}

// Layers are ordered back to front, so the first hit from the front wins.
fn front_layer(c: vec2<f32>) -> i32 {
    for (var l = i32(P.n_layers) - 1; l >= 0; l--) {
        if (inside(u32(l), c)) {
            return l;
        }
    }
    return -1;
}

// The front layer a water surface reflects at `c`: everything but water,
// the near shore, its woods and rocks (rocks throw their own reflections).
fn reflected_layer(c: vec2<f32>) -> i32 {
    for (var l = i32(P.n_layers) - 1; l >= 0; l--) {
        let h = layers[4u * u32(l)];
        let role = u32(layers[4u * u32(l) + 2u].x);
        let near_ground = (role == R_SHORE || role == R_WOODLAND) && h.z <= 0.3;
        if (role == R_WATER || role == R_ROCK || near_ground) {
            continue;
        }
        if (inside(u32(l), c)) {
            return l;
        }
    }
    return -1;
}

// Distance from `c` up to the edge of water layer `l` above it (its local
// shoreline): doubling steps, then bisection. At most 0.5 canvas units.
fn shore_distance(l: u32, c: vec2<f32>) -> f32 {
    var lo = 0.0;
    var hi = 0.004;
    for (var k = 0; k < 8; k++) {
        if (!inside(l, c - vec2<f32>(0.0, hi))) {
            break;
        }
        lo = hi;
        hi *= 2.0;
    }
    for (var k = 0; k < 6; k++) {
        let mid = 0.5 * (lo + hi);
        if (inside(l, c - vec2<f32>(0.0, mid))) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return 0.5 * (lo + hi);
}

fn role_of(l: i32) -> u32 {
    if (l < 0) {
        return 0xffffffffu;
    }
    return u32(layers[4u * u32(l) + 2u].x);
}

// Inside the foreground rocks' bounding box, extended `below` downward.
fn near_rocks(c: vec2<f32>, below: f32) -> bool {
    let r = P.rocks;
    return c.x >= r.x - 0.01 && c.x <= r.z + 0.01 && c.y >= r.y && c.y <= r.w + below;
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
    let lit = clamp(0.55 - 0.75 * rel.y + 0.2 * light_x() * rel.x, 0.0, 1.0);
    return vec2<f32>(crown, lit);
}

// Conifer spires: tall pointed crowns, cells 1.4× as tall as wide. The
// front-most (lowest) spire wins where they overlap. Lit on the light's
// side and toward the tip.
fn spire_field(c: vec2<f32>, size: f32, s: u32) -> vec2<f32> {
    let p = vec2<f32>(c.x / size, c.y / (1.4 * size));
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
            let t = (d.y + 0.75) / 1.5; // 0 at the tip, 1 at the base
            if (t < 0.0 || t > 1.0) {
                continue;
            }
            let half = 0.08 + 0.5 * t;
            let v = 1.0 - abs(d.x) / half;
            if (v > 0.0 && pt.y > best_y) {
                best_y = pt.y;
                crown = min(1.0, 2.5 * v);
                lit = clamp(0.5 + 0.8 * light_x() * d.x / half + 0.3 * (1.0 - t), 0.0, 1.0);
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

// Silhouette height of a plant (matches lakeshore::plant_shape).
fn plant_tall(plant: u32) -> f32 {
    switch plant {
        case P_BROADLEAF: { return 1.15; }
        case P_CONIFER: { return 1.45; }
        case P_BIRCH: { return 1.2; }
        case P_SHRUB: { return 0.45; }
        case P_FLOWERING: { return 0.85; }
        case P_COPPER: { return 1.1; }
        default: { return 1.0; }
    }
}

struct Stands {
    first: u32,
    second: u32,
    // 0 on the boundary between the two nearest stands, 1 well inside.
    inside: f32,
    // Distance past the boundary into the first stand (stand units).
    edge: f32,
    // Crown-size multipliers (young thickets to old growth).
    scale1: f32,
    scale2: f32,
    // Canopy heights (plant height × stand scale).
    height1: f32,
    height2: f32,
    // Unit direction from the first stand's site to the second's.
    toward2: vec2<f32>,
}

// Stand age as a crown-size multiplier, log-uniform 0.62–1.62.
fn stand_scale(cell: vec2<i32>) -> f32 {
    return exp2(-0.7 + 1.4 * hash3(cell.x, cell.y, 406u));
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
    var c1 = vec2<i32>(0);
    var c2 = vec2<i32>(0);
    var p1 = vec2<f32>(0.0);
    var p2 = vec2<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            let pt = vec2<f32>(cell) + vec2<f32>(hash3(cell.x, cell.y, 403u), hash3(cell.x, cell.y, 404u));
            let d = length(p - pt);
            let u = hash3(cell.x, cell.y, 405u);
            if (d < d1) {
                d2 = d1;
                u2 = u1;
                c2 = c1;
                p2 = p1;
                d1 = d;
                u1 = u;
                c1 = cell;
                p1 = pt;
            } else if (d < d2) {
                d2 = d;
                u2 = u;
                c2 = cell;
                p2 = pt;
            }
        }
    }
    let a = pick_hill_plant(u1);
    let b = pick_hill_plant(u2);
    let s1 = stand_scale(c1);
    let s2 = stand_scale(c2);
    let dir = p2 - p1;
    return Stands(a, b, smoothstep(0.0, 0.35, d2 - d1), d2 - d1, s1, s2,
                  plant_tall(a) * s1, plant_tall(b) * s2,
                  dir / max(length(dir), 1e-6));
}

fn stand_plant(c: vec2<f32>, depth: f32) -> u32 {
    return stands(c, depth).first;
}

// One plant's painted canopy: its own crown shape, colors and details.
// `bb` is the layer's bounding box, for trunks under near canopies.
// Crowns at the true size for this depth would shear into streaks where
// depth changes continuously down the ground, so crowns are drawn at the two
// nearest power-of-two sizes and blended (as texture mipmaps are).
fn plant_color(plant: u32, c: vec2<f32>, depth: f32, shade: f32, bb: vec4<f32>, scale: f32) -> vec3<f32> {
    let near = 1.0 - depth;
    let size = (0.003 + 0.03 * near * near) * P.handling.z * scale;
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

// Emergent trees: sparse big crowns standing above the canopy. Returns
// (crown, lit) of the nearest active cell; about one cell in eight is active.
fn emergent_field(c: vec2<f32>, size: f32, s: u32) -> vec2<f32> {
    let p = c / size;
    let i = floor(p);
    var best = 1e9;
    var rel = vec2<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            if (hash3(cell.x, cell.y, s + 3u) > 0.12) {
                continue;
            }
            let pt = vec2<f32>(cell) + vec2<f32>(
                0.25 + 0.5 * hash3(cell.x, cell.y, s),
                0.25 + 0.5 * hash3(cell.x, cell.y, s + 1u)
            );
            let radius = 0.28 + 0.14 * hash3(cell.x, cell.y, s + 2u);
            let q = (p - pt) / radius;
            let d = length(q);
            if (d < best) {
                best = d;
                rel = q;
            }
        }
    }
    let crown = clamp(1.0 - best, 0.0, 1.0);
    let lit = clamp(0.6 - 0.7 * rel.y + 0.3 * light_x() * rel.x, 0.0, 1.0);
    return vec2<f32>(crown, lit);
}

// Forested hills: a patchwork of plant stands of different ages (crown
// sizes) and heights, fading to the generic forest with distance so far
// slopes read through their planes. A taller stand shades its lower
// neighbour on the side away from the light and catches light on its
// exposed edge; emergent trees stand over the canopy with their shadows.
fn mixed_forest(c: vec2<f32>, depth: f32, shade: f32, bb: vec4<f32>) -> vec3<f32> {
    let generic = forest_color(c, depth, shade);
    let amount = smoothstep(0.75, 0.35, depth);
    if (amount <= 0.0) {
        return generic;
    }
    let st = stands(c, depth);
    var rgb = plant_color(st.first, c, depth, shade, vec4<f32>(0.0), st.scale1);
    if (st.second != st.first && st.inside < 1.0) {
        let other = plant_color(st.second, c, depth, shade, vec4<f32>(0.0), st.scale2);
        rgb = mix(other, rgb, 0.5 + 0.5 * st.inside);
    }
    // Height steps between stands (stand units; a stand is 0.14 canvas).
    let facing = dot(st.toward2, to_light());
    let step_h = st.height2 - st.height1;
    let wobble = 0.8 + 0.4 * fbm(c, 0.02, 2u, 407u);
    if (facing > 0.0 && step_h > 0.15) {
        // The taller neighbour stands toward the light: its shadow falls here,
        // darkest at its foot and fading out across the band.
        let band = 0.4 * step_h * facing * wobble;
        let shaded_edge = 1.0 - smoothstep(0.0, band, st.edge);
        let dark = mix(col(FOLIAGE_COOL), col(PLANTS + 2u * (st.first - 1u) + 1u), 0.5) * 0.6;
        rgb = mix(rgb, mix(rgb, dark, 0.5), shaded_edge);
    } else if (facing > 0.0 && step_h < -0.15) {
        // Taller than the neighbour toward the light: an exposed, lit edge.
        let band = 0.12 * (-step_h) * facing * wobble;
        let rim = 1.0 - smoothstep(0.0, band, st.edge);
        rgb = mix(rgb, mix(rgb, col(FOLIAGE_WARM), 0.45) * 1.08, 0.5 * rim * (0.4 + 0.6 * shade));
    }
    // Emergent trees over the canopy (in shrubland the emergent is a lone
    // broadleaf). Drawn at the two nearest power-of-two sizes and blended,
    // like the crowns, so varying ground depth does not shear them.
    var eplant = st.first;
    if (eplant == P_SHRUB) {
        eplant = P_BROADLEAF;
    }
    let near = 1.0 - depth;
    let esize = (0.003 + 0.03 * near * near) * P.handling.z * 1.8;
    let lvl = log2(esize / 0.001);
    let l0 = floor(lvl);
    let s0 = 0.001 * exp2(l0);
    let e0 = emergents(rgb, eplant, c, s0, shade);
    let e1 = emergents(rgb, eplant, c, 2.0 * s0, shade);
    rgb = mix(e0, e1, lvl - l0);
    return mix(generic, rgb, amount);
}

// Emergent trees of `plant` at crown size `size` over the canopy color
// `under`, each with its shadow away from the light. Fades out below a few
// pixels.
fn emergents(under: vec3<f32>, plant: u32, c: vec2<f32>, size: f32, shade: f32) -> vec3<f32> {
    let resolve = smoothstep(3.0, 6.0, size / P.look.w);
    if (resolve <= 0.0) {
        return under;
    }
    let e = emergent_field(c, size, 431u);
    let sh = emergent_field(c + to_light() * 0.4 * size, size, 431u);
    let shadow = smoothstep(0.05, 0.3, sh.x) * (1.0 - smoothstep(0.0, 0.2, e.x));
    var rgb = mix(under, under * 0.62, 0.5 * shadow * resolve);
    let light = col(PLANTS + 2u * (plant - 1u));
    let dark = col(PLANTS + 2u * (plant - 1u) + 1u);
    let lit = smoothstep(0.2, 0.85, e.y) * (0.45 + 0.55 * shade);
    let tree = mix(mix(dark, under, 0.3), mix(light, col(FOLIAGE_WARM), 0.25), lit);
    return mix(rgb, tree, smoothstep(0.04, 0.22, e.x) * resolve);
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

struct Surface {
    rgb: vec3<f32>,
    // Effective depth (ground layers follow the ground's perspective).
    depth: f32,
}

// A layer's color at `c` before palette intensity and wash/gouache, with
// aerial perspective: everything but water (see `water`), which reflects
// these colors.
fn land(l: u32, c: vec2<f32>) -> Surface {
    let h = layers[4u * l];
    let role = u32(layers[4u * l + 2u].x);
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
            let bb = layers[4u * l + 1u];
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
            rgb = mixed_forest(c, depth, shade, layers[4u * l + 1u]);
        }
        case R_NEAR_RIDGE: {
            // Mixed forest with a few soft meadow clearings.
            let clearing = smoothstep(0.68, 0.8, fbm(c, 0.07, 3u, 61u));
            rgb = mix(mixed_forest(c, depth, shade, layers[4u * l + 1u]),
                      meadow_color(c, depth, shade), 0.7 * clearing);
        }
        case R_WOODLAND: {
            var plant = u32(layers[4u * l + 2u].y);
            if (plant == 0u) {
                plant = stand_plant(c, depth);
            }
            // Each stand has its own age (crown size).
            let age = exp2(-0.45 + 0.9 * hash3(i32(l), 0, 409u));
            rgb = plant_color(plant, c, depth, shade, layers[4u * l + 1u], age);
            rgb = grass_over_base(rgb, l, c, depth, shade);
        }
        case R_WATER: {
            rgb = col(WATER_FAR); // painted by `water`
        }
        case R_SHORE: {
            if (depth > 0.45) {
                rgb = shaded(SAND, shade);
            } else {
                let sand = smoothstep(0.7, 0.76, fbm(c, 0.05, 3u, 81u));
                rgb = mix(meadow_color(c, depth, shade), shaded(SAND, shade), sand);
                rgb *= 1.0 - 0.4 * rock_contact(c);
            }
        }
        case R_ROCK: {
            // Stronger plane contrast: rocks are the solid forms up front.
            rgb = shaded(ROCK, clamp((shade - 0.5) * 1.8 + 0.5, 0.0, 1.0));
            rgb = rock_surface(rgb, c, shade);
            rgb = mossy(rgb, c, shade);
            rgb = rock_edges(rgb, c);
        }
        default: {
            rgb = shaded(ROCK, shade);
        }
    }

    return Surface(aerial(rgb, role, depth), depth);
}

// Aerial perspective: distance dissolves toward the haze color.
fn aerial(rgb: vec3<f32>, role: u32, depth: f32) -> vec3<f32> {
    if (role == R_SKY || role == R_CLOUD) {
        return rgb;
    }
    let far = clamp((depth - 0.1) / 0.9, 0.0, 1.0);
    // Distance softens, but the main massif keeps its substance.
    var amt = clamp(P.look.z * far * far * 1.2, 0.0, 0.85);
    if (role == R_MOUNTAIN) {
        amt *= 0.55;
    }
    let haze = mix(col(HAZE), col(SKY_HORIZON), 0.4);
    return mix(rgb, haze, amt);
}

// Meadow grass growing up over the base of a near stand, so it stands in
// the meadow rather than on a ruled line: ragged blades 0.003–0.011 canvas
// units tall (the point that far below is outside the stand).
fn grass_over_base(rgb: vec3<f32>, l: u32, c: vec2<f32>, depth: f32, shade: f32) -> vec3<f32> {
    if (depth > 0.3) {
        return rgb;
    }
    let blades = fbm(vec2<f32>(c.x * 3.0, c.y * 0.5), 0.006, 3u, 311u);
    let tall = 0.003 + 0.008 * smoothstep(0.3, 0.8, blades);
    if (inside(l, c + vec2<f32>(0.0, tall))) {
        return rgb;
    }
    let resolve = smoothstep(1.0, 2.5, tall / P.look.w);
    return mix(rgb, meadow_color(c, depth, shade), 0.9 * resolve);
}

// Fractures on a rock: a few long, wandering lines, not a net. The ridge of
// an fbm stretched along the stone's grain (`along`), kept only near its
// crest and only where a patchy mask allows, so cracks come in groups.
// Returns 0–1 crack strength before band-limiting.
fn fractures(c: vec2<f32>, along: vec2<f32>, size: f32, s: u32) -> f32 {
    let q = vec2<f32>(dot(c, along), dot(c, vec2<f32>(-along.y, along.x)));
    let n = fbm(vec2<f32>(q.x * 0.35, q.y), size, 3u, s);
    let ridge = 1.0 - abs(2.0 * n - 1.0);
    let group = smoothstep(0.5, 0.68, fbm(c, 6.0 * size, 2u, s + 3u));
    return smoothstep(0.93, 0.985, ridge) * group;
}

// Small stone facets: a cellular field of flat chips, stretched along the
// grain. Returns (tilt, crevice, lip): each chip's tilt toward (+) or away
// from (-) the light, -1 to 1; a dark crevice along the chip edges turned
// away from the light, and a lit lip along those turned toward it, 0 to 1
// each. `size` in canvas units.
fn rock_facets(c: vec2<f32>, size: f32, grain: vec2<f32>, s: u32) -> vec3<f32> {
    let q = vec2<f32>(dot(c, grain) / 1.7, dot(c, vec2<f32>(-grain.y, grain.x))) / size;
    let i = floor(q);
    var d1 = 1e9;
    var d2 = 1e9;
    var p1 = vec2<f32>(0.0);
    var p2 = vec2<f32>(0.0);
    var h1 = 0.0;
    var c1 = vec2<i32>(0);
    var c2 = vec2<i32>(0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cell = vec2<i32>(i32(i.x) + dx, i32(i.y) + dy);
            let pt = vec2<f32>(cell) + vec2<f32>(hash3(cell.x, cell.y, s), hash3(cell.x, cell.y, s + 1u));
            let d = distance(q, pt);
            if (d < d1) {
                d2 = d1;
                p2 = p1;
                c2 = c1;
                d1 = d;
                p1 = pt;
                c1 = cell;
                h1 = hash3(cell.x, cell.y, s + 2u);
            } else if (d < d2) {
                d2 = d;
                p2 = pt;
                c2 = cell;
            }
        }
    }
    // Distance to the edge between the two nearest chips, in cells.
    let edge = 0.5 * (d2 - d1);
    let px = P.look.w / size;
    // About half the edges between chips are open joints; the rest are
    // only a change of plane.
    let pair = c1 + c2;
    let open = step(hash3(pair.x, pair.y, s + 3u), 0.5);
    let line = open * (1.0 - smoothstep(0.5 * px, 1.6 * px, edge));
    // The edge's outward normal, back in canvas directions.
    let nq = normalize(p2 - p1 + vec2<f32>(1e-6, 0.0));
    let n = normalize(grain * nq.x / 1.7 + vec2<f32>(-grain.y, grain.x) * nq.y);
    let toward = dot(n, to_light());
    return vec3<f32>(2.0 * h1 - 1.0, line * smoothstep(-0.1, -0.5, toward), line * smoothstep(0.1, 0.5, toward));
}

// A rock's surface (renderer v2, task 25: "their surfaces will need to be
// less chonky geometric and more complex"; round 7: "you didn't ... add
// more complex surfaces"). Over the generator's planes: warm and cool
// staining, two scales of flat chips, each tilted to or from the light,
// with dark crevices and lit lips along their edges; tilted strata; rain
// streaks down the shaded faces; grouped cracks; dark and pale flecks in
// the grain; and pale lichen rosettes on the lit, dry faces. Everything
// finer than a couple of pixels fades out.
fn rock_surface(rgb: vec3<f32>, c: vec2<f32>, shade: f32) -> vec3<f32> {
    let s = P.handling.z;
    var out = rgb;
    // Staining: cooler grey in places, warmer iron in others.
    let grey = vec3<f32>(luminance(out));
    let cool = mix(out, grey * vec3<f32>(0.9, 0.97, 1.08), 0.7);
    let warm = out * vec3<f32>(1.08, 0.98, 0.84);
    let stain = fbm(c + vec2<f32>(5.3, 1.9), 0.06 * s, 3u, 129u);
    out = mix(out, cool, 0.6 * smoothstep(0.45, 0.7, stain));
    out = mix(out, warm, 0.5 * smoothstep(0.55, 0.3, stain));
    out *= 0.92 + 0.16 * fbm(c, 0.003 * s, 2u, 121u);
    let tilt = normalize(vec2<f32>(0.35 * light_x(), 1.0));
    let grain = tilt.yx * vec2<f32>(1.0, -1.0);
    // Facets at three scales: broad planes within the generator's planes
    // (mostly a change of plane, few open joints), then chips, then small
    // chips within them.
    let lit = 1.0 - smoothstep(0.1, 0.9, shade) * 0.5;
    let sizes = array<f32, 3>(0.03, 0.012, 0.0055);
    let tilts = array<f32, 3>(0.24, 0.14, 0.09);
    let joints = array<f32, 3>(0.35, 1.0, 0.65);
    for (var o = 0u; o < 3u; o++) {
        let size = sizes[o] * s;
        let resolve = smoothstep(3.0, 7.0, size / P.look.w);
        if (resolve > 0.0) {
            let f = rock_facets(c, size, grain, 140u + 7u * o);
            out *= 1.0 + tilts[o] * resolve * f.x;
            let j = joints[o] * resolve;
            out = mix(out, out * 0.5, 0.6 * j * f.y);
            out = mix(out, mix(out, col(SUN), 0.3) * 1.15, 0.45 * j * f.z * lit);
        }
    }
    let strata = fbm(vec2<f32>(dot(c, tilt), dot(c, vec2<f32>(-tilt.y, tilt.x)) * 0.08), 0.008 * s, 3u, 122u);
    out *= 0.88 + 0.24 * strata;
    let streak = fbm(vec2<f32>(c.x * 6.0, c.y * 0.6), 0.03 * s, 3u, 123u);
    out *= 1.0 - 0.2 * smoothstep(0.5, 0.78, streak) * (1.1 - shade);
    // Cracks: thin enough to need about 2 px before they show.
    let resolve = smoothstep(1.2, 2.6, 0.0012 * s / P.look.w);
    if (resolve > 0.0) {
        let crack = fractures(c, grain, 0.02 * s, 124u) * resolve;
        // A lit lip on the crack's side toward the light: a cut, not a line.
        let lip = fractures(c - to_light() * 0.0014 * s, grain, 0.02 * s, 124u) * resolve;
        out = mix(out, mix(out, col(SUN), 0.25) * 1.12, 0.5 * max(lip - crack, 0.0));
        out = mix(out, out * 0.38, 0.9 * crack);
    }
    // Flecks in the grain: dark and pale, about 2 px and up.
    let fleck_res = smoothstep(1.5, 3.0, 0.0022 * s / P.look.w);
    if (fleck_res > 0.0) {
        let n = vnoise(c / (0.0022 * s), 131u);
        out *= 1.0 - 0.3 * fleck_res * smoothstep(0.78, 0.9, n);
        out = mix(out, out * 1.25 + 0.03, 0.4 * fleck_res * smoothstep(0.24, 0.12, n));
    }
    let rosette = crown_field(c, 0.0045 * s, 126u);
    let lichen = smoothstep(0.55, 0.75, fbm(c, 0.02 * s, 3u, 127u)) * smoothstep(0.42, 0.75, shade)
        * smoothstep(0.2, 0.55, rosette.x) * smoothstep(1.5, 3.0, 0.0045 * s / P.look.w);
    let pale = mix(col(ROCK), col(MOSS), 0.3) * 1.2;
    let ochre = mix(col(SUN), col(SAND), 0.55) * 0.95;
    let tint = mix(pale, ochre, smoothstep(0.55, 0.7, fbm(c, 0.05, 2u, 128u)));
    return mix(out, tint, 0.65 * lichen);
}

// Moss on a rock: soft cushions that follow the form rather than patches
// with ruled edges. It gathers on the shadowed planes, at the rock's foot
// and along its top, fades out through a feathered fringe of speckles,
// and is tufted from yellow-green where lit to deep green in its hollows.
fn mossy(rgb: vec3<f32>, c: vec2<f32>, shade: f32) -> vec3<f32> {
    let s = P.handling.z;
    // Where moisture keeps moss: shadowed planes, the foot, the top.
    let foot = select(0.0, 1.0, role_of(front_layer(c + vec2<f32>(0.0, 0.012))) != R_ROCK);
    let top = select(0.0, 1.0, role_of(front_layer(c - vec2<f32>(0.0, 0.008))) != R_ROCK);
    let damp = 0.45 * (1.0 - shade) + 0.3 * foot + 0.2 * top;
    // Warped, multi-scale cover with a fine, fibrous fringe.
    let warp = vec2<f32>(fbm(c, 0.03 * s, 2u, 92u), fbm(c + vec2<f32>(3.1, 7.7), 0.03 * s, 2u, 93u)) - 0.5;
    let broad = fbm(c + 0.02 * s * warp, 0.035 * s, 4u, 91u);
    let fringe = fbm(c, 0.004 * s, 2u, 94u) - 0.5;
    let cover = smoothstep(0.3, 0.65, broad + 0.7 * damp - 0.27 + 0.3 * fringe);
    // Tufts: bright on their lit tops, deep in between.
    let tuft = crown_field(c, 0.0035 * s, 95u);
    let tuft_res = smoothstep(2.0, 4.0, 0.0035 * s / P.look.w);
    let bright = mix(col(MOSS), col(FOLIAGE_WARM), 0.45) * (0.85 + 0.35 * shade);
    let deep = mix(col(MOSS), col(FOLIAGE_COOL), 0.35) * 0.55;
    let moss = mix(mix(deep, bright, 0.55), mix(deep, bright, smoothstep(0.2, 0.9, tuft.y) * tuft.x + 0.2), tuft_res);
    return mix(rgb, moss, 0.9 * cover);
}

// Selective accents on a rock (gouache): a lit rim where its edge faces the
// light and a dark line on the edge turned away. Widths in canvas units,
// fading out below about 1.5 px.
fn rock_edges(rgb: vec3<f32>, c: vec2<f32>) -> vec3<f32> {
    let w = 0.003;
    let resolve = smoothstep(1.0, 2.0, w / P.look.w);
    if (resolve <= 0.0) {
        return rgb;
    }
    var out = rgb;
    if (role_of(front_layer(c + to_light() * w)) != R_ROCK) {
        let rim = mix(shaded(ROCK, 1.0), col(SUN), 0.35) * 1.08;
        out = mix(out, rim, 0.6 * resolve);
    } else if (role_of(front_layer(c - to_light() * w)) != R_ROCK) {
        out = mix(out, out * 0.5, 0.5 * resolve);
    }
    return out;
}

// How close a rock stands above `c`: probes `n` steps of `step` canvas
// units upward; 1 at a rock's foot, fading with distance, 0 if none.
fn rock_above(c: vec2<f32>, step: f32, n: i32) -> f32 {
    if (!near_rocks(c, step * f32(n))) {
        return 0.0;
    }
    for (var k = 1; k <= n; k++) {
        if (role_of(front_layer(c - vec2<f32>(0.0, step * f32(k)))) == R_ROCK) {
            return 1.0 - f32(k - 1) / f32(n);
        }
    }
    return 0.0;
}

// A rock's contact shadow on the ground at its foot (about 0.012 canvas).
fn rock_contact(c: vec2<f32>) -> f32 {
    return rock_above(c, 0.004, 3);
}

// ---------------------------------------------------------------- water (v2)

// A water point on the ground plane: perspective from the horizon, so
// marks set in ground units shrink and flatten with distance (ripples
// "shorten with distance"). `fp` is a pixel's footprint in ground units
// (the vertical one, the larger), for band-limiting.
struct Ground {
    g: vec2<f32>,
    fp: f32,
}

// One ground unit is one canvas unit of depth at a canvas unit below the
// horizon. Near the viewer (0.4 below the horizon) a ground wavelength W
// spans W × 0.16 canvas units vertically and W × 0.4 horizontally.
const GROUND_SCALE = 1.0;

fn ground(c: vec2<f32>) -> Ground {
    let dy = max(c.y - P.frame.x, 0.004);
    let g = vec2<f32>((c.x - 0.5 * P.frame.z) / dy, 1.0 / dy) * GROUND_SCALE;
    return Ground(g, P.look.w * GROUND_SCALE / (dy * dy));
}

// fbm over ground coordinates, band-limited by the ground footprint `fp`.
fn gfbm(g: vec2<f32>, wl0: f32, octaves: u32, s: u32, fp: f32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amp = 1.0;
    var wl = wl0;
    for (var o = 0u; o < octaves; o++) {
        let keep = smoothstep(1.5, 3.0, wl / fp);
        sum += amp * mix(0.5, vnoise(g / wl, s + o * 101u), keep);
        norm += amp;
        amp *= 0.55;
        wl *= 0.5;
    }
    return sum / norm;
}

// The river's current at `c`: (lines, speed). The channel's banks come from
// the scene row by row; a narrow channel runs fast (and fastest mid-stream),
// a wide lake barely moves. Lines are thin ridges drawn along the channel,
// so they follow its bends and crowd where it narrows past a spur.
fn current(c: vec2<f32>, gr: Ground) -> vec2<f32> {
    let d = P.channel;
    if (d.w < 0.5) {
        return vec2<f32>(0.0);
    }
    let fk = (c.y - d.x) / d.y;
    if (fk < 0.0 || fk > d.z - 1.0) {
        return vec2<f32>(0.0);
    }
    let k0 = u32(floor(fk));
    let k1 = min(k0 + 1u, u32(d.z) - 1u);
    let r = mix(channel_rows[k0], channel_rows[k1], fract(fk));
    if (r.z <= 1e-4) {
        return vec2<f32>(0.0);
    }
    let u = clamp((c.x - r.x) / r.z, 0.0, 1.0);
    let narrow = clamp(1.3 - 1.8 * r.z / P.frame.z, 0.0, 1.0);
    let speed = narrow * (0.35 + 0.65 * 4.0 * u * (1.0 - u));
    // Across the channel in ground units, and along it (ground depth).
    let across = u * r.z / max(c.y - P.frame.x, 0.004) * GROUND_SCALE;
    let ridge = 1.0 - abs(2.0 * gfbm(vec2<f32>(across, gr.g.y * 0.06), 0.08, 3u, 181u, gr.fp) - 1.0);
    let lines = smoothstep(0.88, 0.975, ridge);
    return vec2<f32>(lines, speed);
}

// Water: sky-lit toward the far shore and deep near the viewer, with
// simplified reflections of the far shore and everything beyond it (mirrored
// about the horizon, rippled, fading with distance below it), dark
// reflections under rocks and horizontal ripples on top. Reflections are
// evaluated from the scene at the mirrored point, not read from neighbouring
// pixels, so they need no tile halo.
fn water(l: u32, c: vec2<f32>) -> Surface {
    let h = layers[4u * l];
    let depth = h.z;
    let shade = h.w;
    let horizon = P.frame.x;
    let t = clamp((c.y - horizon) / max(P.frame.w - horizon, 1e-3), 0.0, 1.0);
    var rgb = mix(col(WATER_FAR), col(WATER_DEEP), pow(t, 0.55));

    // Renderer v2 (task 25): the wind and the current. Marks are set on the
    // ground plane (perspective-correct, band-limited), in the wind's frame:
    // `q.x` across the wind, `q.y` along it.
    let gr = ground(c);
    let wd = normalize(P.wind.xy + vec2<f32>(1e-6, 0.0));
    let q = vec2<f32>(dot(gr.g, vec2<f32>(-wd.y, wd.x)), dot(gr.g, wd));
    let strength = P.wind.z;
    // Wind lanes: long streaks along the wind, rough (matte, dark) between
    // slick (glassy, bright) ones.
    let lanes = gfbm(vec2<f32>(q.x, q.y * 0.15), 0.35, 3u, 171u, gr.fp);
    let lane = smoothstep(0.4, 0.66, lanes);
    // Cat's-paws: patches where a gust touches down and breaks the mirror.
    let paw = smoothstep(0.6, 0.75, gfbm(vec2<f32>(q.x * 0.8, q.y * 0.55), 0.8, 3u, 172u, gr.fp));
    let rough = clamp(strength * (0.25 + 0.75 * lane) + 1.2 * strength * paw, 0.0, 1.0);
    // Ripples: crests across the wind at every scale from 0.4 ground units
    // down; each distance keeps the octaves it can resolve, so ripples are
    // finer and flatter toward the far shore by construction. Rough water
    // ripples harder.
    let ripple = gfbm(vec2<f32>(q.x * 0.18, q.y), 0.4, 6u, 173u, gr.fp);
    let cur = current(c, gr);

    // Mirror about the local shoreline above the pixel, broken by ripples,
    // rough lanes and cat's-paws; calm, slick water keeps a crisp mirror.
    let below = shore_distance(l, c);
    let rise = max(horizon - P.frame.y, 0.05);
    var fade = 1.0 - smoothstep(0.0, 1.15 * rise, below);
    fade *= 1.0 - 0.85 * strength * paw - 0.3 * strength * lane;
    if (fade > 0.0) {
        let wob = (ripple - 0.5) * (0.4 + 1.6 * rough) + (cur.x - 0.5) * 0.6 * cur.y;
        let m = vec2<f32>(c.x + wob * 0.02 * (0.2 + below / rise), c.y - 2.0 * below);
        let lr = reflected_layer(m);
        if (lr >= 0) {
            let refl = mix(land(u32(lr), m).rgb, col(WATER_FAR), 0.25) * 0.85;
            rgb = mix(rgb, refl, 0.75 * fade);
        }
    }
    // Rough water is matte and darker (it shows the deep water, not the
    // sky); slick lanes catch the sky on their ripples.
    rgb = mix(rgb, mix(rgb, col(WATER_DEEP), 0.35) * 0.92, 0.6 * rough);
    // Crests broken into dashes, and a fine chop of short marks everywhere,
    // busier where the wind roughens the water.
    let dashes = smoothstep(0.38, 0.62, gfbm(vec2<f32>(q.x * 0.7, q.y * 1.4), 0.12, 3u, 176u, gr.fp));
    let crest = smoothstep(0.56, 0.74, ripple) * dashes;
    let trough = 1.0 - smoothstep(0.26, 0.44, ripple);
    rgb = mix(rgb, col(WATER_SHEEN), (0.28 + 0.2 * (1.0 - rough)) * crest);
    rgb *= 1.0 - (0.08 + 0.1 * rough) * trough;
    let chop = gfbm(vec2<f32>(q.x * 0.45, q.y), 0.06, 4u, 177u, gr.fp);
    let chop_amt = 0.35 + 0.65 * rough;
    rgb = mix(rgb, col(WATER_SHEEN), 0.22 * chop_amt * smoothstep(0.62, 0.78, chop));
    rgb *= 1.0 - 0.07 * chop_amt * (1.0 - smoothstep(0.22, 0.4, chop));
    // Fine glitter where gusts roughen the surface.
    let glint = smoothstep(0.74, 0.88, gfbm(vec2<f32>(q.x * 0.25, q.y), 0.03, 2u, 175u, gr.fp));
    rgb = mix(rgb, col(WATER_SHEEN), 0.35 * glint * rough);
    // The current: pale lines along the channel over slightly darker water.
    rgb = mix(rgb * (1.0 - 0.07 * cur.y), col(WATER_SHEEN), 0.42 * cur.x * cur.y);
    rgb *= 0.8 + 0.4 * shade;
    // Rocks standing in the water throw dark, rippled reflections and wear
    // a broken ring of foam where the water meets them.
    let jiggle = (fbm(c, 0.01, 2u, 75u) - 0.5) * 0.006 * (1.0 + rough);
    let under = rock_above(c + vec2<f32>(jiggle, 0.0), 0.006, 5);
    rgb = mix(rgb, mix(shaded(ROCK, 0.1), col(WATER_DEEP), 0.4) * 0.6, 0.65 * under);
    let contact = rock_above(c, 0.0025, 2);
    let foam = contact * smoothstep(0.45, 0.72, fbm(c, 0.003, 2u, 76u)) * (0.4 + 0.6 * max(rough, cur.y));
    rgb = mix(rgb, col(WATER_SHEEN), 0.55 * foam);
    return Surface(aerial(rgb, R_WATER, depth), depth);
}

fn material(l: u32, c: vec2<f32>) -> vec3<f32> {
    let role = u32(layers[4u * l + 2u].x);
    var sf: Surface;
    if (role == R_WATER) {
        sf = water(l, c);
    } else {
        sf = land(l, c);
    }
    var rgb = sf.rgb;
    let depth = sf.depth;
    // Palette intensity: saturation around luminance, brightness kept.
    let lum = luminance(rgb);
    rgb = max(vec3<f32>(lum) + (rgb - vec3<f32>(lum)) * (0.35 + 1.3 * P.look.y), vec3<f32>(0.0));

    // Wash (transparent glaze over paper) vs gouache (opaque body color).
    // Near forms lean toward gouache, as in the art direction.
    var g = P.handling.y;
    if (role != R_SKY && role != R_CLOUD && depth <= 0.25) {
        g = clamp(g + 0.35, 0.0, 1.0);
    }
    // At the default balance this is the approved look. Thinner handling
    // mottles the wash more (blooms); thicker handling strengthens brush
    // marks and lifts the body color toward a chalky, matte opacity.
    let thin = thinness();
    let thick = thickness();
    if (thin == 0.0 && thick == 0.0) {
        let density = 0.8 + 0.4 * fbm(c, 0.14, 3u, 101u);
        let body = rgb * (0.9 + 0.2 * fbm(c, 0.025 * P.handling.z, 3u, 102u));
        return clamp(over(wash(col(PAPER), rgb, density), body, g), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let density = 1.0 + (fbm(c, 0.14, 3u, 101u) - 0.5) * 0.4 * (1.0 + 1.5 * thin);
    var body = rgb * (1.0 + (fbm(c, 0.025 * P.handling.z, 3u, 102u) - 0.5) * 0.2 * (1.0 + thick));
    body = mix(body, col(PAPER), 0.1 * thick);
    return clamp(over(wash(col(PAPER), rgb, density), body, g), vec3<f32>(0.0), vec3<f32>(1.0));
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
        depth = layers[4u * u32(round(centre.a))].z;
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
    // The disc's mean is the edge blend of opaque samples; it replaces the
    // warped sample in proportion to how many taps crossed a boundary.
    var rgb = over(base.rgb, sum / 8.0, edge);
    // Watercolor pools pigment at wet edges (where two washes overlap).
    // Paper tooth and pigment granulation (canvas-space, band-limited).
    // Both show through thin washes and are covered by opaque gouache. The
    // default balance keeps the approved expressions exactly (a uniform
    // branch, so no divergence).
    let pool = edge * (1.0 - edge) * 4.0 * (1.0 - P.handling.y);
    let grain = fbm(c, 0.004, 2u, 121u) - 0.5;
    let gran = fbm(c, 0.0025, 2u, 122u);
    let thin = thinness();
    let thick = thickness();
    // Above their defaults (0.3), paper grain and granulation ramp up to a
    // clearly rough tooth and strongly settled pigment.
    let rough = max(0.0, P.look.x - DEFAULT_TEXTURE) / (1.0 - DEFAULT_TEXTURE);
    let settled = max(0.0, P.handling.w - DEFAULT_TEXTURE) / (1.0 - DEFAULT_TEXTURE);
    if (thin == 0.0 && thick == 0.0 && rough == 0.0 && settled == 0.0) {
        rgb *= 1.0 - 0.12 * pool;
        rgb *= 1.0 + 0.14 * P.look.x * grain;
        rgb *= 1.0 - 0.18 * P.handling.w * smoothstep(0.55, 0.8, gran) * (1.0 - luminance(rgb));
    } else {
        rgb *= 1.0 - 0.12 * (1.0 + thin) * pool;
        let tooth = (0.14 * P.look.x + 0.36 * rough) * (1.0 + 0.6 * thin) * (1.0 - 0.6 * thick);
        rgb *= 1.0 + tooth * grain;
        let settle = (0.18 * P.handling.w + 0.42 * settled) * (1.0 + 0.8 * thin) * (1.0 - 0.8 * thick);
        rgb *= 1.0 - settle * smoothstep(0.55, 0.8, gran) * (1.0 - luminance(rgb));
    }

    let enc = srgb_encode(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(enc, 1.0));
}

// ---------------------------------------------------------------- reference

// One compositing case per invocation: (under.rgb, op), (color.rgb, amount).
// op 0: glaze(under, color as transmittance, amount); 1: over(under, color,
// amount); 2: wash(under as paper, color, amount). Checked against
// pigment_core::composite by the hardware suite.
@compute @workgroup_size(64)
fn composite_reference_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= arrayLength(&ref_out)) {
        return;
    }
    let a = ref_cases[2u * i];
    let b = ref_cases[2u * i + 1u];
    var r = vec3<f32>(0.0);
    switch u32(a.w) {
        case 0u: { r = glaze(a.xyz, b.xyz, b.w); }
        case 1u: { r = over(a.xyz, b.xyz, b.w); }
        default: { r = wash(a.xyz, b.xyz, b.w); }
    }
    ref_out[i] = vec4<f32>(r, 0.0);
}
