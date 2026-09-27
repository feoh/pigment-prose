# Painting (task 06)

`pigment_gpu::PaintRenderer` (`crates/pigment-gpu/src/paint.rs` + `paint.wgsl`) paints a `Scene` with an authored palette (`pigment_core::palette`). It runs over the shared tile loop, so tiled output equals single-tile output. **Status:** the painting direction was accepted in visual-review rounds 2–5 ([round 5 result](visual-review/round-05/RESULT.md)). This is not the task 08 gate: that comes after task 07. Reflections and finer rock and tree drawing belong to task 07. Settings side-by-sides, 4K edge crops and benchmark logs are in [evidence/paint-06](evidence/paint-06/README.md).

## Palettes

Each palette has a light and a shadow color per material (cloud, far range, rock, snow, forest, meadow, sand), plus sky, sun, haze, paper, foliage accents (warm crowns, cool depths), moss and water colors. A layer's structural `shade` chooses between light and shadow (smoothstep 0.08–0.92). So light and shadow come from the scene's geometry and light pool, never from random colors per polygon.

| Id | Character |
| --- | --- |
| `lakeshore` (default) | verdant high summer: sap and viridian forest with yellow-green light, turquoise water, warm ochre rock against violet shadows |
| `golden-evening` | low gold light, rose clouds, deep blue-green shadows |

## Materials by role (pass 1)

| Role | Material |
| --- | --- |
| Sky | zenith → horizon gradient, faint wash unevenness |
| Cloud | light and shadow; a low shade tips toward the storm color |
| Far ridge | the far material |
| Mountain | forest below a ragged treeline (36–56 % of the summit rise above the horizon), rock above; snow above a ragged snowline on tall peaks (rise > 0.3 × frame height) |
| Foothills, woodland | forest |
| Spurs and framing ridges | forest with soft meadow clearings |
| Water | sky-lit toward the far shore, deep near the viewer; horizontal ripples with sheen |
| Shore | sand for the far beach; meadow with sand patches for the near shore |
| Rocks | rock with steeper plane contrast; moss in the shadowed parts |

- **Forest:** a cellular field of rounded crowns at two scales. Crowns are lit on top (warm) and dark in the gaps (cool). Crowns shrink with distance (0.003 + 0.03 × nearness² canvas units, × mark scale) and fade to their mean color below a few pixels. Stands vary between dark conifer and bright deciduous, but only nearby, so distant slopes read by their planes. Near forest is deeper in value.
- **Meadow:** horizontal grass strokes. Near meadows get sparse wildflowers (gold, white, violet), only where they are big enough to read.
- **Aerial perspective:** distant layers mix toward the haze color by `haze × far² × 1.2` (at most 0.85, and 0.55× for the main massif so it keeps its substance), where `far = (depth − 0.1) / 0.9`.
- **Palette intensity:** saturation around luminance, × (0.35 + 1.3 × intensity). Brightness is unchanged.
- **Wash and gouache:** a wash is a transmittance glaze over paper, `wash(paper, color, density)` ([compositing model](#compositing-model)), with the density varying 0.8–1.2 at low frequency. Gouache is opaque body color with brush-mark value variation. `wash_gouache` blends the two with premultiplied "over". Forms at depth ≤ 0.25 lean 0.35 toward gouache (the art direction's opaque foreground). See [wash and gouache character](#wash-and-gouache-character) for how the control's ends differ.

## Plants

After round 2 the user asked for "a variety of plant life … some trees tall and leafy and others squat and woody". There are six plants (`pigment_core::scene::Plant`). Each has its own palette colors (light and shadow), crown shape and details:

| Plant | Silhouette (generator) | Crowns and details (painting) |
| --- | --- | --- |
| broadleaf | rounded crowns, 1.15× tall, 1.3× wide | big round clumps at two scales |
| conifer | pointed spires, 1.45× tall, 0.7× wide | stacked spires, lit on one side and toward the tip |
| birch | rounded, 1.2× tall, 0.8× wide | small airy crowns, pale bark in the gaps and as trunks |
| shrub | low even mounds, 0.45× tall, 1.4× wide | flattened wide mounds, woody brown in the gaps |
| flowering | rounded, 0.85× tall | round clumps with pink and white blossom on sunlit tops |
| copper | rounded, 1.1× tall, 1.25× wide | red-purple summer foliage (copper beech) |

- **Woodland layers carry their plant.** It is picked per stand by hashing the stand's seed (weights: broadleaf 26 %, conifer 22 %, shrub 18 %, birch 14 %, flowering 12 %, copper 8 %). So picking adds no random draws, and form sliders still morph.
- **Forested hills** (foothills, spurs, framing ridges) use a painted mixed forest: warped Voronoi stands, larger nearby, drawn from a broadleaf matrix (44 %, conifer 24 %, then smaller shares). Colors blend across stand boundaries. The mix fades to the generic forest with distance, so far slopes read by their planes.
- **Trunks** show under near tall canopies. About half the crown columns carry one, each offset and with its own height.
- **Tests:** every woodland layer has a plant and nothing else has one, all six occur across a sample, conifers stand over 1.8× as tall as shrubs, and broadleaf trees over 1.5× (`woodland_stands_have_varied_plants`).

## Ground perspective and crowns (round 4)

- **Ground depth:** ground layers in front of the water (banks, spurs, near shore and woods) can span near and far. Below the horizon, their effective depth is the nearer of their own depth and the ground's depth at that row, `0.44 − 0.3 × ((y − horizon) / (h − horizon))^(2/3)`. That formula inverts the generator's spur placement, so a spur's base matches the ground under it, and a spur's ridge stays nearer and darker than the bank behind it.
- **Crowns at two sizes:** crowns sized continuously by depth would shear into streaks where depth changes down the ground. So crowns are drawn at the two nearest power-of-two sizes and blended, as texture mipmaps are. Forest stands have one fixed size.
- **Ridge light:** a valley spur's light fades to neutral toward its foot (the bottom of its bounding box), where it meets the neutral bank.

## Edges and texture (pass 2)

- **Loose edges:** each pixel's lookup is displaced by a coherent noise warp, and an 8-tap disc is averaged where layers differ. The radius is `0.02 × edge_looseness × (0.15 + 0.85 × depth)` canvas units, so distant edges wander and soften while near ones stay crisp (selective edges). Offsets are rounded independently of the tile origin.
- **Pooling:** watercolor darkens slightly at wet edges, scaled by `1 − wash_gouache`. This is where two washes meet, so it stands in for the overlap band of multiplying glazes.
- **Paper and granulation:** band-limited canvas-space noise (wavelengths 0.004 and 0.0025 canvas units), scaled by `paper_grain` and `granulation`. Granulation settles in darker areas. Up to their defaults (0.3) the amplitudes are those of the approved look (tooth `0.14 × paper_grain`, settling `0.18 × granulation`). Above the defaults they ramp continuously to `0.5` and `0.6` at 1. Before this change, the whole 0–1 range moved a 1920 px image by about 1 level on average (maximum 5 and 11 levels), too little for "rough, visible tooth" and "strongly settled pigment". Now the range moves it by 3–4 levels on average, with a maximum of 16 and 41 ([texture-crops.png](evidence/paint-06/texture-crops.png)). Both textures are band-limited away below about 2 px, so they appear only at 1920 px and above, not in small contact-sheet cells.
- **Support:** the renderer declares `edge_bleed_radius × 1.4` plus one pixel. `painting_is_identical_tiled_and_single` checks looseness 0, 0.4 and 1 with 256 and 333 px tiles.

Paint settings never reach the scene generator, so geometry checksums are unaffected (covered by the core tests).

## Wash and gouache character

Blending a wash and body color of the same pigment changes little: a wash at density 1 *is* its pigment. With only the blend, the two ends of `wash_gouache` looked almost the same, even in 1:1 crops (checked 2026-09-27). So the ends now also change the medium's character. Everything is measured from the default balance (0.25, `settings::WASH_GOUACHE.default`, asserted in `paint.rs`). At the default, `thinness` and `thickness` are both 0, and the shader takes a uniform branch with the approved expressions, so **the default output is byte-identical to the look accepted in round 5** (checked on six sample seeds at 960 px).

| | Toward wash (`thinness` = (0.25 − w) / 0.25) | Toward gouache (`thickness` = (w − 0.25) / 0.75) |
| --- | --- | --- |
| Density mottling (blooms) | amplitude × (1 + 1.5 · thinness), so 0.5–1.5 at w = 0 | — |
| Body color | — | brush-mark amplitude × (1 + thickness); lifted 10 % × thickness toward paper (chalky, matte) |
| Edge pooling | × (1 + thinness) | (already × (1 − w)) |
| Paper tooth | × (1 + 0.6 · thinness) | × (1 − 0.6 · thickness): the paint covers the paper |
| Granulation | × (1 + 0.8 · thinness) | × (1 − 0.8 · thickness) |

Endpoints: `w = 0` is a transparent, mottled watercolor that shows the paper and pools hard lines where washes meet. `w = 1` is flat, matte, opaque body color with visible brush marks, and most of the paper texture covered. See `vary-wash.png` and `wash-crops.png` in [evidence/paint-06](evidence/paint-06/README.md).

## Compositing model

`pigment_core::composite` is the reference, in linear light (architecture: "Color, alpha and output encoding"):

- `glaze(under, T, d) = under · T^d`, the same as `under · exp(−A·d)` with `A = −ln T`. `T` is clamped to `[1e-3, 1]`, so a glaze never adds light and never turns black.
- `wash(paper, pigment, d) = glaze(paper, pigment / paper, d)`: at density 1 the wash reads as its pigment.
- `over(under, color, a) = color · a + under · (1 − a)`: premultiplied over of an opaque color at coverage `a` (gouache, the wash/body blend, the loose-edge blend).
- `edge_blend(samples)`: the mean of opaque samples, which is premultiplied over at coverage `1/n` each.

The shader's `glaze`, `wash` and `over` are the same formulas. Reference cases, portable (`composite::tests`):

| Case | Checked |
| --- | --- |
| Opaque fill | `over(u, c, 1) = c` for any `u`; `over(u, c, 0) = u` |
| Paper preserved | a paper-colored wash is paper at every density; density 0 is paper |
| Glazes | only darken, stay finite and ≥ 0, even for black pigment |
| Overlapping washes | multiply; order does not matter; the overlap is darker than either wash; density 2 = two stacked washes |
| Edge blending | the blend of a dark and a light opaque color stays between them per channel at every coverage. Negative control: a straight color filtered against empty texels and then composited with straight "over" comes out darker than the correct blend (the dark fringe) |

On the GPU, `compositing_matches_the_reference_model` evaluates 576 cases (opaque fill, partial coverage, glazes and washes at densities 0–1.2, over paper, black and white) with the painting shader's own functions (`PaintRenderer::evaluate_compositing`, entry point `composite_reference_main`). The tolerance is 1e-4 relative, because GPU `pow` is not correctly rounded. The worst error measured on the RTX 4070 Ti was 7.6e-8. `painting_is_valid_at_setting_extremes` renders all 128 combinations of paint-setting endpoints (looseness, wash/gouache, mark scale, granulation, paper grain, intensity, haze) with both palettes. Each image must be opaque, contain no black (NaN or collapsed) pixels, keep a green-channel spread over 60 levels and leave the scene checksum unchanged.

## Coverage

Pass 1 finds each pixel's front layer. The first cut tested every vertex of every layer whose bounding box held the pixel. Now:

- **Front to back with early exit.** Layers are stored back to front, so the first layer from the front that holds the pixel wins.
- **Coverage index** (`crates/pigment-gpu/src/coverage.rs`). Each layer's bounding box is cut into up to 256 horizontal bins. Each bin lists the outline edges whose `y` span overlaps it (with one bin of margin), sorted by right-most `x`, descending. The +x parity ray only tests the pixel's bin and stops at the first edge that ends more than 1e-5 canvas units left of the pixel. Such an edge cannot be crossed, and parity does not depend on order, so coverage is **exactly** the brute-force result. `indexed_matches_brute_force` checks this on the CPU, for a pixel grid and every vertex row of four scenes in three aspects. The painted output was byte-identical before and after the change (six seeds, 960 px).
- **Bounded size.** A layer's bin count is halved until its entries fit in 32 × its vertex count (`index_size_is_bounded`), so the index is at most about 1 M entries (8 MB) at the scene limits.
- The index is a function of the scene only. `PaintRenderer` keeps the last scene's uploaded buffers (keyed by `Arc` identity), so paint-only changes and resizes skip the rebuild.

The debug renderer still uses the all-vertex loop. It stays as the simple reference for `debug_layer_ids_match_the_cpu_rasterizer`.

## Measured (Linux, Vulkan, 2026-09-27)

`pigment-prose paint-bench`: warm single-tile render + readback, the median of 5 runs per scene (after 2 warm-up runs), over 30 sample seeds at 16:9 with default settings. Logs: [before](evidence/paint-06/paint-bench-before-linux-2026-09-27.txt), [after](evidence/paint-06/paint-bench-linux-2026-09-27.txt).

| Size | Budget ([architecture](architecture.md)) | RTX 4070 Ti, first cut: median / worst scene | RTX 4070 Ti now | Intel RPL-S iGPU, first cut | Intel now |
| --- | --- | --- | --- | --- | --- |
| 960 × 540 (interaction) | ≤ 33 ms | 2.9 / 3.9 ms | 0.71 / 1.1 ms | 113 / 169 ms | 11 / 16 ms |
| 1920 × 1080 (settled) | ≤ 150 ms | 11.2 / 15.9 ms | 2.7 / 5.9 ms | 441 / 667 ms | 40 / 59 ms |
| 3840 × 2160 | — | 34 / 50 ms | 8.5 / 14 ms | 1.75 / 2.64 s | 159 / 231 ms |

- **Correction:** the first cut recorded "about 0.3 s per 1920 px cell" and called the preview unacceptable. That timed the whole `contact-sheet` loop, not the renderer. The warm render was already within budget on the RTX 4070 Ti, but the Intel iGPU missed both budgets by 3–5×. Most of that time went to coverage. The "first cut" columns are commit `c8b5dad` measured with this `paint-bench`.
- **Worst case** (`paint-bench --stress`, 96 layers with 32 768 vertices, a sawtooth so every bounding box covers the frame and every edge spans every bin): RTX 4070 Ti now 16 ms at 960 px and 72 ms at 1920 px (first cut 41 and 160 ms); Intel iGPU now 0.73 s and 2.5 s (first cut 1.9 and 7.4 s). The index cannot help much when every edge spans the whole layer height. The generator's bands never produce that shape, but an iGPU-class device would miss the preview budget on such a scene.
- Scene generation on the CPU takes 0.5 ms per scene (median).
- Tiled vs single tile is byte-identical at looseness 0, 0.4 and 1 (hardware suite). The 480 px render and the 1920 px render downsampled 4× agree with a PSNR over 24 dB. Hardware suite log: [gpu-tests-linux-2026-09-27-task06.txt](evidence/gpu-tests-linux-2026-09-27-task06.txt).
