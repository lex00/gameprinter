#!/usr/bin/env python3
"""Reuse for the public page's release check (docs/web.md, "Reuse").

A product (a wasm module, gp-web-prep's output, a check's result) is kept
with the inputs it was made from and their SHA-256s. It is reused when
every one of those inputs still has the same contents; nothing is keyed by
a commit, a time or a file's date.

  reuse.py find KIND --extra TEXT
      Prints the folder of a kept product of KIND whose inputs all hash as
      they did and whose TEXT is the same, and exits 0; exits 1 if none.
  reuse.py save KIND --extra TEXT [--depinfo FILE]... [--file PATH]...
                [--tree DIR]... [--listing DIR]... --out PATH... [--since T]
      Keeps the PATHs (files or folders) as a product of KIND, with these
      inputs: the files a cargo dep-info FILE names, each PATH, every file
      under each DIR (and the names in it, so a new file counts), and the
      names, sizes and dates under each listing DIR (for $GP_ROM_DIR, whose
      files are found by a tool, not read by name). With --since (a time
      in seconds), only the files under an out folder written since then
      are kept: what a tool wrote into a folder that held files before.
  reuse.py stamp STAMP --extra TEXT [inputs as for save]
      Writes the inputs' SHA-256s to the file STAMP, after a build.
  reuse.py fresh STAMP --extra TEXT
      Exits 0 if every input STAMP names still hashes as it did and TEXT
      is the same, 1 if not (the justfile's build of gp for the release
      check, so a run with nothing changed doesn't wait for the machine's
      build lock).
  reuse.py digest [--file PATH]... [--tree DIR]...
      Prints one SHA-256 over the contents of those files and of every
      file under each DIR by its path in DIR, whatever the DIR's own path
      (for a folder whose place changes, such as the bundle).

The store is $GP_PAGE_REUSE, default $GP_HOME/work/page-release/reuse. A
product's folder there is KIND/KEY/ with inputs.json and out/. Any of it
can be deleted at any time; the next run makes it again. Paths are kept
relative to the repo, the cargo target folder or $HOME where they are in
one, so a store shared by worktrees checks each worktree's own files.

A folder a dep-info names (gp-exts's build script watches each template
folder to notice a new hook crate, docs/web.md "Reuse") counts by the names
of the files in it, not their contents: the files the build reads are
named one by one beside it. Files under ~/.cargo/registry and ~/.rustup
are named with their version and never change, so they count by name;
Cargo.lock is an input of every cargo product.
"""

import hashlib
import json
import os
import shutil
import sys
import time

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HOME = os.path.expanduser("~")
GP_HOME = os.environ.get("GP_HOME", os.path.join(HOME, ".game-printer"))
STORE = os.environ.get("GP_PAGE_REUSE", os.path.join(GP_HOME, "work", "page-release", "reuse"))
TARGET = os.path.abspath(os.environ.get("CARGO_TARGET_DIR", os.path.join(REPO, "target")))
PINNED = [os.path.join(HOME, ".cargo", "registry"), os.path.join(HOME, ".rustup")]
SKIP = {".git", "node_modules", "target"}
# The newest products of a kind looked through by find.
KEEP = 40


def short(path):
    """A path as kept: repo:, target: or ~: relative where it can be."""
    p = os.path.normpath(os.path.abspath(path))
    for tag, root in (("target:", TARGET), ("repo:", REPO), ("~:", HOME)):
        if p == root or p.startswith(root + os.sep):
            return tag + os.path.relpath(p, root)
    return p


def full(kept):
    for tag, root in (("target:", TARGET), ("repo:", REPO), ("~:", HOME)):
        if kept.startswith(tag):
            return os.path.normpath(os.path.join(root, kept[len(tag):]))
    return kept


def walk(d):
    for root, dirs, names in os.walk(d):
        dirs[:] = sorted(x for x in dirs if x not in SKIP)
        for n in sorted(names):
            yield os.path.join(root, n)


def sha_file(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def digest(kept):
    """The digest of one kept input: `file:`, `names:` or `listing:` and the path."""
    how, _, path = kept.partition(" ")
    p = full(path)
    if how == "pinned":
        return "pinned"
    if how == "names":
        if not os.path.isdir(p):
            return "missing"
        return hashlib.sha256("\n".join(os.path.relpath(f, p) for f in walk(p)).encode()).hexdigest()
    if how == "listing":
        if not os.path.isdir(p):
            return "missing"
        rows = []
        for f in walk(p):
            st = os.stat(f)
            rows.append(f"{os.path.relpath(f, p)} {st.st_size} {st.st_mtime_ns}")
        return hashlib.sha256("\n".join(rows).encode()).hexdigest()
    try:
        return sha_file(p)
    except FileNotFoundError:
        return "missing"


def depinfo(path):
    """The inputs a cargo dep-info file names (the part after `TARGET: `)."""
    text = open(path).read()
    first = text.split("\n", 1)[0]
    at = first.find(": ")
    deps, cur, i = [], "", at + 2
    while i < len(first):
        c = first[i]
        if c == "\\" and i + 1 < len(first):
            cur += first[i + 1]
            i += 2
            continue
        if c == " ":
            if cur:
                deps.append(cur)
            cur = ""
        else:
            cur += c
        i += 1
    if cur:
        deps.append(cur)
    out = []
    for d in deps:
        d = os.path.normpath(d)
        if any(d.startswith(r + os.sep) for r in PINNED):
            out.append("pinned " + short(d))
        elif os.path.isdir(d):
            out.append("names " + short(d))
        else:
            out.append("file " + short(d))
    out.append("file " + short(os.path.join(REPO, "Cargo.lock")))
    return out


def inputs_of(args):
    out = []
    for f in args.get("depinfo", []):
        out += depinfo(f)
    for f in args.get("file", []):
        out.append("file " + short(f))
    for d in args.get("tree", []):
        out.append("names " + short(d))
        out += ["file " + short(f) for f in walk(d)]
    for d in args.get("listing", []):
        out.append("listing " + short(d))
    return sorted(set(out))


def key_of(extra, digests):
    h = hashlib.sha256(extra.encode())
    for k in sorted(digests):
        h.update(f"\n{k}\t{digests[k]}".encode())
    return h.hexdigest()[:32]


def find(kind, extra):
    d = os.path.join(STORE, kind)
    try:
        entries = sorted((e for e in os.scandir(d) if not e.name.endswith(".part")), key=lambda e: e.stat().st_mtime, reverse=True)[:KEEP]
    except FileNotFoundError:
        return None
    seen = {}
    for e in entries:
        try:
            meta = json.load(open(os.path.join(e.path, "inputs.json")))
        except (OSError, ValueError):
            continue
        if meta.get("extra") != extra:
            continue
        now = {}
        for k, was in meta["inputs"].items():
            if k not in seen:
                seen[k] = digest(k)
            now[k] = seen[k]
            if now[k] != was:
                break
        else:
            os.utime(e.path)
            return e.path
    return None


def copy_out(src, dst, since):
    if not os.path.isdir(src):
        shutil.copyfile(src, dst)
        return
    for f in walk(src):
        if since is not None and os.stat(f).st_mtime < since:
            continue
        to = os.path.join(dst, os.path.relpath(f, src))
        os.makedirs(os.path.dirname(to), exist_ok=True)
        shutil.copyfile(f, to)


def save(kind, extra, inputs, outs, since=None):
    digests = {k: digest(k) for k in inputs}
    key = key_of(extra, digests)
    final = os.path.join(STORE, kind, key)
    if os.path.isdir(final):
        # The same product, kept already (by another run).
        os.utime(final)
        return final
    work = f"{final}.{os.getpid()}.part"
    os.makedirs(os.path.join(work, "out"), exist_ok=True)
    for o in outs:
        copy_out(o, os.path.join(work, "out", os.path.basename(os.path.normpath(o))), since)
    meta = {"kind": kind, "extra": extra, "made": time.strftime("%Y-%m-%dT%H:%M:%S"), "repo": REPO, "inputs": digests}
    with open(os.path.join(work, "inputs.json"), "w") as f:
        json.dump(meta, f, indent=0, sort_keys=True)
    try:
        os.rename(work, final)
    except OSError:
        # Another run kept the same product while this one copied; its
        # copy is used and this .part folder is left for a person to delete.
        pass
    return final


def parse(argv):
    args, i = {}, 0
    while i < len(argv):
        a = argv[i]
        if not a.startswith("--") or i + 1 >= len(argv):
            sys.exit(f"reuse.py: {a}?")
        args.setdefault(a[2:], []).append(argv[i + 1])
        i += 2
    return args


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    cmd = sys.argv[1]
    if cmd == "digest":
        args = parse(sys.argv[2:])
        h = hashlib.sha256()
        for i, f in enumerate(args.get("file", [])):
            h.update(f"file {i} {digest('file ' + short(f))}\n".encode())
        for i, d in enumerate(args.get("tree", [])):
            for f in walk(d):
                h.update(f"tree {i} {os.path.relpath(f, d)} {sha_file(f)}\n".encode())
        print(h.hexdigest())
        return 0
    if cmd in ("stamp", "fresh"):
        stamp = sys.argv[2]
        args = parse(sys.argv[3:])
        extra = "\n".join(args.get("extra", []))
        if cmd == "stamp":
            meta = {"extra": extra, "inputs": {k: digest(k) for k in inputs_of(args)}}
            with open(stamp, "w") as f:
                json.dump(meta, f, indent=0, sort_keys=True)
            return 0
        try:
            meta = json.load(open(stamp))
        except (OSError, ValueError):
            return 1
        ok = meta.get("extra") == extra and all(digest(k) == v for k, v in meta["inputs"].items())
        return 0 if ok else 1
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    kind = sys.argv[2]
    args = parse(sys.argv[3:])
    extra = "\n".join(args.get("extra", []))
    if cmd == "find":
        hit = find(kind, extra)
        if hit:
            print(hit)
            return 0
        return 1
    if cmd == "save":
        since = float(args["since"][0]) if "since" in args else None
        print(save(kind, extra, inputs_of(args), args.get("out", []), since))
        return 0
    sys.exit(__doc__)


if __name__ == "__main__":
    sys.exit(main())
