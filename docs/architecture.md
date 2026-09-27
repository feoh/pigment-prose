# Architecture and module contracts (task 03)

This document freezes the interfaces that tasks 04–23 build on. Where it says **exists**, the code is in the repository, tested, and runs today. Where it names a later task, that task implements the item at the stated path with the stated signature. Decision record: [ADR 0001](decisions/0001-renderer-and-desktop-shell.md). Spike evidence: [architecture-spike.md](architecture-spike.md). Build and test commands: [CONTRIBUTING.md](../CONTRIBUTING.md).

**Status of what runs today.** The portable core (including text seeding, the recipe format and the lakeshore scene generator, tasks 04–05), the GPU smoke path and the scene debug views are implemented. There is no painting, no PNG export and no desktop UI yet. The GPU paths were verified on Linux (Vulkan) with the RTX 4070 Ti and the Intel iGPU; see [docs/evidence/](evidence/). Portable CI (tests only, no GPU) passes on Linux, Windows and macOS. GPU rendering on Windows and macOS is unverified.

## Stack

- **Rust 1.98** (pinned in `rust-toolchain.toml`), edition 2024, one Cargo workspace. `Cargo.lock` is committed. Dependencies are declared once in `[workspace.dependencies]` with exact minimum versions (`wgpu = "30.0.1"`), and the lock file pins the full graph. Build with `--locked`.
- **wgpu 30.0.1**, WebGPU core features only, device limits `wgpu::Limits::default()`.
- **eframe/egui 0.36** desktop shell (task 11), sharing the renderer's device.
- **Licenses (survey, not the task 15 audit):** the 72 third-party crates resolved for Linux (`cargo metadata --filter-platform x86_64-unknown-linux-gnu`) are all permissive: MIT and/or Apache-2.0, with Zlib, ISC, BSD-2-Clause, Unicode-3.0 and Unlicense options. No copyleft. egui/eframe and rfd, added in task 11, are MIT OR Apache-2.0 and MIT.

## Workspace layout

| Path | Kind | Responsibility | Status |
| --- | --- | --- | --- |
| `crates/pigment-core/` | lib | Portable contracts: text gate, seeds, settings and controls, frame and coordinates, scene, recipe, requests and sinks, tile planning, job model, invalidation, capability data, errors. **No GPU, window or filesystem dependency**, so its tests run in CI on Linux, Windows and macOS. | exists |
| `crates/pigment-gpu/` | lib | wgpu adapter selection and capability reports, `GpuContext` (device, queue, device-lost and error scopes), the shared tile loop (`tiled.rs`), renderers implementing `pigment_core::request::Renderer`. | exists (smoke and scene debug renderers) |
| `crates/pigment-gpu/tests/gpu_hardware.rs` | test | Hardware GPU suite, `#[ignore]` by default. | exists |
| `crates/pigment-cli/` → binary `pigment-prose` | bin | Diagnostics: `gpu-info`, `gpu-smoke`, `contact-sheet` (task 05, debug views to PNG). Task 14 adds `bench`. | exists |
| `crates/pigment-io/` | lib | PNG `TileSink` with temp-file and atomic finalize (09), recipe files and the document model (10). | task 09 creates |
| `crates/pigment-studio/` → binary `pigment-studio` | bin | eframe/egui desktop app (11–13). | task 11 creates |
| `spikes/gpu-tiles/` | separate Cargo project | Task 02 throwaway spike, excluded from the workspace. | frozen |

Dependency direction: `pigment-core` ← `pigment-gpu` ← `pigment-io` ← `pigment-studio`. `pigment-cli` depends on `core` and `gpu` (and on `io` once it exists). `pigment-core` never depends on the others.

## Contract cross-reference

| Concept (roadmap name) | Rust item | File | Status / owner |
| --- | --- | --- | --- |
| Text input gate | `text::check_source`, `text::MAX_SOURCE_BYTES` (1 MiB) | `crates/pigment-core/src/text.rs` | exists |
| Normalized text | `text::NormalizedText` (length-only `Debug`) | same | exists (`text::normalize`, `nfc-lf-utf8/1`, task 04) |
| SeedBundle | `seed::SeedBundle`, `TextDigest` (256-bit, hex), `Variation(u32)`, `Domain`, `StreamSeed(u64)` | `crates/pigment-core/src/seed.rs` | exists (`TextDigest::of`/`from_source`, `SeedBundle::derive`, `StreamSeed::rng` → `Rng`, `pigment-seed/1`, task 04) |
| Recipe | `recipe::Recipe`, `RecipeVersions`, `RecipeSeed` | `crates/pigment-core/src/recipe.rs` | exists (`from_json`, `to_canonical_json`, `validate`, `seeds`, `version_notices`, `SCHEMA`; task 04); files → 10 |
| Scene | `scene::Scene`, `SceneKey`, `SceneLayer` (with structural `shade`), `LayerRole` (incl. `Mountain`, `Cloud`), `CanvasPoint`, `SceneGenerator` | `crates/pigment-core/src/scene/mod.rs` | exists (plus diagnostic `TestCard` v1) |
| Scene generator | `scene::lakeshore::{LakeshoreGenerator, Composition, Template}` | `crates/pigment-core/src/scene/lakeshore.rs` (+ `noise.rs`) | exists (task 05, [spec](scene-generation.md)) |
| CPU reference raster | `scene::raster::{front_layers, role_coverage, empty_fraction, is_simple}` | `crates/pigment-core/src/scene/raster.rs` | exists |
| Awe metrics | `scene::metrics::{measure, AweMetrics}` | `crates/pigment-core/src/scene/metrics.rs` | exists (task 05b; [rubric](art-direction.md#awe-metrics)) |
| Scene debug views | `pigment_gpu::{DebugRenderer, DebugView}` (`Flat`, `Regions`, `LayerIds`) | `crates/pigment-gpu/src/debug.rs` + `debug.wgsl` | exists |
| Painting renderer | `pigment_gpu::PaintRenderer`, coverage index `pigment_gpu::coverage` | `crates/pigment-gpu/src/paint.rs` + `paint.wgsl`, `coverage.rs` | exists (task 06, [spec](painting.md)); woodland, rocks and water → 07 |
| Compositing model | `composite::{glaze, wash, over, edge_blend}` | `crates/pigment-core/src/composite.rs` | exists (task 06); GPU parity checked by the hardware suite |
| Palettes | `palette::{Palette, palette, PALETTES, LAKESHORE, GOLDEN_EVENING}` | `crates/pigment-core/src/palette.rs` | exists (task 06) |
| PaintingSettings | `settings::PaintingSettings` (+ `Appearance`, `PaletteSettings`, `AtmosphereSettings`) | `crates/pigment-core/src/settings.rs` | exists; effects → 06/07 |
| Form settings | `settings::FormSettings` | same | exists; effects → 05/07 |
| Control specification | `settings::CONTROLS`, `ControlSpec`, `Channel`, `Group` | same | exists; UI → 12 |
| Frame / export dimensions | `frame::Frame`, `AspectRatio`, `CanvasExtents`, `CanvasMapping`, `UHD_4K`, `UHD_8K` | `crates/pigment-core/src/frame.rs` | exists |
| RenderRequest | `request::RenderRequest`, `RenderTarget`, `RenderPurpose`, `RequestId`, `RequestIds` | `crates/pigment-core/src/request.rs` | exists |
| RenderResult | `request::RenderReport`, `RenderOutcome`, `RenderTimings`; pixels flow through `TileSink` | same | exists |
| Output sink | `request::TileSink`, `MemorySink` | same | exists; `PngSink` → 09 in `crates/pigment-io/src/png_sink.rs` |
| Renderer | `request::Renderer` trait | same | exists; `SmokeRenderer` exists, `PaintRenderer` → 06/07 |
| Cancellation | `job::CancelToken` | `crates/pigment-core/src/job.rs` | exists |
| Progress | `job::Progress`, `Phase`, `ProgressSink` | same | exists |
| Preview queue / stale results | `job::Mailbox`, `Submitted`, `PreviewState` | same | exists; wired by 11 |
| Invalidation | `invalidate::Invalidation::between` | `crates/pigment-core/src/invalidate.rs` | exists |
| Tiling and halo | `tiles::TilePlan`, `Tile`, `TilePolicy`, `Support`, `apron_pixels`, `TileCostModel` | `crates/pigment-core/src/tiles.rs` | exists |
| Capability reporting | `capability::GpuCapabilities`, `AdapterReport`, `LimitsReport`, `AdapterPolicy`, `rank` | `crates/pigment-core/src/capability.rs` | exists |
| GPU device | `pigment_gpu::GpuContext` (`new`, `check_alive`, `scoped`) | `crates/pigment-gpu/src/context.rs` | exists |
| Adapter selection | `pigment_gpu::adapter::{enumerate, select, report, is_software}` | `crates/pigment-gpu/src/adapter.rs` | exists |
| Errors | `error::{RenderError, ValidationError, Problem, TextError, RecipeError, MalformedKind, SinkError}`, `tiles::TilePlanError` | `crates/pigment-core/src/error.rs` | exists |
| Versions | `version::{RECIPE_SCHEMA_VERSION, GENERATOR_VERSION, RENDERER_VERSION, NORMALIZATION_ID, SEED_ALGORITHM_ID}` | `crates/pigment-core/src/version.rs` | exists |

## A request from recipe to result

Written against the frozen API. Lines marked `// task NN` call functions that the named task adds. Everything else compiles today.

```rust
use std::sync::Arc;
use pigment_core::{frame::UHD_8K, recipe::Recipe, request::*, job::*, tiles::TilePolicy,
                   capability::AdapterPolicy, scene::SceneGenerator, seed::*, text};

// 1. Prose → seeds. The prose is never logged or stored unless the user opts in.
text::check_source(&prose)?;                                   // exists: empty/blank/oversize
let normalized = text::normalize(&prose)?;                     // exists: NFC + LF
let recipe = Recipe::new(TextDigest::of(&normalized), UHD_8K); // exists
recipe.validate_values()?;                                     // exists
let seeds = recipe.seeds();                                    // exists: SeedBundle::derive

// 2. Seeds + form + aspect → immutable scene (built once, shared by preview and export).
let scene = Arc::new(LakeshoreGenerator.generate(&seeds, &recipe.form, recipe.frame.aspect())?); // exists

// 3. Snapshot request. Later edits to controls cannot reach it.
let ids = RequestIds::new();
let request = RenderRequest {
    id: ids.next(),
    purpose: RenderPurpose::Export,
    scene,
    seeds,
    appearance: recipe.appearance(),
    target: RenderTarget { width: recipe.frame.width, height: recipe.frame.height,
                           policy: TilePolicy::default_export() },
};

// 4. Render tile by tile into a sink; cancellation is checked between tiles.
let ctx = Arc::new(pigment_gpu::GpuContext::new(&AdapterPolicy::default())?);
let renderer = pigment_gpu::PaintRenderer::new(ctx)?;          // tasks 06–07 (SmokeRenderer today)
let mut sink = pigment_io::PngSink::create(&destination)?;     // task 09 (MemorySink today)
let cancel = CancelToken::new();
let report: RenderReport = renderer.render(&request, &cancel, &mut |p: Progress| ui.show(p), &mut sink)?;
match report.outcome {
    RenderOutcome::Completed => { /* sink.finish() has renamed the temp file into place */ }
    RenderOutcome::Cancelled { tiles_done } => { /* sink.abort() removed the partial file */ }
}
```

**Runnable today:** `pigment-prose gpu-smoke` ([crates/pigment-cli/src/main.rs](../crates/pigment-cli/src/main.rs)) runs steps 2–4 with `diagnostic_seeds`, `TestCard`, `SmokeRenderer` and `MemorySink`. It builds real `RenderRequest`s under four tile policies and checks the resulting `RenderReport`s. `pigment-prose contact-sheet` runs steps 1–4 for real prose with `LakeshoreGenerator` and `DebugRenderer` in place of the painting renderer.

## Coordinates and units

- **Pixels:** origin at the top-left, +x right, +y down. Pixel `(i, j)` has its centre at `(i + 0.5, j + 0.5)`. Tiles use whole-image pixel coordinates. An apron region may have negative coordinates or run past the frame, because the canvas continues beyond it.
- **Canvas units:** the frame's **short side is 1.0**. The origin is the frame's top-left, +y down. `CanvasExtents` is `(aspect, 1)` for landscape and `(1, aspect)` for portrait. `CanvasMapping::pixel_centre` maps pixels to canvas **per axis**. A preview whose rounded pixel size only approximates the aspect ratio is therefore stretched by under one pixel, never cropped.
- **Pixel footprint** (`CanvasMapping::pixel_footprint`) is for antialiasing, band-limiting and converting supports to pixels (`support_pixels`, rounded up). It must never place structure.
- **Scene key:** the scene depends on the *reduced integer aspect ratio* (`AspectRatio`), never on pixel size. 3840×2160 and 7680×4320 share a scene. Changing the aspect ratio (portrait, square, custom) **recomposes** the scene. Task 13's export dialog must say so rather than presenting it as a resolution change.
- **Frame bounds (provisional):** each edge 64–16384 px. `long/short ≤ 4`. 16384 is just above the largest size measured (15360×8640, task 02); raise it only with task 09 evidence. Integer pixel coordinates stay exact in `f32` far beyond that (2²⁴).
- **Depth:** `SceneLayer::depth`, 0 = nearest and 1 = farthest. Layers are stored back to front and depth never increases along the list.

## Color, alpha and output encoding

- **Working space:** linear-light RGB in `f32`. Intermediates are `rgba16float`.
- **Watercolor washes** are transmittance glazes, `c *= exp(-A · density)` with `A = -ln(srgb_to_linear(pigment))`. Glazes multiply, so paper light survives and washes stay luminous.
- **Gouache and opaque marks** use premultiplied "over": `c = color·a + c·(1 − a)`. Any intermediate that stores color with coverage stores it **premultiplied**, so soft edges never show dark fringes.
- **Paper is opaque.** Final alpha is always 1. `TileSink` receives RGBA8 rows with alpha 255.
- **sRGB encoding happens once**, in the final compute pass, which writes `rgba8unorm`. `*-srgb` formats are not storage-capable in WebGPU core, so the encode is explicit in WGSL (`srgb_encode`).
- **PNG (task 09):** 8-bit **RGB** (the constant alpha is dropped), an `sRGB` chunk (perceptual intent) and **no text, time or EXIF chunks**. No prose, no paths, no user identity, no watermark. 16-bit output is optional and decided by task 09; the pipeline already carries more than 8 bits.

## Texture, brush sizing and halo accounting

- Everything that should look the same at any resolution is specified in **canvas units**: mark widths, stroke lengths, texture wavelengths, blur and bleed radii. `painting.mark_scale` (0.5–2.0) multiplies mark and texture sizes. It never moves structure.
- **Band-limiting:** each noise octave fades out as its wavelength drops below 2–3 output pixels, and a faded octave contributes its mean. A 540p preview and a downsampled 8K export therefore agree in value structure. Task 02 measured 47 dB PSNR between them.
- **Random samples** are hashes of *integer whole-image lattice coordinates* plus a stream seed. The GPU carries no RNG state. Tile-local coordinates are forbidden; the task 02 negative control shows the resulting seam.
- **Halo accounting:** every neighbourhood pass declares a `Support { pass, radius }` in canvas units, and `Renderer::supports(&Appearance)` returns the chained list. The apron is `apron_pixels(mapping, supports)`: the sum over passes of `ceil(radius / footprint)`. Supports may depend on settings. Edge looseness contributes up to `settings::MAX_EDGE_BLEED_RADIUS` = 0.02 canvas units, which is 87 px at 8K.
- **Unbounded effects** (global normalization, flood fill, fluid simulation, histogram matching) are forbidden in the tile pass. If needed, compute them in a **global pre-pass** at a fixed canvas resolution, independent of output size, and sample the result in every tile.

## Portable GPU features and formats

- Device: `Features::empty()`, `Limits::default()` (WebGPU defaults: `max_texture_dimension_2d` 8192, `max_buffer_size` 256 MiB). Nothing may depend on one vendor's larger limits. Tiling makes the output size independent of the texture limit.
- Formats: `rgba16float` and `rgba8unorm` **write-only** storage textures, sampled textures read with `textureLoad` (no filtering), uniform and read-only storage buffers, `copy_texture_to_buffer` with 256-byte row alignment. No read-write `rgba16float` storage (not core). No texture is used as storage and sampled in the same dispatch.
- Compute workgroups are 8×8 (64 invocations, within every WebGPU limit).
- Per-tile submissions stay short, well under the Windows TDR (~2 s). Task 21 verifies this on Direct3D 12.

## Tiling and memory budget

- **Preview:** `TilePolicy::Single`, one tile at preview size. A 1920×1080 test-card tile allocates 33.6 MB.
- **Export:** `TilePolicy::Budget { gpu_bytes, host_bytes }` with defaults of **256 MiB** of renderer-owned GPU allocations and a **256 MiB** host band buffer. The planner takes the largest of 2048/1024/512/256 px that fits both budgets and the device texture limit. At 8K with the spike's cost model that is 2048 px (102.7 MiB). With a 28 MiB budget it is 1024 px (27.4 MiB, the task 02 measurement, reproduced by `tiles::tests`).
- **Regression:** `TilePolicy::Fixed { edge }` forces a tile size, including non-divisors such as 333, for seam tests.
- **Out of memory:** resource creation runs inside `GpuContext::scoped`, which turns an allocation failure into `RenderError::OutOfMemory`. Task 09 retries with half the GPU budget until the 256 px floor, then reports the error. It never silently falls back to a lower resolution.
- **Host memory:** one band (`image_w × tile_h × 4` bytes) plus the PNG encoder's buffers. The full image is never held in memory or on the GPU. Exception: `MemorySink` is for previews and tests only.
- **wgpu allocator:** nvidia-smi showed no increase over an empty context for tiled 8K/16K runs (task 02). Budgets govern *our* allocations, not driver reservations.

## Preview lifecycle (tasks 11–12)

- **Threads:** the egui UI thread never waits on GPU work. One **render worker thread** owns the renderer and loops on `Mailbox::next()`. `wgpu::Device`/`Queue` are `Send + Sync` on native targets, so the UI and the worker share one `Arc<GpuContext>`. Scene generation runs on the worker too (CPU, milliseconds for the spike's scene sizes).
- **Queue bound:** `job::Mailbox` has one pending slot. A new submission **replaces the pending job** and **cancels the running one**, so at most one job runs and one waits, however fast the user types or drags (`rapid_submissions_stay_bounded`).
- **Stale results:** request ids increase monotonically (`RequestIds`). The UI shows a result only if `PreviewState::accept(id)`, meaning it is newer than what is on screen. A late old result is dropped. While `PreviewState::is_pending()` is true, show a quiet "rendering…" state. The controls always show the newest values.
- **Cancellation granularity:** between tiles, and before the first tile. A preview is one tile (under 25 ms at 4K for the spike's shader), so for previews the effective mechanism is *superseding before start*. A single dispatch cannot be interrupted. Export cancel latency is one tile.
- **Debounce:** prose edits wait **300 ms** after the last keystroke, because they re-derive seeds and rebuild the scene. Slider drags are **not debounced**: every value is submitted and latest-wins coalescing absorbs the rate. Window/preview resizes wait **100 ms**.
- **Preview size:** while a slider is being dragged, render at a long edge of **960 px** ("interaction preview"). After 150 ms without input, re-render at the preview area's physical pixel size, capped at a long edge of **1920 px** ("settled preview"). Both use the document's scene (`Frame::fit_within`) and never stretch to the window's aspect.
- **Exports** use a separate single slot and are never superseded by previews. They take an immutable `RenderRequest` snapshot, and one export runs at a time (task 13).

## Controls and invalidation

Source of truth: `settings::CONTROLS`. Ranges are inclusive. The UI clamps by construction. Recipe/API input outside a range is **rejected**, never clamped ([load policy](seeds-and-recipes.md#load-policy)).

| Key | Label | Channel | Group | Range | Default | Low end → high end | Effect task |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `form.faceting` | Form | Structure | Main | 0–1 | 0.55 | rounded, eroded masses → angular, faceted planes | 05 |
| `painting.edge_looseness` | Edge Looseness | Paint handling | Main | 0–1 | 0.4 | controlled, crisp edges → soft, bleeding edges | 06 |
| `painting.wash_gouache` | Wash / Gouache | Paint handling | Main | 0–1 | 0.25 | translucent washes → opaque gouache body color | 06 |
| `atmosphere.haze` | Atmosphere | Appearance | Main | 0–1 | 0.4 | clear distance → hazy, dissolving distance | 06 |
| `palette.intensity` | Color intensity | Appearance | Main | 0–1 | 0.6 | muted → vivid | 06 |
| `form.relief` | Relief | Structure | Advanced | 0–1 | 0.5 | low rolling hills → high, steep ridge | 05 |
| `form.woodland_density` | Woodland density | Structure | Advanced | 0–1 | 0.5 | open, sparse groves → dense wooded masses | 07 |
| `painting.mark_scale` | Mark scale | Paint handling | Advanced | 0.5–2 | 1.0 | fine marks → broad marks | 06 |
| `painting.granulation` | Granulation | Paint handling | Advanced | 0–1 | 0.3 | smooth pigment → strongly settled pigment | 06 |
| `painting.paper_grain` | Paper grain | Paint handling | Advanced | 0–1 | 0.3 | smooth hot-press → rough tooth | 06 |
| `palette.id` | Palette | Appearance | Main | `lakeshore`, `golden-evening` | `lakeshore` | — | 06 |

**Form**, **Edge Looseness** and **Wash / Gouache** are three separate channels. Faceting changes geometry only. Looseness changes edge and boundary behaviour (bleed radius, soft/hard selection) only. Wash/gouache changes opacity and material character only. No control may secretly drive another channel. A coupled "style" slider is not allowed. Seasons and biomes are **not** controls until tasks 16–17, so no placeholder UI.

**Invalidation** (`invalidate::Invalidation::between`, tested):

| Change | Seeds | Scene | Paint |
| --- | --- | --- | --- |
| prose (new digest), variation, normalization/seed algorithm | ✓ | ✓ | ✓ |
| any `form.*`, frame aspect ratio, generator version | | ✓ | ✓ |
| any `painting.*`, `palette.*`, `atmosphere.*`, renderer version | | | ✓ |
| frame pixel size at the same aspect ratio; `source_text` kept or dropped | | | |
| preview area resized (not in the recipe) | | | ✓ (new target size, same scene) |

"Another Composition" increments `Variation`. That changes the composition, terrain and vegetation streams (`Domain::uses_variation`) and leaves paint detail unchanged, so the paper and pigment texture stays familiar while the layout changes.

## Errors

All errors are structured enums with `Display` text that is safe to log. **No error value contains prose or recipe contents.** Text errors carry byte counts only. `NormalizedText`'s `Debug` prints only its length (tested).

| Type | Raised by | UI treatment |
| --- | --- | --- |
| `TextError::{Empty, WhitespaceOnly}` | `text::check_source` | inline prompt "enter some prose"; nothing is rendered or saved |
| `TextError::TooLong` | same | inline message with the byte limit |
| `ValidationError { field, problem }` | settings, frame, scene and recipe validation | name the field; the recipe is not applied (task 10 keeps the current document) |
| `RenderError::{NoAdapter, SoftwareOnly, NoAdapterMatches, DeviceRequest}` | `GpuContext::new` | blocking diagnostics screen with driver hints; a software adapter is never called hardware |
| `RenderError::TilePlan`, `OutOfMemory` | renderer | export: retry with a smaller budget (09), then an actionable message |
| `RenderError::DeviceLost` | `GpuContext::check_alive` | recreate the context; tell the user the preview was reset |
| `RenderError::Gpu` | validation/internal wgpu errors | bug report text; never shown as the user's fault |
| `RenderError::Sink(SinkError { kind: Io/DiskFull/Encode })` | sinks | show the destination and reason; the partial file is removed (09) |
| `RenderOutcome::Cancelled` | not an error | quiet status |

## Versioning and compatibility

- `RECIPE_SCHEMA_VERSION` = 1. A recipe with a **newer** schema is rejected with a clear message. An older schema loads only through an explicit, tested migration. No invented migrations ([migration limits](seeds-and-recipes.md#versions-and-migration-limits)).
- `GENERATOR_VERSION` and `RENDERER_VERSION` start at **0 = pre-approval**: fixtures and images may change without a bump until the task 08 visual gate. After approval they become 1. From then on, **any** change to scene checksums for the same key bumps `GENERATOR_VERSION`, and any intentional pixel change on the same device bumps `RENDERER_VERSION`.
- The app ships exactly **one** generator and one renderer. It does not keep old versions alive. A recipe whose recorded versions differ opens with a visible notice ("made with generator vN; this version may compose/paint differently"). The recorded versions update only when the user saves.
- `NORMALIZATION_ID` (`nfc-lf-utf8/1`) and `SEED_ALGORITHM_ID` (`pigment-seed/1`) name the algorithms. Changing either one creates a new identifier, never a silent change.

## Reproducibility tiers

| Tier | What | Guarantee | How it is checked |
| --- | --- | --- | --- |
| 0 | Text → normalized bytes → digest → stream seeds | exact on every platform and build | frozen vectors in `fixtures/seed-vectors.json`, reproduced by an independent Python reference; portable CI on Linux, Windows and macOS ([spec](seeds-and-recipes.md)) |
| 1 | Seeds + form + aspect → scene geometry (`geometry_checksum`) for one `GENERATOR_VERSION` | exact on every platform | exact arithmetic only in generators: `+ − × ÷`, `sqrt`, comparisons, or a pure-Rust `libm`; never platform `sin`/`exp`. Checksum fixtures in CI. Frozen: the `TestCard` v1 checksum `22572651a5ccf378` and three lakeshore checksums |
| 2 | Pixels on one device, driver, backend and build, including tiled vs single tile | byte-identical | hardware suite and `gpu-smoke`: verified on NVIDIA and Intel Vulkan (tasks 02, 03) |
| 3 | Pixels across devices, backends or drivers | **within a measured tolerance, never identical** | per-backend baselines (tasks 14, 21, 22) |

Tier 3 measurements so far: task 02's painting shader, NVIDIA vs Intel on Vulkan, differed by at most 1/255 on 7.8% of pixels (PSNR 65.1 dB). Provisional cross-device acceptance: **PSNR ≥ 50 dB and max channel difference ≤ 4/255**, to be replaced by measured per-backend values. Direct3D 12 and Metal: **unmeasured**.

## Responsiveness targets (provisional)

These are **targets, not results**. They are derived from the task 02 pipeline overheads and this task's smoke timings on the RTX 4070 Ti, with headroom for a real painting shader, which will be much heavier. Task 14 measures against them and records any miss as a blocker or an accepted limitation.

| Scenario | Measured so far | Target (p95 unless noted) |
| --- | --- | --- |
| Interaction preview, 960 px long edge, render + readback | spike painting shader: 0.99 ms at 960×540 | ≤ 33 ms |
| Settled preview, ≤ 1920 px long edge | spike: 5.16 ms at 1080p; test card: 1.4 ms at 720p | ≤ 150 ms |
| Prose edit → settled preview visible | — | ≤ 300 ms debounce + ≤ 250 ms |
| UI frame time while rendering | — | never blocked by render work (worker thread) |
| 4K export incl. PNG | spike: 72 ms | ≤ 5 s |
| 8K export incl. PNG | spike: 241 ms | ≤ 20 s |
| Export cancel latency | one tile (spike: ≤ 10 ms per 2048 tile at 16K) | ≤ 250 ms |

## Downstream assignments

| Task | Entry points and files |
| --- | --- |
| 04 seeds and recipe schema | **Done.** Spec: [seeds-and-recipes.md](seeds-and-recipes.md). `text::normalize`; `TextDigest::of`, `SeedBundle::derive`, `StreamSeed::rng` (xoshiro256\*\*); `Recipe::from_json`/`to_canonical_json`; vectors in `fixtures/seed-vectors.json` from `scripts/seed-vectors.py`. CPU generators (05, 07) draw from `seeds.stream(Domain::…).rng()`, one generator per domain |
| 05 composition and landforms | **Done.** Spec: [scene-generation.md](scene-generation.md); evidence: [evidence/scene-05/](evidence/scene-05/README.md). `scene/lakeshore.rs` (`LakeshoreGenerator`), `scene/noise.rs`, `scene/raster.rs`; `SceneLayer::shade`, `LayerRole::Mountain`; `pigment-gpu/src/debug.rs` (views) over the shared `tiled.rs` loop; `pigment-prose contact-sheet` |
| 06 color planes and washes | `pigment-gpu/src/paint.rs` + `paint.wgsl` (`PaintRenderer: Renderer`) on `tiled::drive`; palettes in `pigment-core/src/palette.rs`; control effects per the table above. Map `SceneLayer::shade` to plane value/temperature. **Done:** coverage uses per-layer row bins of edges sorted by right-most x (`coverage.rs`), exact against brute force; timings in [painting.md](painting.md#measured-linux-vulkan-2026-09-27) |
| 07 woodland, rocks, water | same renderer; placements are part of `Scene` (vegetation stream), never generated on the GPU per tile |
| 08 visual gate | `docs/visual-review/`, contact sheets from `pigment-prose contact-sheet` |
| 09 tiled PNG export | new `crates/pigment-io/src/png_sink.rs` (`PngSink: TileSink`, temp file + atomic rename, abort deletes); OOM retry around `Renderer::render`; export job in `pigment-io/src/export.rs` |
| 10 recipe persistence | `crates/pigment-io/src/recipe_file.rs`, `document.rs` (dirty state, path); privacy per `Recipe::source_text` |
| 11 shell and preview | new `crates/pigment-studio/` (eframe/egui, `WgpuSetup::Existing` with `GpuContext`); render worker on `job::Mailbox`; `PreviewState`; diagnostics panel from `GpuCapabilities` |
| 12 controls | `pigment-studio/src/controls.rs` generated from `settings::CONTROLS`; `Invalidation::between` decides what reruns |
| 13 export UI | `pigment-studio/src/export_dialog.rs`; `Frame::new` validation; progress from `job::Progress` |
| 14 qualification | `scripts/check.sh`, `scripts/gpu-tests.sh`, `crates/*/tests/`, `docs/qualification.md` |
| 15 Linux package | `packaging/linux/`, third-party notices, `docs/user-guide.md` |
| 23 CPU study | `docs/cpu-fallback-decision.md`; any CPU path implements `request::Renderer` and must report `software_adapter: true` |

## Known limits of this task

- Only the diagnostic `TestCard` scene and `SmokeRenderer` exist. They are not art and must not be shown for visual review.
- Portable CI (`.github/workflows/ci.yml`) first ran after task 04 was pushed and passed on Linux, Windows and macOS. It has no GPU, so GPU rendering on Windows (Direct3D 12) and macOS (Metal) is still unverified.
- Device-lost handling is wired (`set_device_lost_callback`, `check_alive`) but was not exercised by a real device loss.
