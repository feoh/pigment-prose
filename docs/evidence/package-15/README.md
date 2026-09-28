# Task 15 evidence: the Linux package (2026-09-28)

**Artifact:** `target/package/pigment-prose-0.1.0-x86_64-linux.tar.zst` in the owner's checkout (not committed, not published), 30,425,948 bytes.

- SHA-256 `b33f564b05315eed7f7fe9ac8e8914153591235be25abad216538d2405024d90`
- Built by `packaging/linux/build-package.sh` from commit `83c1f69`, Rust 1.98.1, `Cargo.lock` SHA-256 `571c29ff…7285`. A rebuild from a fresh worktree at another path gave the same SHA-256.

**Test environment:** CachyOS (Arch-based), glibc 2.44, NVIDIA open kernel module and `nvidia-utils` 615.71.09, Mesa 26.2.3 (`vulkan-intel`), KDE Plasma on Wayland, NVIDIA GeForce RTX 4070 Ti and Intel Raptor Lake-S iGPU, wgpu 30.0.1 (Vulkan).

| File | What |
| --- | --- |
| `smoke-test-linux-2026-09-28.txt` | `packaging/linux/smoke-test.sh` on the artifact: 16/16 PASS (install to a scratch prefix, both programs, studio `--script` with an 8K export, a recipe reopened and exported at 8K and decoded, offline runs, notices, uninstall) |
| `window-packaged.png` | the installed studio's window at the end of its scripted run (mid-drag, so the "earlier settings" chip shows) |
| `clean-container-arch-2026-09-28.txt` | the artifact in a fresh `archlinux:latest` container on the same host with the Intel iGPU: no Vulkan loader → 0 adapters with an explanation; with `vulkan-icd-loader` and `vulkan-intel` → an 8K recipe export (a cold shader compile of 32.6 s, 4.1 GB); uninstall |

What these do and do not establish: [linux-package.md](../../linux-package.md#testing). The owner's manual checklist (menu entry, portal dialogs, a desktop first start) is there too; its results are not recorded yet.
