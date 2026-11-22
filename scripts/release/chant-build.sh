#!/usr/bin/env bash
# A release build job (chant.config.ts, release.workflow.builds): on its own
# GitHub runner, gp for TARGET with the release's version, archived as
# dist/gp-TARGET.tar.gz (dist/gp-TARGET.zip for Windows), the file the job
# hands to the cut. It sets up the tools, builds the editor, stamps the
# version into the files release.version names (chant release stamp), runs
# scripts/release/build.sh, then unpacks the archive as a person would and
# runs gp --version and gp self check in an empty home:
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
  sudo apt-get -o Acquire::Retries=5 install -y -qq zip
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
export HOME="$tmp/empty-home" GP_HOME="$tmp/empty-home/.gameprinter"
said="$("$gp" --version)"
echo "$said"
case "$said" in
  "gp $version"*) ;;
  *) echo "chant-build: the built gp says '$said', not gp $version" >&2; exit 1 ;;
esac
"$gp" self check || exit 1
echo "chant-build: dist/gp-$target.$ext"
