# Contributing and build commands

Architecture and module contracts: [docs/architecture.md](docs/architecture.md). Stack decision: [ADR 0001](docs/decisions/0001-renderer-and-desktop-shell.md).

## Prerequisites

- **rustup.** The toolchain is pinned in `rust-toolchain.toml` (Rust 1.98, with rustfmt and clippy). rustup selects it automatically; `rustup toolchain install 1.98` if it is missing.
- **For the GPU suite only:** a hardware GPU with a current driver. On Linux, install the vendor Vulkan driver and check `vulkaninfo --summary`. Portable checks need no GPU.

## Commands

All commands run from the repository root. `--locked` makes Cargo fail rather than silently update `Cargo.lock`.

| Purpose | Command |
| --- | --- |
| Build everything | `cargo build --workspace --locked` |
| **Portable checks (what CI runs)** | `scripts/check.sh` |
| Format | `cargo fmt --all` (check only: `cargo fmt --all -- --check`) |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Portable tests | `cargo test --workspace --locked` (the GPU suite shows as *ignored*) |
| Seed reference check | `python3 scripts/seed-vectors.py --check` (stdlib only; `uv run` also works) |
| Third-party notices (regenerate after a dependency change; CI checks) | `python3 scripts/third-party-notices.py --out packaging/linux/THIRD-PARTY-NOTICES.md` |
| **Linux package** ([docs/linux-package.md](docs/linux-package.md); never published without the owner's permission) | `packaging/linux/build-package.sh`, then `packaging/linux/smoke-test.sh target/package/pigment-prose-VERSION-x86_64-linux.tar.zst [LOG]` |
| **Hardware GPU checks** | `scripts/gpu-tests.sh [LOG_FILE]` |
| **Full qualification** (clean checkout; [docs/qualification.md](docs/qualification.md)) | `scripts/qualify.sh [LOG_FILE]`: exit 0 only if nothing failed, 2 if no hardware GPU (BLOCKED) |
| Qualification benchmark against the targets | `cargo run --release -p pigment-cli -- bench [--adapter NAME] [--strict]` |
| Software-adapter study (task 23, CI runners, manual) | `gh workflow run software-adapter-study.yml`; compare paintings with `python3 scripts/png-compare.py REF_DIR CANDIDATE_DIR` |
| List adapters | `cargo run --release -p pigment-cli -- gpu-info` |
| GPU smoke test | `cargo run --release -p pigment-cli -- gpu-smoke [--width W --height H --tile T --adapter NAME]` |
| Contact sheet (paintings or debug views) | `cargo run --release -p pigment-cli -- contact-sheet --out SHEET.png [--aspect 9:16] [--view paint\|flat\|regions] [--sample N \| --samples I,J] [--palette ID --haze H ...]`; see [docs/visual-review/](docs/visual-review/) |
| Painting benchmark | `cargo run --release -p pigment-cli -- paint-bench [--sizes 960,1920,3840] [--sample N] [--stress] [--adapter NAME]` |
| Settings side-by-side | `contact-sheet ... --vary KEY=V1,V2` (paint or form keys, for example `--vary wash=0,0.25,1` or `--vary relief=0,1`) |
| Individual images with recipes | `contact-sheet ... --cells DIR` writes `DIR/NN.png` and `DIR/NN.recipe.json` (no source text) |
| Full-resolution PNG export | `cargo run --release -p pigment-cli -- export --out FILE.png (--recipe R.recipe.json \| --sample N \| --passage ID) [--size 4k\|8k\|WxH] [--tile T \| --gpu-budget MIB] [--order reverse] [--cancel-after N]`; see [docs/export.md](docs/export.md) |
| Hardware test suite only | `cargo test --release --locked -p pigment-gpu --test gpu_hardware -- --ignored --test-threads=1` |
| Desktop studio | `cargo run --release -p pigment-studio` (`-- --help` for options) |
| Studio acceptance run (real window) | `cargo run --release -p pigment-studio -- --script --preview-delay-ms 1500 --screenshot WINDOW.png`; exits non-zero if a check fails ([docs/studio.md](docs/studio.md)) |
| Studio review screenshots (offscreen, real painter) | `PIGMENT_SCREENS=DIR cargo test --release --locked -p pigment-studio --lib review_screens -- --ignored --nocapture` (default `target/studio-screens`) |
| Hardware export suite only | `cargo test --release --locked -p pigment-io --test gpu_export -- --ignored --test-threads=1 --nocapture` |

`scripts/gpu-tests.sh` builds the CLI, prints `gpu-info`, runs `gpu-smoke` and then the ignored hardware tests. It exits non-zero if any step fails. **A machine without a hardware GPU fails these checks; it never skips them into a pass.** A software rasterizer is refused by default. With `--allow-software` it runs, labelled SOFTWARE, and `gpu-smoke` still exits non-zero.

## Test layers

1. **Portable unit tests** (`crates/*/src/**`, `#[cfg(test)]`): contracts, validation, tile planning, the job model, invalidation, frozen seed and scene checksums, and the studio's interaction tests (headless egui_kittest with a CPU stand-in renderer). They run in CI on Linux, Windows and macOS (`.github/workflows/ci.yml`). Any exact-value fixture here must hold on all three platforms (tiers 0–1 in the architecture doc).
2. **Hardware GPU tests** (`crates/pigment-gpu/tests/gpu_hardware.rs`, `crates/pigment-io/tests/gpu_export.rs`, `crates/pigment-studio/tests/gpu_preview.rs`, `crates/pigment-studio/tests/qualification.rs`, `ui_tests::review_screens` and `ui_tests::the_studio_exports_a_real_8k_png_of_the_snapshot`, `#[ignore = "needs a hardware GPU…"]`): tiled vs single-tile byte identity, repeatability, cancellation, limits. Run them manually on real hardware and save the log under `docs/evidence/` with the date and OS in the file name.
3. **Visual review** (task 08 onward): contact sheets judged by a person against [docs/art-direction.md](docs/art-direction.md). Automated checks never count as visual approval.

## Evidence and claims

- Record measurements with the adapter, backend, driver, OS and wgpu version (`gpu-smoke` prints them). Label software adapters. Never present an estimate as a measurement.
- Windows (Direct3D 12) and macOS (Metal) stay "unverified" until they are run on real hardware.

## Dependencies

- Declare every third-party crate once in the root `[workspace.dependencies]` with a full minimum version, and refer to it from member crates with `.workspace = true`. Commit `Cargo.lock`.
- Before adding a dependency, check its license. Dependencies must be compatible with distributing the MIT-licensed project; so far all are permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode). Note new licenses in the change description; the distribution audit is task 15.
- Upgrade wgpu and egui together: the shell and renderer must share one wgpu major.

## Studio design

The studio's product context is [PRODUCT.md](PRODUCT.md) and its visual system is [DESIGN.md](DESIGN.md) (values in `crates/pigment-studio/src/theme.rs`). New UI inherits that system: graphite neutrals, one accent for the primary action and focus, amber and coral for state only, Atkinson Hyperlegible Next with mono for figures only. Bundled fonts are under the SIL OFL 1.1; any new font or asset needs its license recorded.

## Privacy rules for code

- Never log, print, panic with, or put into an error message any source prose or recipe contents. Use byte counts and field names. `NormalizedText`'s `Debug` shows only its length.
- Test passages come from `fixtures/passages.json` (synthetic). Keep private writing out of the repository.
- Exported images must carry no prose, paths, identity, watermark or attribution.

## Repository

This is a private project. Do not publish releases, packages or crates. The owner's standing go-ahead (2026-09-27): push to the private GitHub remote at the end of each major section or task, without asking. The project source is licensed under the [MIT License](LICENSE). That covers the code only; it adds no watermark, attribution or other terms to images the application exports.
