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
# Local main is brought along, never rebased: a rebase gives the tool gate's
# landings new commits and leaves the published gp's link on one main no
# longer has. Local main at or behind origin/main: the stamp goes on
# origin/main, is pushed, and local main moves forward to it. Local main
# with landings origin/main lacks: the stamp goes on local main, after them,
# and isn't pushed (pushing would push the landings too); `git push origin
# main` then puts both on GitHub. Each having commits the other lacks is
# refused, with nothing written.
#
# Exit codes: 0 main carries the version, 1 refused (no such release, the
# stamp touched a file it shouldn't, or main and origin/main diverged), 2
# couldn't (git, gh or chant failed, or main moved while stamping).
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

# Moves local main from $1 forward to $2, in the worktree that has main
# checked out when one does (a fast-forward), else by its ref.
move_main() {
  local from="$1" to="$2" at
  at="$(git -C "$repo" worktree list --porcelain | awk '/^worktree /{w=substr($0,10)} $0=="branch refs/heads/main"{print w}')"
  if [ -n "$at" ]; then
    [ "$(git -C "$at" rev-parse HEAD)" = "$from" ] && git -C "$at" merge --ff-only -q "$to"
  else
    git -C "$repo" update-ref refs/heads/main "$to" "$from"
  fi
}

# Main may move while this runs (a landing): two tries, each from main's head.
for try in 1 2; do
  git -C "$repo" fetch -q origin main
  origin="$(git -C "$repo" rev-parse origin/main)"
  main="$(git -C "$repo" rev-parse -q --verify refs/heads/main || true)"
  base="$origin"
  ahead=""
  if [ -n "$main" ] && [ "$main" != "$origin" ]; then
    if git -C "$repo" merge-base --is-ancestor "$origin" "$main"; then
      base="$main"
      ahead="$(git -C "$repo" rev-list --count "$origin..$main")"
    elif ! git -C "$repo" merge-base --is-ancestor "$main" "$origin"; then
      echo "version-on-main: main and origin/main each have commits the other lacks; merge origin/main into main (a rebase would move the landings off the published gp), then run it again; nothing written" >&2
      exit 1
    fi
  fi
  cleanup
  git -C "$repo" worktree add -q --detach "$tree" "$base"
  (cd "$tree" && bash scripts/release/chant.sh release stamp "$version" >/dev/null)
  changed="$(git -C "$tree" diff --name-only)"
  if [ -z "$changed" ]; then
    if [ -n "$main" ] && [ "$base" = "$origin" ] && [ "$main" != "$origin" ]; then
      move_main "$main" "$origin" || true
    fi
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
  stamped="$(git -C "$tree" rev-parse HEAD)"
  if [ -n "$ahead" ]; then
    # After local main's own landings, on local main only.
    if move_main "$main" "$stamped"; then
      echo "version-on-main: local main carries $version ($(git -C "$tree" rev-parse --short HEAD)) after its $ahead commit(s) origin/main lacks; nothing pushed: git push origin main puts them all on GitHub"
      exit 0
    fi
    echo "version-on-main: main moved while stamping (try $try)" >&2
    continue
  fi
  if git -C "$tree" push -q origin HEAD:main; then
    if [ -n "$main" ] && ! move_main "$main" "$stamped"; then
      echo "version-on-main: origin/main carries $version, but local main moved while stamping: merge origin/main into main (not a rebase)" >&2
      exit 2
    fi
    echo "version-on-main: main carries $version ($(git -C "$tree" rev-parse --short HEAD))"
    exit 0
  fi
  echo "version-on-main: main moved while stamping (try $try)" >&2
done
echo "version-on-main: couldn't push to main; run it again" >&2
exit 2
