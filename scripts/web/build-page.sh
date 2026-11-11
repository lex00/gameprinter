#!/bin/bash
# Builds the page from the games repo: a static folder with no game data.
#   scripts/web/build-page.sh GAME_DIR OUT
#
# OUT gets the page and nothing of the game. OUT.play gets the game's built
# files, which are published as `play/` in the game's package
# (scripts/web/publish-game.sh) and which the page installs from its source.
# GP_PAGE_SOURCE names the source in scripts/web/sources.toml (default registry,
# lex00/gp-registry).
#
# Last it runs `gp template served` on the game: the files of both folders
# against each ROM the game names (result also in OUT.served.txt).
#
# GAME_DIR is the game's folder (games/donkey-kong). The repo holds no game,
# so the build works in a copy of it with the game's template folders where
# the build reads them. The page itself has no game's hooks: they go in the
# game's own module (`gp game build`), which is one of the play files.
#
# Exit codes: 0 built and clean, 1 the served check found game data, 2 couldn't
# build, or a ROM wasn't in $GP_ROM_DIR to compare with.

set -euo pipefail
trap 'exit 2' ERR

real="$(cd "$(dirname "$0")/../.." && pwd)"
game="$(cd "$1" && pwd)"
out="$2"
home="${GP_HOME:-$HOME/.gameprinter}"
gp="${GP:-$real/target/debug/gp}"
repo="$real/target/page-src"
export CARGO_TARGET_DIR="$real/target/page-target"

rm -rf "$repo"
mkdir -p "$repo/templates" "$repo/sources"
rsync -a --exclude target --exclude .git "$real/" "$repo/"
mkdir -p "$repo/templates/donkey-kong-oe"
# The ROM's template: a ROM is offered its own art and the page's theme
# (GP_PAGE_CONTENT) only.
rsync -a --exclude ext "$game/" "$repo/templates/donkey-kong-oe/"
cp "$game/roms.toml" "$repo/sources/nes.toml"
rom_game="donkey-kong"
now() { perl -MTime::HiRes=time -e 'printf "%.1f", time'; }
t0=$(now)
lap() { local t; t=$(now); echo "time  $1: $(perl -e "printf '%.1f', $t - $t0") s"; t0=$t; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"
# The player only: every game file the page plays comes from the visitor's
# ROM, lifted in their browser.
"$repo/scripts/web/build-preview.sh" "$out/game"
lap "build-preview.sh"
"$repo/scripts/web/build-lift.sh" "$out/game" "$rom_game"
lap "build-lift.sh"

# The page's name: GP_PAGE_TITLE, else the game package's title. The page
# plays a visitor's ROM, so it is named for the game, and GP_PAGE_CONTENT
# names a theme package installed with the game (lex00/turnips, say).
title="${GP_PAGE_TITLE:-$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["title"])' "$game/package.toml")}"
python3 -I - "$out" "$title" "$out/game/lift/game.json" "$repo/sources/nes.toml" "$repo/scripts/web/sources.toml" "${GP_PAGE_SOURCE:-registry}" "$game/package.toml" "${GP_PAGE_CONTENT:-}" <<'EOF'
import json, sys, tomllib
out, title, game_file, registry, sources_file, source_name, package, content = sys.argv[1:]
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
    "game": tomllib.load(open(package, "rb"))["package"]["name"],
    "source": tomllib.load(open(sources_file, "rb"))[source_name],
    "content": content or None,
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
sed "s|<title>Game Printer</title>|<title>$title</title>|; s|<h1 id=\"title\">Game Printer</h1>|<h1 id=\"title\">$title</h1>|" \
  "$repo/scripts/web/public/index.html" > "$out/index.html"
cp "$repo/scripts/web/public/page.js" "$repo/scripts/web/public/boot.js" "$repo/scripts/web/public/page.css" "$repo/scripts/web/public/_headers" "$out/"
printf '/p /index.html 200\n' > "$out/_redirects"
cp "$repo/scripts/web/public/install.js" "$out/game/"
(cd "$repo" && cargo rustc -q --locked -p gp-web-registry --lib --release --target wasm32-unknown-unknown \
  --target-dir "$CARGO_TARGET_DIR/web-registry" --crate-type cdylib \
  --config 'profile.release.lto=true' --config 'profile.release.opt-level="s"' \
  --config 'profile.release.codegen-units=1' --config 'profile.release.strip=true')
registry_wasm="$CARGO_TARGET_DIR/web-registry/wasm32-unknown-unknown/release/gp_web_registry.wasm"
if [ -n "${WEB_TOOLS:-}" ]; then
  "$WEB_TOOLS/wasm-opt" -Oz --enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
    --strip-debug --strip-producers "$registry_wasm" -o "$out/game/gp-registry.wasm"
else
  cp "$registry_wasm" "$out/game/gp-registry.wasm"
fi
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

# The game's files leave the page for the package: only the player stays.
play="$out.play"
rm -rf "$play"
mkdir -p "$play/lift"
for f in bundle.json preview.json profiles.toml profiles assets themes; do
  [ -e "$out/game/$f" ] && mv "$out/game/$f" "$play/"
done
for f in "$out/game/lift/"*; do
  [ "$(basename "$f")" = gp-web-lift.wasm ] || mv "$f" "$play/lift/"
done

"$gp" game kit -o "$real/target/module-kit" >/dev/null
GP_HOME="$home" "$gp" game build "$game" --kit "$real/target/module-kit" -o "$play/module.wasm"
lap "the game's module"
echo "page $(du -sk "$out" | cut -f1) KB in $out"

# Holds none of the game's data: every file of both folders against each ROM.
report="$out.served.txt"
status=0
"$gp" template served "$game" --folder "$out" --folder "$play" > "$report" || status=$?
cat "$report"
lap "gp template served"
exit "$status"
