#!/bin/bash
# The web build against the native interpreter on a list of inputs
# (docs/web.md, "Parity"):
#   scripts/web/parity.sh OUT MODEL ASSETS CLIPS
#
# OUT is a build of scripts/web/build-preview.sh for MODEL and ASSETS.
# CLIPS is a TOML file of [[clip]] tables (scripts/web/parity/), each with
# a `name` and a `baseline` (a baseline file, relative to the repo). For
# each, gp-web-prep (crates/gp-web, built for this machine) plays the
# baseline's input in the interpreter and writes the frames it played and
# the state hash at power-on and after every frame into OUT/parity/; then
# scripts/web/parity.mjs plays the same frames in OUT's gp_web.wasm under
# Node and compares every frame's hash, the spec digest and the hooks'
# label.
#
# A clip with `holds`, `recording` or `level` is refused: gp has no
# `input-log generate`, no recordings store and no `interp run --level`
# now, which those needed (docs/web.md, "Parity").
#
# Exit codes: 0 every frame agreed, 1 something differs, 2 couldn't run.

set -euo pipefail
trap 'exit 2' ERR

if [ $# -ne 4 ]; then
  echo "usage: scripts/web/parity.sh OUT MODEL ASSETS CLIPS" >&2
  exit 2
fi
out="$1" model="$2" assets="$3" clips="$4"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
target="$(cd "$repo" && mkdir -p "${CARGO_TARGET_DIR:-target}" && cd "${CARGO_TARGET_DIR:-target}" && pwd)"
runs="$out/parity"
mkdir -p "$runs"

# One line per clip: name and baseline, split by `|`.
python3 -I - "$clips" > "$runs/clips.txt" <<'PY'
import sys, tomllib
for c in tomllib.load(open(sys.argv[1], "rb")).get("clip", []):
    other = [k for k in ("holds", "recording", "level") if k in c]
    if other or "baseline" not in c:
        sys.exit(f"parity: clip {c.get('name')}: only a `baseline` clip can run now (it has {', '.join(other) or 'no baseline'})")
    print("|".join([c["name"], c["baseline"]]))
PY

names=() baselines=()
while IFS='|' read -r name src; do
  names+=("$name")
  baselines+=("$repo/$src")
done < "$runs/clips.txt"
[ ${#names[@]} -gt 0 ] || { echo "parity: no clip ran" >&2; exit 2; }

# The native run: gp-web-prep plays each baseline in the interpreter built
# for this machine and writes the spec digest in preview.json. It doesn't
# write the hooks' label, so the native label is taken as empty: right for
# Moss Hop, which crates/gp-exts has no hooks for (its own test says so).
# A model with hooks needs its label written here before it can run.
(cd "$repo" && cargo build -q --locked -p gp-web --bin gp-web-prep --target-dir "$target")
prep="$runs/native"
mkdir -p "$prep"
"$target/debug/gp-web-prep" "$model" "$assets" "$prep" "${baselines[@]}" >/dev/null
for i in "${!names[@]}"; do
  name="${names[$i]}"
  stem="$(basename "${baselines[$i]}" .toml)"
  cp "$prep/runs/$stem.frames" "$runs/$name.frames"
  cp "$prep/runs/$stem.hashes" "$runs/$name.hashes"
  python3 -I -c 'import json, sys; p = json.load(open(sys.argv[1])); json.dump({"spec_digest": p["spec_digest"], "ext_hooks": ""}, open(sys.argv[2], "w"))' \
    "$prep/preview.json" "$runs/$name.json"
  : > "$runs/$name.level"
done
# Only this run's clips are compared, whatever an earlier run left in DIR.
printf '%s\n' "${names[@]}" > "$runs/index.txt"

status=0
node "$repo/scripts/web/parity.mjs" --dist "$out" --runs "$runs" | tee "$runs/result.txt" || status=$?
exit "$status"
