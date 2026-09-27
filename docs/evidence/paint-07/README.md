# Task 07 evidence: woodland, rocks and water

Recorded 2026-09-27 on Linux (CachyOS, kernel 7.2.7), Vulkan, wgpu 30, NVIDIA GeForce RTX 4070 Ti unless noted. Spec: [painting.md](../../painting.md#woodland-rocks-and-water-task-07). Default settings throughout. Every `.txt` lists each cell's template and geometry checksum.

| Sheet | Shows |
| --- | --- |
| [corpus-16x9.png](corpus-16x9.png), [corpus-9x16.png](corpus-9x16.png), [corpus-1x1.png](corpus-1x1.png) | the ten reference passages in landscape, portrait and square: woods on land, water in its region with reflections, rocks with planes, rims and contact shadows |
| [density-0.png](density-0.png), [density-1.png](density-1.png) | the same five seeds (15, 33, 8, 16, 0) at woodland density 0 (sparse: forested hills stay, woodland stands mostly vanish) and 1 (dense clusters) |
| [repeat-variation.png](repeat-variation.png) | `shore-a`, variations 0–4: composition variations, each a different layout; re-rendering a variation gives the same checksum (`same_recipe_same_scene`) |

The round 6 review sheets, including 1:1 detail crops, are in [visual-review/round-06](../../visual-review/round-06/README.md).

## Tests

- Portable (`scripts/check.sh`; `pigment-core` 114 tests, `pigment-gpu` 3):
  - `woodland_stands_on_land`: 40 seeds × density 0.1 / 0.5 / 1. Far woods stand on the waterline, and near woods on the near shore.
  - `rocks_have_lit_and_shadowed_planes`: 30 seeds × faceting 0 / 0.55 / 1.
  - `canopies_mix_emergent_and_young_crowns`.
  - `checksums_are_frozen`: re-frozen, since the checksum covers the light side and the canopy and rock-plane changes.
  - `indexed_matches_brute_force` and `axes_agree_away_from_edges` for the per-layer ray axis.
- Hardware ([gpu-tests-linux-2026-09-27-task07.txt](../gpu-tests-linux-2026-09-27-task07.txt), 12 of 12 pass):
  - `painting_is_identical_tiled_and_single`: now also a river valley (seed 15) and two lakes with rocks (seeds 0 and 8), at looseness 0, 0.4 and 1, with 256 and 333 px tiles.
  - The setting-extremes, compositing, repeatability and cross-resolution tests.
- Palette and wash changes cannot move trees or shores: paint settings never reach the generator (`appearance_and_paint_seed_never_touch_geometry`). Every paint-level mark hashes canvas lattices.

## Performance

[paint-bench-linux-2026-09-27.txt](paint-bench-linux-2026-09-27.txt): warm render + readback, 30 seeds, both GPUs, plus `--stress`. Summary in [painting.md](../../painting.md#measured-linux-vulkan-2026-09-27). Memory: the scene's coverage index is at most 32 entries per vertex (8 bytes each), and the tile buffers are unchanged from task 06. No new per-object GPU allocations.
