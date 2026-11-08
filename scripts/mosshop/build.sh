#!/bin/bash
# Builds Moss Hop, the reference game (games/mosshop/), from source:
#   scripts/mosshop/build.sh OUT_DIR
# writes OUT_DIR/mosshop.nes, plus OUT_DIR/mosshop.dbg (ld65's debug file)
# and OUT_DIR/mosshop.labels (VICE labels) for tools that want addresses.
#
# Needs ca65 and ld65 from cc65 2.19, which reports itself as "ca65 V2.18"
# (Homebrew's build and Debian's 2.19-1 both do). The Docker image installs
# Debian's. The committed baselines under baselines/mosshop/ pin the ROM's
# SHA-256, so a toolchain that assembles different bytes fails
# `gp baseline check` there instead of passing quietly.
#
# Exit codes: 0 built, 2 couldn't build (missing tool or assembler error).

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: scripts/mosshop/build.sh OUT_DIR" >&2
  exit 2
fi
out="$1"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
src="$repo/games/mosshop"

missing=()
for tool in ca65 ld65; do
  command -v "$tool" >/dev/null || missing+=("$tool")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "mosshop: missing ${missing[*]}, needed to build the reference game." >&2
  case "$(uname -s)" in
    Darwin) echo "  ca65 and ld65 come with cc65: brew install cc65" >&2 ;;
    *) echo "  ca65 and ld65 come with cc65: apt install cc65 (or your distribution's package)" >&2 ;;
  esac
  echo "  Or run the checks in Docker, whose image has cc65 2.19: just docker-check" >&2
  exit 2
fi

# cc65 2.18 and 2.19 both print "ca65 V2.18", so this can only flag other
# versions; it can't tell 2.18 from 2.19. The baselines' SHA-256 is the real
# check.
version="$(ca65 --version 2>&1 | head -1)"
case "$version" in
  "ca65 V2.18"*) ;;
  *) echo "mosshop: note, $version doesn't report ca65 V2.18 as cc65 2.19 does; the baselines will say if the ROM differs" >&2 ;;
esac

mkdir -p "$out"
ca65 -g -I "$src" -o "$out/mosshop.o" "$src/mosshop.s" || exit 2
ld65 -C "$src/nrom.cfg" -o "$out/mosshop.nes" --dbgfile "$out/mosshop.dbg" -Ln "$out/mosshop.labels" "$out/mosshop.o" || exit 2
echo "mosshop: built $out/mosshop.nes ($version)"
