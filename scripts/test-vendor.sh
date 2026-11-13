#!/bin/bash
# Self-test for scripts/vendor/tetanes-core.sh --check, run on a copy of the
# vendored tree so the real one is never touched. Needs no network.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0
fresh() {
  rm -rf "$work/repo"
  mkdir -p "$work/repo/scripts/vendor" "$work/repo/third_party"
  cp "$repo/scripts/vendor/tetanes-core.sh" "$work/repo/scripts/vendor/"
  cp -R "$repo/third_party/tetanes-core" "$repo/third_party/tetanes-core.patch" \
    "$repo/third_party/tetanes-core.upstream.sha256" "$work/repo/third_party/"
}
expect() {
  local want="$1" label="$2" got
  if "$work/repo/scripts/vendor/tetanes-core.sh" --check >"$work/out" 2>&1; then got=pass; else got=fail; fi
  if [ "$got" = "$want" ]; then
    echo "ok    $label ($got)"
  else
    echo "FAIL  $label: wanted $want, got $got"
    sed 's/^/      /' "$work/out"
    failures=$((failures + 1))
  fi
  fresh
}
t="$work/repo/third_party"

fresh
expect pass "the vendored tree as committed"

printf '\n// an unrecorded change\n' >> "$t/tetanes-core/src/cpu.rs"
expect fail "a vendored file the patch doesn't touch is edited"

sed -i.bak 's/wrapping_add(1)/wrapping_add(2)/' "$t/tetanes-core/src/input.rs" && rm "$t/tetanes-core/src/input.rs.bak"
expect fail "a patched line is edited without updating the patch"

: > "$t/tetanes-core.patch"
expect fail "the patch file is emptied"

sed -i.bak 's/^+    pub reads: u32,$/+    pub reads: u64,/' "$t/tetanes-core.patch" && rm "$t/tetanes-core.patch.bak"
expect fail "the patch file adds a line the tree doesn't have"

# Still applies, but isn't what --update-patch writes, so its hash (part of
# the runtime identity) would differ from a regenerated one.
sed -i.bak -E 's/^index ([0-9a-f]{7})[0-9a-f]+\.\.([0-9a-f]{7})[0-9a-f]+/index \1..\2/' "$t/tetanes-core.patch" && rm "$t/tetanes-core.patch.bak"
expect fail "the patch file is equivalent but formatted differently"

printf 'new\n' > "$t/tetanes-core/src/extra.rs"
expect fail "a file is added"

rm "$t/tetanes-core/README.md"
expect fail "a file is removed"

printf 'edited\n' >> "$t/tetanes-core/UPSTREAM.md"
expect fail "UPSTREAM.md is edited"

sed -i.bak 's/^VERSION="[^"]*"/VERSION="0.99.0"/' "$work/repo/scripts/vendor/tetanes-core.sh" && rm "$work/repo/scripts/vendor/tetanes-core.sh.bak"
expect fail "the pins change without a new import"

[ "$failures" -eq 0 ] || { echo "$failures vendoring checks failed"; exit 1; }
