# Pigment Prose

A local-first desktop painting studio in planning. Arbitrary prose supplies **non-semantic deterministic seeds**, not image prompts. The first target is a rocky wooded lakeshore beneath a mountain ridge, rendered as a fixed scenic painting rather than an explorable world.

- [Product brief](docs/product-brief.md)
- [Art direction and visual review](docs/art-direction.md)
- [Milestones, gates and architecture questions](docs/milestones.md)
- [Public synthetic seed passages](fixtures/README.md)
- [Architecture spike: GPU painting and tiled rendering](docs/architecture-spike.md)
- [High-resolution PNG export](docs/export.md)
- [Recipe files and the source-text choice](docs/recipe-files.md)
- [The desktop studio](docs/studio.md)
- [Architecture and module contracts](docs/architecture.md) and [ADR 0001: Rust + wgpu renderer, eframe/egui shell](docs/decisions/0001-renderer-and-desktop-shell.md)
- [Build, test and contribution rules](CONTRIBUTING.md)

The Cargo workspace has five crates: `crates/pigment-core` (portable, tested contracts), `crates/pigment-gpu` (wgpu device, capability report and the tiled renderers), `crates/pigment-io` (streaming PNG export, recipe files and the document model), `crates/pigment-studio` (the eframe/egui desktop app) and `crates/pigment-cli` (the `pigment-prose` diagnostics and `export`). The GPU smoke path runs on Linux (Vulkan) on the RTX 4070 Ti and the Intel iGPU ([evidence](docs/evidence/gpu-smoke-linux-2026-09-26.txt)). Windows and macOS are unverified. Prose-to-seed derivation and the versioned recipe format are implemented and frozen ([spec](docs/seeds-and-recipes.md)). The structural lakeshore scene generator and its debug views exist ([spec](docs/scene-generation.md), [contact sheets](docs/evidence/scene-05/README.md)). The GPU painting renderer (color planes, washes, gouache, loose edges, varied woodland, faceted rocks and water reflections) exists, and its direction was accepted in visual review ([spec](docs/painting.md)). It passed the task 08 visual gate on 2026-09-27. The final rendering detail (wind and current on water, more complex rocks) was approved on 2026-09-28, and generator and renderer are v2 ([baseline](docs/visual-review/baseline-25/README.md)). Recipes save and reopen through an atomic, validated file layer, and the source text is kept only by explicit choice ([spec](docs/recipe-files.md)). High-resolution PNG export exists: bounded-memory tiles streamed into an atomically finalized file, verified at 8K and 16384×9216 ([spec](docs/export.md)). A desktop studio window exists ([spec](docs/studio.md)): prose editor, live GPU preview kept off the UI thread, the artistic controls (structure kept apart from paint handling), composition variations, recipe open/save with native dialogs and an unsaved-changes prompt, full keyboard control, its own visual system ([DESIGN.md](DESIGN.md)), and 4K/8K/custom PNG export with progress and cancellation. The integrated Linux build passes a repeatable qualification sequence from a clean checkout ([qualification.md](docs/qualification.md)). A per-user Linux package builds reproducibly and passes its smoke test on the test machine, with generated third-party notices and a [user guide](docs/user-guide.md) ([linux-package.md](docs/linux-package.md)). There is no published release.

```sh
scripts/check.sh       # portable format, lint and tests (no GPU needed)
scripts/gpu-tests.sh   # hardware GPU smoke run and test suite
```

The task 02 spike ([`spikes/gpu-tiles/`](spikes/gpu-tiles/)) is kept as a separate, throwaway Cargo project.

Source code is licensed under the [MIT License](LICENSE).
