# Task 09 evidence: tiled high-resolution PNG export

Linux, NVIDIA GeForce RTX 4070 Ti, Vulkan, driver 615.71.09, wgpu 30.0.1, 2026-09-27. Spec: [docs/export.md](../../export.md).

| File | What |
| --- | --- |
| [export-linux-2026-09-27.txt](export-linux-2026-09-27.txt) | `pigment-prose export` runs: 8K at four tilings and orders with identical SHA-256, 16384×9216, 3000×4000 portrait, 4K square, the host-memory baseline, cancellation keeping the old file, refused sizes, an unwritable path, and the chunk list |
| [gpu-memory-linux-2026-09-27.txt](gpu-memory-linux-2026-09-27.txt) | Per-process GPU memory from `nvidia-smi` at four sizes (flat at 200 MiB) next to the cost-model estimate |
| [../gpu-tests-linux-2026-09-27-task09.txt](../gpu-tests-linux-2026-09-27-task09.txt) | `scripts/gpu-tests.sh`, including the new `pigment-io` hardware export suite |
| [tile-joins-1to1.png](tile-joins-1to1.png) | 1:1 crops, 512 px, each **centred on a four-tile corner**: the 8K export at (2048, 2048) (forest edge, rock, meadow) and at (4096, 2048) (meadow flowers, shoreline), and the 16K export at (6144, 4096) (a wide water wash). Inspected: no seam, no texture discontinuity, no shift in marks across the joins |
| [8k-overview.png](8k-overview.png), [portrait-overview.png](portrait-overview.png) | The 8K (sample 15) and 3000×4000 (sample 0, 3:4) exports, downscaled for viewing |

Reproduce:

```sh
cargo build --release --locked -p pigment-cli
./target/release/pigment-prose export --out 8k.png --sample 15
./target/release/pigment-prose export --out 8k-333.png --sample 15 --tile 333 --order reverse
sha256sum 8k.png 8k-333.png        # identical
cargo test --release --locked -p pigment-io --test gpu_export -- --ignored --test-threads=1 --nocapture
```
