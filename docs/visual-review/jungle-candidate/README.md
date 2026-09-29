# Tropical jungle — owner-approved visual study

Owner visual review approved this direction on 2026-09-29. Following feedback and the supplied jungle example, the revision emphasizes a continuous broken overhead canopy, four seed-varied clusters of distinctly painted broad-leaf fans, mixed broadleaf/airy/understory/flowering forms, and stronger near/mid/far vegetation depth. It uses a separate humid-green palette and a stylized dry/wet cycle. The cycle is appearance-only and does not claim to simulate tropical weather or ecology. `season.year` 0 and 1 are the same cycle boundary; the three comparison rows use sample seeds 0, 3 and 7.

## Candidate images

- [`ten-seeds.png`](ten-seeds.png): seeds 0–9, numbered left-to-right then top-to-bottom.
- [`wet-dry.png`](wet-dry.png): three seeds, each compared at `season.year` 0, 0.2, 0.5, 0.82 and 1 from left to right. The repeated first/last value is the boundary check.
- Matching `.txt` files record the generation/render configuration, GPU, geometry checksums, coverage and output hashes.

## Qualification evidence

- Owner approved the ten-seed art sheet and 3-seed dry/wet cycle. Seasonal comparison checksums are identical within each seed across years 0, 0.2, 0.5, 0.82 and 1; years 0 and 1 also reproduce identical image hashes.
- Profile stress bounds: 70 layers and 2,414 vertices in each review cell; automated cases cap scenes below 96 layers, 32,768 outline vertices and 66 plant layers (24 canopy understory/fan layers).
- GPU qualification passed on Linux x86_64, NVIDIA GeForce RTX 4070 Ti (Vulkan, driver 615.71.09), Rust 1.98.1, wgpu 30.0.1. A source-free jungle recipe saves/reopens with its biome and settings intact; the 4K tiled PNG is byte-identical to a single-tile render. The shared 333 px boundary-band seam check measured 0/255 pixel difference; jungle tile equivalence also passed exactly.
- 8K performance (`target/release/pigment-prose export --out /tmp/pigment-jungle-8k.png --sample 7 --biome jungle --size 8k`): 7680×4320, 12 tiles of 2048 px with 50 px apron; estimated renderer GPU allocation 67.2 MiB and host band 60.0 MiB; measured whole-process peak RSS 971.8 MiB; render/readback 264 ms, PNG encode/write 197 ms, total export 470 ms, 23.1 MiB PNG. Device/pipeline initialization was 15.6 s (cold start, not included in export total). A separate optimized studio qualification measured 960×540 preview under 10 ms and 8K tiled export 280 ms (12 tiles, 22.7 MiB); timings vary by run.
- `scripts/check.sh` and `scripts/gpu-tests.sh` passed. The latter also ran existing Alpine, desert, tundra, studio UI, export, preview, and tile-seam regressions.

Recipe samples from the seasonal study are in [`recipes/`](recipes/), five year values each for sample seeds 0, 3 and 7 (`01`–`05`, `06`–`10`, `11`–`15`). They contain no source text.

## Reproduction

```sh
cargo run --locked -p pigment-cli -- contact-sheet --biome jungle \
  --samples 0,1,2,3,4,5,6,7,8,9 \
  --out docs/visual-review/jungle-candidate/ten-seeds.png
cargo run --locked -p pigment-cli -- contact-sheet --biome jungle \
  --samples 0,3,7 --vary season=0,0.2,0.5,0.82,1 --cols 5 \
  --out docs/visual-review/jungle-candidate/wet-dry.png
```

The CLI can render this approved profile, and it is now registered as a selectable biome. CPU/profile and recipe save/load tests pass. Remaining hardware qualification: measure preview and 8K export time/memory, test tile-crossing canopy and mist seams, and record source-free recipe round-trip plus existing-biome regression evidence. These checks are required before task 20 is complete.
