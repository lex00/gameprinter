#!/bin/bash
# Re-signs a signed registry's list files, which expire after 7 days: run it
# weekly on a registry whose readers hold its root. One with no root needs nothing.
#   scripts/web/resign-registry.sh TARGET   (git:FOLDER, a checkout of the registry)
# Each package's newest version is named last, which is what makes it latest.
# Exit codes: 0 done, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
real="$(cd "$(dirname "$0")/../.." && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
if [ $# -ne 1 ]; then
  echo "usage: scripts/web/resign-registry.sh git:FOLDER (no registry here is signed now: gp-registry isn't)" >&2
  exit 2
fi
target="$1"
dir="${target#git:}"
git -C "$dir" pull -q --ff-only origin HEAD
versions="$(cd "$dir/index" && find . -name '*.toml' | sed 's|^\./||; s|\.toml$||' | awk -F/ '{print $1"/"$2"@"$3}' | sort -V)"
# shellcheck disable=SC2086
"${GP:-$home/bin/gp}" registry reindex "$target" $versions --key "$home/keys/staging/signing"
