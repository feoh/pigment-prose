// Renderer v5: paint-space relief and participating cloud volumes.
// Appended to paint.wgsl. All queries use whole-canvas coordinates, never
// tile-local coordinates or screen derivatives. No additional pixel halo.

@group(0) @binding(12) var<storage, read> relief_envelopes: array<vec4<f32>>;

fn mass_owner(l: u32) -> u32 {
    return u32(relief_envelopes[2u * l + 1u].z);
}

fn mass_bounds(l: u32) -> vec4<f32> {
    return relief_envelopes[2u * l];
}

fn mass_span(l: u32, x: f32) -> vec2<f32> {
    let b = mass_bounds(l);
    let d = relief_envelopes[2u * l + 1u];
    if (d.y < 2.0) { return b.yw; }
    let f = clamp((x - b.x) * d.w, 0.0, d.y - 1.0);
    let k = u32(floor(f));
    let k1 = min(k + 1u, u32(d.y) - 1u);
    return mix(relief_envelopes[u32(d.x) + k].xy,
               relief_envelopes[u32(d.x) + k1].xy, fract(f));
}

// Rotated, domain-warped ridged multifractal. Coarse ridges control the
// strength of their children: exposed spurs split into smaller ribs and
// gullies, rather than wearing an unrelated uniform noise texture.
fn lithic_height(c: vec2<f32>, scale: f32, sedimentary: bool) -> f32 {
    var q = c / scale;
    let warp = vec2<f32>(vnoise(q * 2.1, 2101u), vnoise(q * 2.1 + 7.3, 2102u)) - 0.5;
    q += 0.38 * warp;
    // Long downslope ribs for granite; tilted beds for sedimentary cliffs.
    let character = hash3(0, 0, 2099u);
    q = select(vec2<f32>(q.x + 0.28 * q.y, q.y * (0.32 + 0.32 * character)),
               vec2<f32>(q.x * 0.42, q.y + 0.16 * q.x), sedimentary);
    q *= 3.4;
    var amplitude = 1.0;
    var sum = 0.0;
    var norm = 0.0;
    var parent = 1.0;
    var wavelength = scale / 3.4;
    for (var o = 0u; o < 9u; o++) {
        let keep = smoothstep(2.0, 4.0, wavelength / P.look.w);
        let n = vnoise(q, 2111u + o * 31u);
        let ridge = 1.0 - abs(2.0 * n - 1.0);
        let signal = ridge * ridge;
        sum += amplitude * mix(0.46, signal * parent, keep);
        norm += amplitude;
        parent = mix(0.7, 1.0, signal);
        // A small alternating rotation avoids aligned lattice grooves,
        // without turning the fall lines into a swirling marble texture.
        q = vec2<f32>(2.03 * q.x + 0.17 * q.y, -0.17 * q.x + 2.03 * q.y) + vec2<f32>(3.7, 1.9);
        amplitude *= 0.52;
        wavelength /= 2.04;
    }
    return scale * 0.24 * sum / norm;
}

struct StoneRelief {
    normal: vec3<f32>,
    visibility: f32,
    hollow: f32,
    height: f32,
}

fn stone_relief(c: vec2<f32>, scale: f32, sedimentary: bool) -> StoneRelief {
    let e = max(P.look.w, scale * 0.0012);
    let h = lithic_height(c, scale, sedimentary);
    let dx = lithic_height(c + vec2<f32>(e, 0.0), scale, sedimentary)
           - lithic_height(c - vec2<f32>(e, 0.0), scale, sedimentary);
    let dy = lithic_height(c + vec2<f32>(0.0, e), scale, sedimentary)
           - lithic_height(c - vec2<f32>(0.0, e), scale, sedimentary);
    let n = normalize(vec3<f32>(-dx / (2.0 * e), -dy / (2.0 * e), 1.0));
    // Horizon shadowing over the actual relief, at geometrically spaced
    // distances: small cuts and large overhangs both occlude the sun.
    var occlusion = 0.0;
    var step_len = max(scale * 0.007, 2.0 * P.look.w);
    for (var i = 0u; i < 7u; i++) {
        let other = lithic_height(c + to_light() * step_len, scale, sedimentary);
        occlusion = max(occlusion, (other - h) / step_len);
        step_len *= 1.85;
    }
    let visible = 1.0 - 0.82 * smoothstep(0.32, 0.95, occlusion);
    let hollow = 1.0 - smoothstep(0.065, 0.145, h / scale);
    return StoneRelief(n, visible, hollow, h / scale);
}

fn relief_light(rgb: vec3<f32>, r: StoneRelief, shade: f32) -> vec3<f32> {
    let sun = normalize(vec3<f32>(0.66 * light_x(), -0.75, 0.65));
    let diffuse = max(0.0, dot(r.normal, sun));
    let bounce = mix(rgb, col(SKY_HORIZON) * luminance(rgb), 0.28);
    let ambient = 0.24 * (1.0 - 0.35 * r.hollow);
    return bounce * ambient + rgb * (1.5 * diffuse * r.visibility) * (0.55 + 0.8 * shade);
}

// Fine mineral grain, tilted strata and paired dark cuts / sunlit lips.
// These follow the same relief as the lighting, not a random speckle overlay.
fn mineral_surface(c: vec2<f32>, scale: f32, r: StoneRelief, shade: f32) -> vec3<f32> {
    let stain = fbm(c, scale * 0.21, 5u, 2171u);
    let base = shaded(ROCK, 0.60);
    let cool = mix(base, vec3<f32>(luminance(base)) * vec3<f32>(0.79, 0.91, 1.08), 0.55);
    let warm = base * vec3<f32>(1.12, 0.98, 0.79);
    var rgb = mix(cool, warm, smoothstep(0.3, 0.7, stain));
    let strata = fbm(vec2<f32>(c.x * 0.14, c.y + 0.21 * c.x), scale * 0.045, 6u, 2173u);
    rgb *= 0.74 + 0.48 * strata;
    rgb = relief_light(rgb, r, shade);
    let fine = fbm(c, scale * 0.009, 4u, 2177u);
    rgb *= 0.8 + 0.4 * fine;
    let joint_scale = scale * 0.07;
    let resolve = smoothstep(2.0, 5.0, joint_scale * 0.08 / P.look.w);
    if (resolve > 0.0) {
        let grain = normalize(vec2<f32>(0.8, 0.28 * light_x()));
        let cut = fractures(c, grain, joint_scale, 2181u);
        let lip = fractures(c - to_light() * joint_scale * 0.07, grain, joint_scale, 2181u);
        rgb *= 1.0 - 0.65 * cut * resolve;
        rgb += base * 0.3 * max(0.0, lip - cut) * resolve * r.visibility;
    }
    return rgb;
}

fn volume_noise(p: vec3<f32>, seed: u32) -> f32 {
    let cell = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    var value = 0.0;
    for (var z = 0; z <= 1; z++) {
        let s = seed ^ pcg(bitcast<u32>(cell.z + z));
        let a = mix(hash3(cell.x, cell.y, s), hash3(cell.x + 1, cell.y, s), u.x);
        let b = mix(hash3(cell.x, cell.y + 1, s), hash3(cell.x + 1, cell.y + 1, s), u.x);
        value += mix(a, b, u.y) * select(1.0 - u.z, u.z, z == 1);
    }
    return value;
}

fn cloud_turbulence(p: vec3<f32>, scale: f32) -> f32 {
    var q = p;
    var amp = 0.57;
    var n = 0.0;
    var wl = scale;
    for (var i = 0u; i < 5u; i++) {
        let keep = smoothstep(1.5, 3.5, wl / P.look.w);
        n += amp * mix(0.5, volume_noise(q, 2301u + i * 19u), keep);
        q = q * 2.03 + vec3<f32>(3.1, 7.9, 1.7);
        wl /= 2.03;
        amp *= 0.45;
    }
    return n;
}

fn cloud_density(l: u32, p: vec3<f32>, scale: f32) -> f32 {
    let b = mass_bounds(l);
    if (p.x <= b.x || p.x >= b.z) { return 0.0; }
    let span = mass_span(l, p.x);
    let radius = max((span.y - span.x) * 0.5, 0.0001);
    let y = (p.y - 0.5 * (span.x + span.y)) / radius;
    let z = p.z / (0.65 * scale);
    let envelope = 1.0 - y * y - z * z;
    if (envelope <= 0.0) { return 0.0; }
    let q = p / (scale * 0.28) + vec3<f32>(0.0, 0.0, f32(mass_owner(l)) * 3.71);
    let turbulence = cloud_turbulence(q, scale * 0.28);
    // Erosion removes wisps at the edges and cuts deep folds between
    // billows. Interior density stays continuous through the old bands.
    return max(0.0, envelope - 0.28 + (turbulence - 0.5) * 1.55) * 4.0;
}

fn clear_sky(c: vec2<f32>) -> vec3<f32> {
    let t = clamp(c.y / max(P.frame.x, 1e-3), 0.0, 1.0);
    return mix(col(SKY_ZENITH), col(SKY_HORIZON), pow(t, 1.4))
        * (0.94 + 0.12 * fbm(c, 0.4, 3u, 31u));
}

// Premultiplied scattering and remaining transmittance, for compositing
// overlapping banks without punching sky-coloured holes in the bank behind.
fn cloud_volume(l: u32, c: vec2<f32>) -> vec4<f32> {
    let b = mass_bounds(l);
    let scale = max(b.w - b.y, 0.025);
    let span = mass_span(l, c.x);
    let y = (c.y - span.x) / max(span.y - span.x, 0.001);
    let sun_dir = normalize(vec3<f32>(0.66 * light_x(), -0.75, -0.45));
    let dz = scale * 1.3 / 24.0;
    var transmittance = 1.0;
    var rgb = vec3<f32>(0.0);
    for (var k = 0u; k < 24u; k++) {
        let p = vec3<f32>(c, -0.65 * scale + (f32(k) + 0.5) * dz);
        let density = cloud_density(l, p, scale);
        if (density > 0.001) {
            var optical_depth = 0.0;
            for (var j = 0u; j < 5u; j++) {
                let travel = (f32(j) + 0.5) * scale * 0.11;
                optical_depth += cloud_density(l, p + sun_dir * travel, scale) * 0.11;
            }
            let sunlight = exp(-3.4 * optical_depth);
            let ambient = mix(col(CLOUD + 1u), col(STORM), 0.35) * (0.42 + 0.20 * (1.0 - y));
            let scatter = ambient + col(CLOUD) * (1.02 * sunlight);
            let alpha = 1.0 - exp(-density * dz * 9.0 / scale);
            rgb += transmittance * alpha * scatter;
            transmittance *= 1.0 - alpha;
            if (transmittance < 0.006) { break; }
        }
    }
    return vec4<f32>(rgb, transmittance);
}

fn cloud_sky(c: vec2<f32>) -> vec3<f32> {
    var rgb = clear_sky(c);
    for (var l = 0u; l < P.n_layers; l++) {
        if (u32(layers[4u * l + 2u].x) != R_CLOUD || mass_owner(l) != l) {
            continue;
        }
        let b = mass_bounds(l);
        if (c.x <= b.x || c.x >= b.z || c.y <= b.y || c.y >= b.w) { continue; }
        let span = mass_span(l, c.x);
        if (c.y <= span.x || c.y >= span.y) { continue; }
        let volume = cloud_volume(l, c);
        rgb = volume.xyz + volume.w * rgb;
    }
    return rgb;
}

// Fine bent blades with individual heights and lean. The front-most root
// wins overlaps. Pixel footprint widens the coverage transition, not the
// blades themselves, and subpixel grass fades back to its turf colour.
fn grass_blades(c: vec2<f32>, size: f32) -> vec2<f32> {
    let p = c / vec2<f32>(size * 0.65, size);
    let cell = vec2<i32>(floor(p));
    let px = P.look.w / (size * 0.65);
    var result = vec2<f32>(0.0);
    var front = -1e9;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let id = cell + vec2<i32>(x, y);
            let root = vec2<f32>(id) + vec2<f32>(hash3(id.x, id.y, 2451u), hash3(id.x, id.y, 2453u));
            let tall = 0.35 + 0.8 * hash3(id.x, id.y, 2457u);
            let t = (root.y - p.y) / tall;
            if (t <= 0.0 || t >= 1.0 || root.y < front) { continue; }
            let lean = (hash3(id.x, id.y, 2461u) - 0.5) * 0.9;
            let d = abs(p.x - root.x - lean * t * t);
            let width = 0.10 * (1.0 - t);
            let cover = 1.0 - smoothstep(max(0.0, width - px), width + px, d);
            if (cover > 0.01) {
                front = root.y;
                result = vec2<f32>(cover, 0.2 + 0.8 * t);
            }
        }
    }
    return result;
}

fn meadow_detail(rgb: vec3<f32>, c: vec2<f32>, depth: f32, shade: f32) -> vec3<f32> {
    let near = 1.0 - depth;
    let size = (0.001 + 0.013 * near * near) * P.handling.z;
    // Fixed octave grids avoid stretching blades as depth changes down a
    // river bank. Blend adjacent scales, just like the tree crowns.
    let level = log2(size / 0.001);
    let base = 0.001 * exp2(floor(level));
    let blades = mix(grass_blades(c, base), grass_blades(c, base * 2.0), fract(level));
    let resolve = smoothstep(2.5, 6.0, size / P.look.w);
    let turf = fbm(c, size * 2.0, 5u, 2467u);
    var out = rgb * (0.68 + 0.55 * turf);
    let grass = rgb * (0.38 + blades.y * (0.62 + 0.48 * shade));
    out = mix(out, grass, blades.x * resolve * 0.85);
    return out;
}

// The spire cell under `c` (the same lattice as `spire_field`), hashed to a
// stable per-tree attribute.
fn spire_tone(c: vec2<f32>, size: f32, s: u32) -> f32 {
    let p = vec2<f32>(c.x / size, c.y / (1.4 * size));
    let i = vec2<i32>(floor(p));
    return hash3(i.x, i.y, s + 5u);
}

// Per-tree variation applied inside a crown only: value (older, denser
// crowns darker), and a hue lean toward warm new growth or cool shade. The
// mean over many trees is unchanged, so distant stands keep their colour.
fn individual_tree(rgb: vec3<f32>, tone: f32, crown: f32) -> vec3<f32> {
    let inside = smoothstep(0.05, 0.4, crown);
    let value = 0.8 + 0.4 * tone;
    let warm = mix(rgb, col(FOLIAGE_WARM) * luminance(rgb) / max(luminance(col(FOLIAGE_WARM)), 1e-3), 0.35);
    let cool = mix(rgb, col(FOLIAGE_COOL) * luminance(rgb) / max(luminance(col(FOLIAGE_COOL)), 1e-3), 0.35);
    let hue = fract(tone * 7.31);
    let leaned = mix(mix(cool, rgb, smoothstep(0.0, 0.5, hue)), warm, smoothstep(0.5, 1.0, hue));
    return mix(rgb, leaned * value, inside);
}

// Branchlets/leaves within a crown. Coarse canopy shading remains even
// when leaf clusters are subpixel; fine foliage is a band-limited addition.
fn canopy_detail(rgb: vec3<f32>, c: vec2<f32>, size: f32, crown: vec2<f32>, shade: f32) -> vec3<f32> {
    let e = max(P.look.w, size * 0.045);
    let q = c + size * 0.3 * vec2<f32>(vnoise(c / size, 2411u), vnoise(c / size, 2413u));
    let n = fbm(q, size * 0.42, 5u, 2417u);
    let ahead = fbm(q + to_light() * e, size * 0.42, 5u, 2417u);
    let facing = clamp(0.5 + (n - ahead) * size / (3.0 * e), 0.0, 1.0);
    let leaf_res = smoothstep(3.0, 8.0, size / P.look.w);
    let broad = 0.45 + 0.75 * smoothstep(0.15, 0.85, crown.y);
    let leaf = 0.64 + 0.65 * facing;
    let occlusion = mix(0.52, 1.0, smoothstep(0.08, 0.45, crown.x));
    let out = rgb * broad * occlusion * mix(1.0, leaf, leaf_res);
    return out + rgb * (0.22 * shade * leaf_res * smoothstep(0.58, 0.74, n) * crown.y);
}
