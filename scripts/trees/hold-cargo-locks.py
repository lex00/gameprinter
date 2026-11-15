#!/usr/bin/env python3
"""hold-cargo-locks.py TARGET CMD...: runs CMD while holding every cargo
lock in the target folder TARGET, so no build writes there while CMD
(tree-new's clone) reads it, and the clone is never of a half-written
target.

Cargo takes flock(2) locks on `.cargo-lock`, `.cargo-build-lock` and
`.cargo-artifact-lock` in each profile folder (target/debug,
target/fastdev, target/<triple>/release, and the same one level further
down for nested targets such as target/page-target). This takes them all
or none: when one is held by a build, it lets go of the rest, says who it
waits for, and tries again every two seconds, so it can't deadlock with a
cargo that holds one lock and wants another. Builds that start while CMD
runs wait ("Blocking waiting for file lock") and go on when it ends.
"""

import fcntl
import os
import subprocess
import sys
import time

LOCK_NAMES = (".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock")
# Folders of compiled files, never holding a profile's locks; not listed.
SKIP = {"deps", "build", ".fingerprint", "incremental", "examples", "doc"}


def lock_files(target):
    """Every cargo lock file up to three folders below TARGET."""
    found = []
    level = [target]
    for _ in range(4):
        below = []
        for d in level:
            try:
                entries = list(os.scandir(d))
            except OSError:
                continue
            for e in entries:
                if e.name in LOCK_NAMES and e.is_file(follow_symlinks=False):
                    found.append(e.path)
                elif e.is_dir(follow_symlinks=False) and e.name not in SKIP:
                    below.append(e.path)
        level = below
    return sorted(found)


def take_all(paths):
    """Locks every path or none; returns the open files, or the busy path."""
    held = []
    for p in paths:
        try:
            fd = os.open(p, os.O_RDWR)
        except OSError:
            continue
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(fd)
            for h in held:
                os.close(h)
            return None, p
        held.append(fd)
    return held, None


def main():
    if len(sys.argv) < 3:
        sys.exit("usage: hold-cargo-locks.py TARGET CMD...")
    target, cmd = sys.argv[1], sys.argv[2:]
    paths = lock_files(target) if os.path.isdir(target) else []
    said = None
    asked = time.monotonic()
    while True:
        held, busy = take_all(paths)
        if held is not None:
            break
        if busy != said:
            print(f"Waiting for the build that holds {busy} to finish.", file=sys.stderr, flush=True)
            said = busy
        time.sleep(2)
    got = time.monotonic()
    if paths:
        print(f"Holding {len(paths)} cargo locks in {target}: builds there wait until {os.path.basename(cmd[0])} ends.", file=sys.stderr, flush=True)
    try:
        rc = subprocess.call(cmd)
    finally:
        for fd in held:
            os.close(fd)
    done = time.monotonic()
    name = os.path.basename(cmd[0])
    print(
        f"Waited {got - asked:.0f} s for builds in {target} to finish; {name} took {done - got:.0f} s, and builds there were held up that long.",
        file=sys.stderr,
        flush=True,
    )
    sys.exit(rc)


if __name__ == "__main__":
    main()
