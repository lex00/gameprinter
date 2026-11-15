#!/bin/bash
# Moss Hop's preview with two made-up large-art themes, for the page
# check's theme-hud-over section (check-page.mjs; docs/web.md, "Checks"):
#   scripts/web/mosshop-hud.sh OUT
#
# OUT gets what gp-web-prep writes for a copy of templates/mosshop whose
# HUD counters are drawn in the background (Moss Hop's own are sprites,
# which every look draws over the bodies already), and two themes in
# OUT/themes/: `tall` draws the hero 200 pixels tall, so its picture
# stands across the HUD's row, and `tallover` is the same with the line
# `#+ hud_over`, and `instart` (the theme-in-region section) is `tall`
# with a shorter picture in Moss Hop's one region of the stage, `start`
# (its level kit; gp-web-prep writes it into preview.json). The page check
# serves OUT beside the page's own player files, so the frame is drawn by
# the page's gp_web.wasm and preview.js.
#
# Moss Hop's art comes from its own ROM, which `just mosshop` assembles
# into target/mosshop/mosshop.nes: lifted into $GP_HOME/work/assets/mosshop
# (as `just check` does) when that folder has no lift yet. GP names the gp
# to run (default target/debug/gp). Nothing of another game is read.
# Exit codes: 0 made, 2 couldn't, 3 no Moss Hop ROM to lift its art from
# (nothing was tried: the release check notes it and skips the sections).
set -euo pipefail
trap 'exit 2' ERR
if [ $# -ne 1 ]; then
  echo "usage: scripts/web/mosshop-hud.sh OUT" >&2
  exit 2
fi
repo="$(cd "$(dirname "$0")/../.." && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
gp="${GP:-$repo/target/debug/gp}"
target="$(cd "$repo" && mkdir -p "${CARGO_TARGET_DIR:-target}" && cd "${CARGO_TARGET_DIR:-target}" && pwd)"
out="$1"
assets="$home/work/assets/mosshop"
if [ ! -f "$assets/assets.toml" ]; then
  rom="$target/mosshop/mosshop.nes"
  [ -f "$rom" ] || { echo "mosshop-hud: no Moss Hop ROM at $rom; run \`just mosshop\`" >&2; exit 3; }
  "$gp" assets lift "$rom" --map "$repo/assets/mosshop.toml" -o "$assets" --force >/dev/null
fi
[ -e "$out" ] && mv "$out" "$out.old-$$"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
model="$out.template"
[ -e "$model" ] && mv "$model" "$model.old-$$"
cp -R "$repo/templates/mosshop" "$model"
python3 -I - "$model/model.toml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
# Moss Hop's two counters, drawn in the background palette.
old = 'digits = 2\nsprite = true\n'
if s.count(old) != 2:
    sys.exit("mosshop-hud: Moss Hop's HUD fields aren't the two this script knows")
open(p, "w").write(s.replace(old, 'digits = 2\n'))
PY
(cd "$repo" && cargo build -q --locked -p gp-web --bin gp-web-prep --target-dir "$target")
"$target/debug/gp-web-prep" "$model" "$assets" "$out" >/dev/null
python3 -I - "$out" <<'PY'
import json, os, struct, sys, zlib
out = sys.argv[1]
def png(path, w, h, rgba):
    raw = b"".join(b"\x00" + bytes(rgba) * w for _ in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
# The hero (frame-1-0.png, standing at y 176) drawn 200 pixels tall from
# 184 above its picture's top: across the HUD's rows. Made-up colours.
rules = "gp-art 1\nscale 2\nlimit off\npic frame-1-0.png art/tall.png 16 200 0 -184\n"
index = json.load(open(f"{out}/themes/index.json"))
for id_, title, extra in [("tall", "Tall", ""), ("tallover", "Tall Over", "#+ hud_over\n")]:
    d = f"{out}/themes/{id_}"
    os.makedirs(f"{d}/art", exist_ok=True)
    open(f"{d}/look.txt", "w").write(f"gp-look 1\nid {id_}\ntitle {title}\nart art.txt\n")
    open(f"{d}/art.txt", "w").write(rules + extra)
    png(f"{d}/art/tall.png", 16, 200, [200, 40, 160, 255])
    index["themes"].append({"id": id_, "title": title})
# `instart`: the same tall hero, and in the kit's region `start` (where the
# hero starts) a picture 16 pixels tall in another made-up colour.
d = f"{out}/themes/instart"
os.makedirs(f"{d}/art", exist_ok=True)
open(f"{d}/look.txt", "w").write("gp-look 1\nid instart\ntitle In Start\nart art.txt\n")
open(f"{d}/art.txt", "w").write(rules + "#+ in start pic frame-1-0.png art/short.png 16 16 0 0\n")
png(f"{d}/art/tall.png", 16, 200, [200, 40, 160, 255])
png(f"{d}/art/short.png", 32, 32, [40, 200, 220, 255])
index["themes"].append({"id": "instart", "title": "In Start"})
json.dump(index, open(f"{out}/themes/index.json", "w"))
PY
echo "$out"
