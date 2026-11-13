#!/usr/bin/env python3
"""Prints each criterion benchmark's mean as a metric line, `name value`,
for `just bench` to record with `gp metrics record` (docs/benchmarks.md).

Reads <dir>/<id>/new/benchmark.json for the benchmark's ID (such as
nes/step) and <dir>/<id>/new/estimates.json for its mean in nanoseconds,
and names the metric bench.criterion.<id with / as .>.ns. Standard library
only. With --since FILE, only results written after FILE was, so a
benchmark that was renamed or removed isn't recorded from old results.
Exits 2 if no results qualify, so a renamed layout fails loudly instead of
recording nothing.
"""

import json
import pathlib
import sys


def metrics(root, since=None):
    out = []
    for new in sorted(root.glob("*/new")):
        if since is not None and (new / "estimates.json").stat().st_mtime < since:
            continue
        info = json.loads((new / "benchmark.json").read_text())
        mean = json.loads((new / "estimates.json").read_text())["mean"]["point_estimate"]
        name = "bench.criterion." + info["full_id"].replace("/", ".") + ".ns"
        out.append((name, mean))
    return out


def main():
    args = sys.argv[1:]
    since = None
    if len(args) == 3 and args[0] == "--since":
        since = pathlib.Path(args[1]).stat().st_mtime
        args = args[2:]
    if len(args) != 1:
        print("usage: criterion-metrics.py [--since FILE] <target/criterion>", file=sys.stderr)
        return 2
    found = metrics(pathlib.Path(args[0]), since)
    if not found:
        print(f"criterion-metrics: no results under {args[0]}", file=sys.stderr)
        return 2
    for name, value in found:
        print(f"{name} {value:.1f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
