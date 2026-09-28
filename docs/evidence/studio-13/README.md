# Task 13 evidence: export from the studio

Linux 7.2 (CachyOS), KDE Plasma Wayland at 2× scale, NVIDIA GeForce RTX 4070 Ti, Vulkan, driver 615.71.09, wgpu 30.0.1, eframe/egui 0.36.2, 2026-09-28. Spec: [docs/studio.md](../../studio.md#export) and [docs/export.md](../../export.md#from-the-studio-task-13).

| File | What |
| --- | --- |
| [studio-linux-2026-09-28.txt](studio-linux-2026-09-28.txt) | The real-window `--script` runs (undelayed, 1.5 s simulated preview render, simulated device loss). After the task 12 stages, each run exports an 8K PNG from the window into a temporary folder while dragging Edge Looseness every frame. It checks that the file decodes at 7680×4320 with nothing else left in the folder, that the running job's snapshot never changed, and that UI frames kept coming (gap < 250 ms). The folder is deleted afterwards |
| [window.png](window.png) | The real window at the end of the undelayed run, with the export's result notice |
| [08-export-dialog.png](08-export-dialog.png) | The export dialog: what is exported, shape (recomposes, not a resize), 4K/8K/custom with real pixel sizes, the DPI note, the privacy statement (50%) |
| [09-exporting.png](09-exporting.png) | An export in progress: "Tile 3 of 28" (counted by the renderer, held at a tile by the test so it can be seen) and Cancel export, with the window still usable (50%) |
| [10-exported.png](10-exported.png) | The result notice with the file, size, folder and Copy path (50%) |
| [../gpu-tests-linux-2026-09-28-task13.txt](../gpu-tests-linux-2026-09-28-task13.txt) | `scripts/gpu-tests.sh`, including `the_studio_exports_a_real_8k_png_of_the_snapshot` |

## Acceptance, item by item

| Requirement | Evidence |
| --- | --- |
| Export and decode a real 8K PNG from the UI on Linux | `ui_tests::the_studio_exports_a_real_8k_png_of_the_snapshot` (hardware): Ctrl+E, 8K, Export…, then the studio's export worker with the real `PaintRenderer`. The file decodes at 7680×4320, 8-bit RGB, sRGB perceptual, with chunks exactly `IHDR sRGB IDAT… IEND`. 224–231 ms. The real window's `--script` exports 8K the same way (360–480 ms while dragging a slider) |
| Pixels and colours match the export contract; sliders changed mid-export do not alter the snapshot | The same test moves Edge Looseness about 40 times during the export. The studio's file is **byte-identical** to a direct `export_png` of the snapshot on the same device. Portable: `sliders_moved_during_an_export_do_not_change_it` (the renderer received the snapshot's appearance), `export::tests::exports_a_snapshot_that_later_edits_cannot_reach` |
| No prose or local path in the PNG | The hardware test writes prose with a unique marker (kept in the recipe too) and exports into a folder and file named with path markers. None of them, the recipe word or `$HOME`, appears in the PNG's bytes |
| Custom portrait and square | `portrait_and_square_paintings_export_at_their_own_proportions` (4:5 → 800×1000, 1:1 → 900×900, decoded) |
| Invalid sizes | `invalid_custom_sizes_are_explained_and_cannot_be_exported` (below 64 px, above 16384 px; Export… disabled), `export::tests::sizes_keep_the_paintings_exact_proportions` (snapping, mismatch, bounds) |
| Cancelled destination and overwrite decline | `the_export_dialog_names_real_pixels_and_a_cancelled_destination_writes_nothing`. Declining the save dialog's overwrite question keeps that dialog open, and closing it returns the same "cancelled" answer, so nothing is written. The question itself is the platform dialog's, so it is covered by the manual checks |
| Cancel during render, readback and write | `cancelling_an_export_keeps_the_existing_file_and_leaves_no_partial` (UI, after the first tile), `export::tests::cancelling_mid_render_…`, and on the GPU `pigment-io`'s `cancelled_export_leaves_the_old_file_and_no_partial`. Cancellation is checked before every tile. Each tile's readback and its band's encoding complete first, and the final flush and rename are not interruptible (docs/export.md) |
| Disk error | `a_failed_export_says_what_to_do_and_keeps_the_existing_file` (disk full: "Free some space or choose another folder", old file kept), `export::tests::failures_are_…` (missing folder fails before rendering). The backend's mid-stream full-disk cases are `png_sink` tests from task 09 |
| Window close during export | `closing_during_an_export_asks_and_stopping_leaves_no_partial_file` ("Keep exporting" continues; "Stop export and close" cancels, then closes, leaving no file). `export::tests::shutdown_cancels_a_running_export_and_cleans_up` (the app's exit path) |
| Existing files survive failed replacement | the cancel, disk-full and export-worker tests above all start with an existing file and check it byte for byte |
| UI stays responsive; one export at a time | `--script` UI frame gaps during the 8K export: 21.0, 29.6 and 50.9 ms. `sliders_moved_during_an_export_…` checks that Export PNG… and Ctrl+E are refused while one runs, and that the controls and Another Composition keep working |
| No watermark, attribution or embedded recipe | the chunk list above: there is no text chunk of any kind |

## Manual checks for the owner

1. Ctrl+E, choose 8K, **Export…**. The desktop's save dialog suggests `painting-7680x4320.png` (or the recipe's name), never a name taken from the prose. Pick an existing file: the dialog asks before replacing it.
2. Start an 8K or 16384 px custom export and move a slider during it: the window stays live, the bar counts tiles, and the file shows the settings from when you pressed Export.
3. Start a large export and press **Cancel export**: no file appears and an existing file is untouched.
4. Close the window during an export: you are asked first.
