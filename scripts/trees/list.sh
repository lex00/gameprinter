#!/usr/bin/env bash
# list.sh (`just tree-list`): each tree in $GP_TREES with its branch, base,
# commits since the base (after a rebase on work, work's commits too;
# tree-take picks only the tree's own), whether it is clean, its disk use and its age;
# then what .done holds, what deleting it would free, and the one command
# that deletes it (printed, never run); and the trees' own disk together.
# macOS only. Settings:
# scripts/trees/common.sh.
#
# Disk use is APFS's own count of each tree's private bytes (disk.py):
# blocks no other file shares. A fresh clone costs almost nothing; the
# number grows with what the tree's builds write, and with the old blocks it
# keeps alive after the work tree's builds replace their own copies. `du`
# counts every shared block in full and says nothing about cost here.
set -euo pipefail
# shellcheck source=scripts/trees/common.sh
source "$(dirname "$0")/common.sh"

refuse_inside_tree

if [ ! -d "$trees" ]; then
    echo "No trees: $trees doesn't exist."
    exit 0
fi

now=$(date +%s)
found=0
trees_total=0
for dir in "$trees"/*/; do
    [ -d "$dir" ] || continue
    dir="${dir%/}"
    name="$(basename "$dir")"
    if ! is_tree "$dir"; then
        echo "$name: not a tree made by tree-new (no gp.tree.name in its git config)"
        continue
    fi
    found=$((found + 1))
    base="$(tree_base "$dir")"
    made="$(git -C "$dir" config --get gp.tree.made)"
    branch="$(git -C "$dir" symbolic-ref --quiet --short HEAD || echo "detached HEAD")"
    ahead="$(git -C "$dir" rev-list --count "$base..tree/$name" 2>/dev/null || echo "?")"
    changed="$(tree_changes "$dir" | wc -l | tr -d ' ')"
    if [ "$changed" -eq 0 ]; then
        state="clean"
    else
        state="$changed uncommitted"
    fi
    read -r private apparent _ < <(python3 -I -B "$scripts_dir/disk.py" "$dir")
    trees_total="$(python3 -I -c 'import sys; print(f"{float(sys.argv[1]) + float(sys.argv[2]):.2f}")' "$trees_total" "$private")"
    echo "$name"
    echo "  path:    $dir"
    echo "  branch:  $branch, $ahead commits on it since its base $(git -C "$dir" log -1 --format='%h %s' "$base" | cut -c1-70)"
    echo "  state:   $state"
    echo "  disk:    $private GB its own ($apparent GB as du counts it, shared blocks in full)"
    echo "  age:     $(age_words $((now - made)))"
done
if [ "$found" -eq 0 ]; then
    echo "No trees in $trees."
else
    echo
    echo "All $found trees: $trees_total GB their own, together"
fi

echo
done_items=()
for item in "$done_dir"/*; do
    [ -e "$item" ] || continue
    done_items+=("$item")
done
if [ "${#done_items[@]}" -eq 0 ]; then
    echo "Nothing in $done_dir."
else
    echo "Put away in $done_dir:"
    total=0
    while IFS=$'\t' read -r private apparent path; do
        echo "  $(basename "$path"): $private GB its own"
        total="$(python3 -I -c 'import sys; print(f"{float(sys.argv[1]) + float(sys.argv[2]):.2f}")' "$total" "$private")"
    done < <(python3 -I -B "$scripts_dir/disk.py" "${done_items[@]}")
    echo "Deleting it frees at least $total GB (more where put-away trees share blocks only with each other)."
    echo "The one cleanup command, for the owner or the lead:"
    echo "  rm -rf $done_dir"
fi
echo "Free disk: $(free_gb "$trees") GB"
