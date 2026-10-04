# Linux package (task 15)

The Linux MVP ships as a **per-user tarball**: `pigment-prose-VERSION-x86_64-linux.tar.zst` with both programs, a menu entry, an icon, the license, the third-party notices and the [user guide](user-guide.md), plus `install.sh` and `uninstall.sh`. Files: [`packaging/linux/`](../packaging/linux/). Evidence: [evidence/package-15](evidence/package-15/README.md). Nothing is published: there is no GitHub release, signing or upload.

## Decision

| Option | For | Against |
| --- | --- | --- |
| **Per-user tarball (chosen)** | No root: install, run and uninstall can be tested completely without the owner's password. Works on any distribution with a new enough glibc and the run-time libraries. Nothing to download to build it | No dependency resolution: the README lists what the system must provide. glibc ≥ 2.44 as built here, so in practice rolling distributions only |
| Arch package (PKGBUILD) | Native install and dependency resolution on the test host's family | Arch-family only; installing needs sudo |
| AppImage | One file | appimagetool is not installed (a download); the GPU driver comes from the host anyway, and portal/Wayland integration is more fragile |
| Flatpak | Sandboxing, runtimes | The NVIDIA driver must match a runtime extension; the most moving parts |

The owner was asked on 2026-09-28 and did not choose, so the recommended option was taken. It is easy to change later: a PKGBUILD can wrap the same tarball.

## Requirements (what the package needs from the system)

Established by reading the binaries (`objdump -T` for glibc symbol versions, `ldd`, and the libraries loaded at run time) and by the [clean-container test](#clean-environment):

- **x86_64, glibc ≥ 2.44.** The binaries link only `libc`, `libm` and `libgcc_s`; they use GLIBC_2.44 symbols because they are built on the test host. `BUILD-INFO.txt` records the minimum for each build. An older glibc would need a build on an older base system (not done).
- **Vulkan:** the loader (`libvulkan.so.1`) and a hardware driver. Without the loader, `pigment-prose gpu-info` finds 0 adapters and explains what to install (measured in the clean container).
- **Window:** `libwayland-client` and `libxkbcommon` (Wayland), or `libX11`, `libX11-xcb`, `libxcb`, `libXcursor`, `libXi` (X11); `libEGL` from libglvnd is loaded as well.
- **D-Bus and an XDG desktop portal** with a backend for the desktop, for the Open, Save and Export dialogs (rfd's portal backend) and the accessibility tree. Without a portal the dialogs behave as cancelled.

On the test host (CachyOS) these come from `vulkan-icd-loader`, `nvidia-utils` 615.71.09, `vulkan-intel` (Mesa 26.2.3), `wayland`, `libxkbcommon`, `libx11`, `libxcb`, `libxcursor`, `libxi`, `libglvnd`, `dbus`, `xdg-desktop-portal` and `xdg-desktop-portal-kde`.

**Display servers:** KDE Plasma on Wayland was run. XWayland is present, but the X11 code path has not been run on its own. Plain X11 sessions, GNOME and other compositors have not been run. The known multi-GPU constraints are in [studio.md](studio.md#gpu-initialization-and-failures).

## Build

```sh
packaging/linux/build-package.sh            # -> target/package/pigment-prose-0.2.0-x86_64-linux.tar.zst (+ .sha256)
packaging/linux/smoke-test.sh target/package/pigment-prose-0.2.0-x86_64-linux.tar.zst
```

- **Locked:** `cargo build --release --locked` with the toolchain pinned in `rust-toolchain.toml` (Rust 1.98.1). `BUILD-INFO.txt` in the package records the commit, toolchain, target, `Cargo.lock` SHA-256 and minimum glibc. The script refuses uncommitted changes unless `--allow-dirty`, which BUILD-INFO then states.
- **Path independent:** the checkout, `~/.cargo` and `~/.rustup` paths are remapped out of the binaries (`--remap-path-prefix`), so they carry no home directory or user name (the smoke test checks this).
- **Reproducible:** file times come from the commit (`SOURCE_DATE_EPOCH`), and the archive is sorted with numeric owner 0 and no access or change times. Building commit `83c1f69` twice, from this checkout and from a fresh worktree at another path with an empty target directory, gave the same archive: SHA-256 `b33f564b05315eed7f7fe9ac8e8914153591235be25abad216538d2405024d90`. That is reproducibility on one machine with one toolchain, not an independent rebuild.
- **Notices first:** the build stops if `THIRD-PARTY-NOTICES.md` does not match the locked dependency graph.

## Contents

| Path in the archive | Installed to (default) |
| --- | --- |
| `bin/pigment-studio`, `bin/pigment-prose` | `~/.local/bin/` |
| `share/applications/pigment-prose.desktop` (its `Exec` rewritten to the installed path; `StartupWMClass` matches the Wayland app id) | `~/.local/share/applications/` |
| `share/icons/hicolor/scalable/apps/pigment-prose.svg` (original artwork, MIT) | `~/.local/share/icons/hicolor/scalable/apps/` |
| `share/doc/pigment-prose/`: `LICENSE`, `THIRD-PARTY-NOTICES.md`, `user-guide.md`, `BUILD-INFO.txt` | `~/.local/share/doc/pigment-prose/` |
| `install.sh`, `uninstall.sh`, `README.md` | `uninstall.sh` to `~/.local/share/pigment-prose/`, with `install-manifest.txt` |

`install.sh --prefix DIR` installs under `DIR/bin` and `DIR/share` instead. Installing over an existing install uninstalls it first. `uninstall.sh` removes exactly the manifest's files and then the app's own two directories if empty; shared directories (`bin`, `applications`, `icons`) stay. The program keeps no settings or other files, so recipes and images are never touched.

## Licenses and notices

**Project source:** MIT (`LICENSE`), decided by the owner on 2026-09-26.

**Dependencies:** `scripts/third-party-notices.py` walks `cargo metadata --locked --filter-platform x86_64-unknown-linux-gnu` from the two shipped binaries through normal and build dependencies (dev-dependencies are not shipped), and writes [`packaging/linux/THIRD-PARTY-NOTICES.md`](../packaging/linux/THIRD-PARTY-NOTICES.md): a table of all **258 crates** with declared and chosen license, then every license, copyright and NOTICE file they ship, each distinct text once. It parses SPDX expressions and fails on any license outside a permissive allowlist, or on a crate that ships no license file and cannot be used under Apache-2.0 (whose text alone meets its terms). CI and `scripts/check.sh` check that the file is current.

- Used under: MIT 173, Apache-2.0 64, Zlib 5, Apache-2.0 WITH LLVM-exception 4, Unlicense 3, BSD-3-Clause 2, BSD-2-Clause 2, ISC 1, 0BSD 1, MIT AND Unicode-3.0 1, Apache-2.0 AND MIT 1, and `epaint_default_fonts` under Apache-2.0 AND OFL-1.1 AND Ubuntu-font-1.0.
- `self_cell` is `Apache-2.0 OR GPL-2.0-only` and is used under Apache-2.0. No copyleft license applies.
- 18 crates (egui, eframe, emath, epaint, ecolor, egui-winit, egui-wgpu, egui_glow, epaint_default_fonts, the accesskit crates, profiling, spirv, gl_generator, khronos_api) ship no license file in their published package. All are available under Apache-2.0 and are listed against the canonical Apache License 2.0 text (`packaging/linux/licenses/Apache-2.0.txt`). None ships a NOTICE file.

**Fonts** (compiled into `pigment-studio`, interface only): Atkinson Hyperlegible Next (Regular, SemiBold) and Atkinson Hyperlegible Mono (Regular) under the SIL OFL 1.1; egui's defaults Ubuntu Light (Ubuntu Font Licence 1.0), Hack (MIT and Bitstream Vera), Noto Emoji (OFL 1.1) and emoji-icon-font (MIT), which draw the ◀ ▶ ↺ symbols. Their texts are in the notices. These licenses govern the font software (for example, the fonts may not be sold on their own), not documents or images made with a program that uses them.

**Other assets:** none. Paper grain, granulation, brush marks and every landscape feature are generated by the shader and the scene generator; the repository bundles no photographs, scans, museum images or textures. The icon is original.

**Images people make:** no bundled license imposes a watermark, attribution or non-commercial condition on exported images, and exports contain no fonts, glyphs or third-party artwork (chunks `IHDR sRGB IDAT IEND` only, checked by the smoke test). This is what the audit establishes about the software. It is not a legal guarantee about text users type in (for example, someone else's writing) or about how copyright treats generated images in a given jurisdiction; the user guide says so.

## Testing

**Automated, on the packaged artifact** (`packaging/linux/smoke-test.sh`, [log](evidence/package-15/smoke-test-linux-2026-09-28.txt)): 16/16 PASS on the test host. It checks the SHA-256, unpacks, installs with `--prefix` into a scratch directory, and runs `--version` for both programs. The notices, license, guide and build info are installed, the menu entry validates and points at the installed binary, and the binaries hold no build-host paths. The studio's scripted window check runs (types prose, previews, drags sliders, another composition, an 8K export, closes). The CLI reopens an approved recipe (`baseline-25/corpus-16x9/01`) and exports 8K on the RTX 4070 Ti, and the PNG decodes to 7680 × 4320 with only `IHDR sRGB IDAT IEND`. Both run again offline (`unshare -rn`), and the offline export is byte-identical. Uninstall removes every installed file.

<a id="clean-environment"></a>**Clean environment:** there is no second machine. A fresh `archlinux:latest` container on the same host (glibc 2.44, Intel iGPU passed through as `/dev/dri/renderD128`, no display) is the closest available ([log](evidence/package-15/clean-container-arch-2026-09-28.txt)). The checksum verified, the package installed with `--prefix`, and both `--version` checks ran. `gpu-info` with no Vulkan loader found 0 adapters and explained what to install. After adding only `vulkan-icd-loader` and `vulkan-intel`, it found the Intel GPU and exported the recipe at 8K (7680 × 4320, chunks as above), and uninstall left nothing. It shares the host's kernel and hardware, so it is **not** evidence of broad compatibility, and the NVIDIA path in a clean userland was not tested (no NVIDIA container runtime).

**What the container found:** with the driver's shader cache empty, the first pipeline build took **32.6 s with a 4.1 GB peak** on Mesa's Intel driver. On the host with caching disabled it took 33.5 s (4.2 GB) on Intel and 3.6 s (0.39 GB) on NVIDIA, against about 0.1 s warm. v1 was already 26.5 s and 3.1 s. The studio compiled on its UI thread before the window existed, and twice (previews and exports), so a first start showed nothing for about 34 s. It now compiles once off the UI thread (`painter::Preparation`) and opens at once with "Preparing the GPU painter…" ([studio.md](studio.md#gpu-initialization-and-failures)). Measured cold on Intel: the window at 0.3 s, the painter ready at 34 s, `--script` PASS. The first-start cost itself remains, and is documented in the user guide.

### Manual checklist (the owner, on the desktop)

Automated checks cannot drive the menu, the portal dialogs or a real first start. Mark each:

1. Unpack the archive and run `./install.sh`. `pigment-studio --version` prints `pigment-studio 0.2.0` (if `~/.local/bin` is on PATH).
2. **Pigment Prose** appears in the KDE application menu with its icon; launching it opens the window, and the task bar shows the icon, not a generic one.
3. Type a sentence. The preview paints and the status line names the RTX 4070 Ti.
4. **Save As…** (Ctrl+Shift+S) shows the portal's dialog; save `test.recipe.json`. Change a slider, then **Open…** it: the unsaved prompt appears; choose Don't save; the saved painting returns.
5. **Export PNG…** at 8K: the export bar counts tiles, the file opens in an image viewer at 7680 × 4320, and it has no visible watermark or text.
6. Run `~/.local/share/pigment-prose/uninstall.sh`. The menu entry, programs and documents are gone, and `test.recipe.json` and the PNG are still there.

## Known limits

- One distribution family, one desktop session, one machine. No second physical environment was available.
- glibc ≥ 2.44 (see Requirements).
- The first start after installing or a driver update is slow on some drivers (above).
- The `pigment-studio` binary is 143 MB unpacked (release build with line-table debug info, so panics and crash reports have file and line). The archive is 30 MB.
- No automatic updates, package registry, signing, or Windows and macOS packages (tasks 21–22).
