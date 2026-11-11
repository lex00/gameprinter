#!/bin/bash
# Builds the live player (crates/gp-play: gp-nes for the browser) that the
# mechanic page plays the original in:
#   scripts/web/build-play.sh [OUT]
#
# OUT (default target/play-web) gets gp_play.wasm.
#
# getrandom has no browser backend without JavaScript glue; gp-nes never
# asks for random numbers (it refuses random power-on RAM), so the build
# selects getrandom's `unsupported` backend, which fails any call instead.
# With WEB_TOOLS set to the directory scripts/web/fetch-tools.sh prints,
# wasm-opt shrinks the module.
#
# Exit codes: 0 built, 2 couldn't build.

set -euo pipefail
trap 'exit 2' ERR

repo="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$repo/target/play-web}"
target="$(cd "$repo" && mkdir -p "${CARGO_TARGET_DIR:-target}" && cd "${CARGO_TARGET_DIR:-target}" && pwd)"
features="--enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue"

mkdir -p "$out"
(cd "$repo" && RUSTFLAGS='--cfg getrandom_backend="unsupported"' \
  cargo rustc -q --locked -p gp-play --lib --release --target wasm32-unknown-unknown \
  --crate-type cdylib --target-dir "$target/play-wasm" \
  --config 'profile.release.lto=true' --config 'profile.release.opt-level=3' \
  --config 'profile.release.codegen-units=1' --config 'profile.release.strip=true') || exit 2
wasm="$target/play-wasm/wasm32-unknown-unknown/release/gp_play.wasm"
if [ -n "${WEB_TOOLS:-}" ]; then
  "$WEB_TOOLS/wasm-opt" -O3 $features --strip-debug --strip-producers "$wasm" -o "$out/gp_play.wasm" || exit 2
else
  cp "$wasm" "$out/gp_play.wasm"
fi
echo "gp_play.wasm $(wc -c < "$out/gp_play.wasm" | tr -d ' ') bytes in $out"
