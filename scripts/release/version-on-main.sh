#!/usr/bin/env bash
# Puts a released version on main: stamps it (chant release stamp) into the
# files chant.config.ts's release.version names, on a fresh worktree of
# origin/main, and pushes that one commit to main. The train stamps the
# version only into its builds, but Claude Code's plugin marketplace reads
# plugin/.claude-plugin/plugin.json and .claude-plugin/marketplace.json from
# main, so without this /plugin install gp@gameprinter gives the previous
# version's plugin beside the new gp. chant's after-release job can't do it:
# it runs with contents: read, and release.workflow.env reaches only the cut
# and publish jobs.
#
# `just release` runs it once the run it dispatched has passed; run it by hand
# (`just release-on-main LABEL`) when that wait was cut short. LABEL is the
# released version (0.1.3) or latest, the newest GitHub release's. The
# version must already be released: a version with no release is refused, so
# main never names a version nobody can install. Main already carrying it is
# no change, and exits 0. The owner's to run, like `just release`.
#   scripts/release/version-on-main.sh 0.1.3
#   scripts/release/version-on-main.sh latest
#
# Exit codes: 0 main carries the version, 1 refused (no such release, or the
# stamp touched a file it shouldn't), 2 couldn't (git, gh or chant failed).
set -euo pipefail
trap 'exit 2' ERR

label="${1:?usage: scripts/release/version-on-main.sh LABEL|latest}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
slug=lex00/gameprinter

if [ "$label" = latest ]; then
  tag="$(gh release view --repo "$slug" --json tagName --jq .tagName)"
else
  tag="v${label#v}"
  if ! gh release view "$tag" --repo "$slug" --json tagName >/dev/null 2>&1; then
    echo "version-on-main: $slug has no release $tag, so main keeps its version" >&2
    exit 1
  fi
fi
version="${tag#v}"

# The files the stamp may change: release.version's in chant.config.ts.
allowed=(
  crates/gp-cli/Cargo.toml
  Cargo.lock
  plugin/.claude-plugin/plugin.json
  .claude-plugin/marketplace.json
)

tree="$(mktemp -d)/main"
cleanup() { git -C "$repo" worktree remove --force "$tree" >/dev/null 2>&1 || true; }
trap cleanup EXIT

# Main may move while this runs (a landing): two tries, each from main's head.
for try in 1 2; do
  git -C "$repo" fetch -q origin main
  cleanup
  git -C "$repo" worktree add -q --detach "$tree" origin/main
  (cd "$tree" && bash scripts/release/chant.sh release stamp "$version" >/dev/null)
  changed="$(git -C "$tree" diff --name-only)"
  if [ -z "$changed" ]; then
    echo "version-on-main: main already carries $version"
    exit 0
  fi
  while IFS= read -r path; do
    ok=""
    for a in "${allowed[@]}"; do
      if [ "$path" = "$a" ]; then ok=1; fi
    done
    if [ -z "$ok" ]; then
      echo "version-on-main: chant release stamp $version changed $path, which isn't a version file; nothing pushed" >&2
      exit 1
    fi
  done <<<"$changed"
  git -C "$tree" commit -q -a -m "main carries $version, the version just released (chant release stamp $version): $(echo "$changed" | paste -sd, - | sed 's/,/, /g'). Claude Code's plugin marketplace reads the plugin from main (scripts/release/version-on-main.sh)"
  if git -C "$tree" push -q origin HEAD:main; then
    echo "version-on-main: main carries $version ($(git -C "$tree" rev-parse --short HEAD))"
    exit 0
  fi
  echo "version-on-main: main moved while stamping (try $try)" >&2
done
echo "version-on-main: couldn't push to main; run it again" >&2
exit 2
