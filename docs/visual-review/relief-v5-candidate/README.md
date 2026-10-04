# Renderer v5: relief and volume candidate

2026-10-04. **Awaiting owner visual approval.** Not a new approved baseline.

The owner supplied a flat current landscape and five references calling for
lush natural detail and volume, with minutes per image acceptable. No reference
images were copied into this repository or used as generated textures.

## Compare

Same synthetic text seeds (`sample passage 0`, `8`, `15`), variation 0,
16:9, default alpine settings, 1280×720 per painting:

- [Before: renderer v4](before.png), [settings/hashes](before.txt)
- [After: renderer v5](after.png), [settings/hashes](after.txt)
- Individual candidates: [mountain](paintings/01.png), [lake](paintings/02.png),
  [river valley](paintings/03.png). Each has an adjacent source-free recipe.
- [Golden-evening portrait](portrait.png), same seed 0, recomposed at 9:16
  ([settings](portrait.txt)). This changes palette/aspect deliberately, so it
  is not a controlled before/after comparison.

```sh
cargo build --release --locked -p pigment-cli -p pigment-studio
./target/release/pigment-prose contact-sheet --samples 0,8,15 \
  --cell 1280 --cols 3 --out docs/visual-review/relief-v5-candidate/after.png \
  --cells docs/visual-review/relief-v5-candidate/paintings
./target/release/pigment-prose contact-sheet --samples 0 \
  --palette golden-evening --aspect 9:16 --cell 720 \
  --out docs/visual-review/relief-v5-candidate/portrait.png
```

`before.png` was captured before the renderer edits. Re-running its recipes
with v5 intentionally does not reproduce v4 pixels. Old approved baselines
were not regenerated or relabelled.

## What changed and what remains

See [the renderer notes](../../painting.md#current-renderer-v5-relief-candidate).
Stone has nine-octave ridged relief, normals, horizon shadows, mineral grain
and fractures. Snow and vegetation follow the same relief. Clouds have
integrated 3D density, self-shadowing, and transparent bank compositing instead
of three flat bands. Crown shading, small foliage and grass are richer, each tree carries its own
value and hue lean (the same cells as its crown, so stands stop reading as one
repeated stamp);
river-current highlights are less wire-like.

Two bounded visual passes covered alpine mountains/lakes/valley, desert,
tundra, jungle and a portrait. The first exposed cloud-edge pooling halos
and mismatched relief scales on desert bands; both were corrected. Final
inspection confirms stronger mountain/cloud volume, but also remaining gaps:
forests still resemble cellular patches, large individual foreground trees
are missing, desert geology can look too regularly folded, and tundra remains
restrained. This is **not reference-level art or a measured 10× improvement**.
Reaching those references also needs richer terrain/composition and tree
anatomy, not just more shader samples. Prose remains a deterministic seed,
not semantic image prompting.

## Validation and timing

Linux, NVIDIA GeForce RTX 4070 Ti, Vulkan, NVIDIA 615.71.09, wgpu 30.0.1.

- `scripts/check.sh`: format, clippy (`-D warnings`), portable tests, seed
  reference vectors and third-party notices passed.
- `scripts/gpu-tests.sh`: **36 hardware tests passed**, plus GPU smoke.
  Includes new cloud-band/light-direction and all-biome portrait/landscape
  reverse-tile-order regressions. Existing season, resolution, cancellation,
  preview coalescing, memory and PNG privacy checks passed.
- 47 historical recipes still repaint identically before/after saving under
  v5. Historical approved hashes are gated by renderer/device compatibility;
  none are represented as v5 visual approval.
- Qualification seam comparison: **0/255** difference in 84,120 boundary-band
  pixels. Preview/4K downsample PSNR: 30.5 dB landscape, 29.2 dB portrait,
  32.8 dB square.
- A timing race in the qualification test was exposed: completion could set
  `in_flight` to zero after a drain, leaving the last result unread. The test
  now observes the latest result ID until its deadline and retains results
  drained during export. Production queue behaviour was not changed.
- `cargo build --release --locked -p pigment-studio`: passed. Launch the rebuilt
  `target/release/pigment-studio`; an already-running/installed binary is not
  automatically replaced.

Warm six-seed benchmark (three runs, two warm-ups; render + GPU readback):

| Size | Median | Worst scene |
| --- | ---: | ---: |
| 960×540 | 1.87 ms | 1.93 ms |
| 1920×1080 | 6.38 ms | 6.79 ms |
| 3840×2160 | 26.87 ms | 28.15 ms |

Separate PNG exports: seed 0 at 4K, 86.82 ms export total; seed 8 at 8K,
297.76 ms export total (excludes ~112 ms device/pipeline preparation).
These are local measured samples, not cross-platform guarantees or the cost
of a future offline-quality renderer. Windows/macOS and the Intel iGPU were
not qualified in this session.

[Hardware log](hardware.log) · [Portable checks](portable.log) ·
[Benchmark](benchmark.txt). Logs are from the final re-run after the
per-tree variation pass; the benchmark table predates that pass (its cost is
one extra cell hash per crown sample). Full-size local deliverables (not committed):
`target/relief-review/alpine-4k.png` and `alpine-8k.png`.
