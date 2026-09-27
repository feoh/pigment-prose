# Round 4 result (2026-09-27)

**User verdict:** "Yes, the rivers look great, though 2 and 3 still look more natural than 1. My favorite images from this batch are 6 and 9, mostly I think because the forest/land sections towards the bottom of the image draw the eye and have enough pleasing detail to linger on."

Earlier in this round the user asked what the purple/green/beige sections in round 3 image 19 were (the high-vantage cliff edge) and asked to remove it. It was removed before rating, and the sheets were regenerated.

| Image | Seed | Verdict |
| --- | --- | --- |
| 1 | `sample passage 3` | river fine, less natural than 2 and 3 |
| 2, 3 | `sample passage 12`, `17` | natural |
| 6 | `sample passage 15` | favourite |
| 9 | `sample passage 33` | favourite |

## What made image 1 less natural

On its left bank, two spurs from the **same bank** left a thin, sharp slit of water between them. Real banks do not open V-shaped cracks where the river does not cross over. **Fixed:** when the next spur down grows from the same bank, the farther spur's land runs on under it and eases back to its own bank past the nearer spur's tip. The test `spurs_on_the_same_bank_leave_no_water_slit` fails on seed 3 without the fix and passes with it. The fix changes the geometry of every high-vantage seed that has same-bank spurs, including the favourites. The change is confined to where banks meet, and round 5 shows them again.

## Calibration: what the favourites have

The user's reason was "the forest/land sections towards the bottom … draw the eye and have enough pleasing detail to linger on." These proxies were measured on all 14 high-vantage images of sheet B:

| Proxy (bottom third of the frame) | Favourites 6 and 9 | Range across the 14 | Predicts? |
| --- | --- | --- | --- |
| vegetated land share (new `foreground` metric) | 0.59, 0.65 | 0.43–0.85 | **no** (image 15, the "less natural" seed 3, has the most: 0.85) |
| pixel detail (mean luminance gradient) | 10.7, 10.9 | 8.8–16.7 | no (seed 3 highest) |
| value spread (s.d.) | 24.6, 22.5 | 18.5–26.8 | no |
| hue variety (entropy) | 2.11, 1.99 | 1.62–2.40 | no |

**None of the simple amount or detail measures explain the choice.** Current hypothesis: it is **figure against ground**. In 6 and 9 the foreground is one large, well-shaped peninsula with water around it and a long curving shoreline, holding distinct stands (conifers, blossom, copper, meadow). Seed 3 is almost all land at the bottom: it has plenty of detail but no clear form. The `foreground` metric stays in the notes but is marked as not predictive, and round 5 tests the hypothesis on unseen seeds.

## Noticed, not changed

The largest, nearest spurs can end in a squared-off, near-vertical face. A longer taper fixed that, but it made seed 33's tip (image 9, a favourite) close almost like a wedge, and it changed an approved image, so it was reverted. It is raised with the user instead.

## Follow-up the same day (the user: "You are welcome to alter anything based on my feedback. Just do it")

- **Continuous banks replace the join.** Joining same-bank spurs still left a band of water where a spur from the other bank sat in between (seed 102 at full size), and later attempts to patch it left straight and then vertical edges. The river now has two continuous banks, with the spurs as ridges on them (see `docs/scene-generation.md`, "Meanders"). The valley's look changed noticeably: a river winding out of the lake between forested banks, instead of a lake with separate peninsulas.
- **Rounded prows:** the spurs' high ground slopes down toward the water from halfway out, and the water-level end keeps its blunt cap. The blunt-tip test now judges the last stretch (85 % → 95 %), where a wedge keeps a third and an elliptical end about 60 %. It skips tips joined to the next spur on the same bank, and it still fails on a wedge.
- **Weights:** `high-vantage` is now 40 % of compositions.
