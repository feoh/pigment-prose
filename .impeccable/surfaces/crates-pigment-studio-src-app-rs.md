---
version: 1
slug: "crates-pigment-studio-src-app-rs"
primary_target: "crates/pigment-studio/src/app.rs"
related_targets: []
---

# Studio window (pigment-studio)

Scope: the single studio window (top bar, left control column, painting area, status line, prompts). Visitor mode: **Operate**. Audience and job: see PRODUCT.md (Users). The painting is the product; the window is the instrument around it.

Constraints: native egui 0.36 on wgpu; keyboard-first; low eye strain for long sessions; the painting must be judged against a neutral surround; no fake controls; prose never shown outside the editor.

## Direction contract

THESIS: A graphite working console whose only colour is the painting. Persistent panels divided by 1 px seams; accents mark state, never decoration. It refuses the stock dark editor of floating rounded boxes, tinted section headers and sliders marching down a grey sidebar with no sense of what is current.

OWN-WORLD: Near-neutral graphite ground (surround #111214, panels #1a1c20, raised #202329), 1 px seams #2d3036, no shadows except the one modal. One cool signal accent (#6cc0e5) for the primary action, focus and slider fill; amber (#e0a64a) for pending and unsaved; coral (#ef7a6d) for errors. Atkinson Hyperlegible Next for all UI text, SemiBold small caps for panel headers, Atkinson Hyperlegible Mono for figures in one right-aligned value column. Square-ish 3 px corners.

STORY: Paste prose and the painting appears; read each control's two ends; drag and watch small previews become the settled painting; always know whether the picture is the current recipe; save or open without fear of losing work or leaking the prose.

FIRST VIEWPORT: 36 px top bar (wordmark, Open / Save / Save As / Export as quiet text actions, document name with an amber unsaved dot, Diagnostics at right). 360 px left column in three seam-separated sections: PROSE (editor, byte count and seed code, keep-prose checkbox), COMPOSITION (variation figure, Previous, primary "Another composition", Shape), PAINTING (STRUCTURE / PAINT HANDLING / APPEARANCE groups; slider rows with label and figure, track with a default notch, endpoint words below; Advanced collapsed). Centre: the painting 1:1 on the surround, a state chip only when it is not current. 28 px status line with adapter, figures and a state word.

FORM: graphite working console (dealt challenger, chosen by the user over the assigned field-tin direction, position 5 of the grounded list); seed key 94cc1f93. Signature interaction: the "now" state (chip plus status word: Current / Painting / Earlier settings) and the default notch on every slider. Motion grammar: none beyond egui's hover feedback; state changes are instant.

Raised from declined challengers: prose shown as a seed code, never as meaning (alphabet storm); one unambiguous "now" indicator (step row); tabular two-decimal figures in one column (datamatics); the recipe key on every control's tooltip (Miura sheet); the panel frame itself is the layout grid (elbow panel); destructive "Don't save" isolated from Save by empty space (console canon).

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Unresolved

- Screen-reader session (Orca, Narrator, VoiceOver) not yet run (task 14).
- Windows and macOS rendering of the theme unverified (tasks 21–22).
