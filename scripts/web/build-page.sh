#!/bin/bash
# Builds the page from the games repo: a static folder with no game data.
#   scripts/web/build-page.sh GAME_DIR OUT
#
# OUT gets the page and nothing of the game. OUT.play gets the game's built
# files, which are published as `play/` in the game's package
# (scripts/web/publish-game.sh) and which the page installs from its source.
# GP_PAGE_SOURCE names the source in scripts/web/sources.toml (default registry,
# lex00/gp-registry). GP_PAGE_SOURCE_URL gives one by its https address
# instead, with no edit to the repo (someone's own registry, say:
# docs/host-your-own.md), and GP_PAGE_SOURCE_ROOT its root key line
# ("ssh-ed25519 ...") for a signed one. GP_PAGE_THEMES_URL names a second
# registry the page also takes themes from (page.json's `themes`; games
# still come from the source), with GP_PAGE_THEMES_ROOT its root key line:
# someone's own registry of themes on their own page.
#
# Last it runs `gp template served` on the game: the files of both folders
# against each ROM the game names (result also in OUT.served.txt).
#
# GAME_DIR is the game's folder (a game folder of the games checkout, or a
# game `gp game new` made): an NES game played from a ROM, or an arcade game played
# from a MAME romset zip, by its template's source.platform; or a game that
# plays from its own pictures (a new game, a copy of a template such as Moss
# Hop, whose template says how = "build" or "authored"), whose built files go
# in OUT.play whole, with no lift, and whose play/page.json says rom.kind
# "none" (docs/web.md, "A game with its own pictures"). Its pictures are the
# ones built for it: the folder's own assets/, else
# $GP_HOME/work/assets/SOURCE (gp template build, or gp assets lift of the
# ROM its source builds). The repo holds no game,
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
# One build at a time: two builds share target/page-src (each empties it
# first), so a second one, from another server's rebuild or a shot run,
# waits for the first. A lock whose holder is gone is taken over.
lock="$real/target/page-build.lock"
mkdir -p "$real/target"
until mkdir "$lock" 2>/dev/null; do
  if [ -f "$lock/pid" ] && ! kill -0 "$(cat "$lock/pid" 2>/dev/null)" 2>/dev/null; then
    rm -rf "$lock"
    continue
  fi
  sleep 1
done
echo $$ > "$lock/pid"
trap 'rm -rf "$lock"' EXIT
export CARGO_TARGET_DIR="$real/target/page-target"

# The game's template id, the game it is a template of, and how its assets
# come: the page lifts a game from the visitor's ROM, so only a template
# whose source is a ROM (`how = "rom"`) of an NES game or an arcade board can
# be built here.
read -r tid rom_game how platform model_source < <(python3 -I - "$game/template.toml" "$game/model.toml" <<'PY'
import sys, tomllib
m = tomllib.load(open(sys.argv[1], "rb"))
t, s = m["template"], m.get("source", {})
try:
    src = tomllib.load(open(sys.argv[2], "rb")).get("model", {}).get("source")
except OSError:
    src = None
print(t["id"], t.get("game") or t["id"], s.get("how", "-"), s.get("platform", "nes"), src or s.get("id") or t["id"])
PY
)
own=0
case "$how" in
  build|authored) own=1 ;;
  rom)
    if [ "$platform" != nes ] && [ "$platform" != arcade ]; then
      echo "build-page.sh: $game isn't an NES or arcade game played from a ROM (its source is $platform): the page lifts only an NES ROM or an arcade romset zip" >&2
      exit 2
    fi ;;
  *)
    echo "build-page.sh: $game's template says how = $how: the page plays a game from a ROM (rom) or from its own pictures (build, authored)" >&2
    exit 2 ;;
esac
# A game with its own pictures: the pictures built for it, found where
# gp template build and gp assets lift put them.
if [ "$own" = 1 ]; then
  if [ -f "$game/assets/assets.toml" ]; then assets="$game/assets"; else assets="$home/work/assets/$model_source"; fi
  if [ ! -f "$assets/assets.toml" ]; then
    echo "build-page.sh: $game's pictures aren't built yet ($assets holds none): gp template build $game for a game made here, or gp assets lift on the ROM its source builds" >&2
    exit 2
  fi
fi
# The page's player and the file it takes, by the platform: an NES game's
# .nes file, an arcade game's romset zip (docs/web.md, "Players").
case "$own/$platform" in
  1/*) player="${GP_PAGE_PLAYER:-own}" kind="${GP_PAGE_ROM_KIND:-none}" ;;
  0/nes) player="${GP_PAGE_PLAYER:-nes}" kind="${GP_PAGE_ROM_KIND:-nes}" ;;
  0/arcade) player="${GP_PAGE_PLAYER:-arcade}" kind="${GP_PAGE_ROM_KIND:-zip}" ;;
esac
sources="$repo/sources/$platform.toml"
rm -rf "$repo"
mkdir -p "$repo/templates" "$repo/sources"
rsync -a --exclude target --exclude .git "$real/" "$repo/"
mkdir -p "$repo/templates/$tid"
# The ROM's template: a ROM is offered its own art and the page's theme
# (GP_PAGE_CONTENT) only.
rsync -a --exclude ext "$game/" "$repo/templates/$tid/"
[ "$own" = 1 ] || cp "$game/roms.toml" "$sources"
now() { perl -MTime::HiRes=time -e 'printf "%.1f", time'; }
t0=$(now)
lap() { local t; t=$(now); echo "time  $1: $(perl -e "printf '%.1f', $t - $t0") s"; t0=$t; }
mkdir -p "$out"
out="$(cd "$out" && pwd)"
# The player only: every game file the page plays comes from the visitor's
# ROM, lifted in their browser. A game with its own pictures is built whole
# (gp-web-prep's files: the bundle, its pictures, profiles and themes), and
# those files leave the page for its package below; it has no lift.
if [ "$own" = 1 ]; then
  "$repo/scripts/web/build-preview.sh" "$out/game" "$repo/templates/$tid" "$assets"
  lap "build-preview.sh"
else
  "$repo/scripts/web/build-preview.sh" "$out/game"
  lap "build-preview.sh"
  "$repo/scripts/web/build-lift.sh" "$out/game" "$rom_game"
  lap "build-lift.sh"
fi
# An arcade game's board sound: gp_arcade_play.wasm (crates/gp-arcade-play,
# the board alone, no game data) beside the player, as gp_board.wasm is:
# every page can play an arcade game by its name, and the game frame fetches
# it only for one, powers it on from the visitor's romset and feeds it the
# requests the game's module makes (docs/web.md, "Arcade games").
"$repo/scripts/web/build-play.sh" --arcade "$out/game"
lap "gp_arcade_play.wasm"
# The sets an arcade game's lift takes, asked of the lift itself (it is the
# one that refuses a set it doesn't draw right yet, gp_web_lift::arcade):
# its refusal of a file that isn't a zip lists them in `expected`. The page's
# romset line names those, and the others apart.
taken="$repo/taken.json"
if [ "$own" = 0 ] && [ "$platform" = arcade ]; then
  node --input-type=module - "$out/game" "$taken" <<'JS'
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
const [dir, dst] = process.argv.slice(2);
const { MemFs, runWasi } = await import(pathToFileURL(join(dir, "wasi.js")).href);
const lift = join(dir, "lift");
const fs = new MemFs();
fs.writeFile("/rom.zip", new TextEncoder().encode("not a zip"));
for (const f of JSON.parse(readFileSync(join(lift, "index.json"), "utf8")).files) fs.writeFile(`/template/${f}`, readFileSync(join(lift, f)));
fs.mkdirp("/out");
const run = await runWasi(new WebAssembly.Module(readFileSync(join(lift, "gp-web-lift.wasm"))), ["gp-web-lift", "/rom.zip", "/template", "/out"], fs);
const r = JSON.parse(new TextDecoder().decode(fs.readFile("/out/result.json")));
if (r.ok !== false || !Array.isArray(r.expected)) throw new Error(`the lift didn't refuse a file that isn't a zip (exit ${run.code})`);
writeFileSync(dst, JSON.stringify(r.expected.map((e) => e.id).filter(Boolean)) + "\n");
JS
else
  echo null > "$taken"
fi

# The page's name: GP_PAGE_TITLE, else the game package's title. The page
# plays a visitor's ROM, so it is named for the game, and GP_PAGE_CONTENT
# names a theme package installed with the game (lex00/turnips, say).
title="${GP_PAGE_TITLE:-$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["title"])' "$game/package.toml")}"
# page.json is the page's: its source, its own game and theme, and that
# game's facts. OUT.play/page.json is the game's (docs/web.md, "What the
# page reads from the game's package"): the same facts, published in the
# package's play/, which a page reads to play the game by its name. The
# how-to-play lines and the About line are the package's [page] `howto` and
# `trademark` when it has them, else the page's own (index.html).
python3 -I - "$out" "$title" "$out/game/lift/game.json" "$sources" "$repo/scripts/web/sources.toml" "${GP_PAGE_SOURCE:-registry}" "$game/package.toml" "${GP_PAGE_CONTENT:-}" "$repo/scripts/web/public/index.html" "$player" "$kind" "$platform" "$taken" "$gp" <<'EOF'
import html, json, os, re, subprocess, sys, tomllib
out, title, game_file, registry, sources_file, source_name, package, content, index, player, kind, platform, taken_file, gp = sys.argv[1:]
taken = json.load(open(taken_file))
pkg = tomllib.load(open(package, "rb"))
said = pkg.get("page", {})
source = tomllib.load(open(sources_file, "rb"))[source_name]
by_url = os.environ.get("GP_PAGE_SOURCE_URL", "")
if by_url:
    if not re.fullmatch(r"https://[^\s?#]+", by_url):
        sys.exit(f"build-page.sh: GP_PAGE_SOURCE_URL must be an https address with no query, not {by_url}")
    source = {"name": by_url.rstrip("/"), "url": by_url.rstrip("/")}
    root = os.environ.get("GP_PAGE_SOURCE_ROOT", "")
    if root:
        if not re.fullmatch(r"ssh-ed25519 [A-Za-z0-9+/]{68}( \S+)?", root):
            sys.exit("build-page.sh: GP_PAGE_SOURCE_ROOT must be an ssh-ed25519 public key line")
        source["root"] = root
# GP_PAGE_LINK_REGISTRY=1 builds a page that also takes themes from the
# registry a link names (r=). Off by default: the page plays only its own
# registries' themes.
link_registry = os.environ.get("GP_PAGE_LINK_REGISTRY", "") == "1"
themes = None
themes_url = os.environ.get("GP_PAGE_THEMES_URL", "")
if themes_url:
    if not re.fullmatch(r"https://[^\s?#]+", themes_url):
        sys.exit(f"build-page.sh: GP_PAGE_THEMES_URL must be an https address with no query, not {themes_url}")
    themes = {"name": themes_url.rstrip("/"), "url": themes_url.rstrip("/")}
    troot = os.environ.get("GP_PAGE_THEMES_ROOT", "")
    if troot:
        if not re.fullmatch(r"ssh-ed25519 [A-Za-z0-9+/]{68}( \S+)?", troot):
            sys.exit("build-page.sh: GP_PAGE_THEMES_ROOT must be an ssh-ed25519 public key line")
        themes["root"] = troot
if kind == "none":
    # A game with its own pictures: no ROM to name. Its facts are what it
    # is and who made it, from the package's [page] (`about`, `made_by`,
    # `howto`); the page writes its own lines for what isn't given.
    gid = re.sub(r"[^a-z0-9_-]", "-", pkg["package"]["name"].split("/")[-1].lower())
    facts = {"format": 1, "title": title, "player": player, "rom": {"kind": "none", "game": gid}, "howto": said.get("howto") or []}
    for k, n in (("made_by", 120), ("about", 600)):
        v = said.get(k)
        if v is None:
            continue
        if not isinstance(v, str) or not 0 < len(v) <= n:
            sys.exit(f"build-page.sh: package.toml's [page] {k} isn't a line of text of at most {n} characters")
        facts[k] = v
    page = {"title": title, "game": pkg["package"]["name"], "source": source, "content": content or None, "player": player, "rom": facts["rom"], "howto": facts["howto"]}
    for k in ("made_by", "about"):
        if k in facts:
            page[k] = facts[k]
    if themes:
        page["themes"] = themes
    if link_registry:
        page["link_registry"] = True
    open(f"{out}/page.json", "w").write(json.dumps(page, indent=1) + "\n")
    os.makedirs(f"{out}.facts", exist_ok=True)
    open(f"{out}.facts/page.json", "w").write(json.dumps(facts, indent=1) + "\n")
    sys.exit(0)
game = json.load(open(game_file))
shell = open(index).read()
def plain(fragment):
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", fragment))).strip()
# The page's own lines are an NES game's (index.html's keys and its line
# naming no maker); an arcade game's are the cabinet's keys (preview.js,
# ARCADE_LAYOUT) and no maker's name.
if platform == "arcade":
    own_howto = [
        "Controls: 5 drops a coin and 1 starts, as on a cabinet; the arrows or WASD move and X or Space is the button. On a controller Select drops a coin, Start starts and A is the button.",
        "Settings: press Esc or the Settings button (top right) to switch mode.",
        "Load your own romset zip to play.",
    ]
    # The owner's notice for an arcade game (#3): one short line.
    own_mark = f"You need your own {title} romset to play; {title}'s names and trademarks belong to their owners."
else:
    own_howto = [plain(p) for p in re.findall(r"<p>(.*?)</p>", re.search(r'<section id="howto"[^>]*>(.*?)</section>', shell, re.S).group(1), re.S)]
    own_mark = plain(re.search(r'<p id="about-mark">(.*?)</p>', shell, re.S).group(1))
howto = said.get("howto") or own_howto
trademark = said.get("trademark") or own_mark
sources = tomllib.load(open(registry, "rb"))["source"]
names, known = [], {}
sets = []
for tid in game["templates"]:
    src = next((s for s in sources if s["id"] == tid), None)
    if src:
        rev = src.get("revision", "")
        # An arcade set's revision is its full title (the game's title, then the set's words).
        n = rev if platform == "arcade" and rev.startswith(src["title"]) else f'{src["title"]} ({rev})'.replace(" ()", "")
        names.append(n)
        known[n] = src["sha256"]
        sets.append(tid)
def sentence(names, last):
    return ", ".join(names[:-1]) + f" {last} " + names[-1] if len(names) > 1 else "".join(names)
if platform == "arcade":
    # The sets the lift takes, by MAME's names, which are the zips' names,
    # and apart from them a set it refuses as not drawn right yet.
    took = [s for s in sets if s in taken]
    apart = [s for s in sets if s not in taken]
    file = f"a MAME romset zip of {sentence(took, 'or')}, known by its chips whatever the files in it are named"
    if apart:
        file += f" ({sentence(apart, 'and')} {'is' if len(apart) == 1 else 'are'} refused: not drawn right yet)"
    # The MAME sets the page takes, each with the files its zip must hold
    # (`gp roms sets`, from MAME's list: names, sizes and SHA-1s, nothing of
    # a ROM), so the page can say which set to bring.
    said_sets = subprocess.run([gp, "--json", "roms", "sets", *took], capture_output=True, text=True) if took else None
    if said_sets and said_sets.returncode != 0:
        sys.exit(f"build-page.sh: {gp} roms sets {' '.join(took)} failed (a gp from before `gp roms sets`?): {said_sets.stderr.strip()}")
    rom_sets = [{"name": x["name"], "title": x["title"], "sha256": x["sha256"], "files": [{"name": f["name"], "size": f["size"], "sha1": f["sha1"]} for f in x["files"]]} for x in (json.loads(said_sets.stdout)["sets"] if said_sets else [])]
else:
    file = "an .nes file of any dump: " + ", ".join(names) + ", or another revision"
page = {
    "title": title,
    "game": pkg["package"]["name"],
    "source": source,
    "content": content or None,
    "player": player,
    "rom": {
        "kind": kind,
        "game": game["game"],
        "title": game["title"],
        "name": game["title"],
        "known": names,
        "file": file,
        # The dumps the templates are for, which the page's check plays.
        "known_sha256": known,
    },
}
if platform == "arcade":
    page["rom"]["sets"] = rom_sets
# The page's own game shows its own help and About line (page.js reads
# them from here for the page's own game): the package's, else the page's.
page["howto"] = howto
page["trademark"] = trademark
if themes:
    page["themes"] = themes
if link_registry:
    page["link_registry"] = True
open(f"{out}/page.json", "w").write(json.dumps(page, indent=1) + "\n")
r = page["rom"]
facts = {
    "format": 1,
    "title": title,
    "player": player,
    "rom": {"kind": kind, "game": r["game"], "name": r["name"], "file": r["file"], "known": r["known"], "known_sha256": r["known_sha256"]},
    "howto": howto,
    "trademark": trademark,
}
if "sets" in r:
    facts["rom"]["sets"] = r["sets"]
os.makedirs(f"{out}.facts", exist_ok=True)
open(f"{out}.facts/page.json", "w").write(json.dumps(facts, indent=1) + "\n")
EOF
sed "s|<title>Game Printer</title>|<title>$title</title>|; s|<h1 id=\"title\">Game Printer</h1>|<h1 id=\"title\">$title</h1>|" \
  "$repo/scripts/web/public/index.html" > "$out/index.html"
cp "$repo/scripts/web/public/page.js" "$repo/scripts/web/public/share.js" "$repo/scripts/web/public/explain.js" "$repo/scripts/web/public/what-is-this.js" "$repo/scripts/web/public/what-is-this.css" "$repo/scripts/web/public/boot.js" "$repo/scripts/web/public/page.css" "$repo/scripts/web/public/_headers" "$out/"
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
# The game's files leave the page for the package: only the player stays.
play="$out.play"
rm -rf "$play"
mkdir -p "$play"
for f in bundle.json preview.json profiles.toml profiles assets themes runs; do
  [ -e "$out/game/$f" ] && mv "$out/game/$f" "$play/"
done
if [ "$own" = 0 ]; then
  mkdir -p "$play/lift"
  for f in "$out/game/lift/"*; do
    [ "$(basename "$f")" = gp-web-lift.wasm ] || mv "$f" "$play/lift/"
  done
fi
mv "$out.facts/page.json" "$play/page.json"
rmdir "$out.facts"

# A game with hooks ships its own module; one without (a game `gp game new`
# made) plays in the page's own interpreter.
if python3 -I -c 'import sys, tomllib; sys.exit(0 if "hooks" in tomllib.load(open(sys.argv[1], "rb")) else 1)' "$game/package.toml"; then
  "$gp" game kit -o "$real/target/module-kit" >/dev/null
  GP_HOME="$home" "$gp" game build "$game" --kit "$real/target/module-kit" -o "$play/module.wasm"
  lap "the game's module"
fi
echo "page $(du -sk "$out" | cut -f1) KB in $out"

# Holds none of the game's data: every file of both folders against each ROM.
report="$out.served.txt"
status=0
"$gp" template served "$game" --folder "$out" --folder "$play" > "$report" || status=$?
cat "$report"
lap "gp template served"
exit "$status"
