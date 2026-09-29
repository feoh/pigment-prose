# Open-tundra visual review

The user approved this open, low-relief visual direction for task 19. These fixtures record representative output; they are not embedded in the application.

- `contact-sheet.png`: ten seeds at the neutral midsummer checkpoint (`season=0.5`).
- `seasons.png`: six annual checkpoints for seeds 0–2, including snow season, thaw, the short growth period, midsummer, and autumn.
- `01.recipe.json` / `01.png`: a source-free recipe fixture and its 512 px preview for sample seed 0 in winter.
- `tundra-4k.png`: a 4K tiled export from that recipe (NVIDIA GeForce RTX 4070 Ti, Vulkan).

Recreate the high-resolution image with `cargo run -p pigment-cli -- export --out tundra-4k.png --recipe docs/visual-review/tundra-candidate/01.recipe.json --size 4k`.
- Matching contact-sheet `.txt` files record generator/renderer versions, settings, checksums, and render device.

The scene is intentionally not an alpine recolor: it has rolling low ground, no high peaks or tree stands, and only sparse low shrubs. Seasonal paint leaves terrain and plant placement unchanged.
