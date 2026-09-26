# Pigment Prose

A local-first desktop painting studio in planning. Arbitrary prose supplies **non-semantic deterministic seeds**, not image prompts. The first target is a rocky wooded lakeshore beneath a mountain ridge, rendered as a fixed scenic painting rather than an explorable world.

- [Product brief](docs/product-brief.md)
- [Art direction and visual review](docs/art-direction.md)
- [Milestones, gates and architecture questions](docs/milestones.md)
- [Public synthetic seed passages](fixtures/README.md)
- [Architecture spike: GPU painting and tiled rendering](docs/architecture-spike.md)

This repository contains specifications, test passages and a throwaway architecture spike ([`spikes/gpu-tiles/`](spikes/gpu-tiles/)). The spike validated Rust/wgpu compute painting and seam-free tiled export on Linux (Vulkan, RTX 4070 Ti), and that stack is the recommended backend. Windows and macOS are unverified. There is no product renderer, application or published release yet.
