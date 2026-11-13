#!/usr/bin/env python3
"""List the repo's version pins and what each could move to.

    python3 -I scripts/upgrades/report.py            # pins, newest releases, blockers
    python3 -I scripts/upgrades/report.py --offline  # tool, image and download pins, and drift between them
    python3 -I scripts/upgrades/report.py --docker   # also check base image digests (needs docker)
    python3 -I scripts/upgrades/report.py --json

Brought from game-printer's scripts/upgrades/report.py (its prototype is
game-printer's docs/research/upgrade-tooling.md), with the pins gameprinter
has: the toolchain, the MAME oracle image (docker/mame/Dockerfile), wasmtime
(the justfile's check-wasm), the inspector's browser and the vendored
tetanes-core. game-printer's toolchain and Mesen images, its disassembly
pins and the Bevy generator aren't here. It only reads files in the
repo and asks public indexes what exists: the Rust release channel, the
crates.io sparse index (which carries each version's rust_version), the npm
registry, nodejs.org and `git ls-remote`. It downloads no package, runs no
build and changes nothing. Requests send a User-Agent naming the project.
Crate and npm rows need the network; --offline lists the other pins.

"Usable" is the newest release the pinned toolchain can build (by the
release's rust_version) or the pinned Node can run (by engines.node, for
the common range forms). "Blocked" says why the newest isn't usable or
isn't a drop-in: msrv, breaking (a new major, or a new minor before 1.0),
exact (an `=` pin, moved only on purpose), range (an npm spec that isn't
an exact version).

Exit codes: 0 the report ran; 1 two files pin the same thing differently;
2 couldn't run (a file is missing or unreadable, cargo metadata failed, a
query failed).
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
UA = "gameprinter-upgrade-report/0.1 (local tool of the Game Printer project)"


def fetch(url, accept=None):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    if accept:
        req.add_header("Accept", accept)
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode()


def ver(s):
    """A sortable key for a dotted version, ignoring pre-release and build parts."""
    return tuple(int(x) for x in re.findall(r"\d+", s.split("+")[0].split("-")[0])[:3])


def is_pre(s):
    return "-" in s.split("+")[0]


def satisfies(version, rng):
    """Whether version meets an npm engines range: || alternatives of >=, >, <=, <, ^, ~, A - B or exact."""
    v = ver(version) + (0, 0)
    rng = re.sub(r"(>=|<=|>|<|=|\^|~)\s+", r"\1", rng)
    rng = re.sub(r"(\S+)\s+-\s+(\S+)", r">=\1 <=\2", rng)
    for alt in rng.split("||"):
        ok = True
        for c in alt.split():
            op, want = re.match(r"(>=|<=|>|<|\^|~|=)?v?(.*)", c).groups()
            if want in ("*", "x", ""):
                continue
            w = ver(want) + (0, 0)
            n = len(re.findall(r"\d+", want))
            ok &= {">=": v >= w, ">": v > w, "<=": v[:n] <= w[:n], "<": v < w,
                   "^": v >= w and (v[0] == w[0] if w[0] else v[:2] == w[:2]),
                   "~": v >= w and v[:2] == w[:2]}.get(op, v[:n] == w[:n])
        if ok:
            return True
    return False


def semver_breaking(old, new):
    """Cargo's rule: the first non-zero component changed."""
    o, n = ver(old) + (0, 0), ver(new) + (0, 0)
    for a, b in zip(o, n):
        if a or b:
            return a != b
    return False


def sparse_path(name):
    n = name.lower()
    if len(n) <= 2:
        return f"{len(n)}/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[:2]}/{n[2:4]}/{n}"


def crate_versions(name):
    """(version, rust_version) for every non-yanked stable release, oldest first."""
    rows = []
    for line in fetch(f"https://index.crates.io/{sparse_path(name)}").splitlines():
        e = json.loads(line)
        if not e["yanked"] and not is_pre(e["vers"]):
            rows.append((e["vers"], e.get("rust_version")))
    return sorted(rows, key=lambda r: ver(r[0]))


def builds_with(row, toolchain):
    return row[1] is None or ver(row[1]) <= ver(toolchain)


def git_head(url):
    out = subprocess.run(["git", "ls-remote", url, "HEAD"], capture_output=True, text=True, timeout=60,
                         env={**os.environ, "GIT_TERMINAL_PROMPT": "0"})
    return out.stdout.split()[0] if out.returncode == 0 and out.stdout else None


def grab(path, pattern):
    m = re.search(pattern, (REPO / path).read_text(), re.M)
    if not m:
        raise LookupError(f"{path}: no match for {pattern}")
    return m.group(1)


def pins():
    """The tool, image and download pins, read from their files. Each is (kind, name, file, value)."""
    A = "docker/mame/Dockerfile"
    tc = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"]
    return [
        ("toolchain", "rust", "rust-toolchain.toml", tc["channel"]),
        ("toolchain", "rust targets", "rust-toolchain.toml", " ".join(sorted(tc.get("targets", [])))),
        ("toolchain", "rust (rust-version)", "Cargo.toml",
         tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]),
        ("docker", "debian image (mame)", A, grab(A, r"^ARG BASE_IMAGE=(\S+)")),
        ("docker", "debian snapshot (mame image)", A, grab(A, r"^ARG DEBIAN_SNAPSHOT=(\S+)")),
        ("tool", "mame (apt)", A, grab(A, r"^ARG MAME_DEB_VERSION=(\S+)")),
        ("tool", "wasmtime-cli", "justfile", grab("justfile", r'^wasmtime_version := "([^"]+)"')),
        ("tool", "chrome-headless-shell", "scripts/inspector/fetch-browser.sh",
         grab("scripts/inspector/fetch-browser.sh", r'^version="([^"]+)"')),
        ("vendored", "tetanes-core", "scripts/vendor/tetanes-core.sh",
         grab("scripts/vendor/tetanes-core.sh", r'^VERSION="([^"]+)"')),
    ]


def drift(p):
    """Pins of one thing in two files that disagree."""
    v = {name: value for _, name, _, value in p}
    tc = v["rust"]
    problems = []
    if ver(v["rust (rust-version)"]) != ver(tc)[:len(ver(v["rust (rust-version)"]))]:
        problems.append(f"Cargo.toml rust-version {v['rust (rust-version)']} isn't the toolchain {tc}")
    return problems


def crate_row(name, file, cur, toolchain, exact=False, note=""):
    vs = crate_versions(name)
    new = vs[-1]
    usable = [r for r in vs if builds_with(r, toolchain)]
    blocked = []
    if not builds_with(new, toolchain):
        blocked.append(f"msrv {new[1]}")
    if cur[0].isdigit() and semver_breaking(cur, new[0]):
        blocked.append("breaking")
    if exact:
        blocked.append("exact")
    if note:
        blocked.append(note)
    return ("crate", name, file, cur, new[0], usable[-1][0] if usable else "-", ", ".join(blocked)), vs


def cargo_rows(toolchain):
    """Direct registry dependencies of the workspace members and of the vendored tetanes-core."""
    meta = subprocess.run(["cargo", "metadata", "--offline", "--locked", "--format-version", "1"],
                          cwd=REPO, capture_output=True, text=True)
    if meta.returncode != 0:
        raise RuntimeError("cargo metadata failed:\n" + meta.stderr)
    m = json.loads(meta.stdout)
    pkgs = {pkg["id"]: pkg for pkg in m["packages"]}
    nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
    roots = set(m["workspace_members"]) | {i for i, pkg in pkgs.items() if pkg["name"] == "tetanes-core"}
    deps = {}
    for root in roots:
        reqs = {d["rename"] or d["name"]: d["req"] for d in pkgs[root]["dependencies"]}
        for d in nodes[root]["deps"]:
            pkg = pkgs[d["pkg"]]
            if (pkg.get("source") or "").startswith("registry+"):
                exact = reqs.get(d["name"], "").startswith("=")
                where = "third_party" if pkgs[root]["name"] == "tetanes-core" else "Cargo.lock"
                old = deps.get(pkg["name"])
                deps[pkg["name"]] = (pkg["version"], exact or (old and old[1]), where if not old else old[2])
    return [crate_row(name, where, cur, toolchain, exact)[0] for name, (cur, exact, where) in sorted(deps.items())]


def npm_rows(folder, node):
    pkg = json.loads((REPO / folder / "package.json").read_text())
    lock = json.loads((REPO / folder / "package-lock.json").read_text())["packages"]
    rows = []
    deps = {**pkg.get("dependencies", {}), **pkg.get("devDependencies", {})}
    for name, spec in sorted(deps.items()):
        cur = lock.get(f"node_modules/{name}", {}).get("version", "?")
        meta = json.loads(fetch(f"https://registry.npmjs.org/{name}", "application/vnd.npm.install-v1+json"))
        new = meta["dist-tags"]["latest"]
        engines = meta["versions"][new].get("engines", {}).get("node", "")
        ok = not engines or satisfies(node, engines)
        blocked = []
        if cur != "?" and semver_breaking(cur, new):
            blocked.append("breaking")
        if not re.fullmatch(r"\d[\w.\-]*", spec):
            blocked.append(f"range {spec}")
        if not ok:
            blocked.append(f"node {engines}")
        rows.append(("npm", name, f"{folder}/package-lock.json", cur, new, new if ok else "-", ", ".join(blocked)))
    return rows


def docker_rows():
    """Whether each base image's tag now points at a newer digest than the pinned one (same tag, rebuilt)."""
    rows = []
    for file, comment, arg in (("docker/mame/Dockerfile", r"^# (debian:\S+),", "BASE_IMAGE"),):
        tag = grab(file, comment)
        pinned = grab(file, rf"^ARG {arg}=\S+@(sha256:[0-9a-f]+)")
        out = subprocess.run(["docker", "buildx", "imagetools", "inspect", tag],
                             capture_output=True, text=True, timeout=120)
        m = re.search(r"^Digest:\s+(sha256:[0-9a-f]+)", out.stdout, re.M) if out.returncode == 0 else None
        if not m:
            note, now = "couldn't inspect the tag", "?"
        else:
            now = m.group(1)
            note = "" if now == pinned else "tag rebuilt (OS updates); re-pin and re-run the Docker checks"
        rows.append(("docker", tag, file, pinned[7:19], now[7:19], "-", note))
    return rows


def online_rows(p, docker):
    v = {name: value for _, name, _, value in p}
    tc = v["rust"]
    rows = []
    chan = fetch("https://static.rust-lang.org/dist/channel-rust-stable.toml")
    stable = re.search(r'\[pkg\.rust\]\nversion = "(\S+)', chan)
    if not stable:
        raise LookupError("the stable channel manifest has no [pkg.rust] version")
    rows.append(("toolchain", "rust", "rust-toolchain.toml", tc, stable.group(1), stable.group(1), ""))

    row, vs = crate_row("wasmtime-cli", "justfile", v["wasmtime-cli"], tc)
    lts = [r for r in vs if ver(r[0])[0] >= 24 and ver(r[0])[0] % 12 == 0]
    if lts:
        usable = "builds" if builds_with(lts[-1], tc) else f"needs rust {lts[-1][1]}"
        row = row[:6] + (", ".join(filter(None, [row[6], f"newest LTS {lts[-1][0]} ({usable})"])),)
    rows.append(row)
    rows.append(crate_row("tetanes-core", "scripts/vendor/tetanes-core.sh", v["tetanes-core"], tc,
                          note="re-import; the runtime identity changes")[0])
    if docker:
        rows += docker_rows()
    rows += cargo_rows(tc)
    node_cur = subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip().lstrip("v") or "0"
    for folder in ("editor", "site"):
        if (REPO / folder / "package-lock.json").exists():
            rows += npm_rows(folder, node_cur)
    return rows


def table(header, rows):
    widths = [max(len(str(r[i])) for r in [header] + rows) for i in range(len(header))]
    line = lambda r: "| " + " | ".join(str(c).ljust(w) for c, w in zip(r, widths)) + " |"
    print(line(header))
    print("|" + "|".join("-" * (w + 2) for w in widths) + "|")
    for r in rows:
        print(line(r))


def main(argv):
    ap = argparse.ArgumentParser(description="List the repo's version pins and what each could move to.")
    ap.add_argument("--offline", action="store_true", help="no network: tool, image and download pins, and drift")
    ap.add_argument("--docker", action="store_true", help="also check base image digests (needs docker)")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)
    if args.offline and args.docker:
        ap.error("--docker needs the network, so it can't go with --offline")
    try:
        p = pins()
        problems = drift(p)
        rows = [] if args.offline else online_rows(p, args.docker)
    except Exception as e:  # anything unexpected means the report couldn't run, never drift
        print(f"upgrades: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps({"pins": p, "drift": problems, "rows": rows}, indent=1))
    else:
        table(("kind", "name", "file", "pinned"), [list(x) for x in p])
        if rows:
            print()
            table(("kind", "name", "file", "pinned", "newest", "usable", "blocked"), [list(r) for r in rows])
        for d in problems:
            print(f"upgrades: drift: {d}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
