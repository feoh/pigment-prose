# Round 3 result (2026-09-27)

**User verdict:** "I loved image 2. Of 17-20 I like 19 best. A criticism I have with both 2 and 20 is that the extreme zig zag of the river looks very unnatural. Rivers can't zig zag with such sharp angles like that because of the physics of erosion and water flow."

| Image | Seed | Template | Verdict |
| --- | --- | --- | --- |
| 2 | `sample passage 3` | high-vantage | loved |
| 19 | `sample passage 12` | high-vantage | best of 17–20 |
| 20 | `sample passage 17` | high-vantage | zigzag criticised (as was 2) |

- **Calibration signal:** both favourites are `high-vantage`, the elevated panorama with a winding lake and mixed forest, and so far the user's clearest preference. It has the highest `planes` (8–10) and `expanse` (~0.5) of all templates (see `sheet-*.txt`). This is logged as a first data point, not yet a fitted weight. The template weights are unchanged until more rounds agree.
- **Fix:** the valley spurs now follow a smooth meandering channel, with blunt, rounded tips and curved banks (task 06 work, `docs/scene-generation.md`, "Meanders"). Result in [round 4](../round-04/README.md).
