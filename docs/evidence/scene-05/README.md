# Task 05 contact sheets: structure only, not paintings

These sheets show the **scene geometry** from the lakeshore generator (v0, pre-approval) through the debug renderer. `regions` colors each role, shades it by its structural `shade` and outlines every region boundary. `flat` shows neutral values with depth lightening. **There is no painting here.** Colors are debug codes, not a palette, and these images are not candidates for the task 08 visual review. The specification is [docs/scene-generation.md](../../scene-generation.md).

Region colors: sky pale blue, distant range lavender, mountain gray, foothills olive, framing ridges brown, water blue, shore tan, woodland green, rocks orange. Lighter or darker versions of a color are its `shade`, i.e. which way that plane faces the light.

Generated on 2026-09-26 on Linux with the NVIDIA GeForce RTX 4070 Ti (Vulkan, wgpu 30.0.1), form settings at their defaults unless named. Each `.png` has a `.txt` beside it with, per cell: the passage id, variation, template, mirroring, layer and vertex counts, geometry checksum and visible coverage by role.

| Sheet | What it shows |
| --- | --- |
| `corpus-16x9-{regions,flat}` | the ten fixture passages, variation 0, landscape |
| `corpus-9x16-{regions,flat}` | the same passages in portrait: recomposed, not cropped |
| `corpus-1x1-{regions,flat}` | the same passages in a square |
| `variations-shore-a-16x9-regions` | one passage, variations 0–9 ("Another Composition") |
| `form-{faceting,relief,density}-{0,1}-16x9-regions` | each form control at its extremes, all else default |
| `sample-40-16x9-regions` | 40 generated seeds (`sample passage 0` … `39`), for problem seeds |

Regenerate with (from the repository root, release build):

```text
B="cargo run --release -q -p pigment-cli -- contact-sheet"
D=docs/evidence/scene-05
for v in regions flat; do
  $B --out $D/corpus-16x9-$v.png --aspect 16:9 --view $v --cell 400
  $B --out $D/corpus-9x16-$v.png --aspect 9:16 --view $v --cell 420
  $B --out $D/corpus-1x1-$v.png  --aspect 1:1  --view $v --cell 320
done
$B --out $D/variations-shore-a-16x9-regions.png --passage shore-a --variations 10 --cell 400
for c in faceting relief density; do for x in 0 1; do
  $B --out $D/form-$c-$x-16x9-regions.png --$c $x --cell 320
done; done
$B --out $D/sample-40-16x9-regions.png --sample 40 --cell 320 --cols 8
```

**Cross-device check:** `corpus-16x9-regions` rendered on the Intel iGPU (Raptor Lake-S, Vulkan) had identical geometry: every checksum and coverage figure matched. Rendered debug pixels differed by at most 1/255 on 0.3 % of pixels (sRGB encoding), which is within the tier-3 expectation.

## What to look at

- **Structure reads at thumbnail size.** Every cell has a mountain silhouette, receding ranges, a connected far shoreline, a lake, a near shore and rocks, in distinct depth planes.
- **Planes.** Each summit has a lit flank and a shadowed flank, with the front spur between them. Secondary facets break the flanks into interlocking wedges. At `faceting 0` the planes are few and softly contrasted on rounded masses. At `faceting 1` they are angular and sharp.
- **Variation.** Across the corpus and variations all three templates occur, the light side and mirroring change, and the near-shore shape changes. Palette and paint settings cannot change any of it.

## Known problems and problem seeds

These were found by inspecting the sheets and the coverage columns. They are recorded, not hidden, as input for task 08 and any iteration of 05.

1. **Horizontal banding is similar across seeds.** In 16:9 the sky always covers about 34–48 % and the horizon falls in a narrow band, so scenes feel related but can look alike in layout. The templates vary the summits and the shore more than the horizon. Consider a wider horizon range, or a template with a high viewpoint or low horizon.
2. **Water-dominated foregrounds.** Even after raising the corner shores, some corner-shore scenes still give the foreground mostly to water. Examples: `sample passage 16` (water 31 %, rocks 0.4 %) and `sample passage 13` (shore 7 %). The rocks there are small.
3. **Symmetric bays in portrait.** `shore-b` and `line-break` at 9:16 form a U-shaped bay reaching toward the bottom centre between two framing ridges. It reads as staged.
4. **Crowded corners.** In `sample passage 13` and `sample passage 18`, near woods and rocks pile into the frame corner, partly cut off by the edge.
5. **Tiny summit slivers.** Where the two flank creases meet at a summit, the front spur between them can start as a sliver about 0.01 wide (visible in `emoji` at 9:16, full size). It is cosmetic, but it is a hard, thin shape.
6. **Distant summit small at low relief.** `framing-ridges` scenes at `relief 0` have a small framed summit (mountain coverage down to about 2.4 % in the 1,500-scene measurement).
7. **Woodland outlines are envelopes.** The scalloped crowns (tree-like at full size) are placement regions for task 07, not tree drawings, and should not be judged as foliage.
8. **Repetitive facet rhythm on long ranges.** On wide `twin-summits` or `framing-ridges` ranges with many summits, the flank-plus-facet pattern repeats at a similar scale.
