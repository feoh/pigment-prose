// Scene debug views (task 05): flat values or role colors with region
// outlines. Diagnostic only; not the painting renderer.
//
// Same contracts as smoke.wgsl: whole-image pixel coordinates, canvas units
// with the short side = 1, even-odd coverage with the rule shared by the CPU
// reference rasterizer (pigment_core::scene::raster), sRGB encoded once.

struct Params {
    ext_origin: vec2<i32>,
    ext_size: vec2<u32>,
    interior_size: vec2<u32>,
    canvas_scale: vec2<f32>,
    apron: u32,
    n_layers: u32,
    view: u32, // 0 = flat values, 1 = regions, 2 = layer ids
    _pad: u32,
}

// Per layer, three vec4s:
//   (first vertex, vertex count, depth, shade)
//   (bbox min x, min y, max x, max y)
//   (role, 0, 0, 0)
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> layers: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> verts: array<vec2<f32>>;
@group(0) @binding(3) var field_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var field_r: texture_2d<f32>;
@group(0) @binding(5) var out_tex: texture_storage_2d<rgba8unorm, write>;

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

fn canvas_of(ext_texel: vec2<u32>) -> vec2<f32> {
    let gpx = P.ext_origin + vec2<i32>(ext_texel);
    return (vec2<f32>(gpx) + 0.5) * P.canvas_scale;
}

// Pass 1: index of the front-most covering layer (-1 for none), over the
// extended region.
@compute @workgroup_size(8, 8)
fn field_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.ext_size.x || id.y >= P.ext_size.y) {
        return;
    }
    let c = canvas_of(id.xy);
    var front = -1.0;
    for (var l = 0u; l < P.n_layers; l++) {
        if (inside(l, c)) {
            front = f32(l);
        }
    }
    textureStore(field_w, vec2<i32>(id.xy), vec4<f32>(front, 0.0, 0.0, 1.0));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Debug-only base values (linear) and colors per role; not a palette.
fn role_value(role: u32) -> f32 {
    switch role {
        case 0u: { return 0.80; }  // sky
        case 1u: { return 0.55; }  // far ridge
        case 2u: { return 0.34; }  // mid ridge (foothills)
        case 3u: { return 0.26; }  // near ridge (framing)
        case 4u: { return 0.50; }  // water
        case 5u: { return 0.60; }  // shore
        case 6u: { return 0.10; }  // woodland
        case 7u: { return 0.40; }  // foreground rock
        case 9u: { return 0.70; }  // cloud
        case 10u: { return 0.46; } // mesa
        case 11u: { return 0.38; } // tundra ground
        default: { return 0.45; }  // mountain
    }
}

fn role_color(role: u32) -> vec3<f32> {
    switch role {
        case 0u: { return vec3<f32>(0.62, 0.74, 0.90); }
        case 1u: { return vec3<f32>(0.45, 0.40, 0.62); }
        case 2u: { return vec3<f32>(0.40, 0.46, 0.22); }
        case 3u: { return vec3<f32>(0.46, 0.30, 0.18); }
        case 4u: { return vec3<f32>(0.10, 0.28, 0.60); }
        case 5u: { return vec3<f32>(0.70, 0.58, 0.36); }
        case 6u: { return vec3<f32>(0.04, 0.26, 0.06); }
        case 7u: { return vec3<f32>(0.78, 0.36, 0.10); }
        case 9u: { return vec3<f32>(0.90, 0.90, 0.86); }
        case 10u: { return vec3<f32>(0.65, 0.38, 0.20); }
        case 11u: { return vec3<f32>(0.48, 0.54, 0.34); }
        default: { return vec3<f32>(0.52, 0.52, 0.56); }
    }
}

fn layer_at(t: vec2<i32>) -> i32 {
    return i32(round(textureLoad(field_r, t, 0).r));
}

// Pass 2: color the interior; in the regions view, outline every boundary
// between different front-most layers (reads the 1-pixel apron).
@compute @workgroup_size(8, 8)
fn composite_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= P.interior_size.x || id.y >= P.interior_size.y) {
        return;
    }
    let a = i32(P.apron);
    let t = vec2<i32>(id.xy) + vec2<i32>(a, a);
    let l = layer_at(t);
    if (P.view == 2u) {
        // Raw index + 1 (0 = uncovered), not sRGB encoded: exact bytes.
        textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(f32(l + 1) / 255.0, 0.0, 0.0, 1.0));
        return;
    }
    var rgb = vec3<f32>(1.0, 0.0, 1.0); // uncovered: magenta, never expected
    if (l >= 0) {
        let h = layers[3u * u32(l)];
        let role = u32(layers[3u * u32(l) + 2u].x);
        let depth = h.z;
        let lit = 0.4 + 1.2 * h.w;
        if (P.view == 0u) {
            var v = role_value(role) * lit;
            if (role != 0u && role != 9u) {
                v = mix(v, 0.78, 0.45 * depth);
            }
            rgb = vec3<f32>(v);
        } else {
            rgb = role_color(role) * lit;
            if (a > 0) {
                let edge = layer_at(t + vec2<i32>(1, 0)) != l
                    || layer_at(t - vec2<i32>(1, 0)) != l
                    || layer_at(t + vec2<i32>(0, 1)) != l
                    || layer_at(t - vec2<i32>(0, 1)) != l;
                if (edge) {
                    rgb = vec3<f32>(0.01);
                }
            }
        }
    }
    let enc = srgb_encode(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(enc, 1.0));
}
