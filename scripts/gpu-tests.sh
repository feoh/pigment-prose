#!/usr/bin/env bash
# Hardware GPU checks. Fails (never skips) if no hardware adapter is found.
# Usage: scripts/gpu-tests.sh [LOG_FILE]
set -euo pipefail
cd "$(dirname "$0")/.."
log="${1:-/dev/null}"
{
  echo "# $(date -u +%Y-%m-%dT%H:%M:%SZ) $(uname -srm)"
  rustc --version
  cargo build --release --locked -p pigment-cli
  ./target/release/pigment-prose gpu-info
  ./target/release/pigment-prose gpu-smoke
  cargo test --release --locked -p pigment-gpu --test gpu_hardware -- --ignored --test-threads=1
  cargo test --release --locked -p pigment-io --test gpu_export -- --ignored --test-threads=1 --nocapture
} 2>&1 | tee "$log"
