#!/bin/bash
# A scratch copy of a folder registry with a made-up second game in it, for
# the page check's sections about a game named in the address
# (check-page.mjs, game-*; docs/web.md "One page, any game"):
#   scripts/web/test-registry.sh FROM OUT
#
# FROM is a folder registry with no root (the release check's snapshot of
# lex00/gp-registry, say). OUT gets a copy of it (APFS clones, so it costs
# no disk) with the namespace `test` granted and two packages published in
# it, unsigned:
#   test/made-up       a game: its play/page.json (title "Made Up Game",
#                      the NES player) and a lift folder whose templates
#                      are named made-up-a and made-up-b. Their model and
#                      asset maps are lex00/donkey-kong's latest, copied
#                      from FROM's own blobs (a game package's text, never
#                      a ROM's bytes), so the lift reads them and refuses a
#                      made-up ROM naming "Made Up Game". It has no module
#                      and plays nothing: no ROM is its.
#   test/made-up-look  a theme package offered for test/made-up only.
#   test/own-hop       a game with its own pictures and no ROM: a copy of
#                      the Moss Hop template (the project's own test game)
#                      with a made-up large-art theme of its own (hopglow,
#                      in themes/, as gp game share puts a person's) as a
#                      package, built by build-page.sh's road for such
#                      a game (its play/ holds the bundle, its pictures, its
#                      profiles and themes, and page.json says rom.kind
#                      "none") and published by publish-game.sh, so gp
#                      registry publish compares it with every ROM here
#                      first (check-page.mjs, own-game*). Left out with a
#                      line when Moss Hop's pictures aren't lifted yet
#                      ($GP_HOME/work/assets/mosshop: just check, or
#                      scripts/web/mosshop-hud.sh, makes them).
#   test/tapper-unplaced  only when FROM lists lex00/tapper: a copy of its
#                      files (the lift's folder, page.json and the module,
#                      the model, corpus and its score record in evidence/,
#                      from FROM's own blobs: a package's text and code,
#                      never a ROM's bytes) with every place its play/ asset
#                      maps record for the rbtapper set taken out, so the
#                      lift refuses rbtapper.zip as a set whose tables have
#                      no place (check-page.mjs, arcade-unplaced). gp
#                      registry publish takes an arcade game only with a
#                      score record of every clip exact (docs/web.md,
#                      "Publishing an arcade game"), which the copy carries;
#                      a lex00/tapper published without one is left out
#                      with a line.
# OUT is made again each run. GP names the gp to run (default
# target/debug/gp, which has `gp registry grant`). Exit codes: 0 made, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
if [ $# -ne 2 ]; then
  echo "usage: scripts/web/test-registry.sh FROM OUT" >&2
  exit 2
fi
real="$(cd "$(dirname "$0")/../.." && pwd)"
from="$(cd "$1" && pwd)"
out="$2"
gp="${GP:-$real/target/debug/gp}"
home="${GP_HOME:-$HOME/.gameprinter}"
[ -e "$out" ] && mv "$out" "$out.old-$$"
mkdir -p "$(dirname "$out")"
cp -c -R "$from" "$out" 2>/dev/null || cp -R "$from" "$out"
out="$(cd "$out" && pwd)"
stage="$out.stage"
[ -e "$stage" ] && mv "$stage" "$stage.old-$$"
mkdir -p "$stage/game" "$stage/look"
python3 -I - "$out" "$stage" <<'PY'
import json, os, re, sys, tomllib
reg, stage = sys.argv[1:]
cat = tomllib.load(open(f"{reg}/catalog.toml", "rb"))
dk = next(p for p in cat["package"] if p["name"] == "lex00/donkey-kong")
files = {}
for line in open(f"{reg}/packages/{dk['id']}"):
    m = re.match(r"^file ([0-9a-f]{64}) \d+ (.+)$", line.rstrip("\n"))
    if m:
        files[m.group(2)] = m.group(1)
blob = lambda path: open(f"{reg}/blobs/{files[path]}", "rb").read()
g = f"{stage}/game"
def put(rel, data):
    p = os.path.join(g, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "wb").write(data if isinstance(data, bytes) else data.encode())
names = {"donkey-kong-oe": "made-up-a", "donkey-kong-ju": "made-up-b"}
lift = []
for old, new in names.items():
    for f in ["model.toml", "profiles.toml", "template.toml", "asset-map.toml"]:
        put(f"play/lift/{new}/{f}", blob(f"play/lift/{old}/{f}"))
        lift.append(f"{new}/{f}")
put("play/lift/made-up-b/load.json", blob("play/lift/donkey-kong-ju/load.json").decode().replace("donkey-kong-oe", "made-up-a"))
lift.append("made-up-b/load.json")
fake = lambda c: c * 64
put("play/lift/sources-nes.toml", f'''# Made up for the page check: no ROM has these hashes.
[[source]]
id = "made-up-a"
title = "Made Up Game"
platform = "nes"
revision = "A"
sha256 = "{fake("a")}"

[[source]]
id = "made-up-b"
title = "Made Up Game"
platform = "nes"
revision = "B"
sha256 = "{fake("b")}"
''')
put("play/lift/game.json", json.dumps({"game": "made-up", "title": "Made Up Game", "templates": ["made-up-a", "made-up-b"]}) + "\n")
lift += ["sources-nes.toml", "game.json"]
put("play/lift/index.json", json.dumps({"game": "made-up", "files": lift}) + "\n")
put("play/page.json", json.dumps({
    "format": 1,
    "title": "Made Up Game",
    "player": "nes",
    "rom": {
        "kind": "nes",
        "game": "made-up",
        "name": "Made Up Game",
        "file": "an .nes file of Made Up Game (A) or (B)",
        "known": ["Made Up Game (A)", "Made Up Game (B)"],
        "known_sha256": {"Made Up Game (A)": fake("a"), "Made Up Game (B)": fake("b")},
    },
    "howto": ["Made-up controls: the page check reads this line.", "<b>Not HTML</b>: shown as text."],
    "trademark": "Made Up Game is made up for the page check.",
}, indent=1) + "\n")
put("package.toml", '[package]\nname = "test/made-up"\nversion = "0.1.0"\nkind = "game"\ntitle = "Made Up Game"\n')
# lex00/tapper with rbtapper's table places taken out of its maps: the
# Root Beer set then reads every table at the regular set's place.
tapper = next((p for p in cat["package"] if p["name"] == "lex00/tapper"), None)
if tapper:
    u = f"{stage}/unplaced"
    for line in open(f"{reg}/packages/{tapper['id']}"):
        m = re.match(r"^file ([0-9a-f]{64}) \d+ (.+)$", line.rstrip("\n"))
        if not m:
            continue
        data = open(f"{reg}/blobs/{m.group(1)}", "rb").read()
        if m.group(2).startswith("play/") and m.group(2).endswith("/asset-map.toml"):
            data = re.sub(rb"(?m)^rbtapper = .*\n", b"", data)
        if m.group(2) == "package.toml":
            data = re.sub(rb'(?m)^name = "lex00/tapper"$', b'name = "test/tapper-unplaced"', data)
            data = re.sub(rb'(?m)^version = ".*"$', b'version = "0.1.0"', data, count=1)
        dst = os.path.join(u, m.group(2))
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        open(dst, "wb").write(data)
    if not os.path.exists(f"{u}/package.toml"):
        open(f"{u}/package.toml", "w").write('[package]\nname = "test/tapper-unplaced"\nversion = "0.1.0"\nkind = "game"\ntitle = "Tapper"\n')
l = f"{stage}/look"
os.makedirs(f"{l}/theme", exist_ok=True)
open(f"{l}/package.toml", "w").write('[package]\nname = "test/made-up-look"\nversion = "0.1.0"\nkind = "content"\ntitle = "Made Up Look"\n')
open(f"{l}/theme/theme.toml", "w").write('# Made up for the page check.\ntitle = "Made Up Look"\n')
PY
"$gp" registry grant "$out" test >/dev/null
"$gp" registry publish "$out" "$stage/game" --name test/made-up --version 0.1.0 --kind game --title "Made Up Game" >/dev/null
"$gp" registry publish "$out" "$stage/look" --name test/made-up-look --version 0.1.0 --kind content --title "Made Up Look" --for test/made-up >/dev/null
added=(test/made-up@0.1.0 test/made-up-look@0.1.0)
# The game with its own pictures, made from Moss Hop's template and the
# pictures lifted from the ROM its source builds.
if [ -f "$home/work/assets/mosshop/assets.toml" ]; then
  own="$stage/own-hop"
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
  # A person's own large-art theme of it, as gp game share puts one in
  # (themes/ID/ beside the model, where gp-web-prep offers a finished
  # look): one made-up picture for the hero, in made-up colours.
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
  if ! GP="$gp" GP_PAGE_SOURCE=local "$real/scripts/web/build-page.sh" "$own" "$stage/own-page" > "$stage/own-page.log" 2>&1; then
    tail -5 "$stage/own-page.log" >&2
    echo "test-registry.sh: build-page.sh couldn't build test/own-hop (its log: $stage/own-page.log)" >&2
    exit 2
  fi
  GP="$gp" "$real/scripts/web/publish-game.sh" "$own" "$stage/own-page.play" "$out" >/dev/null
  added+=(test/own-hop@0.1.0)
else
  echo "test-registry.sh: no Moss Hop pictures in $home/work/assets/mosshop, so test/own-hop (a game with its own pictures) is left out" >&2
fi
if [ -d "$stage/unplaced" ]; then
  if "$gp" registry publish "$out" "$stage/unplaced" --name test/tapper-unplaced --version 0.1.0 --kind game --title "Tapper" >/dev/null 2>"$stage/unplaced.err"; then
    added+=(test/tapper-unplaced@0.1.0)
  else
    echo "test-registry.sh: test/tapper-unplaced is left out: $(head -c 600 "$stage/unplaced.err")" >&2
  fi
fi
# The list files keep what they list and add these.
"$gp" registry reindex "$out" --keep "${added[@]}" >/dev/null
mv "$stage" "$out.stage-used-$$"
echo "$out"
