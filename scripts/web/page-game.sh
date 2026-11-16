#!/bin/bash
# The public page's game folder: the one a games checkout's page.toml names
# (`game = "FOLDER"`, at the top of the games repo). Prints GAMES_DIR/FOLDER.
#   scripts/web/page-game.sh [GAMES_DIR]
# GAMES_DIR is the games checkout beside this repo when not given.
# Exit codes: 0 printed, 2 no page.toml or it names no game.
set -euo pipefail
real="$(cd "$(dirname "$0")/../.." && pwd)"
games="${1:-$real/../games}"
[ -f "$games/page.toml" ] || { echo "page-game.sh: $games has no page.toml naming the page's game" >&2; exit 2; }
folder="$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["game"])' "$games/page.toml")" \
  || { echo "page-game.sh: $games/page.toml names no game (game = \"FOLDER\")" >&2; exit 2; }
echo "$games/$folder"
