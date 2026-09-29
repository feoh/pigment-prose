# Rocky-desert biome visual review

Owner-approved as the visual direction for task 18 before recipe/UI/export integration. These fixtures record the reviewed geometry and seasonal paint; they are not embedded in the application.

- [10-seed contact sheet](contact-sheet.png)
- [Three-seed dry/growth comparisons](seasons.png)
- Exact per-cell composition checksums, coverage and pixel hashes: [contact-sheet.txt](contact-sheet.txt), [seasons.txt](seasons.txt)

Rendered on the NVIDIA GeForce RTX 4070 Ti over Vulkan using the existing GPU paint/tiling pipeline. The seasonal comparison shows year `0.20` (brief growth/flowering) and `0.58` (peak dry) for seeds 0–2. Each seed keeps the same geometry checksum between seasons; only seasonal paint changes.

Reproduce from the repo root:

```sh
cargo run -p pigment-cli -- contact-sheet --biome desert \
  --samples 0,1,2,3,4,5,6,7,8,9 \
  --out docs/visual-review/desert-candidate/contact-sheet.png --cell 512 --cols 5 --view paint
cargo run -p pigment-cli -- contact-sheet --biome desert \
  --samples 0,1,2 --vary season=0.20,0.58 \
  --out docs/visual-review/desert-candidate/seasons.png --cell 640 --view paint
```

## Scope and limitations

This is one seeded rocky-desert family: irregularly capped mesas with mineral strata, an undulating dry basin/wash, sparse low shrubs, no persistent open water, and a restrained stylized dry/growth seasonal cycle. The dry/wet profile never adds snow; no climate simulation, animals, buildings, dunes, or general canyon/oasis generator is implied. Selecting a biome applies its structural, palette and haze defaults while retaining paint settings and season. Palette selection remains manually editable afterward. The 4K recipe-reopen plus tiled export qualification passed on the RTX 4070 Ti (Vulkan), bit-identical to a single render. Cross-device pixel identity is not promised.
