#!/usr/bin/env -S python3 -I
"""cargo, one build at a time on this machine, with jobs from free memory.

Installed as $GP_HOME/bin/locked/cargo by `just setup`; the justfile,
`gp worker env --shell` and gp's own builds put that folder first, so
every cargo they start comes here (docs/workers.md, "Memory").

A compiling command (build, test, run, check, clippy, doc and the like)
waits for $GP_HOME/work/build.lock, a lock the kernel drops when its
holder exits, so it never goes stale. Once it holds the lock it sets
CARGO_BUILD_JOBS to what free memory allows (GP_BUILD_GB_PER_JOB, 3 GB
by default, a job), never more than CARGO_BUILD_JOBS already said.
`cargo test`, `bench` and `run` compile under the lock (`--no-run`, or
`build`), then let it go and run without it (doc tests, which cargo can't
compile apart, run whole under it), so a long test run doesn't
hold up another build. Cargo started under the lock (a build script, a
test that builds) skips it. Each locked build appends a line to
$GP_HOME/work/build-log.jsonl: the wait, the time, the jobs, the free
memory and the largest child's peak resident size.

When sccache is on PATH and RUSTC_WRAPPER is unset, every compiling
command runs rustc through it, with its cache in $GP_HOME/work/sccache
capped at SCCACHE_CACHE_SIZE (30G by default), so a new worktree's target
folder takes the dependencies other folders compiled. GP_SCCACHE=0 turns
it off; an RUSTC_WRAPPER already set, even empty, is kept.
"""

import fcntl
import json
import os
import resource
import shutil
import subprocess
import sys
import time

COMPILING = {
    "build", "b", "test", "t", "bench", "run", "r", "check", "c", "clippy",
    "doc", "d", "install", "rustc", "rustdoc", "fix",
}
# Global flags that take a value, so the value isn't read as the command.
VALUED = {"--config", "-Z", "-C", "--color", "--explain"}
HELD = "GP_CARGO_LOCK_HELD"


def real_cargo():
    if os.environ.get("GP_REAL_CARGO"):
        return os.environ["GP_REAL_CARGO"]
    me = os.path.realpath(__file__)
    for d in os.environ.get("PATH", "").split(os.pathsep):
        p = os.path.join(d, "cargo")
        if os.access(p, os.X_OK) and os.path.realpath(p) != me:
            return p
    return os.path.expanduser("~/.cargo/bin/cargo")


def command_at(args):
    """The index of cargo's command in `args`, or None."""
    i = 0
    while i < len(args):
        a = args[i]
        if a in VALUED:
            i += 2
            continue
        if a.startswith("-") or a.startswith("+"):
            i += 1
            continue
        return i
    return None


def free_gb():
    """Memory a build can take without the machine swapping, in GB."""
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) / 2**20
    except OSError:
        pass
    try:
        out = subprocess.run(["vm_stat"], capture_output=True, text=True).stdout
    except OSError:
        return 0.0
    page = 16384
    pages = 0
    for line in out.splitlines():
        if "page size of" in line:
            page = int(line.split("page size of")[1].split()[0])
        name, _, value = line.partition(":")
        if name.strip() in ("Pages free", "Pages inactive", "Pages speculative", "Pages purgeable"):
            pages += int(value.strip().rstrip("."))
    return pages * page / 2**30


def home():
    return os.environ.get("GP_HOME") or os.path.expanduser("~/.gameprinter")


def take_lock(argv):
    work = os.path.join(home(), "work")
    os.makedirs(work, exist_ok=True)
    f = open(os.path.join(work, "build.lock"), "a+")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        try:
            who = open(os.path.join(work, "build.lock.who")).read().strip()
        except OSError:
            who = "another build"
        print(f"cargo-locked: waiting for {who}", file=sys.stderr, flush=True)
        fcntl.flock(f, fcntl.LOCK_EX)
    try:
        with open(os.path.join(work, "build.lock.who"), "w") as w:
            w.write(f"pid {os.getpid()} in {os.getcwd()}: cargo {' '.join(argv)[:200]}\n")
    except OSError:
        pass
    return f


def jobs_for(gb):
    per = float(os.environ.get("GP_BUILD_GB_PER_JOB", "3"))
    cap = max(1, int(gb / per))
    asked = os.environ.get("CARGO_BUILD_JOBS", "")
    upper = int(asked) if asked.isdigit() and int(asked) > 0 else (os.cpu_count() or 2)
    return min(cap, upper)


def compile_cache(env):
    """Sets sccache as rustc's wrapper in `env`, when it is installed and
    no wrapper is set. Returns the wrapper used, or None."""
    if "RUSTC_WRAPPER" in env:
        return env["RUSTC_WRAPPER"] or None
    if env.get("GP_SCCACHE") == "0":
        return None
    sccache = shutil.which("sccache", path=env.get("PATH"))
    if not sccache:
        return None
    env["RUSTC_WRAPPER"] = sccache
    env.setdefault("SCCACHE_DIR", os.path.join(home(), "work", "sccache"))
    env.setdefault("SCCACHE_CACHE_SIZE", "30G")
    return sccache


def as_flags(args, at, env):
    """`args` with CARGO_TARGET_DIR and CARGO_BUILD_JOBS moved from `env`
    to cargo's flags. sccache hashes every CARGO_ variable rustc sees, so
    with them in the environment no two worktrees, and no two builds with
    other free memory, would share a cache entry."""
    out = list(args)
    flags = []
    target = env.pop("CARGO_TARGET_DIR", None)
    if target and "--target-dir" not in args and not any(a.startswith("--target-dir=") for a in args):
        flags += ["--target-dir", target]
    jobs = env.pop("CARGO_BUILD_JOBS", None)
    given = any(a in ("-j", "--jobs") or a.startswith("--jobs=") or (a.startswith("-j") and a[2:].isdigit()) for a in args)
    if jobs and not given:
        flags += ["--jobs", jobs]
    out[at + 1 : at + 1] = flags
    return out


def log(entry):
    try:
        with open(os.path.join(home(), "work", "build-log.jsonl"), "a") as f:
            f.write(json.dumps(entry) + "\n")
    except OSError:
        pass


def peak_child_gb():
    r = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    # Bytes on macOS, kilobytes on Linux.
    return round(r / (2**30 if sys.platform == "darwin" else 2**20), 2)


def main():
    cargo = real_cargo()
    args = sys.argv[1:]
    at = command_at(args)
    sub = args[at] if at is not None else None
    if sub not in COMPILING or os.environ.get(HELD) == "1":
        os.execv(cargo, [cargo] + args)

    # What compiles under the lock, and what runs after it.
    compile_args, then = args, None
    # Doc tests can't be compiled apart from running (cargo refuses
    # `--doc --no-run`), so they run whole under the lock.
    cut = args.index("--") if "--" in args else len(args)
    if sub in ("test", "t", "bench") and "--no-run" not in args and "--doc" not in args[:cut]:
        cut = args.index("--") if "--" in args else len(args)
        compile_args, then = args[:cut] + ["--no-run"], args
    elif sub in ("run", "r"):
        cut = args.index("--") if "--" in args else len(args)
        compile_args = args[:at] + ["build"] + args[at + 1 : cut]
        then = args

    asked = time.time()
    lock = take_lock(args)
    started = time.time()
    gb = free_gb()
    jobs = jobs_for(gb)
    env = dict(os.environ, CARGO_BUILD_JOBS=str(jobs), **{HELD: "1"})
    wrapper = compile_cache(env)
    after = dict(env)
    if wrapper and os.path.basename(wrapper).startswith("sccache"):
        compile_args = as_flags(compile_args, at, env)
    if "RUST_TEST_THREADS" not in env:
        env["RUST_TEST_THREADS"] = str(max(1, min((os.cpu_count() or 2) // 2, int(gb / 2))))
    code = subprocess.call([cargo] + compile_args, env=env)
    log({
        "at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "cwd": os.getcwd(),
        "command": " ".join(compile_args)[:300],
        "waited_s": round(started - asked, 1),
        "secs": round(time.time() - started, 1),
        "jobs": jobs,
        "free_gb": round(gb, 1),
        "peak_child_gb": peak_child_gb(),
        "wrapper": os.path.basename(wrapper) if wrapper else None,
        "exit": code,
    })
    lock.close()
    if then is None or code != 0:
        sys.exit(code)
    # The run sees the environment as it was, target folder included.
    after.pop(HELD)
    after["RUST_TEST_THREADS"] = env["RUST_TEST_THREADS"]
    os.execve(cargo, [cargo] + then, after)


if __name__ == "__main__":
    main()
