# Seasons and biomes (task 16)

A painting has a time of year. The **Season** control moves it around a continuous yearly cycle, not a set of presets. It changes what is on the land (snow, leaves, grass, flowers) and never the land itself: the mountains, the water, where every tree stands, and the geometry checksum are the same all year.

Code: `pigment_core::season` (the model), `settings::SEASON` (the control), `recipe` schema 2 (persistence), and the seasonal paths in `crates/pigment-gpu/src/paint.wgsl`. Review: [round 8](visual-review/round-08/README.md).

## The year coordinate

- `season.year` is a number in `[0, 1]`. **0 and 1 are the same moment, midwinter.** 0.25 is the spring equinox, 0.5 midsummer and 0.75 the autumn equinox. Values are wrapped (`season::wrap`), so `at(0) == at(1)` exactly, and the painter gives the same bytes for both (tested).
- **Default: 0.5, midsummer.** That is the approved painting: at midsummer every channel is neutral, and the seasonal code is skipped. When the seasons were added, all 47 task 25 images still repainted byte for byte. The round 8 review then changed the mountainsides and rock forms (renderer and generator v3), so the approved midsummer images are now [baseline-16](visual-review/baseline-16/README.md) (`approved_recipes_repaint_identically_after_save_and_load`).
- **Labels** divide the year into twelfths centred on their times: midwinter, late winter, early spring, spring, late spring, early summer, midsummer, late summer, early autumn, autumn, late autumn, early winter (`season::label`). The slider's value text uses them, for example "0.62 (late summer)".
- It is a northern-hemisphere, mid-latitude mountain year, **stylized**, not simulated: there is no weather, day length, latitude or ecology model.

## Channels

A biome's `SeasonProfile` gives keyframes of a `SeasonState`, eight channels in `[0, 1]`, kept apart so each can be reasoned about:

| Channel | Meaning | Where it paints |
| --- | --- | --- |
| `snow` | how far down the terrain seasonal snow reaches: 0 is only the permanent snow on high peaks, 1 reaches the valley floor | mountains, distant ranges |
| `ground_snow` | snow lying on the ground | meadows, shores, forest floor, rock tops |
| `tree_snow` | snow on crowns | crown tops (conifers hold the most) |
| `leaf` | how much deciduous canopy is in leaf (0 bare) | deciduous crowns |
| `autumn` | how far deciduous leaves have turned (0 green) | deciduous crowns |
| `fresh` | spring's pale new green | deciduous crowns, grass |
| `dry` | grass cured to straw | meadows |
| `bloom` | wildflowers and blossom, relative to midsummer (1) | meadow flowers, flowering trees |

**Interpolation** (`SeasonProfile::at`): between two neighbouring keyframes, and across the wrap from the last to the first, each channel is a smoothstep blend. So every channel is periodic, C¹ continuous (with zero slope at each keyframe, including across the wrap), bounded by the keyframes it blends, and exactly the keyframe's value at its time. Tests: `season::tests` (continuity sampled at 20,000 points, wrap values and slopes, bounds, exact keyframes).

### The alpine profile (`season::ALPINE`)

The approved lakeshores and wooded valleys:

| t | Keyframe | snow | ground | tree | leaf | autumn | fresh | dry | bloom |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0.00 | midwinter | 1.00 | 1.00 | 0.70 | 0.00 | 1.00 | 0.00 | 1.00 | 0.00 |
| 0.14 | late winter | 0.85 | 0.70 | 0.30 | 0.00 | 1.00 | 0.00 | 0.80 | 0.00 |
| 0.23 | early spring | 0.50 | 0.05 | 0.00 | 0.55 | 0.00 | 1.00 | 0.15 | 0.80 |
| 0.35 | late spring | 0.25 | 0.00 | 0.00 | 0.95 | 0.00 | 0.60 | 0.00 | 1.00 |
| 0.50 | midsummer | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 1 (neutral) |
| 0.62 | late summer | 0.00 | 0.00 | 0.00 | 1.00 | 0.08 | 0.00 | 0.40 | 0.55 |
| 0.75 | autumn | 0.10 | 0.00 | 0.00 | 0.90 | 0.80 | 0.00 | 0.65 | 0.10 |
| 0.86 | late autumn | 0.45 | 0.15 | 0.10 | 0.35 | 1.00 | 0.00 | 0.90 | 0.00 |

A long snowy winter; a late spring with the snow line retreating up the slopes; a short, dry late summer; a bright autumn; and the first snow on the heights before all the leaves are down. The keyframe times are chosen so each named quarter reads as itself (the first spring keyframe is at 0.23, so 0.25 is already green).

## How each channel is painted

**Snow is derived from the terrain, never overlaid as noise.**

- On the massif and the distant ranges, the snow line comes down from the top as `snow` rises. It is measured as height on the landform (0 at its foot, 1 at its top), ragged with the terrain's own noise, and lower on faces turned from the light, which hold snow longer (`slope_snow`). On the massif it also lingers lower in the gullies ([mountainsides](painting.md#mountainsides-renderer-v3-review-round-8)). Above the treeline it lies as a sheet. Below it, it lies among the trees (a share of white, more as the crowns hold more), so the forest's structure still reads.
- **Monotone:** every snow cover is a fixed noise field against a threshold that falls as its channel rises. A pixel that is snow stays snow as winter deepens, so the landforms stay readable and the snow does not shimmer. Tested on the rock and distant ranges of three scenes from late summer to midwinter: no pixel ever becomes less snowy (`snow_only_grows_as_winter_deepens`).
- **Lying snow** (`ground_snow_cover`) comes and goes in patches and covers the ground completely at 1. It lies on meadows and shores, more thinly and in shadow on the forest floor between trees, and as ragged caps on rock tops once it is deep enough.

**Trees keep their places and change their clothes.** Every crown has a stable attribute, a hash of its own cell in the crown field, so each tree turns and drops its leaves at its own time, in its own colour, every year, and nothing is reseeded.

- **Autumn:** birches turn gold, copper beeches deep red, and the rest gold, orange or red.
- **Leaf fall:** bare branches (the palette's twig colour, with the bark tone for birches), and crowns cover less of the ground, so the snow or ground shows through.
- **Spring:** a fresh pale green.
- **Snow:** on the lit tops of crowns; conifers hold the most, bare branches hardly any, so a winter wood stays dark against the snow.
- **Colour changes keep lightness:** each point is recoloured at its own lightness (`recolor`), so crowns keep their lit and shaded sides and grass its strokes.
- **Conifers** stay green all year.

**Grass and flowers:** fresh spring green, then straw (the palette's dry-grass colour) from late summer; wildflowers and blossom fade out after midsummer.

**Water** is unchanged, and its reflections show the seasonal land they reflect.

## Precedence: season, palette and atmosphere

Each control owns its own part of the picture, so they never fight:

- **The palette owns the sky, clouds, light and every base colour.** The season never changes the palette choice or its intensity, and never touches sky or cloud pixels. Tested for both palettes and four seasons: no sky or cloud pixel away from the land changes (`the_sky_and_clouds_stay_the_palettes_in_every_season`).
- **Seasonal colours come from the palette too.** Autumn gold, orange and red, bare twigs and dry grass are palette entries, so Golden evening in autumn is that palette's autumn.
- **The Atmosphere slider owns the haze.** The season has no atmosphere channel and never changes haze; distance dissolves seasonal colours exactly as it does summer ones.
- **The season owns only the land's seasonal state:** snow, leaves, grass and flowers. Changing it never moves another slider (tested in the studio, `the_season_wraps_from_the_keyboard_and_leaves_palette_and_air_alone`).

## The control and persistence

- **Season** is a Main control in the Appearance group (`settings::SEASON`), after Wash / Gouache. Its ends are both midwinter and its value text names the season. It is **cyclic**: Page Up/Down and the arrow keys carry on past the ends (from the year's end to 0.01, from 0 to 0.99). Home and End go to 0 and 1, and Delete resets to midsummer. Changing it repaints without rebuilding the scene.
- **Recipe schema 2** adds `season.year` (required). Schema 1 files, the only format before seasons, load through one explicit migration: they are checked against the schema 1 table, then get `season.year = 0.5`, the midsummer they were painted in, so they paint exactly as before. Saving writes schema 2, and builds that read only schema 1 reject such a file with "recipe schema 2 is not supported". Tested: the 47 approved schema 1 recipes migrate, repaint byte for byte, and re-save as schema 2 with only the schema number and the season added ([seeds-and-recipes.md](seeds-and-recipes.md#recipe-schema-2)).
- The season is an appearance setting, so it is part of the export snapshot and of every recipe, and has nothing to do with the prose seed.

## Biome registration and extension contract

The owner-approved task 17 scope is one registered **Alpine** profile covering the existing mountain lakeshores and wooded valleys. Prose remains non-semantic seed material: every input continues to generate this same landscape family. The profile is registered in `crates/pigment-core/src/biome.rs`; `BiomeId::Alpine` has the stable internal ID `alpine`. The profile collects the existing structure defaults, allowed vegetation, the existing lake water form, base palette/atmosphere defaults and `season::ALPINE`. It does not duplicate or fork the scene generator, season model, paint shaders or export path.

There is no biome picker while Alpine is the only fully implemented profile. Existing schema-1 and schema-2 recipes stay unchanged, and their current output/season defaults are preserved; loading them resolves to the sole registered Alpine profile. Do not add a redundant biome field to those recipes. When a second biome is approved, expose a selector and add an explicit stable biome ID in a new recipe schema with tested migrations that map old recipes to `alpine`; unknown IDs must fail clearly. Switching biome may regenerate structure. Changing season, palette, atmosphere or paint handling must not move geometry. Keep structural defaults separate from paint-only overrides, and preserve user appearance settings on a biome switch unless a documented control is biome-specific.

A biome brings a `SeasonProfile`: a name, a `Cycle` and its keyframes over the same eight channels.

- **`Cycle::Temperate`** (the alpine profile): winter, spring, summer, autumn.
- **`Cycle::DryWet`** (jungle, task 20): a dry season and a wet season. Such a profile keeps `snow`, `ground_snow` and `tree_snow` at 0 (no generic snow in every biome) and moves `dry` (cured, dusty ground and canopy), `leaf` (dry-season leaf drop for deciduous species), `fresh` (the flush after the rains) and `bloom`. A wet-season channel (standing water, saturated greens) would be a new channel added then, with neutral 0, so existing profiles are unaffected.
- **Tundra** (task 19) is `Temperate` with a long `snow` season and a short `fresh`/`bloom` summer. **Desert** (task 18) may keep most channels neutral and use `bloom` for the brief flowering.

Rules for every profile:

- The keyframe at the biome's default season must be `SeasonState::NEUTRAL` if that biome's approved painting predates its seasons.
- Every channel stays in `[0, 1]`, and a profile never changes geometry or placement.
- A profile is selectable only after its structure, vegetation/effects, season response, palette and recipe behavior are implemented and tested. Today every scene is the alpine lakeshore.

### Desert implementation status

A preliminary, deliberately non-selectable desert profile is being authored in `crates/pigment-core/src/biome.rs`: rocky-desert defaults use low shrub density, an arid water capability, a warm mineral palette with cool shadows, and a restrained stylized wet/dry seasonal cycle. It is **not** in `biome::PROFILES`; the existing lakeshore generator, versioned recipe identity and UI still produce/select Alpine only. The profile must not be presented as a working biome or used to imply that changing palette alone creates desert structure. The seasonal values describe artistic appearance, not a climate simulation.

### Add-a-biome checklist

1. Add a stable `BiomeId` and complete `BiomeProfile` in `crates/pigment-core/src/biome.rs`; include structure defaults/ranges, permitted `scene::Plant` forms and density, supported water form, palette and atmosphere defaults, and a `SeasonProfile`. Keep values within `settings::CONTROLS` and season channels within `[0, 1]`.
2. Extend the shared scene-generation inputs in `crates/pigment-core/src/scene/` and existing shared GPU painting paths only where a new structural/effect capability needs it. Do not copy `LakeshoreGenerator`, `paint.wgsl`, the season system, or export services.
3. Add profile validation and tests in `crates/pigment-core/src/biome.rs`; add deterministic generator/geometry, season and painting tests in the existing module test suites. Demonstrate that paint/season edits preserve geometry and that biome changes reproduce from their recipe and seed domains.
4. Add reviewed visual fixtures under `docs/visual-review/` and verify recipe round trips, migrations, tiled/export parity and supported GPU backends. Obtain explicit owner approval of the visual quality before advertising the biome.
5. Only after all of the above passes, add it to `biome::PROFILES` and show the accessible picker in `crates/pigment-studio/src/`. For the second biome, version recipe identity in `crates/pigment-core/src/recipe.rs`, update `docs/seeds-and-recipes.md` and examples, and test that legacy recipes resolve to `alpine` while unknown IDs are rejected.

## Cost

Measured on sample 15 at 1920 × 1080, single runs, render and readback: 3.9 ms at midsummer, 5.3 ms at midwinter and 4.4 ms in autumn on the RTX 4070 Ti. The Intel iGPU took 109–150 ms, within the noise of single runs, near its 150 ms settled budget where the adaptive settled cap takes over. At midsummer the seasonal code is skipped by a uniform branch.

## Tests

| What | Where |
| --- | --- |
| Periodic, C¹, bounded, exact keyframes, midsummer neutral, labels | `pigment_core::season::tests` |
| The control spec, value text, invalidation (repaint only) | `settings::tests`, `controls::tests`, `invalidate::tests`, `ui_tests::each_control_invalidates_only_its_stage_and_paint_keeps_geometry` |
| Geometry unchanged in every season | `lakeshore::tests::appearance_and_paint_seed_never_touch_geometry` |
| Schema 2 serialization, schema 1 migration, per-schema tables, range checks | `recipe::tests`, `pigment-io/tests/recipes.rs` |
| Midsummer is the default, 0 and 1 paint the same, every season differs | `gpu_hardware::midsummer_is_the_default_and_the_year_wraps` |
| Palette precedence on sky and clouds | `gpu_hardware::the_sky_and_clouds_stay_the_palettes_in_every_season` |
| Snowfall mask stability | `gpu_hardware::snow_only_grows_as_winter_deepens` |
| Tile seams for snow and every other seasonal effect at 2K (tiles of 333 and 512 px) | `gpu_hardware::every_season_is_identical_tiled_and_single_at_high_resolution` |
| Cyclic keyboard control; season leaves palette and haze alone | `ui_tests::the_season_wraps_from_the_keyboard_and_leaves_palette_and_air_alone` |
| The approved midsummer images repaint exactly | `pigment-io/tests/gpu_export.rs::approved_recipes_repaint_identically_after_save_and_load` |
| Schema 1 recipes migrate | `pigment-io/tests/recipes.rs::schema_1_recipes_migrate_to_midsummer_schema_2` |
