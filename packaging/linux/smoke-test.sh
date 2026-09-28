#!/usr/bin/env bash
# Smoke-test a built package, from the archive (task 15; docs/linux-package.md).
#
#   packaging/linux/smoke-test.sh ARCHIVE.tar.zst [LOG_FILE]
#
# Verifies the checksum, unpacks into a scratch directory, installs with
# --prefix into it (never into your home), and runs the installed programs:
# --version; the studio's scripted window check (types prose, previews,
# drags sliders, exports 8K, closes) with a screenshot; an 8K export of an
# approved recipe through the CLI, decoded and checked (size, chunks, no
# prose); both again offline (unshare -rn); the notices are installed and the
# menu entry validates; uninstall removes every installed file. Needs a
# hardware GPU and a desktop session. Exit 0 only if every check passes.
set -uo pipefail

archive="$(cd "$(dirname "${1:?usage: smoke-test.sh ARCHIVE.tar.zst [LOG]}")" && pwd)/$(basename "$1")"
log="${2:-}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
recipe="$repo/docs/visual-review/baseline-25/corpus-16x9/01.recipe.json"
work="$(mktemp -d /tmp/pigment-smoke-XXXXXX)"
trap 'rm -rf "$work"' EXIT
if [[ -n "$log" ]]; then
  exec > >(tee "$log") 2>&1
fi

results=()
check() { # check NAME COMMAND...
  local name="$1"
  shift
  echo "=== $name"
  if "$@"; then
    results+=("PASS  $name")
  else
    results+=("FAIL  $name")
  fi
}

# Decode a PNG with the stdlib: exact size, chunk list, no marker text.
png_ok() { # png_ok FILE W H
  python3 - "$@" <<'EOF'
import struct, sys, zlib
path, w, h = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
data = open(path, "rb").read()
assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
pos, chunks, idat = 8, [], b""
while pos < len(data):
    n, kind = struct.unpack(">I4s", data[pos:pos + 8])
    body = data[pos + 8:pos + 8 + n]
    assert zlib.crc32(kind + body) == struct.unpack(">I", data[pos + 8 + n:pos + 12 + n])[0], "bad CRC"
    chunks.append(kind.decode())
    if kind == b"IHDR":
        size = struct.unpack(">II", body[:8])
    if kind == b"IDAT":
        idat += body
    pos += 12 + n
raw = zlib.decompress(idat)
bpp = 3 if data[25] == 2 else 4
assert size == (w, h), f"size {size}, expected {(w, h)}"
assert len(raw) == h * (1 + w * bpp), "pixel data length"
assert sorted(set(chunks)) == ["IDAT", "IEND", "IHDR", "sRGB"], chunks
print(f"decoded {path.rsplit('/', 1)[-1]}: {w}x{h}, chunks {' '.join(dict.fromkeys(chunks))}, {len(data)} bytes")
EOF
}

echo "archive: $archive"
check "checksum matches" bash -c "cd '$(dirname "$archive")' && sha256sum -c '$(basename "$archive").sha256'"
check "unpacks" tar --zstd -xf "$archive" -C "$work"
pkg="$(find "$work" -mindepth 1 -maxdepth 1 -type d -name 'pigment-prose-*' | head -1)"
prefix="$work/prefix"
check "installs with --prefix" "$pkg/install.sh" --prefix "$prefix"
bin="$prefix/bin"
check "pigment-studio --version" "$bin/pigment-studio" --version
check "pigment-prose --version" "$bin/pigment-prose" --version
check "notices, license, guide and build info installed" test -s "$prefix/share/doc/pigment-prose/THIRD-PARTY-NOTICES.md" \
  -a -s "$prefix/share/doc/pigment-prose/LICENSE" -a -s "$prefix/share/doc/pigment-prose/user-guide.md" \
  -a -s "$prefix/share/doc/pigment-prose/BUILD-INFO.txt"
check "menu entry validates and runs the installed binary" bash -c \
  "desktop-file-validate '$prefix/share/applications/pigment-prose.desktop' && grep -qx 'Exec=\"$bin/pigment-studio\"' '$prefix/share/applications/pigment-prose.desktop'"
check "binaries carry no build-host paths" bash -c "! grep -a -q -e '$HOME' -e '$repo' '$bin/pigment-studio' '$bin/pigment-prose'"
cat "$prefix/share/doc/pigment-prose/BUILD-INFO.txt"
"$bin/pigment-prose" gpu-info 2>&1 | head -20

check "studio --script (preview, drags, 8K export, close)" "$bin/pigment-studio" --script --screenshot "$work/window.png"
[[ -f "$work/window.png" ]] && cp "$work/window.png" "${SMOKE_SCREENSHOT:-/dev/null}" 2>/dev/null
check "CLI reopens an approved recipe and exports 8K" "$bin/pigment-prose" export --recipe "$recipe" --size 8k --out "$work/recipe-8k.png"
check "8K PNG decodes: 7680x4320, IHDR sRGB IDAT IEND only" png_ok "$work/recipe-8k.png" 7680 4320
check "offline: CLI 8K export (unshare -rn)" unshare -rn "$bin/pigment-prose" export --recipe "$recipe" --size 8k --out "$work/offline-8k.png"
check "offline export is byte-identical" cmp "$work/recipe-8k.png" "$work/offline-8k.png"
check "offline: studio --script (unshare -rn)" unshare -rn "$bin/pigment-studio" --script

manifest="$prefix/share/pigment-prose/install-manifest.txt"
mapfile -t installed <"$manifest"
check "uninstall" "$prefix/share/pigment-prose/uninstall.sh"
leftover() {
  local left=0
  for f in "${installed[@]}"; do
    [[ -e "$f" ]] && { echo "left behind: $f"; left=1; }
  done
  [[ ! -e "$prefix/share/pigment-prose" && ! -e "$prefix/share/doc/pigment-prose" ]] || { echo "app directories left"; left=1; }
  return $left
}
check "uninstall removed every installed file" leftover

echo
echo "=== summary"
printf '%s\n' "${results[@]}"
! printf '%s\n' "${results[@]}" | grep -q '^FAIL'
