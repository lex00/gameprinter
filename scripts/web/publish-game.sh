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
