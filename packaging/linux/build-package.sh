#!/usr/bin/env bash
# Build the Linux per-user package (task 15; docs/linux-package.md).
#
#   packaging/linux/build-package.sh [--allow-dirty] [OUT_DIR]
#
# Produces OUT_DIR/pigment-prose-VERSION-x86_64-linux.tar.zst and a .sha256
# next to it (default OUT_DIR: target/package). The build is locked
# (`--locked`, the pinned toolchain in rust-toolchain.toml) and path
# independent: the checkout, cargo and rustup paths are remapped out of the
# binaries, file times come from the commit (SOURCE_DATE_EPOCH) and the
# archive is sorted with numeric owner 0, so the same commit and toolchain
# give the same archive. It refuses a dirty tree unless --allow-dirty (then
# BUILD-INFO says so). It never publishes or uploads anything.
set -euo pipefail

allow_dirty=0
if [[ "${1:-}" == "--allow-dirty" ]]; then
  allow_dirty=1
  shift
fi
repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"
out="${1:-$repo/target/package}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

dirty=""
if [[ -n "$(git status --porcelain)" ]]; then
  if [[ $allow_dirty -eq 0 ]]; then
    echo "build-package: the working tree has uncommitted changes; commit them or pass --allow-dirty" >&2
    exit 1
  fi
  dirty=" (with uncommitted changes)"
fi
commit="$(git rev-parse HEAD)"
SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
export SOURCE_DATE_EPOCH
version="$(cargo metadata --format-version 1 --locked --no-deps |
  python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "pigment-studio"))')"
name="pigment-prose-$version-x86_64-linux"

# The notices must match the dependency graph being shipped.
python3 scripts/third-party-notices.py --check packaging/linux/THIRD-PARTY-NOTICES.md

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_TARGET_DIR="$repo/target/package-build"
export RUSTFLAGS="--remap-path-prefix=$repo=/build/pigment-prose --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$rustup_home=/rustup"
cargo build --release --locked -p pigment-studio -p pigment-cli

stage="$out/stage"
rm -rf "$stage"
root="$stage/$name"
install -Dm755 "$CARGO_TARGET_DIR/release/pigment-studio" "$root/bin/pigment-studio"
install -Dm755 "$CARGO_TARGET_DIR/release/pigment-prose" "$root/bin/pigment-prose"
install -Dm644 packaging/linux/pigment-prose.desktop "$root/share/applications/pigment-prose.desktop"
install -Dm644 packaging/linux/pigment-prose.svg "$root/share/icons/hicolor/scalable/apps/pigment-prose.svg"
doc="$root/share/doc/pigment-prose"
install -Dm644 LICENSE "$doc/LICENSE"
install -Dm644 packaging/linux/THIRD-PARTY-NOTICES.md "$doc/THIRD-PARTY-NOTICES.md"
install -Dm644 docs/user-guide.md "$doc/user-guide.md"
install -Dm644 packaging/linux/README.md "$root/README.md"
install -Dm755 packaging/linux/install.sh "$root/install.sh"
install -Dm755 packaging/linux/uninstall.sh "$root/uninstall.sh"
{
  echo "Pigment Prose $version for x86_64 Linux"
  echo "commit: $commit$dirty"
  echo "commit date (SOURCE_DATE_EPOCH): $SOURCE_DATE_EPOCH"
  echo "toolchain: $(rustc -V), $(cargo -V)"
  echo "target: x86_64-unknown-linux-gnu, release profile, cargo build --locked"
  echo "Cargo.lock sha256: $(sha256sum Cargo.lock | cut -d' ' -f1)"
  echo "requires glibc >= $(objdump -T "$root/bin/pigment-studio" "$root/bin/pigment-prose" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)"
} >"$doc/BUILD-INFO.txt"
find "$root" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +

archive="$out/$name.tar.zst"
tar -C "$stage" --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner \
  --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
  -cf - "$name" | zstd -q -19 -T1 -o "$archive" -f
(cd "$out" && sha256sum "$name.tar.zst" >"$name.tar.zst.sha256")
rm -rf "$stage"
echo "built $archive"
cat "$archive.sha256"
