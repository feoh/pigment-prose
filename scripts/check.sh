#!/usr/bin/env bash
# Portable checks: no GPU needed. This is exactly what CI runs.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# The independent seed reference (stdlib Python) must agree with the fixture.
python3 scripts/seed-vectors.py --check
# The shipped third-party notices must match the locked dependency graph.
python3 scripts/third-party-notices.py --check packaging/linux/THIRD-PARTY-NOTICES.md
