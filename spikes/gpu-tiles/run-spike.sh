#!/usr/bin/env bash
# Reproduces every measurement in docs/architecture-spike.md.
# Large renders go to out/ (gitignored, deleted by the caller when done);
# logs, crops and a small preview go to artifacts/ (committed).
set -euo pipefail
cd "$(dirname "$0")"

BIN=./target/release/gpu-tiles
OUT=out
ART=artifacts
mkdir -p "$OUT" "$ART"
cargo build --release --quiet

LOG="$ART/results.txt"
: >"$LOG"
say() { echo "$*" | tee -a "$LOG"; }
run() { say "\$ ${*/#.\/target\/release\//}"; { "$@" 2>&1; } | { grep -v '^MESA' || true; } | tee -a "$LOG"; say ""; }

# Peak whole-GPU memory while a command runs (nvidia-smi, 20 ms sampling).
# Includes the desktop session's own usage, so report peak minus baseline.
vram() {
    local samples
    samples=$(mktemp)
    nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits -lms 20 >"$samples" &
    local pid=$!
    sleep 0.3
    run "$@"
    sleep 0.2
    kill "$pid"
    wait "$pid" 2>/dev/null || true
    say "vram_mib baseline=$(head -1 "$samples") peak=$(sort -n "$samples" | tail -1) samples=$(wc -l <"$samples")"
    say ""
    rm -f "$samples"
}

say "# Pigment Prose task 02 spike results"
say "date: $(date -Iseconds)"
say "host: $(uname -srm); $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"
say "gpu: $(nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader)"
say "vulkan: $(vulkaninfo --summary 2>/dev/null | grep -m1 'Vulkan Instance Version' | cut -d: -f2 | xargs)"
say "toolchain: $(rustc --version); $(cargo --version)"
say ""

say "## 1. Adapter diagnostics"
run $BIN adapters --gl
say "software adapter check (Mesa llvmpipe exposed through GL):"
export_sw() { LIBGL_ALWAYS_SOFTWARE=1 __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json "$@"; }
run export_sw $BIN render --gl --adapter llvmpipe --width 64 --height 64 --out "$OUT/sw-refused.png" || true
run export_sw $BIN render --gl --adapter llvmpipe --allow-software --width 320 --height 180 --out "$OUT/sw.png"

say "## 2. Seam comparison at 3840x2160 (seed 7)"
run $BIN render --width 3840 --height 2160 --seed 7 --tile 0 --out "$OUT/ref-4k.png"
run $BIN render --width 3840 --height 2160 --seed 7 --tile 0 --out "$OUT/ref-4k-again.png"
run $BIN compare "$OUT/ref-4k.png" "$OUT/ref-4k-again.png"
for t in 2048 1024 1000 512 333 256 128; do
    run $BIN render --width 3840 --height 2160 --seed 7 --tile $t --out "$OUT/tiled-4k-$t.png"
    run $BIN compare "$OUT/ref-4k.png" "$OUT/tiled-4k-$t.png" --tile $t
done
say "negative controls (must differ, proving the comparison detects seams):"
run $BIN render --width 3840 --height 2160 --seed 7 --tile 512 --apron 0 --out "$OUT/neg-apron0.png"
run $BIN compare "$OUT/ref-4k.png" "$OUT/neg-apron0.png" --tile 512 --diff "$OUT/diff-apron0.png"
run $BIN render --width 3840 --height 2160 --seed 7 --tile 512 --apron 6 --out "$OUT/neg-apron6.png"
run $BIN compare "$OUT/ref-4k.png" "$OUT/neg-apron6.png" --tile 512
run $BIN render --width 3840 --height 2160 --seed 7 --tile 512 --tile-local-noise --out "$OUT/neg-localnoise.png"
run $BIN compare "$OUT/ref-4k.png" "$OUT/neg-localnoise.png" --tile 512 --diff "$OUT/diff-localnoise.png"

say "## 3. Portrait, square and odd custom sizes"
run $BIN render --width 2160 --height 3840 --seed 7 --out "$OUT/ref-portrait.png"
run $BIN render --width 2160 --height 3840 --seed 7 --tile 512 --out "$OUT/tiled-portrait.png"
run $BIN compare "$OUT/ref-portrait.png" "$OUT/tiled-portrait.png" --tile 512
run $BIN render --width 3000 --height 3000 --seed 7 --out "$OUT/ref-square.png"
run $BIN render --width 3000 --height 3000 --seed 7 --tile 700 --out "$OUT/tiled-square.png"
run $BIN compare "$OUT/ref-square.png" "$OUT/tiled-square.png" --tile 700
run $BIN render --width 5001 --height 7003 --seed 11 --out "$OUT/ref-odd.png"
run $BIN render --width 5001 --height 7003 --seed 11 --tile 1000 --out "$OUT/tiled-odd.png"
run $BIN compare "$OUT/ref-odd.png" "$OUT/tiled-odd.png" --tile 1000

say "## 4. 8K (7680x4320) export timing and memory"
say "context-only baseline (64x64 job):"
vram $BIN render --width 64 --height 64 --seed 7 --out "$OUT/tiny.png"
vram $BIN render --width 7680 --height 4320 --seed 7 --tile 0 --out "$OUT/ref-8k.png"
for t in 2048 1024 512 256; do
    vram $BIN render --width 7680 --height 4320 --seed 7 --tile $t --out "$OUT/tiled-8k-$t.png"
done
run $BIN compare "$OUT/ref-8k.png" "$OUT/tiled-8k-1024.png" --tile 1024
run $BIN compare "$OUT/ref-8k.png" "$OUT/tiled-8k-256.png" --tile 256
say "portable WebGPU default limits (max_texture_dimension_2d=8192):"
vram $BIN render --width 7680 --height 4320 --seed 7 --tile 1024 --default-limits --out "$OUT/tiled-8k-deflimits.png"
run $BIN compare "$OUT/tiled-8k-1024.png" "$OUT/tiled-8k-deflimits.png" --tile 1024
say "16K (15360x8640) exceeds the default 8192 texture limit as a single tile, but tiles fine:"
run $BIN render --width 15360 --height 8640 --seed 7 --tile 0 --default-limits --out "$OUT/refused-16k.png" || true
vram $BIN render --width 15360 --height 8640 --seed 7 --tile 2048 --default-limits --out "$OUT/tiled-16k.png"
say "(self-comparison below only reports the reference-free seam ratio for the 16K output)"
run $BIN compare "$OUT/tiled-16k.png" "$OUT/tiled-16k.png" --tile 2048
ls -l "$OUT"/tiled-8k-1024.png "$OUT"/tiled-16k.png | awk '{print $5, $9}' | tee -a "$LOG"
say ""

say "## 5. Preview timing (render + readback, no PNG; scene rebuilt each frame)"
run $BIN bench-preview --width 960 --height 540 --seed 7 --iters 60
run $BIN bench-preview --width 1280 --height 720 --seed 7 --iters 60
run $BIN bench-preview --width 1920 --height 1080 --seed 7 --iters 60
run $BIN bench-preview --width 3840 --height 2160 --seed 7 --iters 30
run $BIN bench-preview --width 3840 --height 2160 --seed 7 --iters 30 --tile 1024

say "## 6. Cross-device comparison (Intel iGPU, same shader and scene)"
run $BIN render --adapter intel --width 3840 --height 2160 --seed 7 --out "$OUT/intel-ref-4k.png"
run $BIN render --adapter intel --width 3840 --height 2160 --seed 7 --tile 512 --out "$OUT/intel-tiled-4k.png"
run $BIN compare "$OUT/intel-ref-4k.png" "$OUT/intel-tiled-4k.png" --tile 512
run $BIN compare "$OUT/ref-4k.png" "$OUT/intel-ref-4k.png" --diff "$OUT/diff-nv-intel.png"
run $BIN bench-preview --adapter intel --width 1280 --height 720 --seed 7 --iters 30

say "## 7. Resolution-aware texture: preview vs downsampled export"
run $BIN render --width 960 --height 540 --seed 7 --out "$OUT/preview-540.png"
run $BIN downsample "$OUT/ref-4k.png" 4 "$OUT/ref-4k-down4.png"
run $BIN compare "$OUT/preview-540.png" "$OUT/ref-4k-down4.png"
run $BIN downsample "$OUT/ref-8k.png" 8 "$OUT/ref-8k-down8.png"
run $BIN compare "$OUT/preview-540.png" "$OUT/ref-8k-down8.png"

say "## 8. Committed artifacts"
cp "$OUT/preview-540.png" "$ART/preview-960x540-seed7.png"
cp "$OUT/ref-4k-down4.png" "$ART/export-4k-downsampled-960x540-seed7.png"
# Tile-512 corner at (2048,1024), crossed by the diagnostic mark and a tree mark.
CROP="1856 960 384 384"
$BIN crop "$OUT/ref-4k.png" $CROP "$ART/seam-crop-reference.png" --scale 2
$BIN crop "$OUT/tiled-4k-512.png" $CROP "$ART/seam-crop-tiled512.png" --scale 2
# Tile-512 corner at (3584,1024), where the soft ridge edge and wash pooling
# (the finite-support effects) meet tile edges: the zero-apron failure is here.
CROP_SOFT="3392 832 384 384"
$BIN crop "$OUT/ref-4k.png" $CROP_SOFT "$ART/apron-crop-reference.png" --scale 2
$BIN crop "$OUT/neg-apron0.png" $CROP_SOFT "$ART/apron-crop-neg-apron0.png" --scale 2
$BIN crop "$OUT/diff-apron0.png" $CROP_SOFT "$ART/apron-crop-neg-apron0-diff32.png" --scale 2
$BIN crop "$OUT/neg-localnoise.png" $CROP "$ART/seam-crop-neg-localnoise.png" --scale 2
$BIN crop "$OUT/diff-localnoise.png" $CROP "$ART/seam-crop-neg-localnoise-diff32.png" --scale 2
$BIN crop "$OUT/diff-nv-intel.png" $CROP "$ART/crop-nvidia-vs-intel-diff32.png" --scale 2
# Same canvas region at preview and 8K scale: brush and grain scale with the canvas.
$BIN crop "$OUT/preview-540.png" 300 400 120 120 "$ART/scale-crop-540p.png" --scale 4
$BIN crop "$OUT/ref-8k.png" 2400 3200 960 960 "$ART/scale-crop-8k.png" --scale 1
sha256sum "$OUT"/ref-4k.png "$OUT"/tiled-4k-512.png "$OUT"/ref-8k.png "$OUT"/tiled-8k-1024.png | tee -a "$LOG"
say "done"
