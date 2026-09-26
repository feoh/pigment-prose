# Product brief

## Purpose and scope

Pigment Prose is a **private, local-first desktop creative tool** for making reproducible, high-resolution procedural fractal landscape paintings from arbitrary prose. Prose is *only* non-semantic seed material: do not parse its meaning, infer a subject or style, send it to a service, train on it, or embed museum reference art. A one-character edit may result in an entirely new scene. No mandatory cloud account or network access.

The first scene family is mountainous and wooded valleys; the initial review subject is a rocky, wooded lakeshore below a mountain ridge. Generate a fixed scenic painting, not a navigable 3D world. The art goal is solid forms with loose paint, described in [art direction](art-direction.md). Animals and buildings are not in the initial scope.

## MVP workflow

1. Paste prose and request a preview; show a clear empty-input state.
2. Explore composition variations *without changing the prose*. Adjust **Form** separately from **Paint Handling**; palette and atmosphere affect appearance but **must not rearrange geography or vegetation placements**. Preview updates should be non-blocking.
3. Save and reopen versioned recipes containing seed derivation/variation, generator and renderer versions, dimensions and artistic settings. Source text is private: any recipe inclusion must be explicit and visible; the seed alone cannot recover the prose.
4. Export clean, lossless images at 4K, 8K and validated custom pixel dimensions in landscape, portrait or square format. Plan seamless tiled export, bounded GPU memory, resolution-aware marks, progress and cancellation. No mandatory watermark or attribution; exclude original prose from image metadata by default and avoid accidentally embedding it in filenames or logs.

Later work adds biome-sensitive seasons and distinct biomes only after the first scene passes visual review. CPU-only rendering is a separate feasibility study, **not** an MVP promise.

## Platform and rights

Target the user's Linux machine with NVIDIA GeForce RTX 4070 first. Design interfaces and packaging for Windows (Direct3D 12) and macOS (Metal) from the outset, but validate on actual hardware before claiming support. GPU acceleration is required. Rust/wgpu (with Vulkan/Direct3D 12/Metal) is a *candidate to prototype*, not a selected stack; the fixed-scene rendering architecture, backend, UI framework and tiled-export strategy must be tested before selection.

Outputs should be clean and usable without application-imposed attribution or watermark; do not infer a legal guarantee of unrestricted downstream use from that product goal. Audit licenses and obligations of every bundled asset, font and dependency before distribution. Do not package museum images or train on them.

## Reproducibility contract to implement and test

- Define versioned text preprocessing before hashing: proposed v1 is Unicode NFC, normalize CRLF and CR to LF, then hash UTF-8 bytes. Preserve other whitespace and case. Reject empty or whitespace-only input *before* hashing; do not silently substitute a default scene. A nonblank passage with leading/trailing whitespace remains distinct from one without it. Record this algorithm and its version in the recipe; freeze only when tested in task 04.
- Derive independent, domain-separated streams for composition, terrain, vegetation and paint detail from the text seed and recorded variation. Form changes may move structural features; changing paint handling, palette or atmosphere must leave scene geometry and vegetation positions unchanged. Composition variation should intentionally change layout while keeping the same text. Validate invariance rather than trusting shared mutable RNG state.
- Given the same versioned generator, recipe and environment, recompute the same structural scene and comparable painting. Preview and export of a recipe should agree in geometry. Cross-GPU/backend or driver pixel identity is **not** promised: shader precision, blending and color management can vary. Record enough versions/settings to detect and explain changes; use structural assertions and perceptual review instead of cross-device byte equality.
- Keep brush scale and topology resolution-aware for different aspect ratios and tile sizes; exports must not reveal tile edges. Explicitly measure memory, time and image quality on the RTX 4070 in the architecture spike and export work. No performance or maximum-dimension guarantee is established yet.

This is a product specification, not evidence of a working GPU implementation or validated operating-system support.
