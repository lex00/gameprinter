#!/bin/bash
# Fetches the tools the playable web build needs (docs/web.md)
# into GP_HOME/work/web-tools/ and prints the directory that holds their
# executables:
#   scripts/web/fetch-tools.sh GP_HOME
#
# binaryen (for wasm-opt) is the GitHub release named below, checked against its pinned SHA-256
# before it's unpacked. The wasm32-unknown-unknown target comes from
# rustup, the official channel; this script only checks that it's there.
# A finished fetch is reused.
#
# Exit codes: 0 ready, 1 a download failed or its hash differs from the pin,
# 2 couldn't run (no pin for this platform, a missing tool).

set -euo pipefail
# A command failing unexpectedly means the script couldn't run; a failed
# download or a hash that differs exits 1 explicitly.
trap 'exit 2' ERR

if [ $# -ne 1 ]; then
  echo "usage: scripts/web/fetch-tools.sh GP_HOME" >&2
  exit 2
fi
home="$1"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
toolchain="${RUSTUP_TOOLCHAIN:-"$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["toolchain"]["channel"])' "$repo/rust-toolchain.toml")"}"

binaryen="version_133"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    platform="arm64-macos"
    sha256="ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8"
    ;;
  *)
    echo "fetch-tools: no pinned binaryen for $(uname -s) $(uname -m)." >&2
    echo "  Add the SHA-256 of binaryen-$binaryen-<platform>.tar.gz from https://github.com/WebAssembly/binaryen/releases/tag/$binaryen to $0." >&2
    exit 2
    ;;
esac

for tool in curl shasum tar cargo rustup python3; do
  command -v "$tool" >/dev/null || { echo "fetch-tools: needs $tool" >&2; exit 2; }
done

for tc in "$toolchain" "$(cd "$repo" && rustup show active-toolchain | cut -d' ' -f1)"; do
  installed="$(rustup target list --installed --toolchain "$tc")"
  if ! grep -qx wasm32-unknown-unknown <<<"$installed"; then
    echo "fetch-tools: toolchain $tc has no wasm32-unknown-unknown target." >&2
    echo "  rustup target add wasm32-unknown-unknown --toolchain $tc" >&2
    exit 2
  fi
done

dir="$home/work/web-tools"
bin="$dir/bin"
mkdir -p "$bin"

bdir="$dir/binaryen-$binaryen"
if ! { [ -x "$bdir/binaryen-$binaryen/bin/wasm-opt" ] && [ "$(cat "$bdir/.sha256" 2>/dev/null)" = "$sha256" ]; }; then
  mkdir -p "$bdir"
  tgz="$bdir/binaryen-$binaryen-$platform.tar.gz"
  url="https://github.com/WebAssembly/binaryen/releases/download/$binaryen/binaryen-$binaryen-$platform.tar.gz"
  if [ ! -f "$tgz" ]; then
    echo "fetch-tools: downloading binaryen $binaryen for $platform (about 8MB)" >&2
    curl -sSfL --retry 2 -o "$tgz.partial" "$url" || { echo "fetch-tools: download failed from $url" >&2; exit 1; }
    mv "$tgz.partial" "$tgz"
  fi
  actual="$(shasum -a 256 "$tgz" | cut -d' ' -f1)"
  if [ "$actual" != "$sha256" ]; then
    echo "fetch-tools: $tgz has SHA-256 $actual, expected $sha256" >&2
    echo "  move it aside and run again; if it still differs, the release changed upstream: stop and review before changing the pin" >&2
    exit 1
  fi
  tar -xzf "$tgz" -C "$bdir"
  echo "$sha256" > "$bdir/.sha256"
fi
ln -sf "$bdir/binaryen-$binaryen/bin/wasm-opt" "$bin/wasm-opt"
echo "$bin"
