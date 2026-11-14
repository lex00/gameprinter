#!/bin/bash
# Builds the interpreter preview of a model for the browser, the fast path
# of the playable web build (docs/web.md):
#   scripts/web/build-preview.sh OUT [MODEL ASSETS [BASELINE...]]
#
# OUT gets index.html and preview.js (scripts/web/page/), gp_web.wasm (the
# interpreter, crates/gp-web, for wasm32-unknown-unknown), and what
# gp-web-prep writes: the model bundle, the generator's pictures,
# preview.json and each baseline's frames and native hashes. With no MODEL there
# is no gp-web-prep step: the player only, for a page whose game files all
# come from a ROM lifted in the browser (scripts/web/build-page.sh).
#
# The wasm build uses the workspace's toolchain, which needs the
# wasm32-unknown-unknown target. With WEB_TOOLS set to the directory
# scripts/web/fetch-tools.sh prints, wasm-opt shrinks the module.
# Builds go to CARGO_TARGET_DIR (default target/). Prints each step's time.
#
# With GP_WEB_REUSE=1 (the release check, docs/web.md "Reuse"), the
# optimized module and gp-web-prep's output are taken from
# scripts/web/reuse.py's store when none of the files they were made from
# changed, and kept there when they are made; cargo and wasm-opt don't run
# for a kept one. GP_WEB_REUSE=save makes them again and keeps them.
#
# Exit codes: 0 built, 2 couldn't build.

set -euo pipefail
trap 'exit 2' ERR

if [ $# -ne 1 ] && [ $# -lt 3 ]; then
  echo "usage: scripts/web/build-preview.sh OUT [MODEL ASSETS [BASELINE...]]" >&2
  exit 2
fi
out="$1" model="${2:-}" assets="${3:-}"
shift "$(( $# < 3 ? $# : 3 ))"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
target="$(cd "$repo" && mkdir -p "${CARGO_TARGET_DIR:-target}" && cd "${CARGO_TARGET_DIR:-target}" && pwd)"

# The features rustc enables by default for wasm32-unknown-unknown. Not
# --all-features: that lets wasm-opt use proposals browsers don't ship
# (Chrome 156 refused a module using compact imports).
features="--enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue"

now() { python3 -I -c 'import time; print(time.time())'; }
lap() { python3 -I -c 'import sys; print(f"{float(sys.argv[2]) - float(sys.argv[1]):.2f}")' "$1" "$(now)"; }

reuse="$repo/scripts/web/reuse.py"
kept() { [ "${GP_WEB_REUSE:-0}" = 1 ] && python3 -I "$reuse" find "$@"; }
keep() { case "${GP_WEB_REUSE:-0}" in 1|save) python3 -I "$reuse" save "$@" >/dev/null ;; esac; }
rustc_v="$(cd "$repo" && rustc -vV | tr '\n' ' ')"

mkdir -p "$out"
out="$(cd "$out" && pwd)"
t0=$(now)
# gp-web-prep's output: its own code (the files cargo's dep-info names),
# the model's folder (its themes too), the lifted files by content, the
# personal themes and the package mirrors it reads, and the sound captures
# and banks under $GP_HOME (by their names, sizes and dates).
home="${GP_HOME:-$HOME/.gameprinter}"
prep_extra="gp-web-prep ${GP_WEB_PREP_FLAGS:-} $* assets $( [ "${GP_WEB_REUSE:-0}" = 0 ] || python3 -I "$reuse" digest --tree "$assets") $rustc_v"
prep_in=(--tree "$model" --tree "$home/themes" --file "$home/mirrors.toml" --file "$repo/mirrors.toml"
  --listing "$home/work/sound" --listing "$home/sound-banks")
if [ -z "$model" ]; then
  echo "prep: none, no model"
elif from="$(kept web-prep --extra "$prep_extra")"; then
  cp -R "$from/out/$(basename "$out")/." "$out/"
  echo "prep kept ($from)"
else
  since=$(now)
  (cd "$repo" && cargo build -q --locked -p gp-web --bin gp-web-prep --target-dir "$target") || exit 2
  # GP_WEB_PREP_FLAGS: theme options for gp-web-prep (--themes, --stack FILE,
  # --theme DIR), split on spaces.
  # shellcheck disable=SC2086
  "$target/debug/gp-web-prep" ${GP_WEB_PREP_FLAGS:-} "$model" "$assets" "$out" "$@" >/dev/null || exit 2
  keep web-prep --extra "$prep_extra" --depinfo "$target/debug/gp-web-prep.d" "${prep_in[@]}" --out "$out" --since "$since"
  echo "prep $(lap "$t0") s"
fi

t0=$(now)
# Sized for download: one codegen unit, LTO, optimized for size.
cargo_args=(rustc -q --locked -p gp-web --lib --release --target wasm32-unknown-unknown
  --crate-type cdylib --target-dir "$target/web-preview"
  --config 'target.wasm32-unknown-unknown.rustflags=["-Clink-arg=--max-memory=268435456"]'
  --config 'profile.release.lto=true' --config 'profile.release.opt-level="s"'
  --config 'profile.release.codegen-units=1' --config 'profile.release.strip=true')
# The key names the target folder TARGET, so worktrees share the module.
wasm_extra="${cargo_args[*]//$target/TARGET} wasm-opt ${WEB_TOOLS:-none} $features $rustc_v"
if from="$(kept web-preview-wasm --extra "$wasm_extra")"; then
  cp "$from/out/gp_web.wasm" "$out/gp_web.wasm"
  echo "wasm kept ($from)"
else
  (cd "$repo" && cargo "${cargo_args[@]}") || exit 2
  wasm="$target/web-preview/wasm32-unknown-unknown/release/gp_web.wasm"
  echo "wasm $(lap "$t0") s"
  t0=$(now)
  if [ -n "${WEB_TOOLS:-}" ]; then
    "$WEB_TOOLS/wasm-opt" -Oz $features --strip-debug --strip-producers "$wasm" -o "$out/gp_web.wasm" || exit 2
    echo "wasm-opt $(lap "$t0") s"
  else
    cp "$wasm" "$out/gp_web.wasm"
  fi
  keep web-preview-wasm --extra "$wasm_extra" --depinfo "${wasm%.wasm}.d" --out "$out/gp_web.wasm"
fi
# The page and the scripts preview.js imports (crates/gp-cli/src/mechanic.rs,
# PAGE_SCRIPTS, lists the same).
for f in index.html preview.js level.js shell-keys.js storage.js storage-wit.js module-host.js module-worker.js; do
  cp "$repo/scripts/web/page/$f" "$out/" || exit 2
done
echo "gp_web.wasm $(wc -c < "$out/gp_web.wasm" | tr -d ' ') bytes"
