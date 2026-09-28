# Rating round 7: wind and current on water, complex rocks (task 25)

**Numbers run on across the sheets.** Everything is at default settings, rendered on 2026-09-28 on the RTX 4070 Ti. "Before" is the approved round 6 painting (generator v1, renderer v1, [baseline-08](../baseline-08/README.md)); "after" is the candidate (generator v2, renderer v2, [baseline-25](../baseline-25/README.md)).

| Sheet | Numbers | What it shows |
| --- | --- | --- |
| `sheet-a-before-after.png` | 1–6 | whole scenes before and after: three lakes (`short`, `accent`, `shore-a`) and three rivers (seeds 15, 12, 3) |
| `sheet-b-corpus.png` | 7–16 | the ten reference passages after the change |
| `sheet-c-water.png` | 17–18 | 1:1 crops of 4K exports: 17 wind on a lake (`accent`), 18 current in a river (seed 15) |
| `sheet-d-rocks.png` | 19–21 | rock close-ups from 4K exports, before and after: 19 `accent`, 20 seed 16, 21 `short` (20 and 21 scaled to fit) |
| `image-22-rock-surface.png` | 22 | the stone surface at 1:1 |

## Your round 6 notes, and what changed

**Water: "they'll need to show the effects of wind and current and have more ripples."**

- Every scene now has a seeded wind (direction and strength). Lakes show wind lanes, cat's-paws (gusty patches), ripples at several scales that shorten with distance, broken crests and chop. Reflections break up where the wind roughens the surface, and foam gathers around rocks at the waterline.
- Rivers carry a current along their channel: lines that follow the banks, faster (and more streaked) where the channel narrows, slower where it widens into the lake.

**Rocks: "surfaces will need to be less chonky geometric and more complex."**

A first attempt kept the old outlines and only added small relief and faint texture. As you pointed out, it didn't change the form. This version replaces the rock builder:

- **Four kinds of rock**, from each rock's own seed: rounded boulders of two or three lobes, jointed blocks with stepped tilted tops, tilted slabs with a long back and a broken end, and boulders split by a deep cleft. Outlines carry larger weathered relief and chipped notches.
- **Groups:** some rocks have a smaller companion leaning against one side.
- **Planes:** each rock is cut into a mosaic of planes by crooked ridge lines falling from its silhouette corners to its foot, each plane lit by the way it faces. The near rocks add a top face, a dark crevice (the cleft on split boulders) and lower facets: shelves turned up to the light and undercuts turned away.
- **Surface:** facets at three scales within those planes, each tilted toward or away from the light, with open joints (dark on the side away from the light, a lit lip on the side toward it), tilted strata, rain streaks, cracks, grain flecks, and warm and cool staining.
- **Moss** keeps the form-following look you liked. It now gathers a little less on the lit faces, so the stone shows there.

## Cost

Measured with `pigment-prose paint-bench` (12 samples, median / worst scene, render + readback):

| | before (round 6) | after |
| --- | --- | --- |
| RTX 4070 Ti, 3840×2160 | 11.7 / 13.7 ms (water only, earlier today) | 12.1 / 18.4 ms |
| Intel iGPU, 3840×2160 | 317.8 / 331.5 ms (water only, earlier today) | 311.5 / 325.9 ms |

## What I'd like to know

1. **Rocks (19–22, and the rocks in 1–3):** do the forms and surfaces now read as the more complex stone you meant? Anything too much (for example the stone joints) or still too simple?
2. **Water (1–6, 17–18):** do the wind and current read, and are there enough ripples?
3. If you approve this as the final rendering detail, say so explicitly. The tests then move to baseline-25, and task 15 (the Linux package) can start.
