#!/usr/bin/env bash
# Remove a Pigment Prose install made by install.sh.
#
#   uninstall.sh [--quiet]
#
# Run the copy that install.sh put in share/pigment-prose/ (or this one from
# the unpacked package for the default location). It removes exactly the
# files in install-manifest.txt, then the app's own two directories if
# empty. It never touches your recipes or exported images: the program
# keeps no files of its own anywhere else.
set -euo pipefail

quiet=0
[[ "${1:-}" == "--quiet" ]] && quiet=1
here="$(cd "$(dirname "$0")" && pwd)"
if [[ -f "$here/install-manifest.txt" ]]; then
  state="$here"
else
  state="${XDG_DATA_HOME:-$HOME/.local/share}/pigment-prose"
fi
manifest="$state/install-manifest.txt"
if [[ ! -f "$manifest" ]]; then
  echo "uninstall.sh: no install found ($manifest is missing)" >&2
  exit 1
fi

while IFS= read -r f; do
  [[ -n "$f" ]] || continue
  rm -f -- "$f"
done <"$manifest"
rm -f -- "$manifest"
# Only the app's own directories; shared ones (bin, applications, icons)
# stay even if empty.
rmdir -- "$(dirname "$state")/doc/pigment-prose" 2>/dev/null || true
rmdir -- "$state" 2>/dev/null || true
[[ $quiet -eq 1 ]] || echo "Pigment Prose is uninstalled. Your recipes and images were not touched."
