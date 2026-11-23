#!/usr/bin/env bash
# Checks a release archive the way a person runs it (docs/release.md, "The
# check of the built archive"): unpacks it into an empty folder and, in an
# empty HOME and GP_HOME with the checkout gp was built from hidden, runs
# gp --version (which must say VERSION, when given), gp self check
# --as-installed --strict (no --strict for Windows) and gp game kit:
#   bash scripts/release/check-archive.sh ARCHIVE [--version VERSION] [--move-checkout]
#
# An Intel Mac archive (x86_64-apple-darwin) checked on an Apple silicon Mac
# runs under Rosetta, which it installs first when it is missing (sudo -n:
# the GitHub runner has it; on a workstation, install it yourself with
# softwareupdate --install-rosetta). --move-checkout also moves this
# checkout away for the check and puts it back after (the release's build
# job, scripts/release/chant-build.sh, passes it); without it the checkout
# is hidden by GP_HIDE_BUILT_SOURCE=1 alone, which is safe on a workstation.
# After `just release-build OUT [--target TARGET]`, `just release-check
# OUT/gp-VERSION-COMMIT-TARGET.tar.gz` runs it.
#
# Exit codes: 0 passed, 1 the gp in it failed its check, 2 couldn't check.
set -euo pipefail
trap 'exit 2' ERR

archive=""
version=""
move=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) version="${2:?--version needs a version}"; shift 2 ;;
    --version=*) version="${1#--version=}"; shift ;;
    --move-checkout) move=1; shift ;;
    -*) echo "check-archive: unknown option $1" >&2; exit 2 ;;
    *) archive="$1"; shift ;;
  esac
done
[ -n "$archive" ] || { echo "usage: bash scripts/release/check-archive.sh ARCHIVE [--version VERSION] [--move-checkout]" >&2; exit 2; }
[ -f "$archive" ] || { echo "check-archive: no archive $archive" >&2; exit 2; }
archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"
repo="$(cd "$(dirname "$0")/../.." && pwd)"

base="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if [ "${RUNNER_OS:-}" = Windows ]; then base="$(cygpath -u "$base")"; fi
tmp="$(mktemp -d "${base%/}/gp-check-archive.XXXXXX")"
mkdir -p "$tmp/unpacked" "$tmp/empty-home"

# The Rust that gp self check builds a module with, found by its homes:
# HOME is about to be the empty one.
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"

cd "$tmp/unpacked"
run=()
case "$archive" in
  *.zip)
    7z x -bso0 "$archive"
    gp="$(ls -d "$tmp"/unpacked/gp-*/)gp.exe"
    USERPROFILE="$(cygpath -w "$tmp/empty-home")"
    export USERPROFILE
    ;;
  *)
    tar -xzf "$archive"
    gp="$(ls -d "$tmp"/unpacked/gp-*/)gp"
    ;;
esac
case "$gp" in
  *x86_64-apple-darwin*)
    if [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ]; then
      if ! arch -x86_64 /usr/bin/true 2> /dev/null; then
        echo "check-archive: installing Rosetta to run the Intel Mac gp"
        if ! sudo -n softwareupdate --install-rosetta --agree-to-license; then
          echo "check-archive: Rosetta isn't installed and couldn't be: softwareupdate --install-rosetta --agree-to-license" >&2
          exit 2
        fi
      fi
      echo "check-archive: running the Intel Mac gp under Rosetta"
      run=(arch -x86_64)
    fi
    ;;
esac
cd "$tmp"
export HOME="$tmp/empty-home" GP_HOME="$tmp/empty-home/.gameprinter" GP_HIDE_BUILT_SOURCE=1

# gp self check hides the checkout gp was built from itself (issue 46: 0.1.2's
# gp game kit read crates/ under the runner's checkout, so its check there
# passed while gp model gate failed everywhere else), and GP_HIDE_BUILT_SOURCE
# hides it from the other two. self check opens the page's files and the
# templates, makes the module kit and builds a module from it, and asks the
# page's server for its first page, each from the bundle alone; gp game kit is
# then the command gp model gate runs first. --strict fails the check when
# self check skips a use (0.1.3's module build from the kit was never tried
# in a shell without Rust, and failed on every archive; docs/release.md), so
# this machine must have Rust with wasm32 and the sandbox. Windows has no
# sandbox gp game build knows, so there the module build is skipped and
# named, and the check is the rest.
strict=(--strict)
if [ "${RUNNER_OS:-}" = Windows ]; then strict=(); fi
case "$archive" in *.zip) strict=() ;; esac
check() {
  local said
  said="$(${run[@]+"${run[@]}"} "$gp" --version)" || return 1
  echo "$said"
  if [ -n "$version" ]; then
    case "$said" in
      "gp $version"*) ;;
      *) echo "check-archive: the gp in it says '$said', not gp $version" >&2; return 1 ;;
    esac
  fi
  ${run[@]+"${run[@]}"} "$gp" self check --as-installed ${strict[@]+"${strict[@]}"} || return 1
  ${run[@]+"${run[@]}"} "$gp" game kit -o "$tmp/kit" || return 1
}

status=0
if [ -n "$move" ] && [ "${RUNNER_OS:-}" != Windows ]; then
  # What a person's machine has: the archive, and not the checkout.
  away="$repo.away-$$"
  mv "$repo" "$away"
  trap 'mv "$away" "$repo"' EXIT
  check || status=1
  mv "$away" "$repo"
  trap - EXIT
else
  check || status=1
fi
if [ "$status" != 0 ]; then
  echo "check-archive: the gp in $archive failed its check away from its source" >&2
  exit 1
fi
echo "check-archive: $archive passed"
