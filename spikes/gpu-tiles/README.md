# gpu-tiles: task 02 architecture spike

Throwaway Rust/wgpu prototype that measures GPU procedural painting and seam-free tiled export on real hardware. It is **not** the product renderer, and its image is **not** a visual-quality candidate. Findings, method and numbers are in [`docs/architecture-spike.md`](../../docs/architecture-spike.md).

```sh
cargo build --release
./target/release/gpu-tiles adapters --gl
./target/release/gpu-tiles render --width 7680 --height 4320 --tile 1024 --seed 7 --out out/8k.png
./target/release/gpu-tiles compare out/ref.png out/tiled.png --tile 1024
./run-spike.sh   # every measurement in the doc; writes artifacts/results.txt
```

- `src/scene.rs`: CPU scene description from domain-separated seed streams.
- `src/gpu.rs`: adapter ranking and software refusal, device setup, the tiled three-pass compute renderer with streaming readback.
- `src/paint.wgsl`: whole-image-coordinate painting: glazes, facets, soft edge, wash pooling, marks, band-limited paper.
- `src/main.rs`: CLI, PNG streaming, exact comparison and seam statistics, crop and downsample helpers.
- `artifacts/`: the committed log and small crops from the recorded run. `out/` is scratch and gitignored.

Inputs are numeric seeds only; no prose is read or logged. Output PNGs carry no text metadata.
