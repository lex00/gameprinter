#!/usr/bin/env bash
# A release build job (chant.config.ts, release.workflow.builds): on its own
# GitHub runner, gp for TARGET with the release's version, archived as
# dist/gp-TARGET.tar.gz (dist/gp-TARGET.zip for Windows), the file the job
# hands to the cut. It sets up the tools, builds the editor, stamps the
# version into the files release.version names (chant release stamp), runs
# scripts/release/build.sh, then unpacks the archive as a person would and,
# with the checkout moved out of reach, runs gp --version, gp self check
# and gp game kit in an empty home (docs/release.md):
#   CHANT_RELEASE_LABEL=0.1.2 scripts/release/chant-build.sh TARGET
#
# chant sets CHANT_RELEASE_LABEL in the job. It publishes nothing. On a
# workstation, `just release-build OUT` is the same build without the stamp.
#
# Exit codes: 0 built and checked, 1 the built gp failed its check, 2
# couldn't build.
set -euo pipefail
trap 'exit 2' ERR

target="${1:?usage: scripts/release/chant-build.sh TARGET}"
label="${CHANT_RELEASE_LABEL:?chant-build: no CHANT_RELEASE_LABEL (the release workflow sets it)}"
version="${label#v}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"
windows=""
case "$target" in *windows*) windows=1 ;; esac
tmp="${RUNNER_TEMP:-$(mktemp -d)}"
if [ "${RUNNER_OS:-}" = Windows ]; then tmp="$(cygpath -u "$tmp")"; fi

# Windows checks out with CRLF line ends by default, which would put CRLF in
# the files gp carries and in the shell scripts: check the tree out again
# with LF.
if [ "${RUNNER_OS:-}" = Windows ]; then
  git config core.autocrlf false
  git rm -rq --cached .
  git reset -q --hard HEAD
fi

# rustup 1.28 and later install the toolchain file's Rust only when asked.
rustup toolchain install
rustup show
rustup target add wasm32-unknown-unknown "$target"
cargo install --locked cargo-about --version 0.9.2
if [ "${RUNNER_OS:-}" = Linux ]; then
  sudo apt-get -o Acquire::Retries=5 update -qq
  # bubblewrap: gp self check --strict builds a module from the kit in
  # gp game build's sandbox below.
  sudo apt-get -o Acquire::Retries=5 install -y -qq zip bubblewrap
  # Ubuntu 24.04 keeps unprivileged user namespaces from programs with no
  # AppArmor profile, bwrap among them: allow them on this runner, or the
  # sandboxed module build fails at "setting up uid map".
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0 || true
fi

(cd editor && npm ci --silent --no-audit --no-fund && npm run --silent build)

# Every crate Cargo.lock names, for every target: the module kit (gp game
# kit) vendors them --offline, and a fresh runner's cargo cache holds only
# what the gp build itself fetched.
cargo fetch --locked

# chant reads chant.config.ts through tsx (release stamp below); a runner
# has none, and Windows' Node can't load the file without it.
npm install --no-save --no-package-lock --silent --no-audit --no-fund tsx@4.19.2

# The version gp says it is, in the files the cut stamps too.
bash scripts/release/chant.sh release stamp "$label"

# The runner has no page that passed the release check, so this gp carries
# none; gp theme look play then needs --page.
out="$tmp/gp-release"
GP_HOME="$tmp/gp-home" scripts/release/build.sh "$out" --target "$target"

if [ -n "$windows" ]; then ext=zip; else ext=tar.gz; fi
mkdir -p dist
built=("$out"/gp-*-"$target".$ext)
cp "${built[0]}" "dist/gp-$target.$ext"

# The archive, unpacked as a person would, run in an empty home with the
# checkout out of reach.
mkdir -p "$tmp/unpacked" "$tmp/empty-home"
cd "$tmp/unpacked"
if [ -n "$windows" ]; then
  7z x -bso0 "$repo/dist/gp-$target.zip"
  gp="$(ls -d "$tmp"/unpacked/gp-*/)gp.exe"
  USERPROFILE="$(cygpath -w "$tmp/empty-home")"
  export USERPROFILE
else
  tar -xzf "$repo/dist/gp-$target.tar.gz"
  gp="$(ls -d "$tmp"/unpacked/gp-*/)gp"
fi
cd "$tmp"
# The empty home hides nothing the check needs: rustup and cargo keep their
# own homes (by default under HOME, which moves below), or gp self check
# finds no Rust, skips the module build and --strict fails.
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export HOME="$tmp/empty-home" GP_HOME="$tmp/empty-home/.gameprinter"

# What a person's machine has: the archive, and not the checkout gp was
# built from. gp self check hides that checkout itself (issue 46: 0.1.2's
# gp game kit read crates/ under the runner's checkout, so its check here
# passed while gp model gate failed everywhere else); here it is moved
# away as well, and put back before the job uploads dist/. self check
# opens the page's files and the templates, makes the module kit and
# builds a module from it, and asks the page's server for its first page,
# each from the bundle alone; gp game kit is then the command gp model
# gate runs first. --strict fails the check when self check skips a use
# (0.1.3's module build from the kit was never tried in a shell without
# Rust, and failed on every archive; docs/release.md), so
# this runner must have Rust with wasm32 (above) and the sandbox. Windows
# has no sandbox gp game build knows, so there the module build is skipped
# and named, and the check is the rest.
self_check() {
  if [ -n "$windows" ]; then
    "$gp" self check --as-installed
  else
    "$gp" self check --as-installed --strict
  fi
}
check() {
  local said
  said="$("$gp" --version)" || return 1
  echo "$said"
  case "$said" in
    "gp $version"*) ;;
    *) echo "chant-build: the built gp says '$said', not gp $version" >&2; return 1 ;;
  esac
  self_check || return 1
  "$gp" game kit -o "$tmp/kit" || return 1
}
status=0
if [ -n "$windows" ]; then
  GP_HIDE_BUILT_SOURCE=1 check || status=1
else
  away="$repo.away-$$"
  mv "$repo" "$away"
  check || status=1
  mv "$away" "$repo"
fi
if [ "$status" != 0 ]; then
  echo "chant-build: the built gp failed its check away from its source" >&2
  exit 1
fi
echo "chant-build: dist/gp-$target.$ext"
