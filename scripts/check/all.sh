#!/usr/bin/env bash
# all.sh GP (`just check-all`): `just check`, then the checks that until
# now ran only by hand, each when what it needs is on this machine:
#
#   web-parity-arcade-GAME  each arcade game's model in the page's player
#                      against the native run (a game of the games checkout
#                      with model.toml and checks.toml's [web_parity], and
#                      the romset its corpus is for in the ROM folder)
#   web-page           the public page built and played headless, and its
#                      lift's tests (the games checkout beside the repo, the
#                      network for the registry and the page's tools)
#   page-tests         the public page's share.test.mjs and theme-line.test.mjs
#   score-GAME         gp corpus score on each game of the games checkout
#                      with a corpus.toml: every clip exact (the ROM its
#                      corpus.toml names in the ROM folder; for an arcade
#                      game, roms.toml's system for it, SYSTEM.zip)
#   mame-NAME          each MAME capture a game's oracle/mame-captures.toml
#                      names, played on gp-arcade: every frame, AY writes
#                      too (SYSTEM.zip, the capture NAME under
#                      $GP_HOME/work/oracle, the input log the entry names,
#                      with its hash); builds gp-arcade's compare example in
#                      release
#   baselines          gp baseline check over the repo's baselines/
#
# A check it can't run is one line: what was skipped and what is missing.
# It exits 1 when a check that ran failed, and 0 otherwise, skips and all.
# CHECK_ALL_DRY=1 prints each command instead of running it (just check
# included), with the same skip lines.
set -uo pipefail

gp=$1
repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo" || exit 2
gp_home="${GP_HOME:-$HOME/.gameprinter}"
roms="${GP_ROM_DIR:-$gp_home/roms}"
games="${GP_GAMES_DIR:-}"
facts() { python3 -I -B "$repo/scripts/check/game-facts.py" "$@"; }
dry="${CHECK_ALL_DRY:-}"
logs="${CARGO_TARGET_DIR:-$repo/target}/check-all"
mkdir -p "$logs"

failed=()
what=""
ran=()
skipped=()

skip() {
    echo "check-all: skipped $1: $2"
    skipped+=("$1")
}

# step NAME CMD...: runs CMD, its output kept in $logs/NAME.log and its
# last lines printed; a failure is counted, never stops the rest.
step() {
    local name=$1
    shift
    if [ -n "$dry" ]; then
        echo "check-all: would run $name: ${what:-$*}"
        what=""
        return 0
    fi
    echo "check-all: $name ..."
    local start=$SECONDS
    if "$@" > "$logs/$name.log" 2>&1; then
        echo "check-all: $name passed in $((SECONDS - start)) s"
        ran+=("$name")
        return 0
    fi
    echo "check-all: $name FAILED after $((SECONDS - start)) s; the end of its output ($logs/$name.log):"
    tail -20 "$logs/$name.log" | sed 's/^/  /'
    failed+=("$name")
    return 1
}

# score NAME DIR: gp corpus score on the package DIR, passing only when
# every clip is exact, none failed to score and it exits 0.
score() {
    local name=$1 dir=$2 out
    out=$(GP_GAMES_DIR="$games" GP_ROM_DIR="$roms" "$gp" corpus score --run "check-all-$name" "$dir" 2>&1)
    local status=$?
    echo "$out"
    local line
    line=$(grep -Eo '[0-9]+ of [0-9]+ clips exact, fidelity [0-9.]+%, [0-9]+ couldn.t be scored' <<<"$out" | head -1)
    if [ -z "$line" ]; then
        echo "no score line in gp corpus score's output (exit $status)"
        return 1
    fi
    read -r exact _ clips _ <<<"$line"
    if [ "$exact" != "$clips" ] || ! grep -q ', 0 couldn.t be scored' <<<"$line"; then
        echo "not every clip exact: $line"
        return 1
    fi
    # Exit 1 with every clip exact: the corpus has problems (its check's,
    # listed above), which a score line alone doesn't show.
    if [ "$status" -ne 0 ]; then
        echo "gp corpus score exited $status with every clip exact: $line"
        return 1
    fi
    echo "check-all: $line"
}

# mame_match ZIP CAPTURE LOG: the log on gp-arcade against MAME's capture.
mame_match() {
    local out
    out=$(GP_GAMES_DIR="$games" cargo run -q --release -p gp-arcade --example compare -- --rom "$1" \
        --capture "$2" --log "$3" --all 2>&1)
    local status=$?
    echo "$out"
    [ "$status" -eq 0 ] && grep -Eq 'all [0-9]+ frames match MAME' <<<"$out"
}

page_tests() {
    node --test scripts/web/public/share.test.mjs scripts/web/public/theme-line.test.mjs
}

# The network: the registry the page's theme and lists come from.
online() {
    curl -sfI --max-time 10 https://raw.githubusercontent.com/lex00/gp-registry/main/registry.toml > /dev/null
}

if ! step check just check; then
    echo "check-all: just check failed, so the rest isn't run"
    exit 1
fi

# zip_of GAME: the romset an arcade game's corpus is for, SYSTEM.zip in
# the ROM folder, whether or not it is there.
zip_of() {
    echo "$roms/arcade/$(facts "$1" system).zip"
}

arcade_games=$(facts arcade "$games" 2>/dev/null || true)
parity=0
for g in $arcade_games; do
    name=$(basename "$g")
    if [ ! -f "$g/model.toml" ] || ! facts "$g" check web_parity.clips > /dev/null; then
        continue
    fi
    parity=1
    zip=$(zip_of "$g")
    if [ ! -f "$zip" ]; then
        skip "web-parity-arcade-$name" "no $zip"
    else
        step "web-parity-arcade-$name" just --no-deps web-parity-arcade "$g"
    fi
done
if [ "$parity" -eq 0 ]; then
    skip web-parity-arcade "no arcade game with model.toml and checks.toml's [web_parity] in the games checkout at $games"
fi

# The recipe builds from the games checkout GP_GAMES_DIR names, the game its
# page.toml names.
if [ -z "$games" ] || ! page_game=$(scripts/web/page-game.sh "$games" 2>/dev/null) || [ ! -f "$page_game/package.toml" ]; then
    skip web-page "no games checkout (GP_GAMES_DIR) whose page.toml names a game package"
elif [ -z "$dry" ] && ! online; then
    skip web-page "no network (the registry at raw.githubusercontent.com doesn't answer)"
else
    step web-page just --no-deps web-page check
fi

if command -v node > /dev/null; then
    what="node --test scripts/web/public/share.test.mjs scripts/web/public/theme-line.test.mjs"
    step page-tests page_tests
else
    skip page-tests "no node on PATH"
fi

# The scan is read whole before it is searched: piped into grep -q, gp
# could be cut off once grep has its line, and pipefail took that for no ROM.
scan=""
scored=0
for g in $(facts list "$games" corpus.toml 2>/dev/null || true); do
    name=$(basename "$g")
    scored=1
    rom=$(facts "$g" rom || true)
    if [ "$(facts "$g" platform || true)" = arcade ]; then
        zip=$(zip_of "$g")
        if [ ! -f "$zip" ]; then
            skip "score-$name" "no $zip, the romset $name's corpus is for (sha256 $rom)"
            continue
        fi
    else
        if [ -z "$scan" ] && [ -d "$roms" ]; then
            scan=$(GP_ROM_DIR="$roms" "$gp" --json roms scan 2>/dev/null)
        fi
        if [ -z "$rom" ] || ! grep -q "\"sha256\": \"$rom\"" <<<"$scan"; then
            skip "score-$name" "no ROM $name's corpus is for (sha256 $rom) in $roms"
            continue
        fi
    fi
    what="$gp corpus score --run check-all-$name $g, every clip exact"
    step "score-$name" score "$name" "$g"
done
if [ "$scored" -eq 0 ]; then
    skip score "no game with a corpus.toml in the games checkout at $games"
fi

captured=0
for g in $(facts list "$games" oracle/mame-captures.toml 2>/dev/null || true); do
    while read -r cap system input input_sha _frames _sha; do
        [ -n "$cap" ] || continue
        captured=1
        zip="$roms/arcade/$system.zip"
        dir="$gp_home/work/oracle/$cap"
        log="$g/$input"
        if [ ! -f "$zip" ]; then
            skip "mame-$cap" "no $zip"
        elif [ ! -d "$dir" ]; then
            skip "mame-$cap" "no MAME capture at $dir (just mame-record $zip $log $cap 0 makes one)"
        elif [ "$(shasum -a 256 < "$log" 2>/dev/null | cut -d' ' -f1)" != "$input_sha" ]; then
            what=""
            echo "check-all: mame-$cap FAILED: $log isn't the input $g/oracle/mame-captures.toml names (sha256 $input_sha)"
            failed+=("mame-$cap")
        else
            what="cargo run --release -p gp-arcade --example compare -- --rom $zip --capture $dir --log $log --all, every frame matching"
            step "mame-$cap" mame_match "$zip" "$dir" "$log"
        fi
    done < <(facts "$g" captures || true)
done
if [ "$captured" -eq 0 ]; then
    skip mame "no game with oracle/mame-captures.toml in the games checkout at $games"
fi

step baselines env GP_ROM_DIR="$roms" "$gp" baseline check

echo
if [ -n "$dry" ]; then
    echo "check-all: dry run; ${#skipped[@]} skipped"
    exit 0
fi
echo "check-all: ${#ran[@]} passed (${ran[*]:-none}), ${#failed[@]} failed (${failed[*]:-none}), ${#skipped[@]} skipped (${skipped[*]:-none})"
[ "${#failed[@]}" -eq 0 ]
