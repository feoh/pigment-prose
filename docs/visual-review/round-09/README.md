# Rating round 9: mountainsides and rock forms (task 16, round 8 notes)

**Numbers run on across the sheets.** Rendered on 2026-09-28 on the RTX 4070 Ti. "Before" is round 8 (commit `1e68d4c`); "after" is the candidate (generator v3, renderer v3, [baseline-16](../baseline-16/README.md)).

| Sheet | Numbers | What it shows |
| --- | --- | --- |
| `sheet-a-mountains.png` | 1–4 | 1:1-scale mountain crops from 4K exports, before and after: 1 Golden evening at midsummer (your image 47), 2 Golden evening in autumn (image 48), 3 Lakeshore at midsummer (all seed 16), 4 seed 8 at midwinter (whole scene) |
| `sheet-b-rocks.png` | 5–6 | 5 `accent` at midwinter (your image 50), 6 the rocks of seed 16 in summer, before and after |
| `sheet-c-year.png` | 7–46 | round 8's year sheet again with both changes, to check that the seasons are as you approved them: columns 0.98, 0.00, 0.02, 0.25, 0.50, 0.62, 0.75, 0.86; rows seeds 15, 12, 16, 8 and `accent` |

## Your round 8 notes, and what changed

**Mountains: "you don't see huge FLAT areas of a single color and texture ... the interface between the 'green zone' and the more barren peak is also very simplistic and unnatural."**

- **Anatomy.** Every mountainside now has fall lines running down it, fanning across the massif: broad ribs and spurs, with finer couloirs (gullies) cut into them. Each rib is lit on its side toward the light and shaded on the other, within the big lit and shadowed planes you already had.
- **Rock** carries tilted strata, grain, broken cliff ledges (a shadow under a lit lip) and pale scree fans spilling below the cliffs.
- **The treeline** climbs higher in the gullies and drops lower on the ribs. It is ragged, with clumps of trees straggling above it. A band of alpine meadow, broken by rock outcrops on the ribs, lies between the forest and the bare rock.
- **The forest** follows the same ground more softly, with paler avalanche chutes running down the upper gullies.
- **Snow** on the high peaks reaches down the couloirs in fingers, with ribs showing through its edge. In winter, the seasonal snow also lingers longer in the gullies.

**Rocks: "a bit more variation in form ... You don't generally see very uniform anthill like shapes like that in nature."**

- **Proportions:** a rock of a given width can now be low and broad or tall, from 0.65 to 1.25 times the usual height.
- **Tops:** rounded boulders are worn flat on top to different degrees.
- **Lean:** every rock's crest leans to one side by its own amount, so they are no longer symmetric mounds.
- **Kinds:** more jointed blocks and tilted slabs, and fewer split boulders, whose twin peaks read most like anthills.

## Versions

Both changes alter the approved painting: `GENERATOR_VERSION` 3 (rock forms) and `RENDERER_VERSION` 3 (the seasons and the mountainsides). The checksums are re-frozen, and the tests use the candidate baseline-16. The task 25 baseline stays as it was; its schema 1 recipes are now the test of the schema migration.

## What I'd like to know

1. **Mountains (1–4):** do the mountainsides and the treeline now read as natural? Is it too busy anywhere, for example the gully streaks or the snow fingers?
2. **Rocks (5–6):** is that enough variety of form?
3. **Seasons (7–46):** still as you approved them?
4. If you approve, say so explicitly: task 16 then closes with baseline-16 as the approved baseline.
