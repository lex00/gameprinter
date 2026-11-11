#!/bin/bash
# Builds the public page's in-browser ROM lift (docs/web.md, "The public
# page"):
#   scripts/web/build-lift.sh OUT [GAME]
#
# OUT gets wasi.js and lift.js (scripts/web/public/), beside the preview's
# preview.js, which imports lift.js for `files=idb:GAME`, and OUT/lift/:
# gp-web-lift.wasm (crates/gp-web-lift, release, for wasm32-wasip1) and the
# game folder it reads (gp_web_lift::GAME_FILE): game.json, sources/nes.toml
# as sources-nes.toml, and a folder for each template whose template.game
# is GAME (default donkey-kong) and whose source is a ROM, with its
# model.toml, profiles.toml, template.toml, its asset map as
# asset-map.toml (with the map's content hashes, which the lift finds the
# assets of any dump by) and its themes, and a folder for each ROM a
# template lists under [[loads]] (its model, the ROM's asset map and
# load.json, with what the ROM's game does without, `[[loads.without]]`,
# which must agree with the package.toml entry of the same id when the
# template folder has one). Nothing fills an asset a ROM lacks: the lift
# leaves blank only those a load's `without` names. With GP_LIFT_LEVELS
# set (level names joined by commas, such as `main`), game.json's `levels`
# lists them: the lift then plays each stage of a model's stages list whose
# level isn't one of them by the list's stand-in (gp_web_lift::play_only).
# index.json names every file. Nothing here is from a
# ROM: the visitor's ROM is lifted in their browser.
#
# With WEB_TOOLS set to the directory scripts/web/fetch-tools.sh prints,
# wasm-opt shrinks the module. Builds go to CARGO_TARGET_DIR (default
# target/). With GP_WEB_REUSE=1 the optimized module is taken from
# scripts/web/reuse.py's store when none of the files it was made from
# changed (as in build-preview.sh).
#
# Exit codes: 0 built, 2 couldn't build.

set -euo pipefail
trap 'exit 2' ERR

if [ $# -lt 1 ]; then
  echo "usage: scripts/web/build-lift.sh OUT [GAME]" >&2
  exit 2
fi
out="$1"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
game="${2:-donkey-kong}"
target="$(cd "$repo" && mkdir -p "${CARGO_TARGET_DIR:-target}" && cd "${CARGO_TARGET_DIR:-target}" && pwd)"

reuse="$repo/scripts/web/reuse.py"
cargo_args=(build -q --locked --release -p gp-web-lift --target wasm32-wasip1
  --target-dir "$target/web-lift"
  --config 'profile.release.lto=true' --config 'profile.release.opt-level="s"'
  --config 'profile.release.codegen-units=1' --config 'profile.release.strip=true')
opt_args=(-Oz --enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int
  --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue
  --strip-debug --strip-producers)
extra="${cargo_args[*]//$target/TARGET} wasm-opt ${WEB_TOOLS:-none} ${opt_args[*]} $(cd "$repo" && rustc -vV | tr '\n' ' ')"
mkdir -p "$out/lift"
if [ "${GP_WEB_REUSE:-0}" = 1 ] && from="$(python3 -I "$reuse" find web-lift-wasm --extra "$extra")"; then
  cp "$from/out/gp-web-lift.wasm" "$out/lift/gp-web-lift.wasm"
  echo "lift wasm kept ($from)"
else
  (cd "$repo" && cargo "${cargo_args[@]}") || exit 2
  wasm="$target/web-lift/wasm32-wasip1/release/gp-web-lift.wasm"
  if [ -n "${WEB_TOOLS:-}" ]; then
    "$WEB_TOOLS/wasm-opt" "${opt_args[@]}" "$wasm" -o "$out/lift/gp-web-lift.wasm" || exit 2
  else
    cp "$wasm" "$out/lift/gp-web-lift.wasm"
  fi
  case "${GP_WEB_REUSE:-0}" in
    1|save) python3 -I "$reuse" save web-lift-wasm --extra "$extra" --depinfo "${wasm%.wasm}.d" --out "$out/lift/gp-web-lift.wasm" >/dev/null ;;
  esac
fi
cp "$repo/scripts/web/public/wasi.js" "$repo/scripts/web/public/lift.js" "$out/"
python3 -I - "$repo" "$out/lift" "$game" "${GP_LIFT_LEVELS:-}" <<'PY'
import json, os, shutil, sys, tomllib
repo, out, game, levels = sys.argv[1:]
files = []
def put(src, rel):
    dst = os.path.join(out, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copyfile(src, dst)
    files.append(rel)
def write(rel, text):
    dst = os.path.join(out, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    open(dst, "w").write(text)
    files.append(rel)
ids, title = [], None
tdir = os.path.join(repo, "templates")
# The templates the page lifts, by folder name, with their manifests.
bundled = {}
for name in sorted(os.listdir(tdir)):
    t = os.path.join(tdir, name)
    try:
        m = tomllib.load(open(os.path.join(t, "template.toml"), "rb"))
    except FileNotFoundError:
        continue
    if m["template"].get("game") != game or m.get("source", {}).get("how") != "rom":
        continue
    bundled[name] = m
for name, m in bundled.items():
    t = os.path.join(tdir, name)
    tid = m["template"]["id"]
    ids.append(tid)
    title = title or m["template"]["title"]
    for f in ["model.toml", "profiles.toml", "template.toml"]:
        put(os.path.join(t, f), f"{tid}/{f}")
    put(os.path.join(t, m["recipe"]["asset_map"]), f"{tid}/asset-map.toml")
    # The template's themes (each folder with theme.toml): the lift draws
    # each on the visitor's lifted art as a look the settings offer beside
    # the original (gp_gen_bevy::looks). They are the template's own text.
    # A folder holding only same_as.toml points at another template's theme
    # (gp_theme::same_as): the pointer ships, naming the target by its
    # folder here, when the target's template is in the bundle; else the
    # target's files ship in its place.
    th = os.path.join(t, "themes")
    if os.path.isdir(th):
        for d in sorted(os.listdir(th)):
            src = os.path.join(th, d)
            if not os.path.isfile(os.path.join(src, "theme.toml")):
                pointer = os.path.join(src, "same_as.toml")
                if not os.path.isfile(pointer):
                    continue
                rel = tomllib.load(open(pointer, "rb"))["theme"]
                to = os.path.normpath(os.path.join(src, rel))
                parts = os.path.relpath(to, tdir).split(os.sep)
                if len(parts) != 3 or parts[1] != "themes" or not os.path.isfile(os.path.join(to, "theme.toml")):
                    sys.exit(f"{pointer}: `{rel}` isn't a theme folder of a template beside this one")
                if parts[0] in bundled:
                    target_id = bundled[parts[0]]["template"]["id"]
                    write(f"{tid}/themes/{d}/same_as.toml", f'theme = "../../../{target_id}/themes/{parts[2]}"\n')
                    continue
                src = to
            for f in sorted(os.listdir(src)):
                if os.path.isfile(os.path.join(src, f)):
                    put(os.path.join(src, f), f"{tid}/themes/{d}/{f}")
    # A ROM a person may load to play the template's model ([[loads]],
    # docs/templates.md "ROMs to play"): a folder named by its source with
    # the template's model, its own asset map, and load.json (the model's
    # template, the levels the ROM has, the assets it takes in place of
    # the model's and what its game does without). Its themes point at the
    # template's.
    package = os.path.join(t, "package.toml")
    entries = tomllib.load(open(package, "rb")).get("rom", {}).get("entry", []) if os.path.isfile(package) else []
    for load in m.get("loads", []):
        lid = load["source"]
        ids.append(lid)
        for f in ["model.toml", "profiles.toml", "template.toml"]:
            put(os.path.join(t, f), f"{lid}/{f}")
        put(os.path.join(t, load["asset_map"]), f"{lid}/asset-map.toml")
        said = {k: load.get(k, d) for k, d in [("levels", []), ("assets", {}), ("without", [])]}
        for e in entries:
            if e.get("id") == lid and said != {k: e.get(k, d) for k, d in [("levels", []), ("assets", {}), ("without", [])]}:
                sys.exit(f"{name}: template.toml's [[loads]] for {lid} and package.toml's entry disagree on its levels, assets or without")
        write(f"{lid}/load.json", json.dumps({"plays": tid, **said}) + "\n")
        if os.path.isdir(th):
            for d in sorted(os.listdir(th)):
                if os.path.isfile(os.path.join(th, d, "theme.toml")):
                    write(f"{lid}/themes/{d}/same_as.toml", f'theme = "../../../{tid}/themes/{d}"\n')
if not ids:
    sys.exit(f"no template's template.game is {game}")
put(os.path.join(repo, "sources", "nes.toml"), "sources-nes.toml")
gamefile = {"game": game, "title": title, "templates": ids}
if levels:
    gamefile["levels"] = [l for l in levels.split(",") if l]
open(os.path.join(out, "game.json"), "w").write(json.dumps(gamefile) + "\n")
files.append("game.json")
open(os.path.join(out, "index.json"), "w").write(json.dumps({"game": game, "files": files}) + "\n")
PY
echo "gp-web-lift.wasm $(wc -c < "$out/lift/gp-web-lift.wasm" | tr -d ' ') bytes"
