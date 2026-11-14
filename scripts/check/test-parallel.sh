#!/usr/bin/env bash
# The checks of scripts/check/parallel.sh, which `just check` runs its
# lanes with: the lanes run at once, the first failure stops the rest and
# is the exit status, and the failed lane's output is printed.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
logs=${1:-$(mktemp -d "${TMPDIR:-/tmp}/test-parallel.XXXXXX")}
mkdir -p "$logs"
fail() { echo "test-parallel: $*" >&2; exit 1; }
now() { python3 -I -c 'import time; print(time.monotonic())'; }
took() { python3 -I -c 'import sys, time; print(time.monotonic() - float(sys.argv[1]) < float(sys.argv[2]))' "$1" "$2"; }

t=$(now)
out=$("$here/parallel.sh" "$logs/a" "x=sleep 1; echo x-done" "y=sleep 1" "z=sleep 1") || fail "three good lanes failed: $out"
[ "$(took "$t" 2.5)" = True ] || fail "three 1 s lanes took more than 2.5 s"
grep -q '^x-done$' <<<"$out" || fail "a lane's output wasn't printed: $out"
[ "$(grep -c '^check-step: lane-' <<<"$out")" = 3 ] || fail "not a timing line a lane: $out"

t=$(now)
"$here/parallel.sh" "$logs/b" "slow=sleep 30; touch '$logs/slow-ended'" "bad=echo the reason; exit 3" > "$logs/b.out" 2> "$logs/b.err"
rc=$?
[ "$rc" = 3 ] || fail "a failing lane's status is 3, got $rc"
[ "$(took "$t" 4)" = True ] || fail "the failing lane didn't stop the 30 s one"
grep -q 'the reason' "$logs/b.err" || fail "the failed lane's output wasn't printed: $(cat "$logs/b.err")"
grep -q 'check-step: lane-bad .* (failed)' "$logs/b.err" || fail "no failed timing line"
sleep 1
[ ! -e "$logs/slow-ended" ] || fail "the slow lane ran on"
echo "test-parallel: lanes at once, the first failure stops the rest and is printed"
