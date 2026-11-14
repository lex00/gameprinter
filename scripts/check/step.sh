#!/usr/bin/env bash
# step.sh NAME CMD...: runs CMD, then prints how long it took as
# `check-step: NAME S s`, with ` (failed)` after it when CMD failed, and
# exits with CMD's status. The tool gate reads these lines into its
# steps.jsonl (docs/gate.md, "Timings").
name=$1
shift
start=$(python3 -I -c 'import time; print(time.monotonic())')
"$@"
rc=$?
secs=$(python3 -I -c 'import sys, time; print(f"{time.monotonic() - float(sys.argv[1]):.1f}")' "$start")
if [ "$rc" -eq 0 ]; then
    echo "check-step: $name $secs s"
else
    echo "check-step: $name $secs s (failed)"
fi
exit "$rc"
