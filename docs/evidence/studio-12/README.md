# Task 12 evidence: artistic controls, compositions, recipe open/save and the studio design

Linux 7.2 (CachyOS), KDE Plasma Wayland at 2× scale, NVIDIA GeForce RTX 4070 Ti, Vulkan, driver 615.71.09, wgpu 30.0.1, eframe/egui 0.36.2, 2026-09-28. Spec: [docs/studio.md](../../studio.md). Visual system: [DESIGN.md](../../../DESIGN.md).

| File | What |
| --- | --- |
| [studio-linux-2026-09-28.txt](studio-linux-2026-09-28.txt) | Every scripted real-window run: undelayed (PASS), a simulated 1.5 s render (PASS) and simulated device loss after 3 previews (PASS). Each run types, resizes twice, drags Form and then Edge Looseness every frame for 1 s each, asks for Another Composition, and checks what settles |
| [window.png](window.png) | The real window captured by the app at the end of the undelayed run, 2720×1720 at 2× |
| [window-delayed.png](window-delayed.png), [device-lost.png](device-lost.png) | The same capture from the delayed and device-loss runs (50%) |
| [01-launch.png](01-launch.png) | First launch with one slider off its default (Reset shown), 1440×900 pt at 2× (full size). Rendered offscreen by `review_screens` with the real painter and theme |
| [02-advanced-focus.png](02-advanced-focus.png) | Advanced open, Granulation reached with Tab and raised with Page Up: the focus ring, signal handle and default notch, scrolled into view (50%) |
| [03-unsaved-prompt.png](03-unsaved-prompt.png) | Ctrl+O with unsaved changes: the prompt, with "Don't save" set apart (50%) |
| [04-open-failed-and-earlier-settings.png](04-open-failed-and-earlier-settings.png) | A broken recipe reported in the notice bar, and the "Showing earlier settings" chip with "Painting…" while a newer preview is on its way (50%) |
| [05-source-free-portrait-old-version.png](05-source-free-portrait-old-version.png) | A recipe saved without prose, 4:5, from renderer v0: the source-free note, empty editor and version notice (50%) |
| [06-minimum-window.png](06-minimum-window.png) | The smallest window (760×480 pt) with a wrapped error and keyboard focus on the primary button (50%) |
| [07-painting-only.png](07-painting-only.png) | **Painting only** (Ctrl+\\): the controls folded away (50%) |
| [atspi-tree-linux-2026-09-28.txt](atspi-tree-linux-2026-09-28.txt) | The accessibility tree on AT-SPI ([`scripts/atspi-dump.py`](../../../scripts/atspi-dump.py)). `IsEnabled` and `ScreenReaderEnabled` were switched on for the dump and set back to `false` afterwards (checked) |
| [../gpu-tests-linux-2026-09-28-task12.txt](../gpu-tests-linux-2026-09-28-task12.txt) | `scripts/gpu-tests.sh`, now including `review_screens` |

Reproduce: `cargo build --release --locked -p pigment-studio`, then the commands at the top of each block in `studio-linux-2026-09-28.txt` with `./target/release/pigment-studio`. The review screenshots: `PIGMENT_SCREENS=DIR cargo test --release --locked -p pigment-studio --lib review_screens -- --ignored --nocapture`.

## Measured

| Scenario | Result |
| --- | --- |
| Undelayed script run: request → shown | median 10.1 ms, p95 20.2 ms, max 33.2 ms; 180 previews displayed, 174 of them 960 px interaction previews during the drags |
| Paint-only drag (Edge Looseness, 1 s, one value per frame) | 0 scenes built |
| With a simulated 1.5 s render: longest UI frame gap | 19.7 ms over 1051 frames (the check requires < 250 ms). An earlier run in this session measured 37.3 ms |
| Settled preview at 3840×2160 (`paint-bench`) | render + readback 8.9 ms median, worst scene 14.6 ms for the whole call |
| Whole-process GPU memory (nvidia-smi, 1360×860 pt window at 2×) | 262 MiB peak. A maximised window on a 4K-class display has not been measured |

## Manual interaction checklist

Run `cargo run --release -p pigment-studio` and check each item. Items marked **(automated)** are also covered by the headless tests in `crates/pigment-studio/src/ui_tests.rs`; the native dialogs are only exercised by hand.

1. **Native open dialog.** Ctrl+O shows the desktop's file chooser (the XDG portal on Linux). Cancel it: nothing changes. Open `docs/visual-review/baseline-08/corpus-16x9/01.recipe.json`: the painting, variation and sliders change, and the editor says the recipe was saved without its prose.
2. **Native save dialog.** Type some prose, then Ctrl+S. The save dialog suggests `painting.recipe.json`, never a name derived from the prose. Choose an existing file: the desktop's dialog asks before replacing it. The notice says "Saved … (without the prose)".
3. **Keep the prose.** Tick "Save the prose in the recipe file", save, reopen: the words come back. Untick, save: the file no longer contains them. **(automated)**
4. **Unsaved work.** Move a slider, then Ctrl+O or close the window: the prompt appears. Cancel keeps everything; Don't save proceeds; Save… saves first. **(automated)**
5. **Keyboard only.** Tab to Form: the focus ring shows and the panel scrolls to it. Left/Right step 0.01, Page Up/Down step 0.1, Home/End jump, and Delete resets. Alt+Right and Alt+Left (⌥ on macOS) step through compositions. Ctrl+S saves. **(automated)**
6. **Drag.** Drag Wash / Gouache quickly: the preview follows at once (smaller while dragging), then sharpens when you let go. The mountains and trees never move. Drag Form: the land recomposes.
7. **Now state.** While a preview is on its way, the status line says "Painting…" and, if the image is out of date, a chip says "Showing earlier settings". **(automated)**
8. **Painting only.** Ctrl+\\ folds the controls away and brings them back.
