# Task 25 baseline: the approved final rendering detail (generator v2, renderer v2)

The user approved the final rendering detail on 2026-09-28 ([round 7 result](../round-07/RESULT.md)): "I approve this as the final rendering detail". It adds wind and current on the water and more complex rocks to the look approved at the task 08 gate ([baseline-08](../baseline-08/README.md), kept as the v1 record). This directory is the reference for the regression tests.

- **Versions:** recipe schema 1, `nfc-lf-utf8/1`, `pigment-seed/1`, `GENERATOR_VERSION` 2, `RENDERER_VERSION` 2. Scenes are frozen by `checksums_are_frozen` (portable, every OS).
- **Device:** NVIDIA GeForce RTX 4070 Ti, Vulkan, Linux, wgpu 30.0.1, RTX driver 615.71.09. Image hashes hold for this device and driver only. Other GPUs and backends are not expected to be byte-identical (architecture, tier 2).
- **Settings:** everything at its default unless the cell label says otherwise.

| Contents | What |
| --- | --- |
| `corpus-16x9.png`, `corpus-9x16.png`, `corpus-1x1.png` | the ten synthetic reference passages ([fixtures/passages.json](../../../fixtures/passages.json)) in landscape, portrait and square, labelled by test id in the `.txt` |
| `round-06-scenes.png` | the 17 round 6 scenes (`sample passage` 3, 12, 17, 15, 33, 16, 19, 31, 0, 10, 22, 8, 18, 24, 1, 2, 9), kept as the same set so the two baselines compare cell for cell |
| `corpus-*/NN.png`, `round-06-scenes/NN.png` | each cell as its own image, 400 px on the long side |
| `…/NN.recipe.json` | the recipe that reproduces that image, without source text (only the digest). The `.txt` next to each sheet lists every cell's geometry checksum and `image fnv` (FNV-1a 64 of its RGBA8 pixels) |
| `form-faceting.png`, `form-relief.png`, `form-density.png` | Form comparisons: seeds 15, 33, 8 and 16 at each setting's ends, paint settings fixed |

Close-ups of the water and the rocks, before and after, are in [round-07](../round-07/README.md).

## Regenerate

From this directory:

```sh
B=../../../target/release/pigment-prose
P=../../../fixtures/passages.json
$B contact-sheet --passages $P --out corpus-16x9.png --cells corpus-16x9 --cell 400 --cols 5
$B contact-sheet --passages $P --out corpus-9x16.png --aspect 9:16 --cells corpus-9x16 --cell 400 --cols 10
$B contact-sheet --passages $P --out corpus-1x1.png --aspect 1:1 --cells corpus-1x1 --cell 400 --cols 5
$B contact-sheet --out round-06-scenes.png --samples 3,12,17,15,33,16,19,31,0,10,22,8,18,24,1,2,9 \
    --cells round-06-scenes --cell 400 --cols 6
for k in faceting relief density; do
  $B contact-sheet --out form-$k.png --samples 15,33,8,16 --vary $k=0,1 --cell 400
done
```

On the baseline device, a regenerated `image fnv` that differs from the baseline means the painting changed. That needs a `RENDERER_VERSION` bump and a fresh review.
