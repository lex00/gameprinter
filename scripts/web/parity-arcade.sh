#!/bin/bash
# An arcade game's model in the page's player against the native run, frame
# by frame (docs/web.md, "Parity"):
#   scripts/web/parity-arcade.sh OUT GAME_DIR ASSETS MODULE ROM CLIP...
#
# OUT is a build of scripts/web/build-preview.sh for GAME_DIR's model and
# ASSETS (its bundle.json). MODULE is the game's module (`gp game build`),
# the bytes the page runs the game in. ROM is the game's romset zip. Each
# CLIP is a clip of the game's corpus by its ID: `gp clip log` writes its
# input from power-on, `gp interp translate` turns the original's state at
# the clip's start into the model's (the start the mechanic page's PLAY
# THE REBUILD and REVIEW REBUILD use), and from there `gp interp run` plays
# the clip's own input natively through the module and writes the state
# hash at the start and after every frame. scripts/web/parity.mjs then
# plays the same frames from the same state in MODULE under Node, through
# the page's exports (gp_load, gp_load_state, gp_run_frames), and every
# hash, the spec digest and the hooks' label must be the native run's.
#
# GP is the gp to run (default: gp on PATH). Exit codes: 0 every frame
# agreed, 1 something differs, 2 couldn't run.

set -euo pipefail
trap 'exit 2' ERR

if [ $# -lt 6 ]; then
  echo "usage: scripts/web/parity-arcade.sh OUT GAME_DIR ASSETS MODULE ROM CLIP..." >&2
  exit 2
fi
out="$1" game="$2" assets="$3" module="$4" rom="$5"
shift 5
repo="$(cd "$(dirname "$0")/../.." && pwd)"
gp="${GP:-gp}"
runs="$out/parity-arcade"
mkdir -p "$runs"
: > "$runs/index.txt"

for clip in "$@"; do
  log="$runs/$clip.log"
  # "... the clip starts at frame N"
  at="$("$gp" clip log "$game" "$clip" --rom "$rom" --out "$log" | sed -n 's/.*the clip starts at frame \([0-9]*\).*/\1/p')"
  [ -n "$at" ] || { echo "parity-arcade: gp clip log said no start frame for $clip" >&2; exit 2; }
  "$gp" interp translate "$game" --rom "$rom" --input "$log" --frame "$at" \
    --module "$module" --assets "$assets" --out "$runs/$clip.state" > /dev/null
  # The clip's own frames: the log less its first `at` frames.
  python3 -I - "$log" "$at" "$runs/$clip.rest.log" <<'PY'
import sys
src, at, dst = sys.argv[1], int(sys.argv[2]), sys.argv[3]
head, rows = [], []
for line in open(src):
    p = line.split()
    if p and p[0].isdigit():
        rows.append((int(p[0]), p[1]))
    elif not rows:
        head.append(line)
out, skip = [], at
for n, s in rows:
    take = min(skip, n)
    skip -= take
    if n - take:
        out.append(f"{n - take} {s}\n")
if skip:
    sys.exit(f"parity-arcade: {src} has fewer than {at} frames")
open(dst, "w").write("".join(head + out))
PY
  "$gp" interp run "$game" --json --module "$module" --assets "$assets" \
    --state "$runs/$clip.state" --input "$runs/$clip.rest.log" \
    --hashes "$runs/$clip.hashes" --frames "$runs/$clip.frames" > "$runs/$clip.native.json"
  python3 -I -c 'import json, sys; p = json.load(open(sys.argv[1])); json.dump({"spec_digest": p["spec_digest"], "ext_hooks": p["ext_hooks"]}, open(sys.argv[2], "w"))' \
    "$runs/$clip.native.json" "$runs/$clip.json"
  : > "$runs/$clip.level"
  echo "$clip" >> "$runs/index.txt"
done

status=0
node "$repo/scripts/web/parity.mjs" --dist "$out" --runs "$runs" --wasm "$module" | tee "$runs/result.txt" || status=$?
exit "$status"
