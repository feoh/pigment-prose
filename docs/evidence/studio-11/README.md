# Task 11 evidence: desktop shell and preview lifecycle

Linux 7.2 (CachyOS), KDE Plasma Wayland at 2× scale, NVIDIA GeForce RTX 4070 Ti, Vulkan, driver 615.71.09, wgpu 30.0.1, eframe/egui 0.36.2, 2026-09-27. Spec: [docs/studio.md](../../studio.md).

| File | What |
| --- | --- |
| [studio-linux-2026-09-27.txt](studio-linux-2026-09-27.txt) | Every scripted run: the responsiveness check with a simulated 1.5 s render (PASS) plus the app's GPU memory, the undelayed run with per-preview timings (PASS), simulated device loss (PASS), initialization failure (exit 1), and the iGPU that cannot present (exit 2) |
| [window.png](window.png) | The actual window captured by the app itself (egui screenshot command), full 2720×1720 at 2× |
| [window-delayed.png](window-delayed.png) | The same capture from the delayed run (50%) |
| [device-lost.png](device-lost.png) | The device-lost state (50%) |
| [init-failure.png](init-failure.png) | The initialization-failure window for `--adapter no-such-gpu` (50%) |
| [atspi-tree-linux-2026-09-27.txt](atspi-tree-linux-2026-09-27.txt) | The accessibility tree on AT-SPI ([`scripts/atspi-dump.py`](../../../scripts/atspi-dump.py)). The session's `IsEnabled` and `ScreenReaderEnabled` flags were switched on for the dump and back off afterwards |
| [../gpu-tests-linux-2026-09-27-task11.txt](../gpu-tests-linux-2026-09-27-task11.txt) | `scripts/gpu-tests.sh`, including `gpu_preview.rs` (1000-request storm) |

Reproduce: the commands at the top of each block in `studio-linux-2026-09-27.txt`, run from the repository root with `B=./target/release/pigment-studio` after `cargo build --release --locked -p pigment-studio`.
