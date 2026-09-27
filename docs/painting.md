# Painting (task 06, first cut)

`pigment_gpu::PaintRenderer` (`crates/pigment-gpu/src/paint.rs` + `paint.wgsl`) paints a `Scene` with an authored palette (`pigment_core::palette`). It runs over the shared tile loop, so tiled output equals single-tile output. **Status:** the first cut responds to the user's direction ("a verdant forest… more greens, more colors… more LIFE"), and is waiting on round 2 feedback ([visual-review/round-02](visual-review/round-02/README.md)). Reflections, faster coverage and finer rock and tree drawing are still open.

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
- **Wash and gouache:** a wash is a transmittance glaze over paper, `paper × (color / paper)^density`, with the density varying 0.8–1.2 at low frequency. Gouache is opaque body color with brush-mark value variation. `wash_gouache` blends the two. Forms at depth ≤ 0.25 lean 0.35 toward gouache (the art direction's opaque foreground).

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

## Edges and texture (pass 2)

- **Loose edges:** each pixel's lookup is displaced by a coherent noise warp, and an 8-tap disc is averaged where layers differ. The radius is `0.02 × edge_looseness × (0.15 + 0.85 × depth)` canvas units, so distant edges wander and soften while near ones stay crisp (selective edges). Offsets are rounded independently of the tile origin.
- **Pooling:** watercolor darkens slightly at wet edges, scaled by `1 − wash_gouache`.
- **Paper and granulation:** band-limited canvas-space noise, scaled by `paper_grain` and `granulation`. Granulation settles in darker areas.
- **Support:** the renderer declares `edge_bleed_radius × 1.4` plus one pixel. `painting_is_identical_tiled_and_single` checks looseness 0, 0.4 and 1 with 256 and 333 px tiles.

Paint settings never reach the scene generator, so geometry checksums are unaffected (covered by the core tests).

## Measured (RTX 4070 Ti, Vulkan, 2026-09-26)

| Check | Result |
| --- | --- |
| Tiled vs single tile, looseness 0 / 0.4 / 1 | byte-identical |
| 480 px render vs 1920 px render downsampled 4× | PSNR 32.2 dB (asserted > 24) |
| Opaque, repeatable, not flat | all three test scenes; value s.d. 26–30 (sRGB 0–255); 19–28 % clearly green pixels |
| Time for one 1920 px cell, including generation and setup | about 0.3 s |
| Time for one 3840 px cell | about 1.1 s |

**Performance is not acceptable for the interactive preview yet** (the target is ≤ 150 ms settled, [architecture.md](architecture.md)). Both passes are cheap except coverage: pass 1 still tests every vertex of every layer whose bounding box contains the pixel. That must be replaced by per-row crossing tables or rasterized coverage masks before task 11.
