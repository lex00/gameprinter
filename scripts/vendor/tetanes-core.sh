#!/bin/bash
# Imports tetanes-core, the NES emulator core Game Printer's NES runtime is
# built on, into third_party/tetanes-core, and checks that it's still exactly
# what was imported plus our patch. See third_party/README.md.
#
#   scripts/vendor/tetanes-core.sh                 import, then apply our patch
#   scripts/vendor/tetanes-core.sh --update-patch  rewrite the patch from the tree
#   scripts/vendor/tetanes-core.sh --check         verify the tree (offline)
#
# The import downloads the published crate at a pinned checksum (the crate's
# Cargo.toml is self-contained, unlike the one in upstream's workspace),
# drops the test ROMs it ships (test ROMs are never committed here; the
# accuracy runner downloads them at pinned hashes) and its two binary data
# files (our patch carries them as text), adds the license files
# from the tagged commit, records the SHA-256 of every file it imported in
# third_party/tetanes-core.upstream.sha256, applies our changes from
# third_party/tetanes-core.patch, and writes third_party/tetanes-core/UPSTREAM.md.
# Downloads are cached in $GP_HOME/work/tetanes-core (outside the repo) and
# checked against the pins every time.
#
# --update-patch rewrites the patch file as the difference between the
# pristine crate and the vendored tree. Run it after changing vendored code.
#
# --check needs no network. It reverse-applies the patch to a copy of the
# tree, checks that the result is the pristine import file for file, and
# checks that the patch file is exactly what --update-patch would write. So
# it fails if a vendored file was edited without updating the patch, if the
# patch was edited by hand, or if the pins changed without a new import.
#
# Exit codes: 0 ok; 1 a check, download or patch failed; 2 bad arguments or
# a missing tool.
set -euo pipefail

VERSION="0.17.0"
CRATE_SHA256="c12a52c1ee5d08354b57b57e858367518e9a7a266ad19ca65138ceb054d831c4"
# The commit the crate was published from (its .cargo_vcs_info.json), which
# is upstream's tag tetanes-core-v0.17.0.
COMMIT="94e3db3684ced606c6865c51a9a61b0e15bc08fe"
LICENSE_MIT_SHA256="31c76124b54c86f20be5871fec6eb34738df92bf1aba2469282fa34f2b2fc6b3"
LICENSE_APACHE_SHA256="aaff376532ea30a0cd5330b9502ad4a4c8bf769c539c87ffe78819d188a18ebf"

# Paths in the crate that aren't imported, and why.
EXCLUDE=(
  "test_roms"            # 190 test ROMs: never committed (docs/open-questions.md)
  "Cargo.lock"           # this workspace has its own
  ".cargo_vcs_info.json" # recorded in UPSTREAM.md instead
  # Binary data the crate compiles in. The repo holds only text: our patch adds
  # them as ntscpalette.txt and game_db.txt, and build.rs rebuilds these exact
  # bytes from those, checking them against the SHA-256 pinned in build.rs.
  "ntscpalette.pal"
  "game_db.dat"
)

mode=import
case "${1:-}" in
  "") ;;
  --update-patch) mode=update-patch ;;
  --check) mode=check ;;
  -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "vendor: unknown argument $1 (try --help)" >&2; exit 2 ;;
esac
[ $# -le 1 ] || { echo "vendor: one argument at most (try --help)" >&2; exit 2; }

repo="$(cd "$(dirname "$0")/../.." && pwd)"
dest="$repo/third_party/tetanes-core"
patch_file="$repo/third_party/tetanes-core.patch"
hashes_file="$repo/third_party/tetanes-core.upstream.sha256"
cache="${GP_HOME:-$HOME/.game-printer}/work/tetanes-core"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Every git call here works on plain directories in $tmp, never on the repo,
# and ignores the user's git config so the patch comes out byte for byte the
# same on every machine (the patch's hash is part of the runtime identity).
export GIT_CEILING_DIRECTORIES="$tmp" GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null

if command -v sha256sum >/dev/null; then
  sha256_cmd=(sha256sum)
elif command -v shasum >/dev/null; then
  sha256_cmd=(shasum -a 256)
else
  echo "vendor: needs sha256sum or shasum" >&2
  exit 2
fi
sha256() { "${sha256_cmd[@]}" "$@"; }

fail() { echo "vendor: $*" >&2; exit 1; }

# Downloads $1 into the cache as $2 unless a copy with SHA-256 $3 is there.
fetch() {
  local url="$1" out="$cache/$2" want="$3" got
  if [ -f "$out" ] && [ "$(sha256 "$out" | cut -d' ' -f1)" = "$want" ]; then
    return
  fi
  mkdir -p "$cache"
  curl -sSfL --retry 2 -o "$out.partial" "$url" || { rm -f "$out.partial"; fail "download failed: $url"; }
  got="$(sha256 "$out.partial" | cut -d' ' -f1)"
  if [ "$got" != "$want" ]; then
    rm -f "$out.partial"
    fail "$url has SHA-256 $got, expected $want"
  fi
  mv "$out.partial" "$out"
}

# Builds the pristine import in $1: the crate minus EXCLUDE, plus the licenses.
pristine() {
  local out="$1" src
  fetch "https://static.crates.io/crates/tetanes-core/tetanes-core-$VERSION.crate" "tetanes-core-$VERSION.crate" "$CRATE_SHA256"
  fetch "https://raw.githubusercontent.com/lukexor/tetanes/$COMMIT/LICENSE-MIT" "LICENSE-MIT-$COMMIT" "$LICENSE_MIT_SHA256"
  fetch "https://raw.githubusercontent.com/lukexor/tetanes/$COMMIT/LICENSE-APACHE" "LICENSE-APACHE-$COMMIT" "$LICENSE_APACHE_SHA256"
  mkdir -p "$tmp/crate"
  tar -xzf "$cache/tetanes-core-$VERSION.crate" -C "$tmp/crate"
  src="$tmp/crate/tetanes-core-$VERSION"
  local vcs
  vcs="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["git"]["sha1"])' "$src/.cargo_vcs_info.json")"
  [ "$vcs" = "$COMMIT" ] || fail "the crate was published from $vcs, expected $COMMIT"
  for path in "${EXCLUDE[@]}"; do
    rm -rf "${src:?}/$path"
  done
  cp "$cache/LICENSE-MIT-$COMMIT" "$src/LICENSE-MIT"
  cp "$cache/LICENSE-APACHE-$COMMIT" "$src/LICENSE-APACHE"
  mv "$src" "$out"
}

# Copies the vendored tree to $1, without the generated UPSTREAM.md.
tree_copy() {
  cp -R "$dest" "$1"
  rm -f "$1/UPSTREAM.md"
}

hashes_header() {
  echo "# tetanes-core $VERSION, crate SHA-256 $CRATE_SHA256, commit $COMMIT"
  echo "# The SHA-256 of every file scripts/vendor/tetanes-core.sh imports, before our patch. Generated; don't edit."
}

# SHA-256 of every file under $1, by path, sorted.
hash_tree() {
  (cd "$1" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 "${sha256_cmd[@]}" | sed 's|  \./|  |')
}

# The diff from $1 to $2 (directories in $tmp), with options pinned so every
# machine writes the same bytes.
make_patch() {
  rm -rf "$tmp/a" "$tmp/b"
  cp -R "$1" "$tmp/a"
  cp -R "$2" "$tmp/b"
  local status=0
  (cd "$tmp" && git diff --no-index --no-prefix --binary --full-index --no-color --no-ext-diff \
    --no-renames --diff-algorithm=myers --indent-heuristic --unified=3 a b) || status=$?
  # git diff exits 1 when the trees differ.
  [ "$status" -le 1 ] || fail "git diff failed"
  rm -rf "$tmp/a" "$tmp/b"
}

# Applies the patch file to directory $1; extra arguments go to git apply.
apply_patch() {
  local dir="$1"
  shift
  [ -s "$patch_file" ] || return 0
  (cd "$dir" && git apply -p1 --whitespace=nowarn "$@" "$patch_file")
}

upstream_md() {
  echo "# tetanes-core $VERSION (vendored)"
  echo
  echo "Generated by scripts/vendor/tetanes-core.sh. Don't edit; rerun the script."
  echo
  echo "| | |"
  echo "|---|---|"
  echo "| Upstream | https://github.com/lukexor/tetanes, crate \`tetanes-core\` |"
  echo "| Version | $VERSION (tag \`tetanes-core-v$VERSION\`) |"
  echo "| Commit | \`$COMMIT\` |"
  echo "| Crate SHA-256 | \`$CRATE_SHA256\` (matches crates.io) |"
  echo "| License | MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE, from the commit above) |"
  echo
  echo "Not imported:"
  echo
  echo "| Path | Why |"
  echo "|---|---|"
  echo "| \`test_roms/\` | Test ROMs are never committed to this repo; upstream tests that need them don't run here |"
  echo "| \`Cargo.lock\` | This workspace has its own |"
  echo "| \`.cargo_vcs_info.json\` | Recorded above |"
  echo "| \`ntscpalette.pal\`, \`game_db.dat\` | Binary; our patch adds them as text (\`ntscpalette.txt\`, \`game_db.txt\`), which \`build.rs\` turns back into the same bytes |"
  echo
  echo "Game Printer's changes to this code are in \`third_party/tetanes-core.patch\`, applied after the import. \`third_party/tetanes-core.upstream.sha256\` lists every imported file's hash before the patch, and \`scripts/vendor/tetanes-core.sh --check\` (part of \`just check\`) verifies that this directory is exactly the import plus the patch."
}

case "$mode" in
  import)
    pristine "$tmp/pristine"
    cp -R "$tmp/pristine" "$tmp/patched"
    rejected=0
    if ! apply_patch "$tmp/patched" 2>"$tmp/apply.log"; then
      # Apply what fits and leave the rest in .rej files to resolve by hand.
      rm -rf "$tmp/patched"
      cp -R "$tmp/pristine" "$tmp/patched"
      apply_patch "$tmp/patched" --reject 2>"$tmp/apply.log" || true
      rejected=1
    fi
    upstream_md > "$tmp/patched/UPSTREAM.md"
    { hashes_header; hash_tree "$tmp/pristine"; } > "$hashes_file"
    rm -rf "$dest"
    mkdir -p "$(dirname "$dest")"
    cp -R "$tmp/patched" "$dest"
    echo "vendored tetanes-core $VERSION ($COMMIT) into third_party/tetanes-core"
    echo "files: $(find "$dest" -type f | wc -l | tr -d ' '), size: $(du -sh "$dest" | cut -f1)"
    if [ "$rejected" = 1 ]; then
      cat "$tmp/apply.log" >&2
      echo "vendor: some of third_party/tetanes-core.patch didn't apply. Each file it couldn't patch has a .rej file next to it:" >&2
      (cd "$repo" && find third_party/tetanes-core -name '*.rej' | sed 's/^/  /') >&2
      echo "  make those changes by hand, delete the .rej files, then run: $0 --update-patch" >&2
      exit 1
    fi
    if [ -s "$patch_file" ]; then
      echo "applied third_party/tetanes-core.patch"
    fi
    ;;

  update-patch)
    leftovers="$(cd "$repo" && find third_party/tetanes-core -name '*.rej' | head -5)"
    [ -z "$leftovers" ] || fail "resolve and delete these first: $leftovers"
    pristine "$tmp/pristine"
    { hashes_header; hash_tree "$tmp/pristine"; } > "$tmp/hashes"
    if ! cmp -s "$tmp/hashes" "$hashes_file"; then
      fail "third_party/tetanes-core.upstream.sha256 doesn't match the pinned crate; after changing the pins, import first: $0"
    fi
    tree_copy "$tmp/current"
    make_patch "$tmp/pristine" "$tmp/current" > "$tmp/new.patch"
    if [ -f "$patch_file" ] && cmp -s "$tmp/new.patch" "$patch_file"; then
      echo "third_party/tetanes-core.patch is already current"
    else
      cp "$tmp/new.patch" "$patch_file"
      echo "wrote third_party/tetanes-core.patch, changing $(grep -c "^diff --git " "$patch_file" || true) file(s)"
    fi
    upstream_md > "$dest/UPSTREAM.md"
    ;;

  check)
    problems=0
    problem() { echo "vendor --check: $*" >&2; problems=$((problems + 1)); }
    [ -d "$dest" ] || fail "third_party/tetanes-core is missing; run $0"
    [ -f "$hashes_file" ] || fail "third_party/tetanes-core.upstream.sha256 is missing; run $0"
    [ -f "$patch_file" ] || fail "third_party/tetanes-core.patch is missing; run $0 --update-patch"
    if [ "$(head -2 "$hashes_file")" != "$(hashes_header)" ]; then
      problem "third_party/tetanes-core.upstream.sha256 was made from other pins than the script's; run $0 to import them"
    fi
    if ! cmp -s <(upstream_md) "$dest/UPSTREAM.md"; then
      problem "third_party/tetanes-core/UPSTREAM.md isn't what the script writes; run $0 --update-patch to rewrite it"
    fi
    tree_copy "$tmp/current"
    tree_copy "$tmp/unpatched"
    if ! apply_patch "$tmp/unpatched" -R 2>"$tmp/apply.log"; then
      sed 's/^/  /' "$tmp/apply.log" >&2
      problem "third_party/tetanes-core.patch doesn't reverse-apply to the vendored tree. A vendored file was changed without updating the patch, or the patch was edited. If the tree is right, run $0 --update-patch"
    elif hash_tree "$tmp/unpatched" > "$tmp/hashes" && ! diff <(tail -n +3 "$hashes_file") "$tmp/hashes" > "$tmp/hashes.diff"; then
      echo "  files that differ from the import, after removing the patch (< imported, > vendored tree):" >&2
      { grep '^[<>]' "$tmp/hashes.diff" || true; } | sed 's/^/  /' >&2
      problem "third_party/tetanes-core isn't the pinned import plus third_party/tetanes-core.patch. If the vendored files are right, run $0 --update-patch"
    else
      make_patch "$tmp/unpatched" "$tmp/current" > "$tmp/new.patch"
      if ! cmp -s "$tmp/new.patch" "$patch_file"; then
        { diff -u "$patch_file" "$tmp/new.patch" || true; } | head -40 | sed 's/^/  /' >&2
        problem "third_party/tetanes-core.patch isn't what --update-patch writes for this tree; run $0 --update-patch"
      fi
    fi
    if [ "$problems" -gt 0 ]; then
      exit 1
    fi
    echo "third_party/tetanes-core is tetanes-core $VERSION plus third_party/tetanes-core.patch (changing $(grep -c "^diff --git " "$patch_file" || true) file(s))"
    ;;
esac
