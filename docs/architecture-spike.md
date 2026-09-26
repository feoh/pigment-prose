# Architecture spike: GPU painting and tiled rendering (task 02)

**Status:** complete on Linux. **Recommendation:** use Rust + wgpu with compute-shader painting evaluated per tile, following the conventions below. Windows (Direct3D 12) and macOS (Metal) are **unverified**: nothing in this document was run on them.

The prototype is throwaway code in [`spikes/gpu-tiles/`](../spikes/gpu-tiles/). It is not the product renderer and its image is not a visual-quality candidate. Visual approval belongs to task 08. Every number below comes from [`spikes/gpu-tiles/artifacts/results.txt`](../spikes/gpu-tiles/artifacts/results.txt), produced by one run of `spikes/gpu-tiles/run-spike.sh` on 2026-09-26.

## Test machine (measured, not assumed)

| Item | Value |
| --- | --- |
| GPU | **NVIDIA GeForce RTX 4070 Ti**, 12282 MiB (the project brief says "RTX 4070"; the installed card is the Ti variant) |
| Driver / API | NVIDIA 615.71.09, Vulkan instance 1.4.357, device API 1.4.351 |
| Other adapters | Intel Graphics (RPL-S) iGPU, Mesa 26.2.3 Vulkan; NVIDIA over GL; Mesa llvmpipe (software, via GL) |
| CPU / OS | Intel Core i9-14900K; CachyOS Linux, kernel 7.2.7 |
| Toolchain | rustc/cargo 1.92.0, wgpu 30.0.1, png 0.18.1, pollster 1.0.1 |

## What the prototype does

1. **CPU (f64, seeded):** `scene.rs` builds a structural description from domain-separated SplitMix64 streams (`"terrain"`, `"paint-detail"`): four angular ridge layers with 24 vertices each, per-vertex facet slopes and tints, and 29 tapered stroke polylines. One of those strokes is a diagnostic diagonal that crosses every tile row and column. The inputs are numeric seeds only; no prose is read, hashed or logged.
2. **GPU (WGSL compute, 8×8 workgroups):** three passes per tile.
   - `masks_main` evaluates the hard coverage masks that need neighbourhood effects (the mid-ridge silhouette and the wash footprint) over the tile *plus its apron*.
   - `blur_h_main` does a horizontal Gaussian over the same extended region.
   - `composite_main` does the vertical Gaussian and then paints each interior pixel: paper grain, sky wash, four glazed landform layers with facet planes, a crisp-to-soft ridge edge, a translucent shadow wash with edge pooling, and opaque dry-brush marks. It writes sRGB-encoded RGBA8.
3. **Readback and export:** each tile is copied to a staging buffer, mapped, and placed into one horizontal band of rows. Each finished band streams into a `png` `StreamWriter`. The full image is never held on the GPU or in CPU memory. The PNG gets an sRGB chunk and **no text chunks**: no prose, no attribution, no identifying metadata.

The prototype covers all four required primitives: layered angular landforms, one translucent wash, a hard-to-soft edge, and scale-aware paper/pigment texture. See [`artifacts/preview-960x540-seed7.png`](../spikes/gpu-tiles/artifacts/preview-960x540-seed7.png).

## Recommendation

### 2D / 2.5D / 3D

**Recommended: a 2.5D layered procedural painting.** The CPU produces an ordered stack of depth planes (silhouettes, facets, vegetation masses, marks). Each plane has a depth used for atmospheric perspective and ordering. The GPU evaluates each output pixel as a pure function of whole-image coordinates plus a small set of neighbourhood passes. Reasons:

- The product is a fixed scenic painting, not an explorable world, so a 3D camera, meshes and lighting add cost without serving the brief.
- The art goals are interlocking color planes, selective edges and glazes. Those are expressed directly as planes, masks and compositing rules. A 3D render would still need a painterly reinterpretation afterwards.
- Pure per-pixel evaluation made tiled output **bit-identical** to single-image output (below). Rasterized 3D is also tileable (projection offset per tile), but derivatives, mip selection and screen-space effects add seam risks.

**Not prototyped:** a height-field or 3D terrain path. If task 05 needs more solid geology than silhouettes and facets can give, a per-column height-field raymarch in the same compute framework is the bounded next step. It must follow the same whole-image-coordinate rules. This is a rationale, not a measured result.

### CPU / GPU responsibilities

| CPU (deterministic, f64, versionable) | GPU (per-pixel, f32) |
| --- | --- |
| Text normalization and seeds (task 04), separate streams per domain | Evaluate coverage, facets, washes, marks and paper at each pixel |
| Composition: horizon, ridge vertices, shore, vegetation placement, stroke paths | Neighbourhood effects confined to a known finite support (blur, edge pooling) |
| Tile planning, apron size, memory budget, PNG streaming, progress and cancellation between tiles | sRGB encoding at the final store |

Structural decisions stay on the CPU, so recipes can be asserted structurally (vertex lists, stroke paths) without comparing pixels. This matches the brief's guidance to use structural assertions rather than cross-device byte equality.

### Color and alpha model

- Work in **linear RGB** in f32. Intermediates are stored as `rgba16float`.
- **Watercolor = transmittance glaze:** `c *= exp(-A * density)`, where `A = -ln(srgb_to_linear(pigment))` is the absorbance of a pigment that reads as `pigment` at density 1 on white. Glazes multiply, so layered washes stay luminous and paper shows through.
- **Gouache and opaque marks = premultiplied "over":** `c = color * a + c * (1 - a)`.
- Encode to sRGB **once**, at the final store. Output is 8-bit RGBA with an sRGB chunk. 16-bit PNG output is an option for task 09; the pipeline already carries more precision than 8 bits.

### Texture-scale convention

- **Canvas unit = the image's short side.** Placement, mark widths, blur radii (0.012 canvas units) and texture wavelengths are all defined in canvas units, so a mark covers the same fraction of the painting at 540p and at 16K.
- **Pixel footprint (1/short side) is used only for antialiasing and band-limiting.** Coverage edges are antialiased over one pixel. Each fbm octave fades out as its wavelength falls below about 2–3 output pixels, and faded octaves contribute their mean so overall value stays constant.
- **Evidence:** downsampling the 4K and 8K renders to 960×540 and comparing them with a direct 960×540 render gives PSNR 47.1 dB and 47.0 dB. The structure matches, and the differences are band-limited texture and antialiasing. The same canvas region at preview and 8K ([`scale-crop-540p.png`](../spikes/gpu-tiles/artifacts/scale-crop-540p.png), [`scale-crop-8k.png`](../spikes/gpu-tiles/artifacts/scale-crop-8k.png)) shows 8K adding ragged-edge and dry-brush detail rather than enlarged pixels.

### Finite-support effect strategy

1. Every neighbourhood effect declares its support radius in canvas units. The tile planner converts that to pixels (`ceil(radius * short_side)`) and adds the radii of chained passes to get the **apron**.
2. Tiles are rendered over interior plus apron. Procedural fields are evaluated inside the apron, *including beyond the image frame*: the image is a window onto an unbounded procedural canvas, so frame borders need no special case.
3. Only the interior is written out. The apron is recomputed per tile (overlap), never exchanged between tiles.
4. Effects with **unbounded** support (global value normalization, flood fills, fluid simulation, histogram matching) are not allowed in the tile pass. If they are needed, compute them once in a low-resolution global pre-pass at a fixed canvas resolution, then sample the result in each tile.
5. Apron overhead grows as tiles shrink. At 8K (radius 52 px), a 1024 tile does (1128/1024)² ≈ 1.21× the interior work, and a 256 tile does (360/256)² ≈ 1.98×. **Default to 1024–2048 px tiles** and use smaller tiles only to fit a memory budget.

### Tile-overlap risks (and how the spike checks for them)

| Risk | Mitigation / evidence |
| --- | --- |
| Apron smaller than the effect's support | Subtle, easy to miss by eye (max 9/255 here). Found only by exact reference comparison. Negative controls below. |
| Randomness or texture keyed to tile-local coordinates | Hashes take integer whole-image lattice coordinates. The `--tile-local-noise` control shows the failure. |
| Per-tile RNG state | The GPU never carries RNG state; every sample is a hash of global coordinates plus seed. |
| Workgroup/shared-memory algorithms whose result depends on tile layout | None used. Future reductions must not depend on tile origin. |
| f32 precision at very large coordinates | Integer pixel coordinates are exact up to 2²⁴. Verified bit-identical at 15360×8640; not tested beyond that. |
| Hardware texture sampling with derivatives or mips | Not used. Compute has no implicit derivatives; the analytic pixel footprint replaces them. |
| GPU watchdog (Windows TDR ≈ 2 s) on huge single dispatches | Each tile is its own small submission (tens of ms or less). Windows behaviour is unverified. |

### Backend feature portability

The spike uses **only WebGPU core**: compute shaders, `rgba16float` and `rgba8unorm` write-only storage textures, `textureLoad` from sampled textures, uniform and storage buffers, `copy_texture_to_buffer`, no optional features. A single texture is never used as storage and sampled within one dispatch, so each pass has its own bind group. `rgba16float` read-write storage is not core WebGPU and was deliberately avoided.

With `--default-limits` the device is created with WebGPU's portable defaults (`max_texture_dimension_2d = 8192`). 8K tiled output is bit-identical to the adapter-limits run. A 15360×8640 single tile is **refused with an actionable error**, while the same image tiled at 2048 px succeeds. Tiling makes output size independent of the GPU texture limit (this NVIDIA adapter reports 32768 and the Intel iGPU 16384).

| Backend | Status |
| --- | --- |
| Vulkan / NVIDIA 615.71.09 (RTX 4070 Ti) | **Verified**: all measurements |
| Vulkan / Intel Mesa 26.2.3 (iGPU) | **Verified**: runs; tiled output bit-identical within the device |
| GL / Mesa llvmpipe (software) | Produced a correct-looking 320×180 image with `--allow-software`, labelled software. Refused by default. **Not GPU evidence.** |
| Direct3D 12 (Windows) | **Unverified**: not run |
| Metal (macOS) | **Unverified**: not run |

Cross-device result: NVIDIA and Intel renders of the same 4K scene differ by **at most 1/255 on 7.8% of pixels** (PSNR 65.1 dB). This backs the brief's refusal to promise pixel identity across hardware, and it shows that tile-invariance holds *within* a device ([`crop-nvidia-vs-intel-diff32.png`](../spikes/gpu-tiles/artifacts/crop-nvidia-vs-intel-diff32.png)).

## Hardware selection and diagnostics

`gpu-tiles adapters [--gl]` lists every adapter per backend with its type, software flag, vendor/device ids, driver, and the limits relevant here (max 2D texture, storage buffer binding, max buffer, workgroup sizes, storage textures per stage). Selection ranks discrete > integrated > virtual > other > software, and `--adapter NAME` overrides the choice. On this machine it picks the RTX 4070 Ti on Vulkan over the Intel iGPU and over NVIDIA on GL.

A software adapter is anything reporting `DeviceType::Cpu` or named llvmpipe, lavapipe, SwiftShader or WARP. It is **refused by default** with installation hints for Linux, Windows and macOS, a pointer to `WGPU_BACKEND`, and the note that `--allow-software` runs it with a label. This was exercised against real Mesa llvmpipe exposed through GL (`LIBGL_ALWAYS_SOFTWARE=1`); no lavapipe Vulkan ICD is installed. Unknown `--adapter` filters fail with a pointer to `gpu-tiles adapters`.

## Seam comparison: method and result

**Method (authoritative):** render the same seed and size once as a single tile (`--tile 0`, whose apron also lies outside the frame) and again tiled. Compare the decoded RGBA8 bytes exactly: max channel difference, count of differing pixels, PSNR, and the **maximum distance of any differing pixel from a tile edge or image border**. For negative controls that last number should be at most the effect support, which shows the errors come from the apron and nowhere else. A reference-free "seam ratio" (the pixel step across each tile edge divided by the mean step at offsets 3–8 px) is also reported. It catches gross seams (ratio about 13 for tile-local noise) but **not** subtle apron errors (1.43 against a clean-image range of about 0.6–1.4). Treat the exact reference comparison as the gate.

**Results (seed 7 unless noted):**

| Case | Tile | Result |
| --- | --- | --- |
| 3840×2160, run twice | single | identical (repeatable) |
| 3840×2160 | 2048, 1024, **1000**, 512, **333**, 256, 128 | **identical**, 0 differing pixels (non-divisor tile sizes included) |
| 2160×3840 portrait | 512 | identical |
| 3000×3000 square | 700 | identical |
| 5001×7003 odd custom, seed 11 | 1000 | identical |
| 7680×4320 (8K) | 1024, 256 | identical (reference and 1024-tiled SHA-256 both `0cbb7612…`) |
| 8K, WebGPU default limits | 1024 | identical to the adapter-limits run |
| Intel iGPU 3840×2160 | 512 vs single | identical |
| **Negative:** apron 0 (blur radius 26) | 512 | differs: max 9, 19,036 px, all within **25 px** of an edge |
| **Negative:** apron 6 | 512 | differs: max 3, 8,094 px, all within **19 px** (= 26 − 6 − 1) |
| **Negative:** tile-local noise | 512 | differs: max 158, 7.5 M px, seam ratio 8.6 mean / 13.1 worst |

Crops at the tile-512 corner (2048, 1024), crossed by the diagnostic mark and a vertical tree mark:

- Reference and correct tiled output, byte-identical: [`seam-crop-reference.png`](../spikes/gpu-tiles/artifacts/seam-crop-reference.png), [`seam-crop-tiled512.png`](../spikes/gpu-tiles/artifacts/seam-crop-tiled512.png).
- Tile-local noise, with a visible ridge break: [`seam-crop-neg-localnoise.png`](../spikes/gpu-tiles/artifacts/seam-crop-neg-localnoise.png) and its ×32 diff.
- The zero-apron failure where the soft edge meets tile edges: [`apron-crop-neg-apron0.png`](../spikes/gpu-tiles/artifacts/apron-crop-neg-apron0.png) and [`apron-crop-neg-apron0-diff32.png`](../spikes/gpu-tiles/artifacts/apron-crop-neg-apron0-diff32.png). The error is almost invisible in the painting and plain in the diff.

## Benchmarks (RTX 4070 Ti, Vulkan, release build)

All times are wall clock. "Render + readback" means from submit until the mapped rows are copied out. Device and pipeline creation (`device_init`, about 60–160 ms including WGSL compilation) is reported separately and happens once per process.

**Preview** (`bench-preview`: scene rebuild + render + readback, no PNG):

| Size | First frame | Warm median | Warm p95 | n |
| --- | --- | --- | --- | --- |
| 960×540 | 8.5 ms | 0.99 ms | 1.17 ms | 60 |
| 1280×720 | 8.8 ms | 2.16 ms | 2.55 ms | 60 |
| 1920×1080 | 11.9 ms | 5.16 ms | 5.52 ms | 60 |
| 3840×2160 single tile | 25.2 ms | 21.9 ms | 27.3 ms | 30 |
| 3840×2160, 1024 tiles | 23.8 ms | 22.3 ms | 23.8 ms | 30 |
| 1280×720 on Intel iGPU (comparison) | 52.5 ms | 29.6 ms | 29.9 ms | 30 |

**Final export** (`render` to PNG with `Compression::Fast`, written to NVMe):

| Size | Tile | Tiles | Our GPU allocations | CPU band | Render + readback | PNG stream | Total |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 3840×2160 | single | 1 | 194.6 MiB | 31.6 MiB | 15.6 ms | 47.9 ms | 72 ms |
| 3840×2160 | 512 | 40 | 6.9 MiB | 7.5 MiB | 19.9 ms | 47.5 ms | 74 ms |
| 7680×4320 | single (reference only) | 1 | 778.6 MiB | 126.6 MiB | 77.6 ms | 160.8 ms | 258 ms |
| 7680×4320 | 2048 | 12 | 102.7 MiB | 60.0 MiB | 77.3 ms | 174.1 ms | 260 ms |
| 7680×4320 | **1024** | 40 | **27.4 MiB** | 30.0 MiB | 76.6 ms | 158.2 ms | **241 ms** |
| 7680×4320 | 512 | 135 | 7.8 MiB | 15.0 MiB | 85.8 ms | 173.1 ms | 265 ms |
| 7680×4320 | 256 | 510 | 2.5 MiB | 7.5 MiB | 111.5 ms | 192.8 ms | 311 ms |
| 15360×8640 (16K), default limits | 2048 | 40 | 109.7 MiB | 120.0 MiB | 375.0 ms | 545.6 ms | 927 ms |

The 8K PNG is 16.8 MB and the 16K PNG 47.2 MB for this synthetic scene. A full painting will compress worse and shade more slowly; treat these as pipeline overheads, not product performance.

**GPU memory.** "Our GPU allocations" is the exact sum of the textures and staging buffer the renderer creates. Whole-device use sampled every 20 ms with nvidia-smi (desktop session included):

| Job | nvidia-smi above baseline |
| --- | --- |
| 64×64 context-only job | +214 MiB |
| Every tiled 8K and 16K run | +214 MiB |
| 8K single tile | +1002 MiB |

Tiled runs are indistinguishable from an empty context at nvidia-smi's granularity, which is consistent with wgpu's device-memory allocator reserving blocks up front. **An 8K export needs no full-image GPU allocation**: at 1024 px tiles it allocates 27.4 MiB, against 126.6 MiB for one RGBA8 copy of the image.

**Observations**

- At 8K the export is dominated by PNG encoding (about 160 ms of 241 ms), not the GPU. The tile loop is sequential (submit → wait → copy). Double-buffering readback and moving encoding to another thread are easy wins for task 09 and were not measured here.
- Tiles below 512 px cost more from apron overdraw and per-submission overhead (256 px: 111 ms against 77 ms of render + readback at 8K).
- Cancellation and progress fit naturally *between tiles*. Each tile submission is short (the whole 16K image took 375 ms of render + readback over 40 tiles), so the cancel latency is one tile. Not implemented in the spike.

## Fallback options if the candidate fails elsewhere

1. **Another wgpu backend:** on Windows, Vulkan if D3D12 misbehaves. wgpu's GL backend enumerated NVIDIA here, but compute shaders on GL need GL 4.3 / GLES 3.1 and were **not tested** on the GL hardware adapter.
2. **Native API behind the same shader contract:** ash/Vulkan (plus MoltenVK on macOS) or Metal directly, with WGSL translated by naga. This costs more engineering, but the pure per-pixel design keeps it contained.
3. **CPU renderer:** out of scope here and reserved for task 23. The design helps (pure functions of coordinates, finite supports), and llvmpipe did execute the unmodified shader at 320×180. Its timing was not stable across runs (roughly 280–330 ms uncached, 4.4 ms in the recorded run, probably because of Mesa's shader cache) and says nothing about a purpose-built CPU renderer.

## Dependencies and licenses (spike only)

The 77 resolved Linux dependencies (`cargo metadata --filter-platform x86_64-unknown-linux-gnu`) are all permissively licensed: MIT and/or Apache-2.0 for most; also Zlib (`foldhash`, `zlib-rs`), ISC (`libloading`), BSD-2-Clause option (`zerocopy`), Unlicense option (`termcolor`), and Unicode-3.0 (`unicode-ident`). No copyleft. No bundled assets, fonts or reference images. This is a survey, not the distribution audit (task 15). Windows and macOS pull extra platform crates that were not reviewed.

## Limitations and open questions

- The spike's image is a technical probe with no art-direction review. It must not be presented for visual approval.
- Direct3D 12, Metal, and any GPU other than the two above: **unverified**.
- No desktop UI, surface or swapchain; preview-in-window latency belongs to task 11.
- Device-lost or out-of-memory recovery, and GPU timestamp queries, were not exercised. Timings are wall clock.
- VRAM figures are whole-device samples with allocator granularity, not per-allocation measurements.
- The reference-free seam ratio is weak for subtle errors. Automated regression (task 14) should use exact single-versus-tiled comparison on the same device.
- The spike's domain-separated seed streams are a stand-in for the task 04 seed contract, not that contract.

## Reproduce

```sh
cd spikes/gpu-tiles
./run-spike.sh        # builds, renders to out/ (gitignored), writes artifacts/results.txt and crops
rm -rf out            # large renders (up to ~50 MB each) are not kept
```
