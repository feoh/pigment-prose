# Art direction and review rubric

**The goal is awe.** A painting should make the viewer feel the vastness of the place: sweeping vistas, towering scale, an expanse seen from above, and dramatic light and sky (user direction, 2026-09-26). Everything below serves that.

**Solid forms, loose paint.** Seek Cézanne-like interlocking color planes and substantial geometry, with Winslow Homer-like watercolor handling: luminous translucent washes, selective crisp and soft edges, and restrained gouache-like opaque foreground accents. These are visual principles, not a request to reproduce a particular artwork. No training on, embedding, bundling or tracing the referenced works is required or planned.

## First painting

A rocky wooded lakeshore below a mountain ridge, in the initial family of mountains and wooded valleys. Compose a readable foreground/midground/background at thumbnail scale:

- **Foreground:** weighty, irregular rock planes, shore contours and a few intentional opaque accents. Preserve negative space and varied edges rather than outlining every object.
- **Midground:** coherent lake plane with restrained reflections; wooded masses with irregular spacing and differentiated silhouettes, not identical tree stamps. Interlock foliage, shore and mountain foothills.
- **Background:** a legible mountain-ridge silhouette and receding planes, with lower contrast, atmospheric transparency and selective soft boundaries; avoid mechanically smooth gradients.

Group light, middle and dark **values** into intelligible masses before adding detail. Keep adjacent color planes deliberate and connected. Use a limited, harmonious palette with selective temperature shifts, luminous washes and carefully placed hard accents. Paper grain and granulation should support pigment behavior, never become global confetti or overpower composition. Brush scale should remain convincing at preview and export resolution.

## Review criteria

Use the same public [seed corpus](../fixtures/README.md) for multi-seed contact sheets. Compare at thumbnail and full size, including portrait and square crops when available. Ask the user for **explicit visual approval** before expanding into export/studio work; a successful render or automated image comparison is not approval.

| Criterion | Observable check |
| --- | --- |
| Awe | The scene feels vast. Size reads from scale cues (tiny trees at the foot of a giant, a cliff edge above a valley), height and expanse are generous, framing sits at the edges with the centre open, and the sky and light are staged, not blank. Check with the proxies in [awe metrics](#awe-metrics) and, above all, with the user's ratings in `docs/visual-review/`. |
| Structure | Ridge, lake, shore and wooded masses are identifiable with distinct depth planes and readable silhouettes. |
| Value and color | Three-ish value groups support focal hierarchy; interlocking planes and deliberate palette survive grayscale/thumbnail inspection. |
| Paint handling | Transparent distance, varied wash edges and selective opaque foreground accents read as hand-painted rather than a uniform filter. |
| Variation | Multiple text seeds and composition variations feel related but not cloned; palette/atmosphere edits leave structural geography intact. |
| Detail | Texture, branches and brushwork have meaningful scale and density at both preview and high-resolution crop. |
| Export | No tile seams, resolution-dependent stamp artifacts, cut-off strokes or accidental metadata/attribution. |

Reject or revise images with uniform blur, confetti noise, muddy values, identical/repeating tree stamps, overly even edge sharpness, synthetic airbrush gradients, obvious fractal/heightmap bands, or visible tile seams. Compare successful and failed crops and note the seed, recipe version, settings and renderer/backend used; do not mistake a single lucky seed for general quality.

## Awe metrics

Awe is judged by a person. The scene generator reports measurable parts of the devices painters use for vastness (`pigment_core::scene::metrics`), printed in every contact sheet's notes:

| Metric | Device | Starting direction |
| --- | --- | --- |
| `rise` | summit height above the far shore, as a fraction of the frame height ("high distance") | tower compositions about 0.5 or more |
| `scale` | summit rise ÷ height of the trees at its foot. Near 1, the mountain reads as a hill. | 20× or more |
| `planes` | distinct visible depth planes of ridges and spurs | high-vantage compositions 7–10 |
| `expanse` | lake and valley laid out below the horizon ("level distance") | high-vantage compositions about 0.5 |
| `clutter` | near foreground in the central third | low: frame at the edges and keep the centre open |
| `sky` | visible cloud coverage | some structure in most skies |
| `light` | spread of structural shade: light staged against shadow | higher in dramatic scenes |

These targets are guesses until calibrated: rating rounds log the user's "vastness" and "stop" scores against the metrics, and the targets and template weights follow whatever actually predicts those scores.

Beyond the Homer and Cézanne references, painters of the sublime and of vast views are useful for talking about composition: Guo Xi's "three distances" (high, deep and level), the Hudson River School, Caspar David Friedrich, and Cézanne's Mont Sainte-Victoire seen across the plain. As with the other references, these are for discussion only and are not ingested.

## Reference viewing (links, not assets)

- [Metropolitan Museum of Art collection reference](https://www.metmuseum.org/art/collection/search/435877)
- [Harvard Art Museums collection reference](https://harvardartmuseums.org/collections/object/307937)
- [Art Institute of Chicago Homer reference](https://archive.artic.edu/homer/artwork/16785)

These public pages support a human art-direction conversation only. Do not ingest their image files into the product or imply their reproduction rights are granted.
