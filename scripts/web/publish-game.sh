#!/bin/bash
# Publishes a game with its built play files.
#   scripts/web/publish-game.sh GAME_DIR PLAY_DIR TARGET [KEY]
#
# With no KEY the version is published with no signature.
#
# The package is the game's folder, roms.toml with it (the ROMs a player
# needs, by hash: no ROM bytes), plus PLAY_DIR as play/. A folder
# themes/NAME-src, how a theme was made (its scripts, prompts and the
# images they cut from), is left out. Exit codes: 0
# published, 2 couldn't.
#
# A publish into the page's registry changes what the live public page
# serves with no deploy, so its last line names the two checks to run next
# (row publish-says-the-live-page-check): gp registry check on the
# registry's address (sources.toml's [registry] url) and, from the games
# checkout, just page-live arcade. It prints them and runs neither.
set -euo pipefail
trap 'exit 2' ERR
real="$(cd "$(dirname "$0")/../.." && pwd)"
gp="${GP:-$real/target/debug/gp}"
game="$(cd "$1" && pwd)"; play="$(cd "$2" && pwd)"; target="$3"; key="${4:-}"
stage="$real/target/publish-stage"
rm -rf "$stage"
mkdir -p "$stage"
rsync -a --exclude target --exclude .DS_Store --exclude "/themes/*-src" "$game/" "$stage/"
rsync -a "$play/" "$stage/play/"
read -r name version kind title < <(python3 -I - "$stage/package.toml" <<'PY'
import sys, tomllib
p = tomllib.load(open(sys.argv[1], "rb"))["package"]
print(p["name"], p["version"], p["kind"], p["title"])
PY
)
"$gp" registry publish "$target" "$stage" --name "$name" --version "$version" --kind "$kind" --title "$title" ${key:+--key "$key"}
rm -rf "$stage"
address="$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["registry"]["url"])' "$real/scripts/web/sources.toml")"
echo "published $name $version; the live page serves it now with no deploy, so check it next: gp registry check $address (the registry as a visitor's browser reads it: CORS, every file by its hash, the game's list) and, from the games checkout, just page-live arcade (the page check on the live page's own address: the arcade game installed by its name and played there)"
