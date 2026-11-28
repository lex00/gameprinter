#!/bin/bash
# A scratch copy of a folder registry with a made-up second game in it, for
# the page check's sections about a game named in the address
# (check-page.mjs, game-*; docs/web.md "One page, any game"), and two
# made-up themes for the page's own game:
#   scripts/web/test-registry.sh FROM OUT GAME_DIR [GAMES_DIR]
#
# GAME_DIR is the page's own game's folder (its package.toml names the
# game, its page-check.toml [test_themes] the pictures of the made-up
# themes); GAMES_DIR is the games checkout (default GAME_DIR's parent),
# where the arcade game for test/arcade-unplaced is found.
#
# FROM is a folder registry with no root (the release check's snapshot of
# lex00/gp-registry, say). OUT gets a copy of it (APFS clones, so it costs
# no disk) with the namespaces `test` and `acme` granted and these
# packages published in it, unsigned:
#   test/made-up       a game: its play/page.json (title "Made Up Game",
#                      the NES player) and a lift folder whose templates
#                      are named made-up-a, made-up-b and so on, one for
#                      each of the page's own game's lift templates. Their
#                      model and asset maps are that game's latest, copied
#                      from FROM's own blobs (a game package's text, never
#                      a ROM's bytes), so the lift reads them and refuses a
#                      made-up ROM naming "Made Up Game". It has no module
#                      and plays nothing: no ROM is its.
#   test/made-up-look  a theme package offered for test/made-up only.
#   acme/harbor-lights, test/paper-moon  two made-up large-art themes
#                      offered for the page's own game, drawn from plain
#                      shapes by scripts/web/test-themes.py for the game's
#                      pictures: with the page's own theme the game is
#                      offered three, from namespaces before and after its
#                      own (check-page.mjs, the overlapping theme picks and
#                      the theme line).
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
#   test/arcade-unplaced  only when FROM lists an arcade game of GAMES_DIR
#                      whose page-check.toml names [arcade] unplaced (a
#                      set): a copy of its files (the lift's folder,
#                      page.json and the module, the model, corpus and its
#                      score record in evidence/, from FROM's own blobs: a
#                      package's text and code, never a ROM's bytes) with
#                      every place its play/ asset maps record for that set
#                      taken out, so the lift refuses the set's zip as a set
#                      whose tables have no place (check-page.mjs,
#                      arcade-unplaced). gp registry publish takes an arcade
#                      game only with a score record of every clip exact
#                      (docs/web.md, "Publishing an arcade game"), which the
#                      copy carries; a game published without one is left
#                      out with a line.
# OUT is made again each run. GP names the gp to run (default
# target/debug/gp, which has `gp registry grant`). Exit codes: 0 made, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
if [ $# -lt 3 ] || [ $# -gt 4 ]; then
  echo "usage: scripts/web/test-registry.sh FROM OUT GAME_DIR [GAMES_DIR]" >&2
  exit 2
fi
real="$(cd "$(dirname "$0")/../.." && pwd)"
from="$(cd "$1" && pwd)"
out="$2"
game_dir="$(cd "$3" && pwd)"
games_dir="$(cd "${4:-$game_dir/..}" && pwd)"
game_name="$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["name"])' "$game_dir/package.toml")"
gp="${GP:-$real/target/debug/gp}"
home="${GP_HOME:-$HOME/.gameprinter}"
[ -e "$out" ] && mv "$out" "$out.old-$$"
mkdir -p "$(dirname "$out")"
cp -c -R "$from" "$out" 2>/dev/null || cp -R "$from" "$out"
out="$(cd "$out" && pwd)"
stage="$out.stage"
[ -e "$stage" ] && mv "$stage" "$stage.old-$$"
mkdir -p "$stage/game" "$stage/look"
python3 -I - "$out" "$stage" "$game_name" "$games_dir" <<'PY'
import json, os, re, sys, tomllib
reg, stage, game_name, games_dir = sys.argv[1:]
cat = tomllib.load(open(f"{reg}/catalog.toml", "rb"))
own = next((p for p in cat["package"] if p["name"] == game_name), None)
if own is None:
    sys.exit(f"test-registry.sh: {reg} has no {game_name}, the page's own game, to make the made-up game from")
def files_of(pkg):
    files = {}
    for line in open(f"{reg}/packages/{pkg['id']}"):
        m = re.match(r"^file ([0-9a-f]{64}) \d+ (.+)$", line.rstrip("\n"))
        if m:
            files[m.group(2)] = m.group(1)
    return files
files = files_of(own)
blob = lambda path: open(f"{reg}/blobs/{files[path]}", "rb").read()
g = f"{stage}/game"
def put(rel, data):
    p = os.path.join(g, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "wb").write(data if isinstance(data, bytes) else data.encode())
# The page's own game's lift templates, in its order, each made up as
# made-up-a, made-up-b and so on.
templates = json.loads(blob("play/lift/game.json"))["templates"]
letters = "abcdef"
if not 0 < len(templates) <= len(letters):
    sys.exit(f"test-registry.sh: {game_name}'s lift has {len(templates)} templates; the made-up game takes 1 to {len(letters)}")
names = {old: f"made-up-{letters[i]}" for i, old in enumerate(templates)}
def renamed(text):
    for old, new in names.items():
        text = re.sub(rf'"{re.escape(old)}"', f'"{new}"', text)
    return text
lift = []
for old, new in names.items():
    for f in ["model.toml", "profiles.toml", "template.toml", "asset-map.toml"]:
        put(f"play/lift/{new}/{f}", blob(f"play/lift/{old}/{f}"))
        lift.append(f"{new}/{f}")
    if f"play/lift/{old}/load.json" in files:
        put(f"play/lift/{new}/load.json", renamed(blob(f"play/lift/{old}/load.json").decode()))
        lift.append(f"{new}/load.json")
fake = lambda c: c * 64
put("play/lift/sources-nes.toml", "# Made up for the page check: no ROM has these hashes.\n" + "".join(
    f'\n[[source]]\nid = "{new}"\ntitle = "Made Up Game"\nplatform = "nes"\nrevision = "{new[-1].upper()}"\nsha256 = "{fake(new[-1])}"\n'
    for new in names.values()
))
made = list(names.values())
put("play/lift/game.json", json.dumps({"game": "made-up", "title": "Made Up Game", "templates": made}) + "\n")
lift += ["sources-nes.toml", "game.json"]
put("play/lift/index.json", json.dumps({"game": "made-up", "files": lift}) + "\n")
known = [f"Made Up Game ({n[-1].upper()})" for n in made]
said = " or ".join([", ".join(known[:-1]), known[-1]]) if len(known) > 1 else known[0]
put("play/page.json", json.dumps({
    "format": 1,
    "title": "Made Up Game",
    "player": "nes",
    "rom": {
        "kind": "nes",
        "game": "made-up",
        "name": "Made Up Game",
        "file": f"an .nes file of {said}",
        "known": known,
        "known_sha256": {k: fake(n[-1]) for k, n in zip(known, made)},
    },
    "howto": ["Made-up controls: the page check reads this line.", "<b>Not HTML</b>: shown as text."],
    "trademark": "Made Up Game is made up for the page check.",
}, indent=1) + "\n")
put("package.toml", '[package]\nname = "test/made-up"\nversion = "0.1.0"\nkind = "game"\ntitle = "Made Up Game"\n')
# An arcade game of the games checkout whose page-check.toml names [arcade]
# unplaced, with that set's table places taken out of its maps: the set
# then reads every table at the scored set's place.
def arcade_game():
    for d in sorted(os.listdir(games_dir)):
        f = os.path.join(games_dir, d)
        try:
            pkg = tomllib.load(open(os.path.join(f, "package.toml"), "rb"))
            check = tomllib.load(open(os.path.join(f, "page-check.toml"), "rb"))
        except OSError:
            continue
        unplaced = check.get("arcade", {}).get("unplaced")
        name = pkg.get("package", {}).get("name")
        listed = next((p for p in cat["package"] if p["name"] == name), None)
        if unplaced and listed:
            return listed, name, pkg["package"].get("title", name), unplaced
    return None
found = arcade_game()
open(f"{stage}/unplaced-title", "w").write(found[2] if found else "")
if found:
    arcade, arcade_name, arcade_title, unplaced = found
    u = f"{stage}/unplaced"
    for path, h in files_of(arcade).items():
        data = open(f"{reg}/blobs/{h}", "rb").read()
        if path.startswith("play/") and path.endswith("/asset-map.toml"):
            data = re.sub(rb"(?m)^" + re.escape(unplaced.encode()) + rb" = .*\n", b"", data)
        if path == "package.toml":
            data = re.sub(rb'(?m)^name = "' + re.escape(arcade_name.encode()) + rb'"$', b'name = "test/arcade-unplaced"', data)
            data = re.sub(rb'(?m)^version = ".*"$', b'version = "0.1.0"', data, count=1)
        dst = os.path.join(u, path)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        open(dst, "wb").write(data)
    if not os.path.exists(f"{u}/package.toml"):
        open(f"{u}/package.toml", "w").write(f'[package]\nname = "test/arcade-unplaced"\nversion = "0.1.0"\nkind = "game"\ntitle = {json.dumps(arcade_title)}\n')
l = f"{stage}/look"
os.makedirs(f"{l}/theme", exist_ok=True)
open(f"{l}/package.toml", "w").write('[package]\nname = "test/made-up-look"\nversion = "0.1.0"\nkind = "content"\ntitle = "Made Up Look"\n')
open(f"{l}/theme/theme.toml", "w").write('# Made up for the page check.\ntitle = "Made Up Look"\n')
PY
"$gp" registry grant "$out" test >/dev/null
"$gp" registry publish "$out" "$stage/game" --name test/made-up --version 0.1.0 --kind game --title "Made Up Game" >/dev/null
"$gp" registry publish "$out" "$stage/look" --name test/made-up-look --version 0.1.0 --kind content --title "Made Up Look" --for test/made-up >/dev/null
added=(test/made-up@0.1.0 test/made-up-look@0.1.0)
# Two made-up large-art themes offered for the page's own game, one in a
# namespace before most and one after, so the page offers the game more
# than its own theme whatever the real registry holds.
python3 -I "$real/scripts/web/test-themes.py" "$stage/themes" "$game_dir" >/dev/null
"$gp" registry grant "$out" acme >/dev/null
for t in acme/harbor-lights:"Harbor Lights" test/paper-moon:"Paper Moon"; do
  name="${t%%:*}"
  "$gp" registry publish "$out" "$stage/themes/${name#*/}" --name "$name" --version 0.1.0 --kind content --title "${t#*:}" --for "$game_name" >/dev/null
  added+=("$name@0.1.0")
done
# The game with its own pictures, made from Moss Hop's template and the
# pictures lifted from the ROM its source builds.
if [ -f "$home/work/assets/mosshop/assets.toml" ]; then
  # The Moss Hop copy with its own package and its own large-art theme
  # (scripts/web/own-hop.sh, which just check-web's page check uses too).
  own="$stage/own-hop"
  bash "$real/scripts/web/own-hop.sh" "$own"
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
  if "$gp" registry publish "$out" "$stage/unplaced" --name test/arcade-unplaced --version 0.1.0 --kind game --title "$(cat "$stage/unplaced-title")" >/dev/null 2>"$stage/unplaced.err"; then
    added+=(test/arcade-unplaced@0.1.0)
  else
    echo "test-registry.sh: test/arcade-unplaced is left out: $(head -c 600 "$stage/unplaced.err")" >&2
  fi
fi
# The list files keep what they list and add these.
"$gp" registry reindex "$out" --keep "${added[@]}" >/dev/null
mv "$stage" "$out.stage-used-$$"
echo "$out"
