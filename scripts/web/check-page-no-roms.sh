#!/bin/bash
# The public page checked with no ROM and no games checkout, for just
# check-web (row page-check-without-roms-in-check-web): the page built by
# scripts/web/build-page.sh for Own Hop (scripts/web/own-hop.sh, a copy of
# the Moss Hop template that plays from its own pictures), installing it
# from a registry folder of its own with no signatures (sources.toml's
# `folder`, made by gp registry init and publish-game.sh), and
# scripts/web/public/check-page.mjs's sections that need no ROM run on it:
#   own-game, own-game-tampered-blob, own-game-tampered-manifest
#       the game by its name, playing with no ROM, and a tampered package
#   theme-hud-over, theme-in-region     the made-up themes of
#       scripts/web/mosshop-hud.sh, drawn by the page's own player
#   theme-sound-takes                   scripts/web/mosshop-takes.sh's
# with an empty games folder (--games) and an empty ROM folder
# (GP_ROM_DIR), in a GP_HOME made for the run (a temp folder outside the
# repo, removed at the end), so nothing of the person's
# home, ROMs or games is read. Moss Hop's pictures are lifted from ROM,
# the project's own (`just mosshop` assembles it).
#   scripts/web/check-page-no-roms.sh WORK BROWSER MOSSHOP_ROM
# WORK is emptied and made again. GP names the gp to run (default
# target/debug/gp). Each of the six sections must run and pass: a section
# the page check leaves out (it prints why on a `skip` line) fails it.
# Exit codes: 0 passed, 1 a section failed or didn't run, 2 couldn't build.
set -euo pipefail
if [ $# -ne 3 ]; then
  echo "usage: scripts/web/check-page-no-roms.sh WORK BROWSER MOSSHOP_ROM" >&2
  exit 2
fi
real="$(cd "$(dirname "$0")/../.." && pwd)"
gp="${GP:-$real/target/debug/gp}"
browser="$2"
rom="$3"
[ -f "$rom" ] || { echo "check-page-no-roms: no Moss Hop ROM at $rom (just mosshop)" >&2; exit 2; }
mkdir -p "$1"
w="$(cd "$1" && pwd)"
rm -rf "$w/run"
w="$w/run"
mkdir -p "$w/games"
# gp refuses a GP_HOME or a ROM folder inside a git work tree, and WORK is
# under target/.
home="$(mktemp -d "${TMPDIR:-/tmp}/page-no-roms-home.XXXXXX")"
trap 'rm -rf "$home"' EXIT
mkdir -p "$home/home" "$home/roms"
export GP_HOME="$home/home" GP_ROM_DIR="$home/roms" GP_PAGE_REUSE="$w/reuse"
unset GP_GAMES_DIR GP_PAGE_CONTENT GP_PAGE_SOURCE_URL GP_PAGE_THEMES_URL GP_PAGE_LINK_REGISTRY
say() { echo "check-page-no-roms: $*"; }
built() {
  local what="$1" log="$2"
  shift 2
  if ! "$@" > "$log" 2>&1; then
    tail -20 "$log" >&2
    say "$what failed (its log: $log)" >&2
    exit 2
  fi
}
built "lifting Moss Hop's pictures" "$w/lift.log" "$gp" assets lift "$rom" --map "$real/assets/mosshop.toml" -o "$GP_HOME/work/assets/mosshop" --force
built "making Own Hop" "$w/own-hop.log" bash "$real/scripts/web/own-hop.sh" "$w/own-hop"
built "build-page.sh" "$w/page.log" env GP="$gp" GP_PAGE_SOURCE=folder bash "$real/scripts/web/build-page.sh" "$w/own-hop" "$w/page"
built "gp registry init" "$w/registry.log" "$gp" registry init "$w/registry" --name gp-folder --namespace test
built "publish-game.sh" "$w/publish.log" env GP="$gp" bash "$real/scripts/web/publish-game.sh" "$w/own-hop" "$w/page.play" "$w/registry"
built "gp registry reindex" "$w/reindex.log" "$gp" registry reindex "$w/registry" --keep test/own-hop@0.1.0
built "mosshop-hud.sh" "$w/mosshop-hud.log" env GP="$gp" bash "$real/scripts/web/mosshop-hud.sh" "$w/page.mosshop"
built "mosshop-takes.sh" "$w/mosshop-takes.log" env GP="$gp" bash "$real/scripts/web/mosshop-takes.sh" "$w/page.mosshop-takes"
say "Own Hop's page in $w/page, its registry in $w/registry"

sections="own-game own-game-tampered-blob own-game-tampered-manifest theme-hud-over theme-in-region theme-sound-takes"
only="^($(echo $sections | tr ' ' '|'))\$"
status=0
node "$real/scripts/web/public/check-page.mjs" --dist "$w/page" --browser "$browser" \
  --games "$w/games" --registry-folder "$w/registry" \
  --mosshop "$w/page.mosshop" --mosshop-takes "$w/page.mosshop-takes" \
  --shots "$w/shots" --only "$only" > "$w/check.log" 2>&1 || status=$?
cat "$w/check.log"
missing=()
for s in $sections; do
  grep -q "^-- $s: ran " "$w/check.log" || missing+=("$s")
done
if [ ${#missing[@]} -gt 0 ]; then
  say "the page check didn't run ${missing[*]} (its skip lines above say why)" >&2
  exit 1
fi
if [ "$status" != 0 ]; then
  say "the page check failed (exit $status); its log: $w/check.log" >&2
  exit 1
fi
say "the six sections that need no ROM passed on Own Hop's page, with no ROM and no games checkout"
