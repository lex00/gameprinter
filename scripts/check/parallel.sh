#!/usr/bin/env bash
# parallel.sh LOGDIR NAME=CMD ...: runs every CMD at once, each in a
# process group of its own with its output in LOGDIR/NAME.log. When a CMD
# ends, its log is printed whole (never mixed with another's) with a line
# `check-step: lane-NAME S s`. At the first CMD that fails, the others are
# stopped (TERM to their groups, KILL a second later), the failed CMD's log
# is printed to stderr, and this exits with its status. Exits 0 only when
# every CMD exited 0. A TERM, INT or HUP to this script stops them all too,
# since their groups aren't its own.
set -uo pipefail
set -m
logs=$1
shift
mkdir -p "$logs"
names=() pids=() starts=() running=()
now() { python3 -I -c 'import time; print(time.monotonic())'; }
since() { python3 -I -c 'import sys, time; print(f"{time.monotonic() - float(sys.argv[1]):.1f}")' "$1"; }

stop_all() {
    local i
    for i in "${!pids[@]}"; do
        [ "${running[$i]}" = 1 ] && kill -TERM -- "-${pids[$i]}" 2>/dev/null
    done
    sleep 1
    for i in "${!pids[@]}"; do
        if [ "${running[$i]}" = 1 ]; then
            kill -KILL -- "-${pids[$i]}" 2>/dev/null
            wait "${pids[$i]}" 2>/dev/null
            running[$i]=0
        fi
    done
}
trap 'stop_all; exit 143' TERM INT HUP

for arg; do
    name=${arg%%=*}
    cmd=${arg#*=}
    : > "$logs/$name.log"
    starts+=("$(now)")
    bash -c "$cmd" > "$logs/$name.log" 2>&1 &
    pids+=($!)
    names+=("$name")
    running+=(1)
done

left=${#pids[@]}
while [ "$left" -gt 0 ]; do
    for i in "${!pids[@]}"; do
        [ "${running[$i]}" = 1 ] || continue
        kill -0 "${pids[$i]}" 2>/dev/null && continue
        wait "${pids[$i]}"
        rc=$?
        running[$i]=0
        left=$((left - 1))
        secs=$(since "${starts[$i]}")
        if [ "$rc" -ne 0 ]; then
            stop_all
            {
                echo "parallel: ${names[$i]} failed (exit $rc), the other lanes stopped; its output:"
                cat "$logs/${names[$i]}.log"
                echo "check-step: lane-${names[$i]} $secs s (failed)"
            } >&2
            exit "$rc"
        fi
        cat "$logs/${names[$i]}.log"
        echo "check-step: lane-${names[$i]} $secs s"
    done
    [ "$left" -gt 0 ] && sleep 0.2
done
exit 0
