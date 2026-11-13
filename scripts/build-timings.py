"""Reads cargo's --timings report (cargo-timing.html) and prints compile times.

Default output, one `name value` line per crate we own or vendor, for
`gp metrics record` (milliseconds, summed over the crate's lib, bin, test
and build-script units):

    build.crate.gp-nes.duration 8123

With --summary, a readable table of the slowest units instead.
Used by `just build-timings`; see docs/build-times.md.
"""

import json
import re
import sys

OURS = re.compile(r"^(gp-|tetanes-core$)")


def units(path):
    html = open(path, encoding="utf-8").read()
    m = re.search(r"const UNIT_DATA = (\[.*?\]);\n", html, re.S)
    if not m:
        sys.exit(f"build-timings: no UNIT_DATA in {path}; has cargo's report format changed?")
    return json.loads(m.group(1))


def main(argv):
    summary = argv[:1] == ["--summary"]
    path = argv[-1]
    data = units(path)
    if summary:
        total = max(u["start"] + u["duration"] for u in data)
        print(f"{len(data)} units, {total:.1f}s wall clock; slowest:")
        for u in sorted(data, key=lambda u: -u["duration"])[:12]:
            target = u["target"].strip() or "lib"
            print(f"  {u['duration']:6.1f}s  {u['name']} {u['version']} {target}")
        return
    per_crate = {}
    for u in data:
        if OURS.match(u["name"]):
            per_crate[u["name"]] = per_crate.get(u["name"], 0.0) + u["duration"]
    for name, seconds in sorted(per_crate.items()):
        print(f"build.crate.{name}.duration {round(seconds * 1000)}")


if __name__ == "__main__":
    main(sys.argv[1:])
