#!/usr/bin/env bash
# all.sh GP (`just check-all`): `just check`, then the checks that until
# now ran only by hand, each when what it needs is on this machine:
#
#   web-parity-arcade  Tapper's model in the page's player against the native
#                      run (the games checkout's tapper/, tapper.zip)
#   web-page           the public page built and played headless, and its
#                      lift's tests (the games checkout's donkey-kong/, the
#                      network for the registry and the page's tools)
#   page-tests         the public page's share.test.mjs and theme-line.test.mjs
#   score-dk           gp corpus score on Donkey Kong: every clip exact (the
#                      games checkout, Donkey Kong: Original Edition in the
#                      ROM folder)
#   score-tapper       the same on Tapper (the games checkout, tapper.zip)
#   mame-match         Tapper against MAME's capture, every frame, AY writes
#                      too (tapper.zip, the capture tapper-m3b-cycle under
#                      $GP_HOME/work/oracle); builds gp-arcade's compare
#                      example in release
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
games="${GP_GAMES_DIR:-$repo/../games}"
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

mame_match() {
    local out
    out=$(cargo run -q --release -p gp-arcade --example compare -- --rom "$roms/arcade/tapper.zip" \
        --capture "$gp_home/work/oracle/tapper-m3b-cycle" --log scripts/oracle/inputs/tapper-cycle.input --all 2>&1)
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

tapper_zip="$roms/arcade/tapper.zip"
if [ ! -f "$games/tapper/model.toml" ]; then
    skip web-parity-arcade "no games checkout with tapper/ at $games"
elif [ ! -f "$tapper_zip" ]; then
    skip web-parity-arcade "no $tapper_zip"
else
    step web-parity-arcade just --no-deps web-parity-arcade
fi

# The recipe builds from the games checkout beside the repo.
if [ ! -f "$repo/../games/donkey-kong/package.toml" ]; then
    skip web-page "no games checkout with donkey-kong/ beside the repo, at $repo/../games"
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

dk_rom=$(sed -n 's/^rom = "\([0-9a-f]*\)"$/\1/p' "$games/donkey-kong/corpus.toml" 2>/dev/null | head -1)
if [ ! -f "$games/donkey-kong/corpus.toml" ]; then
    skip score-dk "no games checkout with donkey-kong/ at $games"
# The scan is read whole before it is searched: piped into grep -q, gp
# could be cut off once grep has its line, and pipefail took that for no ROM.
elif [ ! -d "$roms" ] || ! grep -q "\"sha256\": \"$dk_rom\"" <<<"$(GP_ROM_DIR="$roms" "$gp" --json roms scan 2>/dev/null)"; then
    skip score-dk "no Donkey Kong ROM the corpus is for (sha256 $dk_rom) in $roms"
else
    what="$gp corpus score --run check-all-dk $games/donkey-kong, every clip exact"
    step score-dk score dk "$games/donkey-kong"
fi

if [ ! -f "$games/tapper/corpus.toml" ]; then
    skip score-tapper "no games checkout with tapper/ at $games"
elif [ ! -f "$tapper_zip" ]; then
    skip score-tapper "no $tapper_zip"
else
    what="$gp corpus score --run check-all-tapper $games/tapper, every clip exact"
    step score-tapper score tapper "$games/tapper"
fi

if [ ! -f "$tapper_zip" ]; then
    skip mame-match "no $tapper_zip"
elif [ ! -d "$gp_home/work/oracle/tapper-m3b-cycle" ]; then
    skip mame-match "no MAME capture at $gp_home/work/oracle/tapper-m3b-cycle (just mame-record makes one)"
else
    what="cargo run --release -p gp-arcade --example compare -- --rom $tapper_zip --capture $gp_home/work/oracle/tapper-m3b-cycle --log scripts/oracle/inputs/tapper-cycle.input --all, every frame matching"
    step mame-match mame_match
fi

step baselines env GP_ROM_DIR="$roms" "$gp" baseline check

echo
if [ -n "$dry" ]; then
    echo "check-all: dry run; ${#skipped[@]} skipped"
    exit 0
fi
echo "check-all: ${#ran[@]} passed (${ran[*]:-none}), ${#failed[@]} failed (${failed[*]:-none}), ${#skipped[@]} skipped (${skipped[*]:-none})"
[ "${#failed[@]}" -eq 0 ]
