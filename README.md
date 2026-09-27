# Pigment Prose

A local-first desktop painting studio in planning. Arbitrary prose supplies **non-semantic deterministic seeds**, not image prompts. The first target is a rocky wooded lakeshore beneath a mountain ridge, rendered as a fixed scenic painting rather than an explorable world.

- [Product brief](docs/product-brief.md)
- [Art direction and visual review](docs/art-direction.md)
- [Milestones, gates and architecture questions](docs/milestones.md)
- [Public synthetic seed passages](fixtures/README.md)
- [Architecture spike: GPU painting and tiled rendering](docs/architecture-spike.md)
- [Architecture and module contracts](docs/architecture.md) and [ADR 0001: Rust + wgpu renderer, eframe/egui shell](docs/decisions/0001-renderer-and-desktop-shell.md)
- [Build, test and contribution rules](CONTRIBUTING.md)

The Cargo workspace has three crates: `crates/pigment-core` (portable, tested contracts), `crates/pigment-gpu` (wgpu device, capability report and a tiled diagnostic renderer) and `crates/pigment-cli` (the `pigment-prose gpu-info` / `gpu-smoke` diagnostics). The GPU smoke path runs on Linux (Vulkan) on the RTX 4070 Ti and the Intel iGPU ([evidence](docs/evidence/gpu-smoke-linux-2026-09-26.txt)). Windows and macOS are unverified. Prose-to-seed derivation and the versioned recipe format are implemented and frozen ([spec](docs/seeds-and-recipes.md)). The structural lakeshore scene generator and its debug views exist ([spec](docs/scene-generation.md), [contact sheets](docs/evidence/scene-05/README.md)). There is still no painting renderer, image export, desktop application or published release.

```sh
scripts/check.sh       # portable format, lint and tests (no GPU needed)
scripts/gpu-tests.sh   # hardware GPU smoke run and test suite
```

The task 02 spike ([`spikes/gpu-tiles/`](spikes/gpu-tiles/)) is kept as a separate, throwaway Cargo project.

Source code is licensed under the [MIT License](LICENSE).
