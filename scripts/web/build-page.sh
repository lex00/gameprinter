#!/bin/bash
# Builds the page from the games repo: a static folder with no game data.
#   scripts/web/build-page.sh GAME_DIR OUT
#
# GAME_DIR is the game's folder (games/donkey-kong). The repo holds no game,
# so the build works in a copy of it with the game's hook crate put in place
# of crates/gp-ext-game, and its template folders where the build reads them.
#
# Exit codes: 0 built, 2 couldn't build.

set -euo pipefail
trap 'exit 2' ERR

real="$(cd "$(dirname "$0")/../.." && pwd)"
game="$(cd "$1" && pwd)"
out="$2"
home="${GP_HOME:-$HOME/.gameprinter}"
gp="${GP:-$real/target/debug/gp}"
repo="$real/target/page-src"
export CARGO_TARGET_DIR="$real/target/page-target"
export GP_HOOK_MODELS="donkey_kong_oe,rivet_rodeo"

rm -rf "$repo"
mkdir -p "$repo/templates" "$repo/sources"
rsync -a --exclude target --exclude .git "$real/" "$repo/"
rm -rf "$repo/crates/gp-ext-game"
cp -R "$game/ext" "$repo/crates/gp-ext-game"
rm -f "$repo/crates/gp-ext-game/Cargo.lock"
python3 -I - "$repo/crates/gp-ext-game/Cargo.toml" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
s = s.replace('name = "gp-ext-dk"', 'name = "gp-ext-game"')
s = s.replace('../../../gameprinter/crates/gp-sim', '../gp-sim').replace('../../../gameprinter/crates/gp-cycles', '../gp-cycles')
s = s.replace('\n[workspace]\n', '\n')
open(p, 'w').write(s)
PY
mkdir -p "$repo/templates/donkey-kong-oe" "$repo/templates/rivet-rodeo"
rsync -a --exclude open --exclude ext "$game/" "$repo/templates/donkey-kong-oe/"
rsync -a --exclude generators "$game/open/" "$repo/templates/rivet-rodeo/"
cp "$game/roms.toml" "$repo/sources/nes.toml"
stage="$repo/templates/rivet-rodeo"
rom_game="donkey-kong"
assets="$home/work/page-assets/rivet-rodeo"
reuse="$repo/scripts/web/reuse.py"
reusing() { false; }
keeping() { false; }
gp_bin="$gp"
now() { perl -MTime::HiRes=time -e 'printf "%.1f", time'; }
t0=$(now)
lap() { local t; t=$(now); echo "time  $1: $(perl -e "printf '%.1f', $t - $t0") s"; t0=$t; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"
rm -rf "$assets"
"$gp" template build "$stage" -o "$assets" >/dev/null
lap "gp template build rivet-rodeo"
GP_WEB_PREP_FLAGS="--themes" "$repo/scripts/web/build-preview.sh" "$out/game" "$stage" "$assets"
lap "build-preview.sh"
"$repo/scripts/web/build-lift.sh" "$out/game" "$rom_game" "$stage" "$assets"
lap "build-lift.sh"

# The game's name, kept in one place (scripts/rivet-rodeo/name.txt).
title="$(python3 -I -c 'import sys; print(open(sys.argv[1]).read().strip())' "$game/open/generators/name.txt")"
python3 -I - "$out" "$title" "$out/game/lift/game.json" "$repo/sources/nes.toml" <<'EOF'
import json, sys, tomllib
out, title, game_file, registry = sys.argv[1:]
game = json.load(open(game_file))
sources = tomllib.load(open(registry, "rb"))["source"]
names, known = [], {}
for tid in game["templates"]:
    src = next((s for s in sources if s["id"] == tid), None)
    if src:
        n = f'{src["title"]} ({src.get("revision", "")})'.replace(" ()", "")
        names.append(n)
        known[n] = src["sha256"]
page = {
    "title": title,
    "rom": {
        "game": game["game"],
        "title": game["title"],
        "name": game["title"],
        "known": names,
        "file": "an .nes file of any dump: " + ", ".join(names) + ", or another revision",
        # The dumps the templates are for, which the page's check plays.
        "known_sha256": known,
    },
}
open(f"{out}/page.json", "w").write(json.dumps(page, indent=1) + "\n")
EOF
sed "s|<title>Rivet Rodeo</title>|<title>$title</title>|; s|<h1 id=\"title\">Rivet Rodeo</h1>|<h1 id=\"title\">$title</h1>|" \
  "$repo/scripts/web/public/index.html" > "$out/index.html"
sed "s|Rivet Rodeo is an original|$title is an original|" "$out/index.html" > "$out/index.html.tmp" && mv "$out/index.html.tmp" "$out/index.html"
cp "$repo/scripts/web/public/page.js" "$repo/scripts/web/public/boot.js" "$repo/scripts/web/public/page.css" "$repo/scripts/web/public/_headers" "$out/"
# The preview's page keeps its style inline; the page's Content-Security-Policy
# allows none, so it moves to preview.css.
python3 -I - "$out/game" <<'EOF2'
import re, sys
d = sys.argv[1]
s = open(f"{d}/index.html").read()
m = re.search(r"<style>\n(.*?)</style>\n", s, re.S)
if m:
    open(f"{d}/preview.css", "w").write(m.group(1))
    open(f"{d}/index.html", "w").write(s.replace(m.group(0), '<link rel="stylesheet" href="preview.css">\n'))
EOF2

lap "the page's own files"
echo "page $(du -sk "$out" | cut -f1) KB in $out"
