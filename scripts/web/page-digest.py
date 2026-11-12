#!/usr/bin/env python3
"""One SHA-256 for a built page's folder (docs/web.md, "Deploying the public page").

  page-digest.py DIR

Prints the SHA-256 of the lines "SHA256  PATH", one for every file under
DIR (hidden ones too, no folder left out), sorted by PATH. The release
check (scripts/web/release-check.sh) records it on a pass, and
scripts/web/deploy.sh works it out again for the folder it is about to
upload: a file added, removed, renamed or changed gives another digest.
Exit codes: 0 printed, 2 couldn't read DIR.
"""

import hashlib
import os
import sys


def main():
    if len(sys.argv) != 2 or not os.path.isdir(sys.argv[1]):
        print("usage: scripts/web/page-digest.py DIR", file=sys.stderr)
        return 2
    root = sys.argv[1]
    rows = []
    for d, _, names in os.walk(root):
        for n in names:
            p = os.path.join(d, n)
            h = hashlib.sha256()
            with open(p, "rb") as f:
                for b in iter(lambda: f.read(1 << 20), b""):
                    h.update(b)
            rows.append(f"{h.hexdigest()}  {os.path.relpath(p, root)}\n")
    rows.sort(key=lambda r: r[66:])
    print(hashlib.sha256("".join(rows).encode()).hexdigest())
    return 0


if __name__ == "__main__":
    sys.exit(main())
