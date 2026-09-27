#!/usr/bin/env bash
# Portable checks: no GPU needed. This is exactly what CI runs.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
