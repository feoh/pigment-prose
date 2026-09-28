---
name: Pigment Prose studio
description: A graphite working console whose only colour is the painting.
colors:
  surround: "#111214"
  panel: "#1a1c20"
  raised: "#22252a"
  hover: "#2a2e34"
  field: "#15171a"
  seam: "#2d3036"
  seam-strong: "#3d424a"
  ink: "#e4e6ea"
  ink-secondary: "#a9aeb7"
  ink-tertiary: "#8f959f"
  signal: "#6cc0e5"
  signal-dim: "#2f6179"
  on-signal: "#0b1a22"
  amber: "#e0a64a"
  coral: "#ef7a6d"
typography:
  body:
    fontFamily: "Atkinson Hyperlegible Next"
    fontSize: "15pt"
    fontWeight: 400
  title:
    fontFamily: "Atkinson Hyperlegible Next"
    fontSize: "15pt"
    fontWeight: 600
  headline:
    fontFamily: "Atkinson Hyperlegible Next"
    fontSize: "17pt"
    fontWeight: 600
  label:
    fontFamily: "Atkinson Hyperlegible Next"
    fontSize: "12.5pt"
    fontWeight: 600
    letterSpacing: "0.9pt"
  small:
    fontFamily: "Atkinson Hyperlegible Next"
    fontSize: "12.5pt"
    fontWeight: 400
  figure:
    fontFamily: "Atkinson Hyperlegible Mono"
    fontSize: "14pt"
    fontWeight: 400
rounded:
  control: "3pt"
  menu: "4pt"
  window: "6pt"
spacing:
  row: "6pt"
  item: "8pt"
  section: "14pt"
  panel-inset: "16pt"
  target: "28pt"
components:
  button-primary:
    backgroundColor: "{colors.signal}"
    textColor: "{colors.on-signal}"
    typography: "{typography.title}"
    rounded: "{rounded.control}"
    padding: "5pt 10pt"
    height: "28pt"
  button-default:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "5pt 10pt"
    height: "28pt"
  button-default-hover:
    backgroundColor: "{colors.hover}"
  button-quiet:
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "5pt 10pt"
  button-quiet-hover:
    backgroundColor: "{colors.hover}"
  toggle-on:
    backgroundColor: "{colors.signal-dim}"
    textColor: "{colors.ink}"
  field:
    backgroundColor: "{colors.field}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
  state-chip:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.ink}"
    typography: "{typography.small}"
    rounded: "{rounded.control}"
    padding: "5pt 10pt"
---

# Design System: Pigment Prose studio

Source of truth for the values: `crates/pigment-studio/src/theme.rs` (egui visuals, fonts and spacing). Units are egui points: logical pixels, scaled by the display's factor. The direction contract lives in `.impeccable/surfaces/crates-pigment-studio-src-app-rs.md`.

## Overview

**Creative North Star: "The Graphite Console"**

A working instrument around a painting. Persistent panels tile the window and are divided by 1 pt seams, never floated on shadows. Everything that is not the painting is near-neutral graphite, so the painting's colours read truly and it is always the brightest, most colourful thing on screen. It is a dark world because the use scene is long sessions at a desktop monitor judging colour: a dark neutral surround keeps glare down and the painting dominant.

Colour is spent on state, not decoration: one cool signal accent for the primary action, keyboard focus and slider fill, amber for anything pending or unsaved, coral for problems. Type is one legible family (Atkinson Hyperlegible Next, chosen for long sessions), with its mono sibling reserved for figures.

**Key Characteristics:**
- Graphite panels, 1 pt seams, one elevation step, no decorative shadows.
- One accent; state colours only where state exists.
- Small-capital panel headers; figures in a mono column on the right of every control row.
- The painting drawn 1:1 on a neutral surround, with an explicit "now" state.

## Colors

Near-neutral greys (channel spread ≤ 8/255, tested) with one cool signal and two state colours.

### Primary
- **Signal Cyan** (signal): the primary action ("Another composition", "Save…" in prompts), keyboard focus rings, the text caret and the focused or dragged slider handle.
- **Deep Signal** (signal-dim): text selection, the filled part of slider tracks, and toggles that are on ("Painting only", "Diagnostics").
- **Ink on Signal** (on-signal): text on the signal fill.

### Secondary
- **Pending Amber** (amber): the unsaved dot before the document name, "Painting…" and "Earlier settings" states, warnings, and the keep-prose note when prose will be saved.
- **Problem Coral** (coral): failures, "Don't save", the software-renderer label.

### Neutral
- **Surround** (surround): behind the painting.
- **Panel Graphite** (panel): top bar, control column, notice bar, status line.
- **Raised Graphite** (raised): buttons, combo boxes, menus, the modal, the state chip.
- **Hover Graphite** (hover): hovered and pressed button fills.
- **Field** (field): text-entry wells.
- **Seam** (seam) and **Strong Seam** (seam-strong): dividers between panels, and outlines of controls and windows (also the slider rail).
- **Ink** (ink), **Secondary Ink** (ink-secondary), **Tertiary Ink** (ink-tertiary): text in descending emphasis, each at least 4.5:1 on every ground (unit test `text_meets_contrast_on_every_ground`). Disabled controls are dimmed by egui rather than given their own ink.

### Named Rules
**The Only Colour Rule.** The painting is the only colourful thing on screen. Chrome takes colour only to report state or mark the one primary action.

**The State Palette Rule.** Amber means "not current yet", coral means "went wrong", signal means "act here or you are here". Never use them as decoration or as channel identities.

## Typography

**Body Font:** Atkinson Hyperlegible Next (egui's bundled fonts as fallback for other scripts)
**Label/Mono Font:** Atkinson Hyperlegible Mono

**Character:** a hyperlegible humanist sans with distinct letterforms, calm at small sizes; its mono sibling keeps figures aligned and steady while values change.

### Hierarchy
- **Headline** (SemiBold, 17 pt): the modal question only.
- **Title** (SemiBold, 15 pt): the wordmark, channel group names (Structure, Paint handling, Appearance), primary button text.
- **Body** (Regular, 15 pt): labels, buttons, editor text, messages.
- **Label** (SemiBold, 12.5 pt, +0.9 pt tracking, uppercase): panel section headers (PROSE, COMPOSITION, PAINTING, ADVANCED).
- **Small** (Regular, 12.5 pt): notes under headers, endpoint words, the status line.
- **Figure** (Mono, 14 pt; 12.5 pt in notes): slider values, the variation number, the seed code.

### Named Rules
**The Figures-Only Mono Rule.** Mono is for numbers and codes (values, the variation, the seed). Words stay in the proportional face.

## Layout

A fixed three-band window: a 38 pt top bar, the body, and a 28 pt status line. The body is a resizable control column on the left (360 pt default, 300–600 pt) and the painting area on the right with a 20 pt inset. An optional notice bar sits under the top bar and wraps long messages. The control column scrolls, and follows keyboard focus (a newly focused control is scrolled to the centre). **Painting only** (Ctrl+\\) collapses the column so the painting takes the window.

Inside the column: 16 pt left and 18 pt right insets, sections separated by a seam with 14 pt above and 10 pt below, 10 pt before each channel group and 6 pt between rows. Control rows are label and figure on one line, the full-width track, then the two endpoint words in two halves that wrap rather than collide. Hit targets are at least 28 pt tall.

The painting keeps the document's aspect ratio, fitted inside the area. It is drawn at 1:1 physical pixels once settled (up to a 3840 px long edge), and interaction previews are scaled up to that size, never stretched.

## Elevation & Depth

Flat by default. Depth is tonal: surround, then panel, then raised. The unsaved-changes modal is the only element with a shadow (0, 8 offset, 28 blur, black at 55%) and a dimmed backdrop. Menus and tooltips use a small popup shadow (0, 4 offset, 14 blur).

**The Seam Not Shadow Rule.** Panels are divided by 1 pt seams. A shadow means "this is on top of the window", and only the modal and popups qualify.

## Shapes

Nearly square corners: 3 pt on controls and chips, 4 pt on menus, 6 pt on windows and the modal. Slider handles are circles with a 2 pt panel-coloured ring. Status dots are 7 pt circles. No other ornament.

## Components

### Buttons
- **Primary:** signal fill, ink-on-signal SemiBold text, 3 pt corners, at least 28 pt tall. One per region ("Another composition" in the column, "Save…" in the prompt). Keyboard focus draws a 2 pt ink ring outside, because a signal ring would vanish on the signal fill.
- **Default:** raised fill with a strong-seam outline; hover lifts to the hover fill; focus shows the 2 pt signal ring.
- **Quiet:** top-bar actions (Open…, Save, Save As…, Export PNG…, Dismiss) have no frame at rest and the hover fill on hover.
- **Toggle:** a quiet button that shows deep signal while on (Painting only, Diagnostics).
- **Destructive:** "Don't save" is outlined in strong seam with coral text, and set apart on the left of the prompt, away from Cancel and Save.
- **Disabled:** dimmed by egui, with a hover note saying why ("Exporting images arrives in a later version", "This is the first composition").

### Control row (signature)
- **Line one:** the label in ink, and on the right the figure in mono. While the value is off its default, a small framed **Reset** button sits before the figure.
- **Track:** a 4 pt rail in strong seam, filled with deep signal up to the handle, and a 1.5 × 5 pt notch in tertiary ink under the default position. The handle is ink, white on hover, and signal when focused or dragged. Focus also draws a 1.5 pt signal outline around the track.
- **Line three:** the low-end and high-end meanings in tertiary small text.
- **Keyboard:** arrows step 0.01, Page Up/Down step 0.1, Home/End jump to the ends, Delete resets. The tooltip names the value, the two ends and the recipe key.

### Inputs / Fields
- **Style:** the field well with an outline, 3 pt corners, 8 × 6 pt inner margin.
- **Focus:** a 2 pt signal outline and a signal caret.
- **Error:** an amber dot and message under the field. The editor keeps the text, and the preview keeps the last painting.

### Navigation
The top bar is the only navigation: the wordmark, the file actions, the document name (secondary ink, with an amber dot while unsaved), then Painting only and Diagnostics on the right. Shortcuts use Ctrl on Linux and Windows, and ⌘ on macOS.

### State chip and status word (signature)
The painting always says what it is. The status line ends with a dot and a word: **Current** (tertiary), **Painting…** or **Earlier settings** (amber), **Preview failed** or **GPU reset** (coral). When the image on screen is not the current recipe, a chip at its top-left says so: a raised fill with a strong-seam outline, a coloured dot and small ink text.

### Notice bar
A panel-coloured strip under the top bar: a coloured dot, a SemiBold state word (Done, Note, Problem), then the message in ink, with Dismiss on the right. "Done" messages clear after 6 s; notes and problems stay until dismissed.

## Do's and Don'ts

### Do:
- **Do** keep every non-painting surface in the neutral greys (channel spread ≤ 8/255) so colour judgement is not tinted.
- **Do** use the signal colour for exactly one primary action per region, plus focus and slider fill.
- **Do** put numbers in the mono figure style, right-aligned on the control's first line.
- **Do** make keyboard focus unmistakable: a 2 pt signal ring, or an ink ring on signal fills.
- **Do** say plainly when the painting on screen is not the current settings.

### Don't:
- **Don't** colour section headers, channel groups or icons. Channels are told apart by headings and notes, not hues.
- **Don't** separate panels with shadows or floating cards. Use 1 pt seams.
- **Don't** set words in mono.
- **Don't** use Unicode glyphs as icons. Use text labels, or shapes drawn with the painter (dots, notches, handles).
- **Don't** put the destructive choice next to the primary one. Keep it outlined and apart.
