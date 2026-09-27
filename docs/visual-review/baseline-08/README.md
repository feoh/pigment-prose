# Task 08 baseline: the approved painting (generator v1, renderer v1)

The user approved the multi-seed prototype on 2026-09-27 ([round 6 result](../round-06/RESULT.md)): "Yes everything looks fabulous!". This directory preserves that decision as the reference for later regression tests (task 14). It includes the two revisions asked for with the approval: shrubland clumps, and moss that follows the form.

- **Versions:** recipe schema 1, `nfc-lf-utf8/1`, `pigment-seed/1`, `GENERATOR_VERSION` 1, `RENDERER_VERSION` 1. Scenes are frozen by `checksums_are_frozen` (portable, every OS).
- **Device:** NVIDIA GeForce RTX 4070 Ti, Vulkan, Linux, wgpu 30.0.1, RTX driver 615.71.09. Image hashes hold for this device and driver only. Other GPUs and backends are not expected to be byte-identical (architecture, tier 2).
- **Settings:** everything at its default unless the cell label says otherwise.

| Contents | What |
| --- | --- |
| `corpus-16x9.png`, `corpus-9x16.png`, `corpus-1x1.png` | the ten synthetic reference passages ([fixtures/passages.json](../../../fixtures/passages.json)) in landscape, portrait and square, labelled by test id in the `.txt` |
| `round-06-scenes.png` | the 17 round 6 scenes (`sample passage` 3, 12, 17, 15, 33, 16, 19, 31, 0, 10, 22, 8, 18, 24, 1, 2, 9) |
| `corpus-*/NN.png`, `round-06-scenes/NN.png` | each cell as its own image, 400 px on the long side |
| `…/NN.recipe.json` | the recipe that reproduces that image. There is no source text (the privacy default), only the digest. The `.txt` next to each sheet lists every cell's geometry checksum and `image fnv` (FNV-1a 64 of its RGBA8 pixels) |
| `form-faceting.png`, `form-relief.png`, `form-density.png` | Form comparisons: seeds 15, 33, 8 and 16 at each setting's ends, with paint settings fixed. The composition morphs rather than reshuffles |

The paint-handling, palette and atmosphere comparisons (geometry fixed) are in [evidence/paint-06](../../evidence/paint-06/README.md). Close-ups of edges, texture, woodland and water are in [evidence/paint-06](../../evidence/paint-06/README.md) and [round-06/sheet-c-details.png](../round-06/sheet-c-details.png).

## Regenerate

```sh
B=./target/release/pigment-prose
$B contact-sheet --out corpus-16x9.png --cells corpus-16x9 --cell 400 --cols 5
$B contact-sheet --out corpus-9x16.png --aspect 9:16 --cells corpus-9x16 --cell 400 --cols 10
$B contact-sheet --out corpus-1x1.png --aspect 1:1 --cells corpus-1x1 --cell 400 --cols 5
$B contact-sheet --out round-06-scenes.png --samples 3,12,17,15,33,16,19,31,0,10,22,8,18,24,1,2,9 \
    --cells round-06-scenes --cell 400 --cols 6
$B contact-sheet --out form-relief.png --samples 15,33,8,16 --vary relief=0,1 --cell 400
```

On the baseline device, a regenerated `image fnv` that differs from the baseline means the painting changed. That needs a `RENDERER_VERSION` bump and a fresh review.
