# Scene generation (tasks 05 and 05b)

This document specifies the structural scene for the first painting, a rocky wooded lakeshore below a mountain ridge. The code is `crates/pigment-core/src/scene/` (`lakeshore.rs`, `noise.rs`, `raster.rs`). The debug views are in `crates/pigment-gpu/src/debug.rs`. Contact sheets and their notes are in [evidence/scene-05/](evidence/scene-05/README.md). The contracts it builds on are in [architecture.md](architecture.md), and the seed streams are in [seeds-and-recipes.md](seeds-and-recipes.md).

**Task 05b (vistas for awe)** added two templates (`tower-peak`, `high-vantage`), cloud masses, a structural light pool, distance-scaled trees and awe metrics. It followed the user's review that the task 05 scenes read as "some hills and some scrub". Sections marked 05b below describe the additions, and [art-direction.md](art-direction.md#awe-metrics) describes the metrics. Round-by-round ratings are in [visual-review/](visual-review/).

The scene is **geometry only**: depth-ordered polygons with a semantic role and a structural light factor. Color, edges, washes, texture and trees are tasks 06–07. `GENERATOR_VERSION` is **3**: version 1 was approved at the task 08 visual gate (2026-09-27), version 2 (task 25: wind, and the rock builder described below) at review round 7 (2026-09-28), and version 3 (varied rock proportions, wear and lean) at review round 9 (2026-09-28). Any checksum change requires a version bump.

## Output: the layer model

`Scene::layers()` is a back-to-front list of `SceneLayer { role, depth, shade, outline }`:

- **outline:** a closed simple polygon in canvas units. The short side of the frame is 1, the origin is top-left and +y points down.
- **depth:** 0 is nearest and 1 is farthest. Depth never increases along the list (validated).
- **role:** what the region *is*. Later passes select regions by role and never infer them from painted colors.
- **shade** (new in task 05): structural illumination from 0 to 1, meaning how directly the plane faces the scene's light, computed from its geometry. 0.5 means neutral or not a facet. Painting maps it to value and temperature (Cézanne's color planes). It is not a color, and appearance settings never change it.

`Scene::light()` (task 07) says which side of the sky the light comes from (`LightSide::Left` or `Right`, the composition's light side). Painting uses it to light crowns and cast stand shadows consistently with `shade`. The geometry checksum covers it.

`Scene::wind()` (generator v2, task 25) is the breeze over the water: a unit direction `(x, z)` on the water plane (`x` across the image, `z` into the distance) and a strength from 0 (glassy calm) to 1 (a fresh breeze). It comes from its own salted draw of the composition stream (`WIND_SALT`), so no other composition decision moved when it was added, paint settings never change it, and a new composition may bring a different breeze. About one scene in six is nearly calm (0.03–0.13); the rest run from 0.25 to 1, weighted toward moderate. The geometry checksum covers it.

Layers produced, back to front:

| Layer | Role | Depth | Notes |
| --- | --- | --- | --- |
| Sky | `Sky` | 1.0 | a rectangle past the frame on every side, so no pixel is ever uncovered |
| Clouds (05b) | `Cloud` | 0.98 | an optional storm deck with a break, then 1–3 cumulus banks, each with a body, a sunlit crown and a shadowed base; behind all terrain, so the peaks stand in front of the sky |
| Distant range | `FarRidge` | 0.95 | 38 ± 22 % of the summit height, low detail |
| Main massif | `Mountain` | 0.85 | the silhouette body, shade 0.5 |
| Mountain facets | `Mountain` | 0.85 | up to 24 planes laid over the body; shade from facing |
| Foothills | `MidRidge` | 0.66 | rolling band along the far shore |
| Far woods | `Woodland` | 0.60 | up to 6 clusters standing on the far shoreline |
| Framing ridges | `NearRidge` | 0.55 | `FramingRidges` template only, one per side |
| Far beach | `Shore` | 0.52 | a thin strip on the waterline |
| Lake | `Water` | 0.45 | the far shoreline is its top edge |
| Near shore | `Shore` | 0.25 | foreground land; the near shoreline is its top edge |
| Near woods | `Woodland` | 0.20 | up to 5 stands along the near shore, each one plant (see [painting.md](painting.md#plants)) |
| Rocks | `ForegroundRock` | 0.15 → 0.03 | 3–6 rocks, some with a smaller companion, each a body plus at least three planes (see Rocks below), sorted far to near |
| Valley spurs (05b, `high-vantage`) | `NearRidge` | 0.44 → 0.14 | 4–7 ridges from alternating sides, far to near, each with a band of trees sized for its distance; the lake winds between them |
| (removed) cliff edge | — | — | `high-vantage` once framed the view with a ledge the viewer stood on. Removed at the user's request (2026-09-27): the view now ends on the valley's nearest spur, which sits on the frame's bottom edge (nearness 1). There is no near shore, near woods or rocks in this template. |

The two `Woodland` kinds are **placement regions** for task 07's trees. The debug views show them as flat masses, and their scalloped outlines are only a rough canopy envelope. The water's top edge (far shoreline), the near shore's top edge (near shoreline) and the rock bases give task 07 the reflection line and the shore contact.

**Bounds:** at most 96 layers, 4096 vertices per layer and 32,768 per scene (validated by `Scene::new`). The densest case, `high-vantage` at 4:1 with maximum faceting, relief and density, stays under 90 layers and 30,000 vertices (tested; the worst of 180 extreme scenes measured 28,284). Distant spurs are sampled at up to 2 × `PROFILE_STEP`, and far woods at `PROFILE_STEP`, to stay within that bound.

## Topology: x-monotone bands

Every layer is built as a **band**: a top chain over a bottom chain sampled at the same increasing x values, strictly apart between the ends. At the ends they either meet at a point or are joined by a short vertical edge. Full-width masses (the ridges, the water and the near shore) use a flat bottom edge below the frame. Such a polygon is simple by construction. Tests confirm that every layer of every tested scene is simple (`raster::is_simple`, a sweep over edges).

Planes laid over a mass (mountain facets and rock planes) reuse the mass's own top-chain samples, so there are never slivers or gaps between a plane and its mass. Rock planes (generator v2) are not bands: they are cut by ridge lines in the rock's own coordinates (`RockBody`: u across its samples, v from silhouette to foot). Between two samples that map is bilinear over a convex strip, so outlines whose ridges keep their order in u, stay at least a sample apart and are split wherever they cross a sample are simple by construction; the same tests check them. All full-width chains share one grid with spacing `PROFILE_STEP` = 1/320 canvas units, extending `MARGIN` = 0.1 past the frame. Clusters and rocks use finer local grids.

## Streams and draw order

| Stream | Decides |
| --- | --- |
| composition | template, mirroring, light side, horizon, primary summit scale, every summit's position, relative height, asymmetry and concavity, near-shore shape, framing ridges, rock cluster position |
| terrain | seeds for every fractal profile (massif, distant range, foothills, framing ridges, waterline, near shoreline, beach, facet variation, rock detail); rock count and shapes |
| vegetation | woodland cluster positions, sizes, activation rolls and crown seeds |
| paint detail | **never read** (tested: changing it leaves the checksum unchanged) |

The draw order is fixed, and **the number of values drawn never depends on form settings**. For example, all 6 rock and 9 woodland parameter sets are always drawn, and form settings only decide which are used and how large they are. So moving a slider **morphs one composition instead of reshuffling it**: a relief change of 0.01 moves the massif's outline by less than 0.01 canvas units (tested). The number of secondary summits depends on the frame width, which is why changing the aspect ratio recomposes the scene (see below). Terrain noise is *stateless*: lattice values are hashed from integer coordinates and a noise seed, so sampling a profile more finely or more coarsely never changes it.

`Composition::draw(seeds, aspect)` exposes every composition-stream decision for diagnostics. The contact-sheet notes print its template.

## Templates

| Template | Composition | Near shore |
| --- | --- | --- |
| `peak-over-water` | one dominant summit at 24–40 % of the width, secondary summits at 30–60 % of its height | a corner in the lower corner opposite the summit, with the rocks on it |
| `framing-ridges` | ridges descend to the lake from both sides (35–60 % of the sky space high, feet at 22–36 % of the width from each edge), framing a summit at 40–60 %; the lake's edge bends forward under the ridges | a bay across the whole width |
| `twin-summits` | two summits at 22–36 % and 62–78 %, the second 80–97 % as high | a corner (60 %) or a bay (40 %) |
| `tower-peak` (05b) | "high distance": a low horizon (66–74 % of the frame height in portrait and square frames, up to 70–78 % at 2:1 and wider), one steep summit at 35–62 % with two shoulders at 50–82 % of its height, 1.4× detail. The summit reaches 0.56–0.9 of the sky space (up to `MAX_TOWER` = 1.3), with half-width (1.25 − 0.5 × relief) × height | a small corner. Far trees are 0.35× normal size (0.5× under `high-vantage`), near woods stay at the frame edge, rocks are 0.6× |
| `high-vantage` (05b) | "level distance" from above: a high horizon (28–40 %) with a distant massif at 0.45–0.85 of the small sky space | none. Valley spurs descend from the horizon to the frame's bottom edge (nearness k/n, spacing ∝ t^1.5, size (0.05 + 0.32 t²) × (0.6 + 0.8 × relief)) along a meander |

Template weights: `tower-peak` 20 %, `high-vantage` 40 % (raised after rounds 3–4, whose favourites were all high-vantage), `peak-over-water` 13 %, `twin-summits` 14 %, `framing-ridges` 13 %. Mirroring flips every horizontal placement. The light comes from the upper left or the upper right. Secondary summits continue outward from the focal ones every 0.3–0.6 canvas units until past the frame, so a wide frame shows more of the range instead of stretched summits.

**Horizon:** 52–60 % of the frame height for portrait and square frames, rising linearly to 60–68 % at 2:1 and wider. Everything above it is the *sky space* that summit heights are measured against.

## Landforms

**Main massif.** The height above the horizon at x is the smooth maximum of the summit shapes and a low continuous shoulder, plus fractal detail:

- Primary summit height = sky space × (0.3 + 0.5 × relief) × (0.85 + 0.3 × scale). It is capped at 0.85 × sky space and at `MAX_SUMMIT` = 0.75 canvas units, so tall portrait frames keep some sky. A soft cap above 0.9 × sky space keeps the silhouette off the top edge. The summit half-width is (3.0 − 1.8 × relief) × height × an asymmetry factor from 0.75 to 1.35.
- The summit profile blends a quartic dome (`faceting` 0) with straight or concave flanks, `(1 − d)(1 − κd)` where κ is 0–0.6 per summit (`faceting` 1). The smooth-max radius shrinks with faceting, so saddles are rounded at 0 and sharp V-notches at 1.
- Detail: fBm with amplitude (0.14 + 0.16 × relief) × height × (0.4 + 0.6 × local height ÷ summit height), so it is rougher near the summits.

**Facet planes.** The summits are found on the silhouette near each composed summit (summits closer than 0.06 are merged). Saddles are the lowest silhouette points between them. Each face, summit to saddle, gets 1 facet, or 2 when `faceting` ≥ 0.5 and the face is wide enough. A facet hangs from the silhouette: its lower edge falls from one end to a low point and rises to the other.

- The facet at the summit is the **flank**. Its crease runs from the summit toward the foot (low point 60–90 % of the way along, 75–100 % of the way to the horizon, at most 3 × its width deep), so each flank reads as one lit or shadowed plane. The massif between two flanks reads as the summit's front spur.
- Further facets hang shallowly (15–45 % deep, at most 1.2 × their width) with skewed low points, so they slant rather than form teeth. No facet is narrower than 0.08.
- Shade = 0.5 + (facing − 0.5) × (0.6 + 0.4 × faceting) × a variation of ±35 % per facet, where *facing* is the dot product of the face normal with the light direction. Rounded masses therefore get softer plane contrast.

**Distant range, foothills, framing ridges.** These are fBm profiles above the horizon: the distant range at 0.38 ± 0.22 of the massif's peak, the foothills at sky space × (0.07 + 0.08 × relief) × 0.1–1.0, and the framing ridges falling from the frame edge to their foot with an eased profile (`faceting` blends a smooth shoulder with a straight fall) and ±30 % detail.

**Water and shores.** The far shoreline is the horizon ± 0.006 of low-frequency noise, plus the forward bend under the framing ridges. A corner shore enters its side at 15–45 % of the foreground depth and leaves through the bottom 55–85 % of the width along, with curvature from 0.2 to 0.7. A bay shore sits at 45–65 % of the foreground depth with a bay 35–60 % wide and 8–25 % deep, rising at the headlands. Both carry ±0.012 of fBm detail.

**Rocks** (generator v2, task 25). There are 3–6 rocks, clustered around a composed position along the visible near shoreline. Their width is (0.06 + 0.16 × size) × (0.5 + 0.8 × nearness). Most sit on the waterline (a base offset of −0.25 to +0.4 widths, where negative means standing in water). A few stand further forward and larger (cubed placement). The terrain stream still draws generator v1's flank, crease and cap values, unused, so the stream after the rocks is unchanged; everything new comes from per-rock noise constants (`noise::unit`), not RNG draws.

- **Kinds:** a lobed boulder (two or three rounded lobes run together, 32 %), a jointed block (steep sides, a stepped tilted top, 24 %), a tilted slab (a long rising back and a steep broken end, 24 %) or a split boulder (two lobes and a deep cleft, 20 %), possibly mirrored. Each kind has faceted silhouette knots and a rounded version; `faceting` blends between them. The ends slope out to the foot rather than rising sheer.
- **Proportions and wear** (generator v3, review round 8: "You don't generally see very uniform anthill like shapes"): each rock's height is 0.65–1.25× its kind's, so a rock of one width can be low and broad or tall. Rounded boulders are worn flat on top (height cut at 0.62–1, then rescaled). Every crest leans sideways by its own skew of the profile (up to ±0.35; the ridge lines use its exact inverse). The kinds are now 30 % boulders, 28 % blocks, 27 % slabs and 15 % split boulders.
- **Relief:** fBm at two scales (±8 % and ±5 % of the height) and two to four chipped notches, each a steep drop with a gentle recovery.
- **Companions:** a rock of at least 0.04 width has a 45 % chance of a smaller companion (0.32–0.58 of its width) leaning on one side, a little in front or behind, while the layer budget allows.
- **Planes:** ridge lines fall from the most pronounced silhouette corners (the cleft always, on split boulders) to the foot, fanning outward from the crest and bent sideways. The body is cut into the planes between neighbouring ridges; each plane's shade comes from which way it faces (left or right of the crest, ±0.34) and the slope of its stretch of silhouette, with neighbours at least 0.07 apart. Plane contrast is `0.85 + 0.15 × faceting`, so even rounded rocks keep lit and shadowed planes (`rocks_have_lit_and_shadowed_planes`).
- **Detail as the budget allows**, nearest rocks first: up to three more planes, a top face where the silhouette runs nearly level, a dark crevice down one ridge, and up to three lower facets (a crooked break across a plane, below it a shelf turned toward the light or an undercut turned away). A rock has 4 layers and up to 9 more; rocks stop at 88 layers in all, so the scene stays under 90.

**Woodland.** The far woods are clusters stratified along the width. Each is active with probability 0.15 + 0.8 × density, has a half-width of (0.06–0.24) × (0.6 + 0.8 × density), and stands 0.012–0.047 × (0.6 + 0.8 × density) tall on the waterline. The near woods are up to 3 clusters on the near shore toward the frame edge, each active with probability 0.1 + 0.9 × density, standing up to (0.1–0.35) × sky space × (0.5 + 0.7 × density) tall. Density 0 leaves under 5 % woodland coverage, and density 1 always gives more than density 0 (tested). Shrubland (round 6) is a rounded dome, `1 − (2s − 1)²` across the stand, with clumps from 0.4 to 1.0 of its height, not a flat-topped hedge. Canopy edges (task 07): each crown has its own height, and about one crown in seven is an emergent tree 1.35× taller and one in seven a young one at 0.65×. That is decided by a second hash of the crown index, so it adds no random draws, and shrubs vary half as much. Far woods tuck their base 0.006 under the waterline. Near woods stand on the near shore, and `woodland_stands_on_land` checks both at density 0.1, 0.5 and 1.

## Meanders (`high-vantage`, task 06 round 3)

The user pointed out that rivers cannot zigzag with sharp angles, because erosion and flow round them off. The valley's water now follows a meander:

- **Centreline:** `centre + amplitude × (0.4 + 0.6 t) × wave(phase + bends × t)` (fractions of the width, nearness `t` from 0 at the horizon to 1). `wave` is two parabolic half-waves, continuous in value and slope: a sine stand-in built from exact arithmetic. The composition draws centre 0.4–0.6, amplitude 0.12–0.22, 1.0–1.8 bends over the view, and a phase.
- **Width:** a half-width of 0.05 + 0.1 t, so the channel widens toward the viewer.
- **Spurs:** each spur grows from the bank the channel swings away from (the inside of the bend) and reaches that bank's edge (±0.03). The tips therefore line up along smooth banks.
- **Rounded tips:** the last 30 % of each spur closes on an elliptical cap, and the water line curves gently (fBm, ±10 %).
- **Continuous banks (user round 4):** slits of water opened between spurs, so the river now has two banks. Each is a land layer from the frame edge to the channel's edge, from where the river leaves the lake (half the first spur's nearness) to the frame's bottom, just behind all spurs. The lake's shore swings in to become the bank. The banks are built along the depth of the view, so each is a y-monotone polygon. The spurs are ridges standing on the banks, and water shows only inside the channel. Tested by `river_banks_never_break`: below the outlet, the frame's side edges are land on every row, and the test fails without the banks. Spur-top tree bands are omitted in river valleys, since the banks and spurs are one mixed forest.
- **Light on the valley:** spurs keep their facing light and the light pool. The painter fades it to neutral toward each spur's foot, where it meets the neutral banks, so hills read without steps.
- **Weights:** `high-vantage` is now 40 % of compositions, following the user's favourites (rounds 3–4).
- **Tests:** `meanders_are_smooth` and `valley_spurs_have_blunt_tips`.

## Sky, light and scale (05b)

- **Clouds.** 75 % of skies are *dramatic*. A dramatic sky gets a storm deck with a probability of 65 % (calm skies 15 %). The deck's lower edge is at 12–30 % of the sky space, lumpy, and lifts by 40–80 % inside a break 0.3–0.7 wide near the focal summit. Every sky has 1–3 cumulus banks 0.25–0.7 × the width (capped at 1.8 canvas units) wide, with billows 0.1–0.22 across. Cloud outlines use `CLOUD_STEP` = 1/120.
- **Light pool.** This is a sun break: a disc of radius 0.25–0.5 on the primary summit, or (for `high-vantage`, 60 %) in the valley. Each layer's `shade` is scaled between `1 − 0.45 s` (outside, in cloud shadow) and `1 + 0.4 s` (inside), weighted by how much of the layer lies inside the pool. The strength `s` is 0.5–0.9 in dramatic skies and 0.1–0.25 otherwise. Like everything in the scene, it depends only on the seed and form, never on paint settings.
- **Scale.** Trees are the ruler. Far trees are scaled by template. Near woods are capped at 0.35 × the summit's rise, so they can never rival the mountain. Spur trees grow with nearness, from 0.003 to 0.033 × (0.6 + 0.8 × density). Whenever density > 0, at least one far wood always exists.

## Noise algorithm and limits

`noise::Fbm` is 1D value noise summed over octaves:

- **Lattice:** a SplitMix64-finalizer hash of the integer lattice index and a per-octave seed, mapped to [−1, 1). Each octave also has a hashed fractional offset, so kinks from different octaves never align.
- **Interpolation:** quintic fade `6t⁵ − 15t⁴ + 10t³` (C² continuous, rounded) blended toward linear interpolation (C⁰, straight segments meeting at kinks, i.e. facets) by an `angular` weight. Form profiles use `faceting` as the weight.
- **Octaves:** wavelength `base / 2ᵏ`, amplitude `gainᵏ`, stopping before the wavelength drops below `MIN_WAVELENGTH` = 1/160 canvas units (about three pixels at the 960 px interaction preview), with at most 8 octaves. The result is normalized by the amplitude sum, so it lies in [−1, 1].
- **Continuity:** every profile is a continuous function of canvas x evaluated on the shared grid, and the grid covers the frame plus the margin. Endpoints fall outside the frame, so no silhouette ends inside the picture.

**Coherence at preview size (tested):** on the massif's silhouette, no step between neighbouring samples exceeds 0.03 canvas units, and total variation stays under 2.5 × the width. Measured over 1,500 scenes, the maxima were 0.013 and 1.54.

## Form controls

| Control | Effect (structure only) |
| --- | --- |
| `form.faceting` (Form) | rounded, eroded masses ↔ angular planes: dome vs straight or concave summit flanks; soft vs sharp saddles; quintic vs linear noise (curved vs straight silhouette segments); 1 vs 2 facets per face and softer vs stronger plane shading; curved vs straight creases; domed vs polygonal rocks |
| `form.relief` | low rolling hills ↔ high steep ridge: summit height, flank steepness, detail amplitude, foothill and framing-ridge height |
| `form.woodland_density` | how many woodland regions are active and how large they are |

Edge softness, washes and opacity are **not** here. They belong to `painting.*` and cannot move geometry, because the generator never reads them (tested through a recipe).

## Aspect ratio and resolution

- **Resolution:** the scene depends on the reduced aspect ratio only. 3840×2160, 7680×4320 and 1920×1080 give the same checksum (tested).
- **Aspect ratio recomposes, intentionally.** Horizontal placements are fractions of the width, but summit spacing, sizes and detail wavelengths are in short-side units. So a wider frame shows more of the range and a taller one more sky and foreground. Heights are fractions of the sky space, which depends on the horizon (higher in wide frames), with a `MAX_SUMMIT` cap. The same text in portrait or square is related to the landscape version (same template, summit order, light and shore side) but is not a crop of it. The task 13 export dialog must present an aspect change as a recomposition.

## Reproducibility

Generators use only `+ − × ÷`, `sqrt`, `floor`, comparisons and integer hashing in `f64`, followed by one rounding to `f32`. A test scans `lakeshore.rs` and `noise.rs` for transcendental calls. Geometry checksums are therefore identical on every platform (tier 1): three are frozen in `scene::lakeshore::tests::checksums_are_frozen` and checked by portable CI on Linux, Windows and macOS. `TestCard` moved to version 1 because the checksum covered `shade`, to 2 for `plant`, and to 3 (task 07) for the light side.

## Debug views and contact sheets

`pigment_gpu::DebugRenderer` has three views. All use the same coverage rule as the CPU rasterizer and the shared tile loop:

- **`Flat`:** neutral grays from a debug value per role, modulated by `shade`, lightened with depth. Use it for value and depth planes.
- **`Regions`:** a fixed color per role, modulated by `shade`, with a 1-pixel outline wherever the front-most layer changes. Use it for region topology.
- **`LayerIds`:** the raw front-layer index. For tests only.

```text
pigment-prose contact-sheet --out SHEET.png [--aspect W:H] [--view flat|regions]
    [--cell PX] [--cols N] [--variation V]
    [--passages FILE | --passage ID --variations N | --sample N]
    [--faceting F] [--relief R] [--density D] [--adapter NAME]
```

The command writes the grid PNG and `SHEET.txt`: per cell, the passage id, variation, template, mirroring, layer and vertex counts, geometry checksum and visible coverage by role.

Measured on the RTX 4070 Ti (Vulkan), for one debug cell of `shore-a` (40 layers, 10,383 vertices) including generation and readback: 13.7 ms at 960 px wide, 26.7 ms at 1920 px and 84 ms at 3840 px. The shader tests every vertex of every layer whose bounding box contains the pixel. **Task 06 must not copy that per-pixel loop into the painting renderer.** Use per-row crossing tables or rasterized coverage masks.

## Tests

| What | Where |
| --- | --- |
| Corpus × 6 aspect ratios, and 120 seeds × 3 aspect ratios at the corners of the form space: simple polygons, sky at the back, nothing uncovered, minimum visible coverage (sky 10 %, mountain 1.5 %, water 5 %, shore 1 %, rocks > 0, woodland > 0 when density > 0), no role over 70 % | `lakeshore::tests::{corpus_scenes_are_valid_in_every_orientation, a_larger_seed_sample_is_valid_at_extreme_forms}` |
| Layer, vertex and plane bounds; silhouette coherence | `counts_stay_within_bounds`, `silhouettes_stay_coherent` |
| Same inputs give the same scene; frozen checksums | `same_recipe_same_scene`, `checksums_are_frozen` |
| Palette, atmosphere, painting settings and the paint-detail stream leave the checksum unchanged | `appearance_and_paint_seed_never_touch_geometry` |
| Pixel size keeps the scene; aspect ratio recomposes it | `pixel_size_keeps_the_scene_and_aspect_recomposes_it` |
| Variation changes the composition; all templates occur in the corpus | `variation_changes_the_composition` |
| Form morphs without recomposing; relief, faceting and density effects | `form_morphs_the_same_composition`, `relief_raises_the_mountain`, `faceting_changes_shape_and_planes`, `woodland_density_scales_the_woodland_regions` |
| Visible depth order by role (clouds directly in front of the sky) | `layers_run_back_to_front_by_role` |
| Vista templates beat the classic ones on their own devices (rise and scale for the tower, planes and expanse for the high vantage); dramatic skies stage more contrast (05b) | `vista_templates_score_higher_on_their_devices`, `dramatic_light_stages_contrast` |
| Exact arithmetic only | `generators_use_only_exact_arithmetic` |
| Noise values frozen, band-limited, normalized | `noise::tests` |
| Rasterizer and simplicity checker | `raster::tests` |
| GPU `LayerIds` equals the CPU rasterizer; debug views are byte-identical tiled and single-tile | `crates/pigment-gpu/tests/gpu_hardware.rs` (hardware; 0 of 710,400 pixels differed) |

Commands: `scripts/check.sh` (portable), `scripts/gpu-tests.sh` (hardware). The contact-sheet commands used for the evidence are in [evidence/scene-05/README.md](evidence/scene-05/README.md).
