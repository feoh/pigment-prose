# Painting (tasks 06–07, 25)

`pigment_gpu::PaintRenderer` (`crates/pigment-gpu/src/paint.rs` + `paint.wgsl`) paints a `Scene` with an authored palette (`pigment_core::palette`). It runs over the shared tile loop, so tiled output equals single-tile output. **Status:** the painting direction was accepted in visual-review rounds 2–5 ([round 5 result](visual-review/round-05/RESULT.md)). This is not the task 08 gate. Task 07 then added the [woodland, rocks and water](#woodland-rocks-and-water-task-07) cues and is waiting on [round 6](visual-review/round-06/README.md). Task 25 added the [final rendering detail](#wind-current-and-stone-task-25) (wind and current on water, more complex rocks), approved in [round 7](visual-review/round-07/RESULT.md); `RENDERER_VERSION` is 2 and the approved baseline is [baseline-25](visual-review/baseline-25/README.md). Task 06 evidence (settings side-by-sides, 4K edge crops, benchmark logs) is in [evidence/paint-06](evidence/paint-06/README.md), and task 07 evidence is in [evidence/paint-07](evidence/paint-07/README.md).

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
- **Crowns at two sizes:** crowns sized continuously by depth would shear into streaks where depth changes down the ground. So crowns are drawn at the two nearest power-of-two sizes and blended, as texture mipmaps are. Emergent trees (task 07) are drawn the same way.
- **Ridge light:** a valley spur's light fades to neutral toward its foot (the bottom of its bounding box), where it meets the neutral bank.

## Woodland, rocks and water (task 07)

**Light side.** `Scene::light()` (`LightSide::Left` or `Right`) is the side the scene's light comes from, and the geometry checksum covers it. The planes' `shade` was always computed from it, but the painter never received it: crowns were lit from the upper left in every scene, against the planes in right-lit scenes. Crowns, spires, emergent trees and stand shadows now follow `to_light()`, the image direction `(±0.66, −0.75)` that the generator's `face_shade` uses.

**Greenery size and height** (round 5: "a bit more variation in size and height of greenery"):

- **Stand ages.** Each stand of the mixed forest has its own crown size, log-uniform over 0.62–1.62× (young thickets to old growth), and a height of the plant's height × that size. Woodland layers get an age of 0.73–1.37× by layer.
- **Stand shadows.** Where a taller stand lies toward the light, it throws a soft shadow onto its lower neighbour: a band of `0.4 × height step × facing` stand units, darkest at the boundary. A taller stand's own edge facing the light catches a lit rim. Steps under 0.15 are ignored, so like stands merge.
- **Emergent trees.** About one cell in eight of a sparse field holds a big crown (1.8× the canopy's) standing over the canopy, lit toward the light, with its shadow cast away from the light. They are drawn at two power-of-two sizes and fade out below 3–6 px. In shrubland the emergent is a lone broadleaf.
- **Silhouettes** (generator): within a stand, about one crown in seven is an emergent tree 1.35× taller and one in seven a young one at 0.65× (shrubs vary half as much). A broadleaf stand's crowns now span 0.68–1.17 of its height, compared with 0.78–1.0 before (`canopies_mix_emergent_and_young_crowns`).
- **Conifers** from above read as vertical streaks at 4K, so their spire cells are now 1.4× as tall as wide instead of 2×, with wider bases.

**Rocks.** Plane contrast is `0.85 + 0.15 × faceting` (it was `0.6 + 0.4 × faceting`), so even rounded boulders keep a lit and a shadowed flank. `rocks_have_lit_and_shadowed_planes` requires at least 0.1 of shade between planes at every faceting; sample 1 at faceting 0 had 0.077 before. The painter adds selective gouache accents 0.003 canvas units wide (faded below about 1.5 px): a lit rim where the rock's edge faces the light, a dark line on the edge turned away, and a contact shadow on the ground at its foot.

**Moss** (round 6: "moss doesn't grow in perfectly delineated patches like that"). Moss used to be a thresholded noise patch on shadowed planes, with ruled edges. Now it forms soft cushions that follow the form. It is damp-weighted: `0.45 × (1 − shade)`, plus 0.3 at the rock's foot and 0.2 along its top, found by probing 0.012 below and 0.008 above. Its cover is a domain-warped four-octave field with a fine fringe (0.004 × mark scale), so its edges feather into speckles. It is tufted with small crowns (0.0035 × mark scale), bright yellow-green on their lit tops and deep green between, and at most 0.9 opaque.

**Shrubs and grass** (round 6: a flat-topped shrub stand read as "a chonky green block"). Shrubland is a rounded dome of separate clumps (generator). Near stands (depth ≤ 0.3) get meadow grass over their base: ragged blades 0.003–0.011 canvas units tall. A pixel is painted as meadow when the point that far below it lies outside the stand.

**Water.**

- **Reflections.** Each water pixel finds its local shoreline straight above it (`shore_distance`: doubling steps, then six bisections on the water layer) and mirrors about it. The reflected color is the front layer at the mirrored point, as painted, with its own aerial perspective. It excludes water, the near shore, its woods and rocks. The point is rippled sideways (more with distance from the shore), dimmed (× 0.85, 25 % toward the far-water color) and faded out over 1.15 × the mountain's rise below the shore, at most 0.75 opaque. So lakes mirror their mountains and far woods, and rivers their banks.
- **Rocks in water** throw short, rippled dark reflections: up to 0.03 canvas units below a rock, found by probing upward inside the rocks' bounding box (`Params.rocks`, from `rock_bounds`).
- **No halo.** Reflections, rims and contacts evaluate the scene at offset points (coverage plus material). They never read neighbouring pixels, so tiles need no extra apron. `painting_is_identical_tiled_and_single` now also covers a river valley and two lakes with rocks.
- Water stays in its region by construction: water color is painted only where water is the front layer.

## Wind, current and stone (task 25)

Round 6 asked for the final rendering to show "the effects of wind and current and have more ripples", and rock surfaces "less chonky geometric and more complex". Review: [round 7](visual-review/round-07/README.md), approved 2026-09-28 ([result](visual-review/round-07/RESULT.md)).

**Water marks live on the ground plane.** `ground(c)` maps a pixel below the horizon to ground coordinates (across ÷ depth, 1 ÷ depth, where depth is the distance below the horizon), and gives the pixel's ground footprint. `gfbm` band-limits against that footprint, so each distance keeps only the octaves it can resolve: ripples shorten and flatten toward the far shore by construction, and previews and exports agree.

- **Wind** (`Scene::wind()`, see [scene generation](scene-generation.md#output-the-layer-model)), in the wind's frame (across it, along it):
  - **lanes:** long streaks along the wind, rough (matte, darker, showing deep water) between slick ones (glassy, catching the sky);
  - **cat's-paws:** patches where a gust touches down;
  - **ripples:** six octaves from 0.4 ground units down, crests across the wind, broken into dashes, with troughs slightly darker; rough water ripples harder;
  - **chop and glitter:** short marks everywhere, busier where the wind roughens the water, and fine glints in the gusts.
- **Reflections** break up with the wind: they fade by up to 85 % under cat's-paws and 30 % in rough lanes, and the mirrored point wobbles with the ripples (more where rough). Calm, slick water keeps a crisp mirror.
- **Current.** `paint::channel_rows(scene)` records the water's visible channel on 256 rows: the widest stretch of water that no layer in front of it covers (left bank, right bank, width), bound as `channel_rows`. A narrow channel runs fast, fastest mid-stream; a wide lake barely moves. The current draws pale lines along the channel over slightly darker water, and the lines follow its bends and crowd where it narrows past a spur. The rows are a function of the scene only, so tiles agree.
- **Rocks in water** keep their dark rippled reflections (now jiggled more on rough water) and wear a broken ring of foam at the waterline, stronger in wind or current.

**Stone** (`rock_surface`), over the generator's planes ([scene generation](scene-generation.md#landforms): four rock kinds, ridge-cut planes, lower facets, crevices):

- **Facets at three scales** (0.03, 0.012 and 0.0055 × mark scale): a cellular field of flat chips stretched along the grain, each tilted toward or away from the light (±24 %, ±14 %, ±9 % value). About half the edges between chips are open joints: dark on the side turned away from the light, a lit lip on the side toward it. The broad scale is mostly a change of plane, with fewer joints. Each scale fades out below 3–7 px.
- **Staining:** cooler grey in places and warmer iron in others (a 0.06 × mark scale field); tilted strata; rain streaks down the shaded faces; grouped cracks with a lit lip; dark and pale flecks in the grain (from about 2 px); pale lichen rosettes on the lit, dry faces.
- **Moss** keeps the task 07 form-following cushions (the user: "Moss on the rocks looks great!"). Its cover threshold moved so it gathers a little less on the lit faces and the stone shows there.

Measured with `pigment-prose paint-bench` (12 sample seeds at 16:9, warm, render + readback, median / worst scene): RTX 4070 Ti 0.93 / 1.26 ms at 960 px, 3.4 / 4.3 ms at 1920 px, 10.4 / 12.0 ms at 3840 px; Intel iGPU 21.6 / 22.8 ms, 83 / 87 ms and 317 / 328 ms. Both GPUs stay within the 960 px and 1920 px preview budgets. The Intel iGPU is over the 3840 px settled target, as before task 25; the studio's adaptive settled cap ([qualification](qualification.md)) handles it. Logs: [paint-25](evidence/paint-25/), and the hardware suite: [gpu-tests-linux-2026-09-28-task25.txt](evidence/gpu-tests-linux-2026-09-28-task25.txt).

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
- **Coverage index** (`crates/pigment-gpu/src/coverage.rs`). Each layer's bounding box is cut into up to 256 bins across its parity ray. Each bin lists the outline edges whose span overlaps it (with one bin of margin), sorted by how far they reach along the ray, descending. The ray only tests the pixel's bin, and it stops at the first edge that ends more than 1e-5 canvas units before the pixel. Such an edge cannot be crossed, and parity does not depend on order, so coverage is **exactly** the brute-force parity along that ray. `indexed_matches_brute_force` checks this on the CPU for four scenes in three aspects, probing a pixel grid and points level with every vertex on both axes. Before the axis choice below, the painted output was byte-identical to the first cut's (six seeds, 960 px).
- **Ray axis per layer (task 07).** A long, nearly straight chain that runs along the bins puts hundreds of edges in one bin. A shoreline in row bins does this, and the reflection search probes exactly there. So each layer casts its ray along +x (row bins) or +y (column bins), whichever gives the smaller edge-weighted mean list length `Σ len² / Σ len`. Water and flat bands take columns; tall, y-monotone banks keep rows. The two rays disagree only for points within float rounding of an edge (`axes_agree_away_from_edges` asserts under 1e-5 canvas units), so the change is invisible. The debug view and the CPU rasterizer keep the +x ray.
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
- **Worst case** (`paint-bench --stress`, 96 layers with 32 768 vertices, a sawtooth so every bounding box covers the frame): with row bins only, the RTX 4070 Ti took 16 ms at 960 px and 72 ms at 1920 px (first cut 41 and 160 ms), and the Intel iGPU 0.73 s and 2.5 s (first cut 1.9 and 7.4 s), because every tooth spans every row bin. The task 07 axis choice gives those layers column bins: the RTX now takes 1.3 ms and 5.1 ms, and the iGPU 52 ms and 176 ms. That is still over its budgets on this synthetic case, which the generator's bands never produce.
- **After task 07** (reflections, rock accents, stand shadows, emergent trees; [log](evidence/paint-07/paint-bench-linux-2026-09-27.txt)): RTX 4070 Ti median / worst scene 0.74 / 0.89 ms at 960 px, 2.6 / 3.4 ms at 1920 px and 9.9 / 12.9 ms at 3840 px; Intel iGPU 17 / 22 ms, 63 / 81 ms and 244 / 312 ms. Both GPUs meet the preview budgets on real scenes. The reflection's shoreline search alone added about 100 ms at 1920 px on the iGPU, until water switched to column bins.
- Scene generation on the CPU takes 0.5 ms per scene (median).
- Tiled vs single tile is byte-identical at looseness 0, 0.4 and 1 (hardware suite). The 480 px render and the 1920 px render downsampled 4× agree with a PSNR over 24 dB. Hardware suite log: [gpu-tests-linux-2026-09-27-task06.txt](evidence/gpu-tests-linux-2026-09-27-task06.txt).
