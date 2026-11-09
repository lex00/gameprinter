#!/bin/bash
# Re-signs a registry's list files, which expire after 7 days: run it weekly.
#   scripts/web/resign-registry.sh [TARGET]   (default: the rivetrodeo checkout)
# Each package's newest version is named last, which is what makes it latest.
# Exit codes: 0 done, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
real="$(cd "$(dirname "$0")/../.." && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
target="${1:-git:$real/../rivetrodeo}"
dir="${target#git:}"
git -C "$dir" pull -q --ff-only origin HEAD
versions="$(cd "$dir/index" && find . -name '*.toml' | sed 's|^\./||; s|\.toml$||' | awk -F/ '{print $1"/"$2"@"$3}' | sort -V)"
# shellcheck disable=SC2086
"${GP:-$home/bin/gp}" registry reindex "$target" $versions --key "$home/keys/staging/signing"
