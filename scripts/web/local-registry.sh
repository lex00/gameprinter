#!/bin/bash
# A registry in a folder on this machine, and the page built to install from
# it. Rebuilt whole each run: the game comes from the games repo, the page's
# theme from its own repo (lex00/turnips, beside this one).
#   scripts/web/local-registry.sh [GAMES_DIR] [THEME_DIR]
# Then: target/debug/gp registry serve "$GP_HOME/registry" --page target/page-local
#
# Signed with the staging keys. Exit codes: 0 built, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
real="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$real"
games="$(cd "${1:-$real/../games}" && pwd)"
theme="$(cd "${2:-$real/../turnips}" && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
keys="$home/keys/staging"
reg="$home/registry"
gp="$real/target/debug/gp"
out="$real/target/page-local"
GP_PAGE_SOURCE=local GP_PAGE_CONTENT=lex00/turnips GP_HOME="$home" WEB_TOOLS="${WEB_TOOLS:-$(scripts/web/fetch-tools.sh "$home" | tail -1)}" \
  scripts/web/build-page.sh "$games/donkey-kong" "$out"
rm -rf "$reg"
mkdir -p "$reg"
"$gp" registry init "$reg" --name gp-local --root-key "$keys/root" --signing-key "$keys/signing" --namespace "lex00=$keys/lex00"
scripts/web/publish-game.sh "$games/donkey-kong" "$out.play" "$reg" "$keys/lex00"
# The theme's package: its folder less git's files.
stage="$real/target/theme-stage"
rm -rf "$stage"
rsync -a --exclude .git --exclude .gitattributes "$theme/" "$stage/"
theme_version="$(python3 -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' "$stage/package.toml")"
"$gp" registry publish "$reg" "$stage" --name lex00/turnips --version "$theme_version" --kind content --title Turnips --for lex00/donkey-kong --key "$keys/lex00"
game_version="$(python3 -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["package"]["version"])' "$games/donkey-kong/package.toml")"
"$gp" registry reindex "$reg" "lex00/donkey-kong@$game_version" "lex00/turnips@$theme_version" --key "$keys/signing"
echo "registry in $reg, page in $out"
