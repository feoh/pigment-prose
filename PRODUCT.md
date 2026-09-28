# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

Native desktop application (not web, not mobile): Linux first, Windows and macOS designed for from the outset and validated later (tasks 21–22). Built with eframe/egui 0.36 on wgpu ([ADR 0001](docs/decisions/0001-renderer-and-desktop-shell.md)). Follow each desktop OS's conventions for menus, shortcuts (Ctrl on Linux/Windows, Cmd on macOS) and native file dialogs.

## Users

A person who writes or collects prose and wants paintings from it. Today that is the owner alone. The product is headed for an **eventual public release** (owner, 2026-09-28), so the studio must explain itself to someone who has never heard of it. When and how it is released is still a packaging decision (tasks 15, 21, 22): the repository stays private until then.

Two ways of working, about equally common:

- **Exploring:** paste any text, flip through many compositions, adjust the paint, and export the few that strike them (prints, wallpapers).
- **Keepsake:** the prose matters to them (their own writing, a passage they love). They work one painting for that text until it is right, then save it with its recipe.

## Product Purpose

Pigment Prose turns arbitrary prose into a reproducible, high-resolution landscape painting: solid Cézanne-like forms with loose Homer-like watercolor and gouache handling. The prose is **only** seed material. Its meaning is never read. The same text, variation and settings rebuild the same scene. Success is a painting that inspires awe, can be exported cleanly at 4K, 8K or a custom size, and can be recreated from its recipe later. Full brief: [docs/product-brief.md](docs/product-brief.md). Art goals: [docs/art-direction.md](docs/art-direction.md).

## Positioning

A private painting generator, not an AI image service. It runs entirely on the user's own GPU with no account, network or cloud. No model is trained on or prompted with anything. Text is a deterministic seed, not a prompt, so "a one-character edit gives a new scene" is expected behavior, not a bug. Structure and paint are separate: changing palette, atmosphere or paint handling never moves the mountains or the trees.

## Operating Context

- Long sessions at a desktop monitor (the owner's is a Linux machine with an RTX 4070 Ti and a high-DPI display at 2× scale).
- Inputs: pasted or typed prose, and recipe files (`*.recipe.json`, [docs/recipe-files.md](docs/recipe-files.md)) opened from disk.
- Outputs: lossless PNG exports and saved recipes. A recipe holds the source prose only if the user explicitly chooses that.
- Rendering is always non-blocking: the UI never waits on the GPU ([docs/studio.md](docs/studio.md)).

## Capabilities and Constraints

- **Controls** come from `settings::CONTROLS` ([docs/architecture.md](docs/architecture.md#controls-and-invalidation)). Main: Form, Edge Looseness, Wash / Gouache, Atmosphere, Color intensity, Palette. Advanced: Relief, Woodland density, Mark scale, Granulation, Paper grain. Also Shape (aspect ratio), which recomposes the painting. Form, Edge Looseness and Wash / Gouache are independent channels. A coupled "style" slider is not allowed.
- **Another Composition** changes the variation and keeps the text and every artistic setting.
- **Terminology:** prose, painting, composition (variation), recipe, shape, form, paint handling, atmosphere, palette, export.
- **Privacy:** prose is never logged or written to exports, filenames or error messages. It goes into a recipe only when the user opts in, visibly.
- **GPU required.** CPU-only rendering is a separate investigation (task 23). A software renderer is always labelled as such.
- **No fake features:** no season, biome, history or camera controls until they actually work (seasons and biomes are tasks 16–17).
- **Reproducibility:** the same composition is promised across machines. Identical pixels across GPUs are not.

## Brand Commitments

- Name: **Pigment Prose**.
- Voice in the UI: plain, calm and exact. Say what a control does to the painting in painter's terms ("rounded, eroded masses → angular, faceted planes"). Never overstate hardware status or reproducibility.
- Exported images carry no watermark, attribution or metadata about the prose.

## Evidence on Hand

- Approved paintings and their recipes: [docs/visual-review/baseline-25](docs/visual-review/baseline-25/README.md) (final rendering detail, task 25, 2026-09-28); the task 08 visual gate's are in [baseline-08](docs/visual-review/baseline-08/README.md) (2026-09-27).
- Studio shell screenshot and measurements: [docs/evidence/studio-11](docs/evidence/studio-11/README.md).
- Synthetic test passages: [fixtures/passages.json](fixtures/passages.json). There is no private or real user prose in the repository, and none may be added.
- There are no users, testimonials, reviews or public reception to cite.

## Product Principles

1. **The painting is the point.** The interface serves the canvas and gets out of its way.
2. **Change is predictable.** Every control says what it changes, and paint controls never rearrange the landscape.
3. **The prose stays private.** Keeping text is always an explicit, visible choice.
4. **Nothing pretends.** No fake controls, no overstated GPU or reproducibility claims, and no stale preview shown as current.
5. **Explore freely, lose nothing.** Browsing variations and adjusting paint is cheap, and any painting can be recovered through its recipe.

## Accessibility & Inclusion

- **Keyboard-first** (owner requirement): every action works from the keyboard, with shortcuts for the frequent ones (Another Composition, open, save, export, and nudging sliders), a visible focus ring and a logical focus order.
- **Low eye strain for long sessions** (owner requirement): comfortable default text size, restful contrast and no flashing or unnecessary motion. The painting must be judged against a neutral surround.
- Every control has an accessible name and value text (AccessKit/AT-SPI). A screen-reader session has not been tested yet (task 14).
- UI zoom (Ctrl+Plus, Ctrl+Minus, Ctrl+0) and the display's scale factor are honored.
