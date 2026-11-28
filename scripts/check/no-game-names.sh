#!/bin/bash
# The tools are game-agnostic: crates/, mechanic/, plugin/ and scripts/ name
# no ported game (a folder of lex00/games). Tests use made-up games
# (rooftop, hilltop, made-up) and comments cite a quest-log row by its name
# alone, never as GAME/ROW. Fails with each line that names one. gp tool
# gate runs it first, right after the patch applies, before it builds. The ported games are
# listed here; with GP_GAMES_DIR set, each folder of it holding a
# model.toml is added. A name's hyphen also matches a space, an underscore
# or nothing (donkey-kong, "Donkey Kong", donkey_kong, DonkeyKong).
#
# Left out: this file (it lists the names) and scripts/web/deploy-old*, the
# old Cloudflare Worker, whose deployed name is the game's and can't change.
set -euo pipefail
cd "$(dirname "$0")/../.."

games=(tapper donkey-kong)
if [ -n "${GP_GAMES_DIR:-}" ] && [ -d "$GP_GAMES_DIR" ]; then
    for m in "$GP_GAMES_DIR"/*/model.toml; do
        [ -f "$m" ] && games+=("$(basename "$(dirname "$m")")")
    done
fi
pattern=""
for g in "${games[@]}"; do
    pattern="${pattern:+$pattern|}${g//-/[-_ ]?}"
done

dirs=()
for d in crates mechanic plugin scripts; do
    [ -d "$d" ] && dirs+=("$d")
done
set +e
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    hits=$(git grep --untracked -nIiE "$pattern" -- "${dirs[@]}" \
        ':!scripts/check/no-game-names.sh' \
        ':!scripts/web/deploy-old.sh' ':!scripts/web/deploy-old/**')
else
    hits=$(grep -rnIiE "$pattern" "${dirs[@]}" \
        --exclude=no-game-names.sh --exclude=deploy-old.sh --exclude-dir=deploy-old \
        --exclude-dir=node_modules --exclude-dir=target)
fi
rc=$?
set -e
if [ "$rc" -gt 1 ]; then
    echo "no-game-names: the search failed ($rc)" >&2
    exit 2
fi
if [ -n "$hits" ]; then
    echo "$hits"
    echo "no-game-names: the tools name a ported game (${games[*]}) above: use a made-up game (rooftop, hilltop) in tests, cite a quest-log row by its name alone (never GAME/ROW), and name no game's theme" >&2
    exit 1
fi
echo "no-game-names: crates/, mechanic/, plugin/ and scripts/ name no ported game (${games[*]})"
