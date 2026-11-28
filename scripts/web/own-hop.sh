#!/bin/bash
# The made-up game test/own-hop, a game with its own pictures and no ROM:
# a copy of the Moss Hop template (the project's own test game) with a
# package.toml of its own and a made-up large-art theme, hopglow, in
# themes/ (as gp game share puts a person's), one made-up picture for the
# hero in made-up colours. The page check's own-game sections are held to
# its title, its [page] lines and its theme (check-page.mjs, own-game*).
#   scripts/web/own-hop.sh OUT
# OUT must not exist. Used by scripts/web/test-registry.sh (the release
# check's registry) and scripts/web/check-page-no-roms.sh (just check-web).
# Exit codes: 0 made, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
if [ $# -ne 1 ]; then
  echo "usage: scripts/web/own-hop.sh OUT" >&2
  exit 2
fi
real="$(cd "$(dirname "$0")/../.." && pwd)"
own="$1"
[ ! -e "$own" ] || { echo "own-hop.sh: $own is there already" >&2; exit 2; }
cp -R "$real/templates/mosshop" "$own"
cat > "$own/package.toml" <<'TOML'
[package]
name = "test/own-hop"
version = "0.1.0"
kind = "game"
title = "Own Hop"

[page]
made_by = "the page check"
about = "Own Hop is made up for the page check: a copy of the Moss Hop template, with its pictures and no ROM."
TOML
python3 -I - "$own/themes/hopglow" <<'PY'
import os, struct, sys, zlib
d = sys.argv[1]
os.makedirs(f"{d}/art", exist_ok=True)
def png(path, w, h, rgba):
    raw = b"".join(b"\x00" + bytes(rgba) * w for _ in range(h))
    chunk = lambda t, b: struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
png(f"{d}/art/glow.png", 32, 32, [40, 220, 120, 255])
open(f"{d}/look.txt", "w").write("gp-look 1\nid hopglow\ntitle Hop Glow\nart art.txt\n")
open(f"{d}/art.txt", "w").write("gp-art 1\nscale 2\nlimit off\npic frame-1-0.png art/glow.png 16 16 0 0\n")
PY
