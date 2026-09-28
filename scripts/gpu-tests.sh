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
  cargo test --release --locked -p pigment-studio --test gpu_preview -- --ignored --test-threads=1 --nocapture
  # The studio window's states rendered offscreen with the real painter
  # (PNGs in target/studio-screens, or $PIGMENT_SCREENS).
  cargo test --release --locked -p pigment-studio --lib review_screens -- --ignored --nocapture
  # Task 14 qualification: recipe → scene → preview → tiled PNG, seams in
  # boundary bands, brush and texture scale, repeated exports.
  cargo test --release --locked -p pigment-studio --test qualification -- --ignored --test-threads=1 --nocapture
  # An 8K PNG exported through the studio's own export path (task 13).
  cargo test --release --locked -p pigment-studio --lib the_studio_exports_a_real_8k_png -- --ignored --nocapture
} 2>&1 | tee "$log"
