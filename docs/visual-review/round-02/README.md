# Rating round 2: first paintings (task 06)

These are the first painted versions, after the user's direction following round 1: "a verdant forest. More greens, more colors. More … LIFE!" The 16 candidates use the same seeds, order and geometry as [round 1](../round-01/README.md), so the two rounds compare directly. All settings are defaults (palette `lakeshore`), rendered on 2026-09-26 on the RTX 4070 Ti. Details are in [docs/painting.md](../../painting.md).

| File | What it shows |
| --- | --- |
| `candidates.png` | the 16 round-1 layouts, painted (cells 0–15, left to right and top to bottom) |
| `full-high-vantage.png`, `full-peak-over-water.png`, `full-tower.png` | three of them at 1920 px, to judge texture and detail |
| `portrait.png` | four seeds recomposed for 9:16 |
| `golden-evening.png` | four seeds in the second palette |

Each `.txt` beside a sheet lists every cell's seed, template, geometry checksum and awe metrics.

## What I'd like to know

Short answers are fine:

1. **Life and color:** is this the verdant, living feel you meant? More, less or different?
2. **Favourites:** which cells or full-size images work best, and which least? A 1–5 score for "vastness" and "stop" still helps calibrate the awe metrics, if you're willing.
3. **Direction:** what should come next? For example: water reflections, more painterly (looser, wetter) handling, individual trees and rocks up close (task 07), more palettes, or something else.

## Known gaps (not yet done)

- **Water** has ripples and sheen but no reflections of the mountains yet.
- **Mountains** can still look slightly veiled by haze. The `--haze` setting controls this, and 0 is fully clear.
- **Near rocks** are simple planes, and near trees are canopy masses. Individual trees and rock drawing come in task 07.
- **Speed:** a 1920 px painting takes about 0.3 s, which is fine for sheets but too slow for the interactive app. Fixing it is planned before the desktop shell.

Regenerate the candidates with:

```text
cargo run --release -p pigment-cli -- contact-sheet --out docs/visual-review/round-02/candidates.png \
    --samples 10,3,6,21,12,1,30,17,9,0,26,16,8,33,14,41 --cell 480 --cols 4
```
