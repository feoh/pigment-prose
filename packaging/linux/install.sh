#!/usr/bin/env bash
# Install Pigment Prose for the current user (no root needed).
#
#   ./install.sh [--prefix DIR]
#
# Default: programs in ~/.local/bin, the rest under $XDG_DATA_HOME (normally
# ~/.local/share). With --prefix DIR: DIR/bin and DIR/share. Every file
# installed is listed in share/pigment-prose/install-manifest.txt, which
# uninstall.sh (installed there too) reads. Installing again replaces the
# previous install. Nothing outside those directories is touched.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [[ "${1:-}" == "--prefix" ]]; then
  [[ -n "${2:-}" ]] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
  mkdir -p "$2"
  prefix="$(cd "$2" && pwd)"
  bin="$prefix/bin"
  share="$prefix/share"
elif [[ -z "${1:-}" ]]; then
  bin="$HOME/.local/bin"
  share="${XDG_DATA_HOME:-$HOME/.local/share}"
else
  echo "usage: install.sh [--prefix DIR]" >&2
  exit 2
fi
state="$share/pigment-prose"
manifest="$state/install-manifest.txt"

if [[ -f "$manifest" ]]; then
  echo "Replacing the existing install."
  "$state/uninstall.sh" --quiet
fi

mkdir -p "$state"
: >"$manifest.new"
put() { # put SRC DEST MODE
  install -Dm"$3" "$1" "$2"
  echo "$2" >>"$manifest.new"
}
put "$here/bin/pigment-studio" "$bin/pigment-studio" 755
put "$here/bin/pigment-prose" "$bin/pigment-prose" 755
put "$here/share/icons/hicolor/scalable/apps/pigment-prose.svg" \
  "$share/icons/hicolor/scalable/apps/pigment-prose.svg" 644
for f in LICENSE THIRD-PARTY-NOTICES.md user-guide.md BUILD-INFO.txt; do
  put "$here/share/doc/pigment-prose/$f" "$share/doc/pigment-prose/$f" 644
done
put "$here/uninstall.sh" "$state/uninstall.sh" 755
# The menu entry runs the installed program by its full path, so it works
# whether or not the bin directory is on PATH.
desktop="$share/applications/pigment-prose.desktop"
mkdir -p "$(dirname "$desktop")"
sed "s|^Exec=pigment-studio\$|Exec=\"$bin/pigment-studio\"|" \
  "$here/share/applications/pigment-prose.desktop" >"$desktop"
chmod 644 "$desktop"
echo "$desktop" >>"$manifest.new"
mv "$manifest.new" "$manifest"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" 2>/dev/null || true

echo "Installed Pigment Prose:"
echo "  programs: $bin/pigment-studio, $bin/pigment-prose"
echo "  menu entry: $desktop"
echo "  notices and guide: $share/doc/pigment-prose/"
echo "  uninstall: $state/uninstall.sh"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: $bin is not on your PATH; run the programs by their full path, or add it." ;;
esac
