# High-resolution PNG export (task 09)

`pigment_io::export_png` renders a painting at the requested pixel size, tile by tile, straight into a PNG. The whole image never has to fit on the GPU or in host memory, and the output is never an upscaled preview. The export dialog (task 13) calls this backend. Evidence: [evidence/export-09](evidence/export-09/README.md).

```rust
use pigment_io::{export_png, ExportSize, PngCompression};

let frame = ExportSize::Uhd8k.frame(scene.key().aspect)?;           // 7680×4320 for 16:9
let request = RenderRequest { purpose: RenderPurpose::Export, scene, seeds, appearance,
    target: RenderTarget { width: frame.width, height: frame.height,
                           policy: TilePolicy::default_export(), order: TileOrder::RowMajor },
    id: ids.next() };
let report = export_png(&renderer, &request, &destination, PngCompression::Fast, &cancel, &mut progress)?;
```

From the command line: `pigment-prose export --out FILE.png (--recipe R.recipe.json | --sample N | --passage ID) [--size 4k|8k|WxH]` (see [CONTRIBUTING.md](../CONTRIBUTING.md)).

## Sizes

- **Bounds:** each edge 64–16384 px, long/short ≤ 4 (`frame::Frame::validate`). 16384×9216 was exported and inspected on the RTX 4070 Ti. The bounds are what has been validated, not what memory allows: memory never limits the size, because tiles and bands are bounded by the budgets below.
- **Aspect ratio must match the scene exactly** (`export::validate_target`). The scene depends on the reduced aspect ratio, so a different ratio would stretch the painting or recompose it. `7680×4321` for a 16:9 scene is refused with `Problem::AspectMismatch`. To change the aspect ratio, change the document's frame. The scene is then rebuilt, and task 13's dialog must say so.
- **Presets:** `ExportSize::Uhd4k` and `Uhd8k` pick the largest frame of *exactly* the scene's ratio with a long edge of at most 3840 or 7680 px (`Frame::largest_with_aspect`): 16:9 → 3840×2160 and 7680×4320, 9:16 → 2160×3840, 1:1 → 3840×3840, 7:5 → 7679×5485. `ExportSize::Custom { width, height }` takes any valid size of that ratio. Portrait, landscape and square scenes all work the same way.
- **Before allocation:** sizes, aspect ratio and overflow-safe row and band arithmetic are checked first, then the destination directory (a temporary file is created there). An invalid request or an unwritable path fails before any GPU work.
- **No DPI:** no physical size is written. Print size is the user's choice.

## Tiling and memory

- **Plan:** `TilePlan` takes the largest of 2048/1024/512/256 px tiles that fits the device's texture limit and both budgets: `TilePolicy::default_export()` = 256 MiB for the renderer's GPU allocations and a 256 MiB host band. The apron is the sum of the renderer's declared supports at this resolution (`Renderer::supports`). For the painting renderer that is the loose-edge reach, 50 px at 8K and 105 px at 16K with default settings. Each tile is painted over interior + apron in whole-image coordinates, and only the interior is read back. Image edges get the same apron, because the canvas continues beyond the frame.
- **Global inputs** are computed once per request, never per tile. That covers the coverage index built from the scene, the horizon and summit metrics, and the rock bounds. There are no unbounded or normalizing passes.
- **Order independence:** tiles in a band may be visited in any order (`TileOrder`, part of `RenderTarget`). Bands always complete top to bottom so the PNG can stream. Tile size and order do not change a single byte: four 8K exports with 2048, 1000, 512 (budget-derived) and 333 px tiles, one in reverse order, produced the same file (SHA-256 `ce5eff39…`).
- **Host memory:** one band of RGBA rows (`image_w × tile_h × 4`: 60 MiB at 8K, 128 MiB at 16K), the readback staging buffer and the encoder's buffers. The full image is never held.
- **Out of memory:** if the renderer reports `RenderError::OutOfMemory` under a `Budget` policy, `export_png` retries with half the GPU budget, reusing the same scene. It stops when no tile edge down to 256 px would fit, and then returns the error. It never lowers the resolution. Every attempt is listed in `ExportReport::attempts`. `Fixed` and `Single` policies are not retried.

### Measured on Linux, RTX 4070 Ti (Vulkan, driver 615.71.09, wgpu 30.0.1), 2026-09-27

| Export | Tiles | Render + readback | PNG encode + write | Total export | File | Peak host memory (measured) | Renderer GPU allocations (cost-model estimate) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1024×576 | 1 | 1.6 ms | 3.3 ms | 13 ms | 0.7 MiB | 235 MiB | 9.2 MiB |
| 3000×4000 portrait | 2×2 of 2048 | 14.5 ms | 65 ms | 87 ms | 9.6 MiB | 256 MiB | 66.2 MiB |
| 3840×3840 square | 2×2 of 2048 | 19 ms | 77 ms | 104 ms | 9.8 MiB | 262 MiB | 66.9 MiB |
| 7680×4320 | 4×3 of 2048 | 61 ms | 203 ms | 272 ms | 22.1 MiB | 291 MiB | 67.2 MiB |
| 16384×9216 | 8×5 of 2048 | 227 ms | 704 ms | 939 ms | 72.3 MiB | 360 MiB | 70.9 MiB |

- *Total export* excludes device creation and shader compilation (about 90–180 ms, once per process) and scene generation (under 1 ms).
- *Peak host memory* is the whole process's `VmHWM`, driver libraries included. The 1024×576 export is the baseline: it grows by about one band, 57 MiB at 8K and 125 MiB at 16K.
- **GPU memory, measured:** `nvidia-smi` reported **200 MiB for the process at every size**, from 1024×576 to 16K. That is the driver's and allocator's reservation. It does not grow with the output, and it is too coarse to resolve the renderer's own allocations, so the right-hand column stays an estimate. Log: [gpu-memory-linux-2026-09-27.txt](evidence/export-09/gpu-memory-linux-2026-09-27.txt).
- Against the provisional targets (4K ≤ 5 s, 8K ≤ 20 s), exports are two orders of magnitude inside. PNG compression is most of the time.

## PNG encoding

- **8-bit RGB**, non-interlaced, deflate `Fast` (fdeflate) by default; `PngCompression::Balanced` gives smaller files and takes longer. Output is lossless either way. The pipeline encodes sRGB once, in the final shader pass, into `rgba8unorm`. 16-bit output is **not offered**: it would need a second output format through the whole tile path for no visible gain in a painted image. Revisit only on request.
- **Alpha:** none. The painting is opaque paper. The renderer's alpha is always 255 and is dropped.
- **Color:** one `sRGB` chunk, perceptual intent. No `gAMA`, `cHRM` or ICC profile: `sRGB` is the complete statement, and every mainstream decoder treats the data as sRGB.
- **Metadata:** the file holds exactly `IHDR`, `sRGB`, `IDAT`… and `IEND`. There are no text, time, EXIF or physical-size chunks, so no prose, recipe, local path, user identity, software name or watermark can appear. `assert_clean_metadata` in the hardware suite and `streams_rgb_srgb_with_no_metadata` check the chunk list. The recipe's `source_text` is never read by the export path.
- **End marker check:** the png crate writes `IEND` from a destructor and discards any error there. `PngSink` tracks the last 12 bytes written and refuses to finalize unless they are exactly `IEND`, so a failure there is still reported.

## Files, cancellation and errors

- **Atomic finalize** (`pigment_io::atomic::AtomicFile`): the PNG is written to a hidden temporary file beside the destination (`.NAME.<pid>-<n>.pigment-tmp`). `finish` flushes the file and syncs it to disk (`fsync`), renames it over the destination, then syncs the directory entry on Unix. `std::fs::rename` replaces atomically on Linux and macOS and uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows. An existing destination is replaced only by a complete file.
- **Cancel or error:** the temporary file is deleted and an existing destination is kept (`cancelled_export_leaves_the_old_file_and_no_partial`; the CLI log shows `--cancel-after 5` leaving the old file and no temporary). Dropping a `PngSink` without finishing also deletes the temporary, so a panic cannot leave a partial file. Cancellation is checked before every tile. Worst-case latency is one tile plus the encoding of a just-completed band: about 6 ms + 140 ms at 16K (227 ms / 40 tiles; 704 ms / 5 bands), about 5 ms + 70 ms at 8K. That is within the 250 ms target.
- **Errors** are structured and carry no prose: `RenderError::InvalidRequest` (size, aspect ratio, overflow), `TilePlan`, `OutOfMemory` (after retries), `DeviceLost`, `Gpu`, and `Sink(SinkError { kind: Io | DiskFull | Encode | Other })`. `StorageFull` and `QuotaExceeded` I/O errors become `DiskFull` even when the png crate re-wraps them. Sink details name the destination the user chose, for the UI; they never reach the image.
- **Overwrite confirmation** belongs to the UI (task 13), not this backend.
- **Known limits:** a replaced file's permissions are not copied; the new file gets the process defaults. On Windows the final rename fails, and the old file is kept, while another program holds the destination open. The temporary file needs as much free space as the finished PNG, in the destination's directory.

## Tests

| Test | Where | What it proves |
| --- | --- | --- |
| `tiled_png_export_equals_the_single_tile_render` | `crates/pigment-io/tests/gpu_export.rs` (hardware) | Decoded exports with 256 px row-major, 333 px reverse-order and budget-derived tiles (a single tile at this size) equal the single-tile render **byte for byte**, for 3 scenes (river valley, lake with rocks, framing ridges) × looseness 0/0.4/1, including wide washes, tile-crossing trees and rocks, and corners. Tolerance: exact, same device (tier 2). |
| `eight_k_export_keeps_the_preview_composition` | same | Real 7680×4320 export: decoded size, chunk list, and the 8×-downsampled image vs the 960×540 preview: **PSNR 34.0 dB** (floor 24 dB, as for `painting_agrees_across_resolutions`) |
| `custom_portrait_export_keeps_the_preview_composition` | same | 3000×4000 with 1000 px reverse-order tiles vs the 600×800 preview: **PSNR 32.1 dB** |
| `cancelled_export_leaves_the_old_file_and_no_partial` | same | Cancel after 2 tiles on the GPU |
| `impossible_budgets_and_bad_destinations_are_errors` | same | 1 MiB budget, missing directory, 3840×2161 |
| `export_writes_the_whole_image_in_any_tile_order`, `out_of_memory_retries_with_half_the_budget`, `out_of_memory_at_the_floor_is_reported_and_leaves_no_file`, `cancellation_deletes_the_partial_file_and_keeps_the_old_one`, `invalid_sizes_fail_before_anything_is_written`, `unwritable_destinations_fail_before_rendering`, `tile_plan_errors_are_not_retried`, `presets_keep_the_scene_aspect` | `crates/pigment-io/src/export.rs` (portable) | The job logic with a CPU stand-in renderer that simulates allocation failure above a budget (and, like `PaintRenderer`, fails before touching the sink) |
| `streams_rgb_srgb_with_no_metadata`, `abort_removes_…`, `dropping_an_unfinished_sink_cleans_up`, `misordered_or_short_bands_are_errors`, `unwritable_destinations_fail_at_create`, `a_full_disk_is_reported_as_disk_full`, `invalid_sizes_are_rejected_before_writing` | `crates/pigment-io/src/png_sink.rs` (portable) | Encoding, lossless round trip, cleanup, and a writer that fails like a full disk in the header, mid-stream and at `IEND` |
| `atomic::tests::*` | `crates/pigment-io/src/atomic.rs` (portable) | Replace-on-commit, discard, drop cleanup, Unicode names, a directory or missing directory as the destination, a read-only directory (Unix) |
| `reverse_order_visits_the_same_tiles_band_by_band`, `exact_aspect_presets` | `pigment-core` (portable) | Traversal order and preset sizes |

Disk-full is simulated with a failing writer. A real full filesystem was not exercised, because that would need root to mount a small one. Windows and macOS exports are **unverified** until tasks 21–22.
