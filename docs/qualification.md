# Qualification of the Linux MVP (task 14)

A repeatable sequence that qualifies the integrated studio, so that nothing rests on one good-looking image. It is run from a clean checkout, and each step reports **PASS**, **FAIL** or **SKIPPED** with its reason. A skipped step is coverage that did not run on that machine; it is never counted as a pass. Without a hardware GPU the sequence stops as **BLOCKED** (exit status 2).

Latest run: **2026-09-28, commit f9f261e, clean `git worktree` checkout, all 16 checks PASS, 1 SKIPPED** ([log](evidence/qualification-14/qualify-linux-2026-09-28.txt)).

## The sequence

From a clean checkout (the 2026-09-28 run used `git worktree add /tmp/pigment-prose-qual HEAD` and an empty `target/`):

```sh
scripts/qualify.sh [LOG_FILE]        # everything below, with a summary; default log in target/qualification/
```

It runs, in order:

| # | Step | Command | Needs |
| --- | --- | --- | --- |
| 1 | Portable checks: format, clippy, every unit and portable test, seed reference | `scripts/check.sh` | nothing (this is what CI runs on Linux, Windows and macOS) |
| – | Hardware present? | `pigment-prose gpu-info`, `gpu-smoke` | a hardware GPU; otherwise **BLOCKED** |
| 2 | Hardware GPU suite | `scripts/gpu-tests.sh` | a hardware GPU |
| 3 | Benchmark against the targets | `pigment-prose bench --strict`, and on a second, integrated adapter `bench --adapter NAME` plus the slow-GPU studio test | a hardware GPU; the second part is SKIPPED without one |
| 4 | The studio's real window, three ways | `pigment-studio --script` (undelayed, `--preview-delay-ms 1500`, `--lose-device-after 3`) | a display |
| 5 | Prose and path markers in logs and PNGs | CLI export of a recipe that keeps marker prose, into a folder and file named with markers | — |
| 6 | Offline | CLI export and `pigment-studio --script` under `unshare -rn` (a network namespace with no network) | unprivileged user namespaces; otherwise SKIPPED |
| 7 | Unsupported devices | `--adapter no-such-gpu` for the CLI and the studio; a software adapter with `--allow-software` | the software part is SKIPPED if none is installed |

Portable CI (`.github/workflows/ci.yml`) runs step 1 on Linux, Windows and macOS. It needs no NVIDIA device and runs no GPU test.

## What each part of task 14 is checked by

| Requirement | Checked by |
| --- | --- |
| Recipe → scene → preview → tiled PNG | `tests/qualification.rs` `approved_recipes_go_from_scene_to_preview_to_a_tiled_png` (hardware). Approved recipes in 16:9, 9:16 and 1:1 go through the studio's preview and export workers. Exact scene checksums; the tiled PNG (2×2 tiles) is byte-identical to one tile; PSNR 32.2–37.6 dB against the 960 px preview |
| Paint-only invariants, variation | `ui_tests::each_control_invalidates_only_its_stage_and_paint_keeps_geometry` (portable: every control, geometry checksums and worker scene reuse); `gpu_hardware::paint_settings_change_pixels_but_not_the_scene` |
| Source-free recipe round trip | `ui_tests::source_free_recipes_open_and_save_without_inventing_prose`, `document::tests::source_free_recipes_truly_omit_the_prose` |
| Export snapshot isolation | `ui_tests::the_studio_exports_a_real_8k_png_of_the_snapshot` (hardware, byte-identical with sliders moving), `ui_tests::sliders_moved_during_an_export_do_not_change_it`, and `--script`'s export stage |
| Exact expectations and per-backend baselines | Seeds: `fixtures/seed-vectors.json`, reproduced by `scripts/seed-vectors.py` on 3 OSes. Scenes: frozen checksums (`checksums_are_frozen`). Pixels on the baseline device: `approved_recipes_repaint_identically_after_save_and_load`, which requires all 47 approved images to match their FNV hashes on the RTX 4070 Ti/Vulkan and, on any other device, checks only that saving and reopening does not change the painting |
| Tile seams by difference image and boundary bands | `tile_boundaries_show_no_seams`: 333 px tiles (boundaries everywhere), looseness 1 (widest support). Max \|tiled − single\| is **0/255 in the 84,120 pixels within 4 px of a boundary** and 0/255 inside. The tiled image's own luminance step across boundary columns is ×0.82 of its step everywhere, so boundaries do not stand out. Also `painting_is_identical_tiled_and_single` and the 8K export identity tests |
| Brush and texture scale; same-aspect composition at two resolutions | `brush_and_texture_scale_hold_at_two_resolutions`: 1920 vs 3840 px downsampled, **PSNR 37.9 and 38.8 dB**, composition (16× down) 47.4 and 57.7 dB, texture energy (mean \|Laplacian\|) ×0.82 and ×0.71. Bound: 0.7–1.4, since the larger render keeps a little less fine texture after averaging. Also `painting_agrees_across_resolutions` and `eight_k_export_keeps_the_preview_composition` (34.0 dB) |
| Approved references preserved | Nothing in `docs/visual-review/baseline-25` (the current approved baseline, generator and renderer v2) or `baseline-08` (the task 08 record, v1) was regenerated or replaced. A mismatch means a `RENDERER_VERSION` bump and a new review, never a new golden to make a test pass |
| Cold/warm preview and 8K export on the RTX 4070 | `pigment-prose bench` (below) |
| Repeated drags and exports: stale frames, bounded queues, resource growth | `repeated_exports_with_previews_stay_bounded`: 25 exports with 414 preview requests alongside, previews bounded at one running plus one pending, newest delivered last, only finished files left, resident memory **−2.4 MiB** after warm-up. Also `a_request_storm_stays_bounded_and_ends_on_the_newest` (1000 requests, +1.7 MiB) and `--script` (never an older result after a newer one) |
| Prose or path in logs or PNG metadata | qualify.sh step 5: marker prose kept in a recipe, and marker folder and file names. Neither appears in the CLI log or the PNG, whose chunks are exactly `IHDR sRGB IDAT IEND`. Step 4: the prose the script types never appears in the studio's output. Also `kept_prose_never_reaches_the_exported_png` and the studio 8K test (prose, path and `$HOME` markers) |
| Malformed recipes | `recipe::tests` (strict loader: truncated, future schema, unknown/missing/duplicate keys, wrong types, out of range, digest mismatch), `recipe_file::tests`, `ui_tests::failed_opens_and_saves_are_reported_and_change_nothing` |
| Cancellation | `bench` (a 16K export cancelled at 50/150/300 ms), `ui_tests::cancelling_an_export_…`, `cancelled_export_leaves_the_old_file_and_no_partial` (GPU), `worker::tests` |
| Unsupported devices | Step 7: an unknown adapter is refused by the CLI, and the studio shows its explanation window and exits 1. `explain_surface_panics` for an adapter that cannot present (exit 2, task 11). Software adapters: SKIPPED here, see below |
| GPU loss | `--script --lose-device-after 3` (the device-loss state is shown and previews stop), `worker::tests::simulated_device_loss_…`. A real driver reset was not exercised |
| File write failures | `files::tests` (read-only file, missing folder), `ui_tests::a_failed_export_…` (disk full), `png_sink` tests (a writer failing like a full disk in the header, mid-stream and at `IEND`), `atomic::tests` |
| Works with networking unavailable | Step 6: a 4K CLI export and the whole studio script (typing, drags, an 8K export) pass under `unshare -rn`. This shows that nothing *needs* the network. It does not by itself prove every privacy property; those are the tests above |

## Performance against the targets

`pigment-prose bench`, 2026-09-28, default settings, 5 seeds × 9 warm runs (exports n=3). Targets are the provisional ones in [architecture.md](architecture.md#responsiveness-targets-provisional). Measured p95, or the maximum where n=3.

| Scenario | RTX 4070 Ti (Vulkan, driver 615.71.09) | Intel UHD iGPU (Vulkan, Mesa 26.2.3) | Target |
| --- | --- | --- | --- |
| Cold start: device + pipelines + first 1920×1080 preview | 101 ms (80 + 7 + 14) | 153 ms (43 + 9 + 101) | — (measured) |
| Interaction preview 960×540, render + readback | 0.79 ms | 21.8 ms | ≤ 33 ms |
| Settled preview 1920×1080 | 3.15 ms | 81.7 ms | ≤ 150 ms |
| Settled preview 3840×2160 | 13.7 ms | **316.6 ms, missed** | ≤ 150 ms |
| Prose edit → settled 1920×1080 (after the 300 ms debounce) | 5.2 ms | 87.2 ms | ≤ 250 ms |
| 4K export incl. PNG | 62 ms | 310 ms | ≤ 5 s |
| 8K export incl. PNG | 208 ms | 1.20 s | ≤ 20 s |
| Export cancel → stopped and cleaned up (16K, cancelled at 50/150/300 ms) | 90 ms | 40 ms | ≤ 250 ms |

In the studio's real window (`--script`, RTX 4070 Ti, KDE Plasma Wayland at 2×): request → shown median 10.1 ms, p95 20.2 ms. The longest UI frame gap is 13–53 ms while previews render, during an 8K export, and with a simulated 1.5 s render. An 8K export while a slider is dragged throughout takes 400–499 ms.

**Deviation, fixed:** on the Intel iGPU a 3840 px settled preview misses the 150 ms budget. The cap is a task 12 change: it was raised from 1920 px so the painting fills high-DPI displays. The studio now measures its settled previews and, when one goes over budget, lowers its settled cap for the session to the size predicted to take two thirds of the budget, never below 1920 px (`preview::adapt_settled_cap`). Measured through the studio on the iGPU (`a_slow_gpu_lowers_the_settled_cap_and_then_fits_the_budget`), a 3600 px settled preview took 297 ms. The cap became 2089 px, and the next five settled previews took at most **92 ms**. The bench row still reports the raw 3840 px miss, because it measures the renderer, not the studio's policy.

**Accepted limitations:** a 50 ms cancel landing during an export's setup and first tile takes up to 90 ms to stop (still within 250 ms). The studio's GPU memory was measured in a 1360×860 pt window (262 MiB), not in a maximised window on a 4K-class display.

## Art direction

The ten-passage contact sheets were not regenerated for review, because nothing about the painting has changed since the task 08 gate. `GENERATOR_VERSION` and `RENDERER_VERSION` are still 1, and all 47 approved images (the ten passages in three shapes plus the round 6 scenes) repaint **byte-identically** to their approved hashes on the baseline device (`approved_recipes_repaint_identically_after_save_and_load`, in this run). So there is no drift to take back to the user. The next change to the painting (task 25) bumps the versions and goes to a numbered review round.

## Manual desktop checklist

Things an automated run cannot judge. Run `cargo run --release -p pigment-studio`.

1. **Native dialogs** ([studio-12 checklist](evidence/studio-12/README.md#manual-interaction-checklist), [studio-13 checks](evidence/studio-13/README.md#manual-checks-for-the-owner)). Open, Save, Save As and Export show the desktop's file chooser. The suggested names never contain the prose. Choosing an existing file asks before replacing it, and cancelling changes nothing.
2. **Keyboard only.** Tab to every control. The focus ring is visible and the column scrolls to it. Arrows, Page Up/Down, Home/End and Delete work on sliders. Alt+Right/Left step through compositions. Ctrl+S, Ctrl+O, Ctrl+E, Ctrl+\\ and Ctrl+Q work.
3. **Drag feel.** Dragging a paint slider never moves the mountains or trees, and the preview sharpens when you let go.
4. **Unsaved work** is asked about on Open and on closing the window; an export in progress is asked about on closing.
5. **Look.** The painting reads truly against the graphite surround; nothing but the painting is colourful except state (amber, coral) and the one accent.

## Skipped coverage and blockers

| Not covered | Why | Where it goes |
| --- | --- | --- |
| A software adapter labelled SOFTWARE and never passing, on this machine | none installed here (qualify.sh step 7 SKIPPED), and no driver was installed for this. It **is** covered on CI runners by the manual software-adapter study (task 23): lavapipe and WARP are labelled SOFTWARE and `gpu-smoke` fails on both, as designed ([decision](cpu-fallback-decision.md)) | — |
| A real screen-reader session (Orca, Narrator, VoiceOver) | not run; the AT-SPI tree is checked ([dump](evidence/studio-12/atspi-tree-linux-2026-09-28.txt)) | task 24 or a manual session |
| A real GPU driver reset | not reproducible on demand; simulated with `--lose-device-after` | — |
| A real full filesystem | would need root to mount a small one; simulated with failing writers | — |
| Native dialogs and their overwrite question | driven by hand only (checklist above) | owner |
| Windows (Direct3D 12) and macOS (Metal) GPUs and windows | no hardware; portable CI only | tasks 21–22 |
| The iGPU presenting the studio window | the multi-GPU limitation (the iGPU here drives no display) | the "studio on multi-GPU systems" task |
| GPU memory in a maximised window on a 4K-class display | not measured | — |

## Adding to the suite

- A new hardware test is `#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]` and is added to `scripts/gpu-tests.sh`. A test that needs particular hardware, such as a second adapter, runs from `qualify.sh`, which reports SKIPPED when that hardware is absent.
- Save each qualification log under `docs/evidence/` with the date and OS in its name.
- Never regenerate `docs/visual-review/baseline-25` (or `baseline-08`) to make a test pass. A change to the painting needs a version bump and a review.
