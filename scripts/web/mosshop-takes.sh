#!/bin/bash
# Moss Hop's preview with a made-up sound role and a made-up large-art
# theme that gives it three takes, for the page check's theme-sound-takes
# section (check-page.mjs; docs/web.md, "Checks"):
#   scripts/web/mosshop-takes.sh OUT
#
# OUT gets what gp-web-prep writes for a copy of templates/mosshop with a
# counter `beat` that counts every frame from 0 to 39 and wraps, and one
# sound role, `tick`, an effect cued while `beat` is 0, so it starts every
# 40 frames whatever is pressed (Moss Hop has no sound of its own). The
# theme `takes` in OUT/themes/ gives `tick` three takes: its own file
# sound/roles/tick.wav and sound/takes/tick.2.wav and tick.3.wav, short
# square tones made here at three pitches. The page check serves OUT beside
# the page's own player files, so the frame is played by the page's
# gp_web.wasm and preview.js.
#
# Moss Hop's art comes from its own ROM, which `just mosshop` assembles
# into target/mosshop/mosshop.nes: lifted into $GP_HOME/work/assets/mosshop
# (as `just check` does) when that folder has no lift yet. GP names the gp
# to run (default target/debug/gp). Nothing of another game is read.
# Exit codes: 0 made, 2 couldn't, 3 no Moss Hop ROM to lift its art from.
set -euo pipefail
trap 'exit 2' ERR
if [ $# -ne 1 ]; then
  echo "usage: scripts/web/mosshop-takes.sh OUT" >&2
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
  [ -f "$rom" ] || { echo "mosshop-takes: no Moss Hop ROM at $rom; run \`just mosshop\`" >&2; exit 3; }
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
old = "[counters.lives]\n"
if s.count(old) != 1:
    sys.exit("mosshop-takes: Moss Hop's counters aren't the ones this script knows")
beat = "[counters.beat]\nmax = 39\nwrap = true\n\n[[counters.beat.change]]\nevery = 1\n\n"
role = '\n[[present.sound.roles]]\nname = "tick"\nkind = "sfx"\ncounter = "beat"\nequals = 0\ndescription = "A made-up tick every 40 frames, for the page check"\n'
# Every fact carries its evidence: these are this script's own.
why = "".join(
    f'\n[evidence."{k}"]\nsource = "stated"\nconfidence = 100\n\n[[evidence."{k}".refs]]\nat = "scripts/web/mosshop-takes.sh:3"\nquote = "a made-up sound role"\n'
    for k in ["counters.beat", "present.sound"]
)
open(p, "w").write(s.replace(old, beat + old) + role + why)
PY
(cd "$repo" && cargo build -q --locked -p gp-web --bin gp-web-prep --target-dir "$target")
"$target/debug/gp-web-prep" "$model" "$assets" "$out" >/dev/null
python3 -I - "$out" <<'PY'
import json, os, struct, sys
out = sys.argv[1]
def wav(path, period):
    # A square tone of 0.1 s, 16-bit mono at 44,100 Hz: made up here.
    n = 4410
    data = b"".join(struct.pack("<h", 3000 if (i // period) % 2 == 0 else -3000) for i in range(n))
    head = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, 44100, 88200, 2, 16) + b"data" + struct.pack("<I", len(data))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "wb").write(head + data)
d = f"{out}/themes/takes"
files = ["sound/roles/tick.wav", "sound/takes/tick.2.wav", "sound/takes/tick.3.wav"]
for f, period in zip(files, [50, 40, 30]):
    wav(f"{d}/{f}", period)
open(f"{d}/look.txt", "w").write("gp-look 1\nid takes\ntitle Takes\nart art.txt\n" + "".join(f"file {f}\n" for f in files))
open(f"{d}/art.txt", "w").write("gp-art 1\nscale 1\n")
index = json.load(open(f"{out}/themes/index.json"))
index["themes"].append({"id": "takes", "title": "Takes"})
json.dump(index, open(f"{out}/themes/index.json", "w"))
PY
echo "$out"
