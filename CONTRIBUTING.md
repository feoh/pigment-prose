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
| **Hardware GPU checks** | `scripts/gpu-tests.sh [LOG_FILE]` |
| List adapters | `cargo run --release -p pigment-cli -- gpu-info` |
| GPU smoke test | `cargo run --release -p pigment-cli -- gpu-smoke [--width W --height H --tile T --adapter NAME]` |
| Contact sheet (paintings or debug views) | `cargo run --release -p pigment-cli -- contact-sheet --out SHEET.png [--aspect 9:16] [--view paint\|flat\|regions] [--sample N \| --samples I,J] [--palette ID --haze H ...]`; see [docs/visual-review/](docs/visual-review/) |
| Painting benchmark | `cargo run --release -p pigment-cli -- paint-bench [--sizes 960,1920,3840] [--sample N] [--stress] [--adapter NAME]` |
| Settings side-by-side | `contact-sheet ... --vary KEY=V1,V2` (paint or form keys, for example `--vary wash=0,0.25,1` or `--vary relief=0,1`) |
| Individual images with recipes | `contact-sheet ... --cells DIR` writes `DIR/NN.png` and `DIR/NN.recipe.json` (no source text) |
| Hardware test suite only | `cargo test --release --locked -p pigment-gpu --test gpu_hardware -- --ignored --test-threads=1` |

`scripts/gpu-tests.sh` builds the CLI, prints `gpu-info`, runs `gpu-smoke` and then the ignored hardware tests. It exits non-zero if any step fails. **A machine without a hardware GPU fails these checks; it never skips them into a pass.** A software rasterizer is refused by default. With `--allow-software` it runs, labelled SOFTWARE, and `gpu-smoke` still exits non-zero.

## Test layers

1. **Portable unit tests** (`crates/*/src/**`, `#[cfg(test)]`): contracts, validation, tile planning, the job model, invalidation, and frozen seed and scene checksums. They run in CI on Linux, Windows and macOS (`.github/workflows/ci.yml`). Any exact-value fixture here must hold on all three platforms (tiers 0–1 in the architecture doc).
2. **Hardware GPU tests** (`crates/pigment-gpu/tests/gpu_hardware.rs`, `#[ignore = "needs a hardware GPU…"]`): tiled vs single-tile byte identity, repeatability, cancellation, limits. Run them manually on real hardware and save the log under `docs/evidence/` with the date and OS in the file name.
3. **Visual review** (task 08 onward): contact sheets judged by a person against [docs/art-direction.md](docs/art-direction.md). Automated checks never count as visual approval.

## Evidence and claims

- Record measurements with the adapter, backend, driver, OS and wgpu version (`gpu-smoke` prints them). Label software adapters. Never present an estimate as a measurement.
- Windows (Direct3D 12) and macOS (Metal) stay "unverified" until they are run on real hardware.

## Dependencies

- Declare every third-party crate once in the root `[workspace.dependencies]` with a full minimum version, and refer to it from member crates with `.workspace = true`. Commit `Cargo.lock`.
- Before adding a dependency, check its license. Dependencies must be compatible with distributing the MIT-licensed project; so far all are permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode). Note new licenses in the change description; the distribution audit is task 15.
- Upgrade wgpu and egui together: the shell and renderer must share one wgpu major.

## Privacy rules for code

- Never log, print, panic with, or put into an error message any source prose or recipe contents. Use byte counts and field names. `NormalizedText`'s `Debug` shows only its length.
- Test passages come from `fixtures/passages.json` (synthetic). Keep private writing out of the repository.
- Exported images must carry no prose, paths, identity, watermark or attribution.

## Repository

This is a private project. Do not publish releases, packages or crates. The owner's standing go-ahead (2026-09-27): push to the private GitHub remote at the end of each major section or task, without asking. The project source is licensed under the [MIT License](LICENSE). That covers the code only; it adds no watermark, attribution or other terms to images the application exports.
