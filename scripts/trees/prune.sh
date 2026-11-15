#!/usr/bin/env bash
# prune.sh [--dry-run] [TARGET] (`just tree-prune`): moves the stale
# incremental sessions of TARGET (default this checkout's target/, the
# work target trees are cloned from) into $GP_TREES/.done, keeping the
# newest session of each crate (prune-incremental.py says which). It
# holds every cargo lock in TARGET while it moves, as tree-new's clone
# does, so builds there wait a moment. Deletes nothing: the existing one
# cleanup command for .done, which tree-list prints, frees the space.
# Trees cloned before keep their own copies of what was moved: those
# become each tree's own disk until the tree is done. macOS only.
set -euo pipefail
# shellcheck source=scripts/trees/common.sh
source "$(dirname "$0")/common.sh"

refuse_inside_tree

dry=()
if [ "${1:-}" = "--dry-run" ]; then
    dry=(--dry-run)
    shift
fi
target="${1:-$source_tree/target}"
if [ ! -d "$target" ]; then
    die "$target isn't a folder."
fi
target="$(cd "$target" && pwd)"
away="$done_dir/incremental-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$done_dir"
if [ "$(stat -f %d "$target")" != "$(stat -f %d "$done_dir")" ]; then
    die "$done_dir and $target are on different volumes, so the move would be a copy. Set GP_TREES to a folder on the target's volume."
fi
python3 -I -B "$scripts_dir/hold-cargo-locks.py" "$target" \
    python3 -I -B "$scripts_dir/prune-incremental.py" ${dry[@]+"${dry[@]}"} "$target" "$away"
echo "Free disk: $(free_gb "$target") GB (the moved sessions are freed when $done_dir is deleted)"
