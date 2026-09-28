# CPU-only fallback: decision record (task 23)

**Question.** Is a useful experience without a hardware GPU practical, and at what cost? GPU rendering is required for the MVP; this is an investigation, not an MVP condition (product brief).

**Recommendation: defer a CPU renderer and keep the clear GPU requirement.** Keep the opt-in software-adapter mode that already exists (`--allow-software`), labelled everywhere, as an **unsupported** last resort on Linux, and do not advertise it. No second renderer. Details and the decision left to the owner are at the end.

## What was tested (facts)

The existing binaries, unchanged, were run on software graphics adapters on GitHub's GPU-less runners by a manual workflow ([`software-adapter-study.yml`](../.github/workflows/software-adapter-study.yml), run 36373848217, 2026-09-28). Nothing was installed on the owner's machine. Lavapipe was installed inside the throwaway Ubuntu runner; WARP ships with Windows. Logs: [lavapipe](evidence/cpu-fallback-23/study-linux-lavapipe-2026-09-28.txt), [WARP](evidence/cpu-fallback-23/study-windows-warp-2026-09-28.txt). The runners' CPU models were not recorded.

| | Linux: lavapipe (Mesa 25.2.8, LLVM 20.1.2, Vulkan) | Windows: WARP (Microsoft Basic Render Driver, Direct3D 12) | Target (hardware) | RTX 4070 Ti, for scale |
| --- | --- | --- | --- | --- |
| Adapter listed and labelled | `type=Cpu software=true`, shown as SOFTWARE | `type=Cpu software=true`, shown as SOFTWARE | — | — |
| `gpu-smoke --allow-software` | renders, tiled == single, then **FAIL** ("not GPU evidence") as designed | the same | — | PASS |
| Smoke test card 1920×1080 (trivial shader) | 1.5–2.3 s | 12.3–12.7 s | — | 1.1 ms (at 1280×720) |
| Painting, interaction preview 960×540 | **178 ms** median | crashed | ≤ 33 ms | 0.7 ms |
| Painting, settled 1920×1080 | **692 ms** | crashed | ≤ 150 ms | 2.6 ms |
| Painting, 3840×2160 | 2.73 s | crashed | ≤ 150 ms | 9.1 ms |
| 4K export incl. PNG | **2.58 s** | crashed | ≤ 5 s | 62 ms |
| 8K export incl. PNG | **10.3 s** | crashed (segfault) | ≤ 20 s | 208 ms |
| The ten approved corpus paintings vs the hardware baseline | **PSNR 63.0–63.5 dB, max difference 1/255**, about 3% of channels off by 1 ([sheet](evidence/cpu-fallback-23/lavapipe-corpus-16x9.png)) | not produced | tier 3: ≥ 50 dB, ≤ 4/255 | the baseline |

- **Lavapipe paints the real painting correctly.** Same composition, and pixels within the cross-device tolerance. Exports meet the export targets. Previews miss their targets by about 5×.
- **WARP runs trivial shaders, but the painting renderer crashes the process**: a segmentation fault (status 139) in `paint-bench` and the 8K export, and exit status 127 for the 4K export and the contact sheet, all before any painting was written. This was not investigated here: nothing in this decision depends on it. The painting shader reaches Direct3D 12 through naga's HLSL output on real Windows GPUs too, so it is a risk for task 21 and was recorded there.
- **macOS has no software Metal adapter.** There is no fallback to test there.
- **This machine has no software adapter installed**, so the local qualification reports that check as SKIPPED. No driver was installed for this study.

## The options

| | A. Software graphics adapter (same renderer) | B. Reduced-quality CPU path | C. Independent CPU renderer |
| --- | --- | --- | --- |
| What | lavapipe/llvmpipe (Linux), WARP (Windows) running the same WGSL through wgpu | e.g. flat colour planes from the scene's CPU rasterizer (`scene::raster`), no paint | a Rust (rayon) reimplementation of the painting: the 1,133-line `paint.wgsl` plus the coverage index |
| Looks like the product | yes: 63 dB against the approved images (lavapipe) | **no**: a structure diagram, not a painting | only if kept in lockstep |
| Works today | Linux: yes, opt-in. Windows: no (crash). macOS: no | no | no |
| Dependencies and distribution | Linux: the user installs a Mesa package (`vulkan-swrast` / `mesa-vulkan-drivers`); bundling Mesa is possible but heavy. Windows: none (WARP is built in) | none | none new |
| Speed (tested / estimated) | tested: previews 5× over target on a CI runner; exports meet targets | estimate: fast (milliseconds), because it paints almost nothing | estimate: 1920 px in seconds on 8+ cores; unmeasured |
| Maintenance | a CI job and labels; the renderer stays single | a second, simpler look to keep coherent | **every `RENDERER_VERSION` change twice**, plus per-change tier-3 tolerance work |
| Testing cost | one manual workflow (exists) | new golden images for a second look | a second baseline and parity tests for every paint feature |

## How a missing GPU appears today (specified and tested)

| Situation | What the user sees | Tested by |
| --- | --- | --- |
| A hardware GPU | the studio | everything else |
| No adapter, or the driver refuses the device | a window titled "Pigment Prose needs a hardware GPU" with the reason, driver hints per OS, every adapter found, Copy details and Quit; exit status 1. Never a painting | qualify.sh step 7 (`--adapter no-such-gpu`), task 11 evidence |
| Only a software adapter | the same window, plus: "A software renderer is installed (NAME). Starting with --allow-software paints with it instead: previews and exports are many times slower… It is not GPU acceleration." It is **never chosen automatically** | `ui_tests::the_no_gpu_window_offers_an_installed_software_renderer_only_as_a_labelled_choice` |
| Started with `--allow-software` on a software adapter | the studio works: the status line shows "NAME (Vulkan, SOFTWARE)" in coral and "SOFTWARE RENDERER, not GPU accelerated"; Diagnostics says "Hardware acceleration: no (software rasterizer)"; the CLI prints "results are NOT GPU evidence"; `gpu-smoke` and `bench --strict` never pass. Slow settled previews lower the studio's settled cap to its 1920 px floor | `ui_tests::a_software_adapter_is_labelled_and_never_called_hardware`, the CI study logs, `adapter::is_software` |
| The painting GPU cannot show the window | the window goes on another GPU; a forced adapter that fails is named, with advice; exit 2 | ADR 0001 amendment |

No path presents a software rasterizer as hardware acceleration.

## Recommendation

1. **Defer a CPU renderer (option C).** It duplicates the most-changed code in the project (task 25 alone will change the painting) for a fallback the product brief does not promise. Revisit only with a concrete need, such as users who genuinely cannot get a supported GPU.
2. **Reject a reduced-quality CPU path (option B) as a "fallback".** It would not be the product's painting, and presenting it as one would mislead. If a structure-only preview is ever wanted, it is a separate feature.
3. **Keep option A as it is: opt-in, labelled, unsupported.** On Linux with lavapipe it already produces the real painting and exports within the targets, with previews that lag. It costs nothing to keep, and the no-GPU window now tells people it exists. It is not a supported configuration: Windows (WARP crash) and macOS (no adapter) have no equivalent, and preview responsiveness misses its targets.

Any expansion beyond this needs the owner's approval and new, bounded tasks.

## Owner decision

- **Default (recommended): keep deferring.** Nothing further is scheduled. The WARP crash is tracked with Windows validation (task 21).
- **Alternative: make "Linux software mode" supported.** This would be new tasks: a smaller interaction preview on software adapters (for example 480 px), a studio notice that it is running on a software renderer, a documented install step for lavapipe, and the study workflow run before each release. The price is keeping an honest "slow mode" working on one OS.
- **Optional measurement:** installing lavapipe on the development machine (`vulkan-swrast` on Arch/CachyOS) would measure it on the owner's 24-core CPU. llvmpipe scales with cores, so it should be several times faster than the CI runner, but that is an **estimate**. It was not installed, because installing drivers needs approval.

## Estimates, clearly marked

- Speed of lavapipe on the owner's i9-14900K: unmeasured. The CI runner's core count was not recorded.
- Option C's speed and effort: not prototyped. The size argument (1,133 lines of WGSL plus the coverage index, and every future paint change twice) is from the code; the time argument is judgement.
