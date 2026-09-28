#!/usr/bin/env bash
# Task 14 qualification sequence for the Linux MVP (docs/qualification.md).
#
#   scripts/qualify.sh [LOG_FILE]
#
# Runs, in order: the portable checks, the hardware GPU suite, the
# benchmark against the provisional targets, the real-window studio runs,
# privacy scans of logs and PNGs, offline operation, and unsupported-device
# behaviour. Each step prints PASS, FAIL or SKIPPED with its reason, and the
# summary lists them. Exit status: 0 only if nothing failed; 2 if no
# hardware GPU was found (reported as BLOCKED, never as a pass); 1 otherwise.
# A SKIPPED step is coverage that did not run on this machine, not a pass.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
# The privacy step imports scripts/seed-vectors.py; leave no bytecode behind.
export PYTHONDONTWRITEBYTECODE=1
stamp="$(date -u +%Y-%m-%dT%H%M%SZ)"
log="${1:-target/qualification/qualify-$stamp.txt}"
mkdir -p "$(dirname "$log")"
work="$(mktemp -d "${TMPDIR:-/tmp}/pigment-qualify-XXXXXX")"
trap 'rm -rf "$work"' EXIT
exec > >(tee "$log") 2>&1

declare -a summary
status=0
record() { # NAME RESULT [DETAIL]
  summary+=("$(printf '%-44s %s%s' "$1" "$2" "${3:+  ($3)}")")
  case "$2" in FAIL) status=1 ;; esac
}
step() { echo; echo "=== $* ==="; }

step "environment"
echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "git: $(git rev-parse --short HEAD 2>/dev/null || echo unknown)$(git diff --quiet 2>/dev/null || echo ' (uncommitted changes)')"
uname -srm
rustc --version
command -v nvidia-smi >/dev/null && nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
command -v vulkaninfo >/dev/null && vulkaninfo --summary 2>/dev/null | grep -E "deviceName|driverInfo|apiVersion" | head -8

step "1. portable checks (scripts/check.sh)"
if scripts/check.sh; then record "portable checks" PASS; else record "portable checks" FAIL; fi

step "building release binaries"
cargo build --release --locked -p pigment-cli -p pigment-studio || { record "release build" FAIL; }
cli=./target/release/pigment-prose
studio=./target/release/pigment-studio

step "hardware GPU present?"
$cli gpu-info
if ! $cli gpu-smoke >/dev/null 2>&1; then
  echo "BLOCKED: no hardware GPU passed gpu-smoke; the hardware steps cannot qualify."
  record "hardware GPU" BLOCKED "gpu-smoke failed or no adapter"
  printf '\n=== summary ===\n'; printf '%s\n' "${summary[@]}"
  exit 2
fi
record "hardware GPU" PASS "$($cli gpu-smoke | sed -n 's/^RESULT: PASS on //p')"

step "2. hardware GPU suite (scripts/gpu-tests.sh)"
if scripts/gpu-tests.sh; then record "hardware GPU suite" PASS; else record "hardware GPU suite" FAIL; fi

step "3. benchmark against the provisional targets (default adapter, --strict)"
if $cli bench --strict; then record "benchmark (default adapter)" PASS; else record "benchmark (default adapter)" FAIL "a target was missed"; fi
# A second, integrated adapter, if this machine has one: recorded, and the
# studio's adaptive settled cap is checked on it.
second="$($cli gpu-info 2>/dev/null | awk '/type=Integrated/ && /software=false/ {print prev} {prev=$0}' | head -1)"
if [ -n "$second" ]; then
  name="${second#- }"
  echo "second adapter: $name"
  $cli bench --adapter "$name" || true
  if PIGMENT_SLOW_ADAPTER="$name" cargo test --release --locked -p pigment-studio --lib a_slow_gpu -- --ignored --nocapture; then
    record "slow-GPU settled cap ($name)" PASS
  else
    record "slow-GPU settled cap ($name)" FAIL
  fi
  # Painting on the second adapter; the window shows wherever it can.
  if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    if $studio --script --adapter "$name" >"$work/second.log" 2>&1; then
      record "studio painting on $name" PASS "$(sed -n 's/^window: //p' "$work/second.log")"
    else
      cat "$work/second.log"; record "studio painting on $name" FAIL
    fi
  fi
else
  record "slow-GPU settled cap" SKIPPED "no second, integrated adapter"
  record "studio on a second adapter" SKIPPED "no second adapter"
fi

step "4. studio, real window (--script)"
for args in "" "--preview-delay-ms 1500" "--lose-device-after 3"; do
  # shellcheck disable=SC2086
  if $studio --script $args >"$work/studio.log" 2>&1; then
    record "studio --script $args" PASS
  else
    record "studio --script $args" FAIL
  fi
  cat "$work/studio.log"
  # The script types this synthetic prose into the editor; it must never
  # reach stdout or stderr.
  if grep -q "cold water" "$work/studio.log"; then
    record "studio log free of prose $args" FAIL
  else
    record "studio log free of prose $args" PASS
  fi
done

step "5. privacy: prose and path markers in CLI logs and PNG"
marker="Quillwort7Q3 zephyr lantern over marbled tarns"
dir="$work/pathmarker-K81"
mkdir -p "$dir"
# A recipe that keeps the marker prose. Its digest must match the text (the
# loader checks), so it comes from the independent seed reference.
python3 - "$dir/kept.recipe.json" "$marker" <<'EOF'
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("sv", "scripts/seed-vectors.py")
sv = importlib.util.module_from_spec(spec); spec.loader.exec_module(sv)
r = json.load(open("docs/visual-review/baseline-16/corpus-16x9/01.recipe.json"))
r["seed"]["digest"] = sv.digest(sys.argv[2]).hex()
r["source_text"] = sys.argv[2]
json.dump(r, open(sys.argv[1], "w"), indent=2)
EOF
if $cli export --recipe "$dir/kept.recipe.json" --size 1920x1080 --out "$dir/out-marker-Z52.png" >"$work/export.log" 2>&1; then
  cat "$work/export.log"
  leaks=""
  grep -q "Quillwort7Q3\|marbled tarns" "$work/export.log" && leaks="$leaks log"
  for m in "Quillwort7Q3" "marbled tarns" "pathmarker-K81" "out-marker-Z52" "source_text"; do
    grep -qaF "$m" "$dir/out-marker-Z52.png" && leaks="$leaks png:$m"
  done
  chunks="$(python3 - "$dir/out-marker-Z52.png" <<'EOF'
import struct, sys
b = open(sys.argv[1], "rb").read(); i = 8; out = []
while i + 8 <= len(b):
    n, t = struct.unpack(">I4s", b[i:i+8]); out.append(t.decode()); i += 12 + n
print(" ".join(dict.fromkeys(out)))
EOF
)"
  echo "chunks: $chunks"
  [ "$chunks" = "IHDR sRGB IDAT IEND" ] || leaks="$leaks chunks:$chunks"
  if [ -z "$leaks" ]; then record "no prose/path in CLI log or PNG" PASS; else record "no prose/path in CLI log or PNG" FAIL "$leaks"; fi
else
  cat "$work/export.log"; record "no prose/path in CLI log or PNG" FAIL "export failed"
fi

step "6. offline: generation and export with networking unavailable"
if unshare -rn true 2>/dev/null; then
  if unshare -rn $cli export --sample 3 --size 4k --out "$work/offline.png" && [ -s "$work/offline.png" ]; then
    record "offline CLI export (unshare -rn)" PASS
  else
    record "offline CLI export (unshare -rn)" FAIL
  fi
  if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] && unshare -rn $studio --script >"$work/offline-studio.log" 2>&1; then
    record "offline studio --script (unshare -rn)" PASS
  elif [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    record "offline studio --script" SKIPPED "no display"
  else
    cat "$work/offline-studio.log"; record "offline studio --script (unshare -rn)" FAIL
  fi
else
  record "offline checks" SKIPPED "unprivileged network namespaces unavailable"
fi

step "7. unsupported devices"
if $cli gpu-smoke --adapter no-such-gpu >/dev/null 2>&1; then
  record "unknown adapter refused (CLI)" FAIL "gpu-smoke passed"
else
  record "unknown adapter refused (CLI)" PASS
fi
# --screenshot makes the scripted failure window close itself.
$studio --adapter no-such-gpu --script --screenshot "$work/nogpu.png" >"$work/nogpu.log" 2>&1; code=$?
cat "$work/nogpu.log"
if [ "$code" -eq 1 ] && grep -q "no-such-gpu" "$work/nogpu.log" && [ -s "$work/nogpu.png" ]; then
  record "unknown adapter: studio explains, exits 1" PASS
else
  record "unknown adapter: studio explains, exits 1" FAIL "exit $code"
fi
if $cli gpu-info --allow-software 2>/dev/null | grep -q "software=true"; then
  swname="$($cli gpu-info --allow-software | awk '/software=true/ {print prev} {prev=$0}' | head -1 | sed 's/^- //')"
  $cli gpu-smoke --allow-software --adapter "$swname" >"$work/sw.log" 2>&1; code=$?
  cat "$work/sw.log"
  if [ "$code" -ne 0 ] && grep -q "SOFTWARE" "$work/sw.log"; then
    record "software adapter labelled, never a pass" PASS
  else
    record "software adapter labelled, never a pass" FAIL
  fi
else
  record "software adapter labelled" SKIPPED "no software adapter installed"
fi

printf '\n=== summary (%s) ===\n' "$stamp"
printf '%s\n' "${summary[@]}"
echo "log: $log"
exit $status
