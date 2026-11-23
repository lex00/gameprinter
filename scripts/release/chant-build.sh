#!/usr/bin/env bash
# A release build job (chant.config.ts, release.workflow.builds): on its own
# GitHub runner, gp for TARGET with the release's version, archived as
# dist/gp-TARGET.tar.gz (dist/gp-TARGET.zip for Windows), the file the job
# hands to the cut. It sets up the tools, builds the editor, stamps the
# version into the files release.version names (chant release stamp), runs
# scripts/release/build.sh, then checks the archive as a person would run it
# (scripts/release/check-archive.sh: gp --version, gp self check and gp game
# kit in an empty home, with the checkout moved out of reach; an Intel Mac
# archive under Rosetta; docs/release.md):
#   CHANT_RELEASE_LABEL=0.1.2 scripts/release/chant-build.sh TARGET
#
# chant sets CHANT_RELEASE_LABEL in the job. It publishes nothing. At the
# end, and when it stops early, it prints each step's time (build.sh's own
# steps among them), and adds them to the job's summary on GitHub. On a
# workstation, `just release-build OUT` is the same build without the stamp,
# and `just release-check ARCHIVE` the same check.
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
out="$tmp/gp-release"

# Each step's time, in whole seconds, printed by times() when the script
# ends, however it ends.
times="$tmp/chant-build-times.txt"
: > "$times"
started=$SECONDS
t0=$SECONDS
lap() {
  echo "$1: $((SECONDS - t0)) s" >> "$times"
  echo "chant-build: $1 took $((SECONDS - t0)) s"
  t0=$SECONDS
}
times() {
  local rc=$?
  trap - ERR
  {
    echo "chant-build: the times of gp's build for $target, $((SECONDS - started)) s in all:"
    sed 's/^/  /' "$times"
    if [ -f "$out/times.txt" ]; then
      echo "  of which scripts/release/build.sh's steps:"
      sed 's/^/    /' "$out/times.txt"
    fi
  } || true
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
      echo "### gp for $target: $((SECONDS - started)) s"
      echo
      echo '```'
      cat "$times"
      if [ -f "$out/times.txt" ]; then sed 's/^/build.sh: /' "$out/times.txt"; fi
      echo '```'
    } >> "$GITHUB_STEP_SUMMARY" || true
  fi
  exit "$rc"
}
trap times EXIT

# Windows checks out with CRLF line ends by default, which would put CRLF in
# the files gp carries and in the shell scripts: check the tree out again
# with LF.
if [ "${RUNNER_OS:-}" = Windows ]; then
  git config core.autocrlf false
  git rm -rq --cached .
  git reset -q --hard HEAD
  lap "check the tree out again with LF"
fi

# rustup 1.28 and later install the toolchain file's Rust only when asked.
rustup toolchain install
rustup show
rustup target add wasm32-unknown-unknown "$target"
lap "set up Rust"
cargo install --locked cargo-about --version 0.9.2
lap "build cargo-about"
if [ "${RUNNER_OS:-}" = Linux ]; then
  sudo apt-get -o Acquire::Retries=5 update -qq
  # bubblewrap: gp self check --strict builds a module from the kit in
  # gp game build's sandbox (scripts/release/check-archive.sh).
  sudo apt-get -o Acquire::Retries=5 install -y -qq zip bubblewrap
  # Ubuntu 24.04 keeps unprivileged user namespaces from programs with no
  # AppArmor profile, bwrap among them: allow them on this runner, or the
  # sandboxed module build fails at "setting up uid map".
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0 || true
  lap "install zip and bubblewrap"
fi

(cd editor && npm ci --silent --no-audit --no-fund && npm run --silent build)
lap "build the editor"

# Every crate Cargo.lock names, for every target: the module kit (gp-kit,
# in build.sh) vendors them --offline, and a fresh runner's cargo cache
# holds only what the gp build itself fetched.
cargo fetch --locked
lap "fetch the crates"

# chant reads chant.config.ts through tsx (release stamp below); a runner
# has none, and Windows' Node can't load the file without it.
npm install --no-save --no-package-lock --silent --no-audit --no-fund tsx@4.19.2

# The version gp says it is, in the files the cut stamps too.
bash scripts/release/chant.sh release stamp "$label"
lap "stamp the version"

# The runner has no page that passed the release check, so this gp carries
# none; gp theme look play then needs --page.
GP_HOME="$tmp/gp-home" scripts/release/build.sh "$out" --target "$target"
lap "build gp (scripts/release/build.sh)"

if [ -n "$windows" ]; then ext=zip; else ext=tar.gz; fi
mkdir -p dist
built=("$out"/gp-*-"$target".$ext)
cp "${built[0]}" "dist/gp-$target.$ext"

# The archive, run as a person runs it, away from the checkout, which is put
# back before the job uploads dist/.
status=0
bash scripts/release/check-archive.sh "dist/gp-$target.$ext" --version "$version" --move-checkout || status=$?
lap "check the archive"
if [ "$status" != 0 ]; then
  echo "chant-build: the built gp failed its check away from its source" >&2
  exit "$status"
fi
echo "chant-build: dist/gp-$target.$ext"
