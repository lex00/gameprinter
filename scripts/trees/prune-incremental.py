#!/usr/bin/env python3
"""prune-incremental.py [--dry-run] TARGET AWAY: moves the stale sessions
of TARGET's incremental folders (target/debug/incremental and the like)
into the folder AWAY, keeping the newest session of each crate. Deletes
nothing: AWAY is on the same volume (a rename), and deleting it is the
owner's or the lead's one command. With --dry-run, only says what it
would move.

rustc keeps an incremental cache per crate as a folder
(incremental/NAME-HASH/) of sessions (s-TIME-RANDOM-SVH/, each with a
s-TIME-RANDOM.lock beside it). A build reads only the crate's newest
finished session and starts a new one from it; the older ones are dead
weight it was meant to delete. The work target held 459 of them on
2026-11-14, 21.5 GB in target/debug alone.

Kept: the newest finished session of each crate folder, every session
still being written (its name ends in -working), and every session whose
lock file a running rustc holds. Run it under hold-cargo-locks.py, so no
build starts in TARGET meanwhile.
"""

import fcntl
import os
import sys

DEPTH = 4
SKIP = {"deps", "build", ".fingerprint", "examples", "doc"}


def incremental_folders(target):
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
    return sorted(found)


def lock_of(session):
    """s-TIME-RANDOM-SVH's lock file, s-TIME-RANDOM.lock."""
    parts = os.path.basename(session).split("-")
    return os.path.join(os.path.dirname(session), "-".join(parts[:3]) + ".lock")


def held(lock):
    """Whether a running process holds LOCK."""
    try:
        fd = os.open(lock, os.O_RDWR)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return True
    finally:
        os.close(fd)
    return False


def size(path):
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(root, f)).st_blocks * 512
            except OSError:
                pass
    return total


def stale(crate):
    """The crate folder's sessions to move: every finished one but the
    newest, none being written or locked."""
    sessions = []
    for e in os.scandir(crate):
        if e.is_dir(follow_symlinks=False) and e.name.startswith("s-"):
            sessions.append((e.stat(follow_symlinks=False).st_mtime, e.name, e.path))
    finished = sorted(s for s in sessions if not s[1].endswith("-working"))
    out = []
    for _, _, path in finished[:-1]:
        if not held(lock_of(path)):
            out.append(path)
    return out


def main():
    args = sys.argv[1:]
    dry = "--dry-run" in args
    args = [a for a in args if a != "--dry-run"]
    if len(args) != 2:
        sys.exit("usage: prune-incremental.py [--dry-run] TARGET AWAY")
    target, away = (os.path.abspath(a) for a in args)
    if not os.path.isdir(target):
        sys.exit(f"{target} isn't a folder")
    moved = 0
    total = 0
    crates = 0
    for inc in incremental_folders(target):
        for c in sorted(os.scandir(inc), key=lambda e: e.name):
            if not c.is_dir(follow_symlinks=False):
                continue
            crates += 1
            for s in stale(c.path):
                total += size(s)
                moved += 1
                if dry:
                    continue
                rel = os.path.relpath(s, target)
                dest = os.path.join(away, rel)
                os.makedirs(os.path.dirname(dest), exist_ok=True)
                os.rename(s, dest)
                lock = lock_of(s)
                if os.path.exists(lock):
                    os.rename(lock, os.path.join(away, os.path.relpath(lock, target)))
    verb = "Would move" if dry else "Moved"
    print(
        f"{verb} {moved} stale incremental sessions ({total / 1e9:.1f} GB as du counts them) of {crates} crates in {target}"
        + ("" if dry else f" to {away}")
        + "; kept the newest of each crate, and any being written or locked."
    )


if __name__ == "__main__":
    main()
