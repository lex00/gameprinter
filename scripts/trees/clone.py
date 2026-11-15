#!/usr/bin/env python3
"""clone.py SOURCE DEST: tree-new's clone. Makes DEST an APFS clone
(`cp -c -R -p`) of the checkout SOURCE and its target folder, leaving out
the target's incremental folders (target/debug/incremental and the like):
rustc's incremental caches, by far the target's largest part.

Why leave them out: the work target's incremental folders hold many old
sessions per crate (720 crates and 1180 sessions, 50 GB, on 2026-11-14).
A clone shares their blocks at first, but each block becomes the tree's
own once the work tree's builds replace or prune theirs, so every tree
grows by the work tree's churn even while its agent is idle. Without
them a tree's build compiles the crates it must recompile (gp-cli always)
without an incremental cache once, and keeps its own small one from then
on; the other crates are fresh by their fingerprints and don't compile.

Every folder made here gets SOURCE's mtime, as `-p` gives the cloned ones:
cargo and the build scripts that watch folders judge freshness by mtimes.
"""

import os
import shutil
import subprocess
import sys

# How deep under target/ an incremental folder may be:
# target/page-target/<triple>/release/incremental is four down.
DEPTH = 4
# Folders of compiled files that never hold a profile's incremental folder.
SKIP = {"deps", "build", ".fingerprint", "examples", "doc"}


def incremental_folders(target):
    """Every folder named incremental up to DEPTH folders below TARGET."""
    found = []
    level = [target]
    for _ in range(DEPTH):
        below = []
        for d in level:
            try:
                entries = list(os.scandir(d))
            except OSError:
                continue
            for e in entries:
                if not e.is_dir(follow_symlinks=False) or e.name in SKIP:
                    continue
                if e.name == "incremental":
                    found.append(e.path)
                else:
                    below.append(e.path)
        level = below
    return found


def clone(src, dst, left_out, above):
    """Clones SRC to DST whole, unless SRC holds a left-out folder: then
    DST is made and each entry cloned in turn. A left-out folder is made
    empty."""
    if src in left_out:
        os.mkdir(dst)
        shutil.copystat(src, dst, follow_symlinks=False)
        return
    if src not in above:
        subprocess.run(["cp", "-c", "-R", "-p", src, dst], check=True)
        return
    os.mkdir(dst)
    for e in sorted(os.scandir(src), key=lambda e: e.name):
        clone(e.path, os.path.join(dst, e.name), left_out, above)
    shutil.copystat(src, dst, follow_symlinks=False)


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: clone.py SOURCE DEST")
    src, dst = (os.path.abspath(p) for p in sys.argv[1:])
    if os.path.exists(dst):
        sys.exit(f"{dst} exists already")
    target = os.path.join(src, "target")
    left_out = set(incremental_folders(target)) if os.path.isdir(target) else set()
    above = set()
    for p in left_out:
        while p != src:
            p = os.path.dirname(p)
            above.add(p)
    clone(src, dst, left_out, above)
    print(
        f"Left out {len(left_out)} incremental folders of {target}: the tree's builds make their own.",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
