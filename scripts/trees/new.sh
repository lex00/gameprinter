#!/usr/bin/env bash
# new.sh NAME (`just tree-new NAME`): makes $GP_TREES/NAME, a tree for one
# agent: an APFS clone (cp -c -R) of the warm checkout and its target
# folder together, on a throwaway branch tree/NAME cut from that
# checkout's HEAD, with none of the checkout's uncommitted changes.
# macOS only (APFS clones). Settings: scripts/trees/common.sh.
#
# The target's incremental folders are left out (clone.py says why): a
# tree's first build compiles gp-cli and whatever else it must recompile
# without an incremental cache, once.
#
# How the sources come out clean while the cloned target stays fresh:
# the clone keeps every file's mtime, and cargo decides freshness by
# mtimes. `-p` keeps the folders' mtimes too: without it every folder is
# new, and a build script watching a folder (gp-cli's watches ../gp-nes and
# others) reruns and rebuilds gp-cli. The tree gets a git repository of its own (a local clone of
# the source's object store, not the source's .git, which for a linked
# worktree is a pointer to the source's own index and HEAD). Its index is
# read from the base commit and refreshed against the cloned files, so git
# compares contents and only the files that differ from the base (the
# other agents' uncommitted edits, and untracked files) are touched: they
# are moved aside to $GP_TREES/.done/NAME.left-out-STAMP and the base's
# version is checked out in their place. Only those files get new mtimes,
# so only the crates that hold them recompile, as they must.
set -euo pipefail
# shellcheck source=scripts/trees/common.sh
source "$(dirname "$0")/common.sh"

name=${1:-}
check_name "$name"
refuse_inside_tree

tree="$trees/$name"
if [ -e "$tree" ]; then
    die "A tree named $name exists already at $tree. Pick another name."
fi

source_tree="$(cd "$source_tree" && pwd)"
if [ "$(git -C "$source_tree" rev-parse --show-toplevel)" != "$source_tree" ]; then
    die "$source_tree isn't the top of a git checkout."
fi
common="$(git -C "$source_tree" rev-parse --path-format=absolute --git-common-dir)"

mkdir -p "$trees" "$done_dir"

# A clone needs APFS, and the tree on the same volume as the source.
# Refuse plainly otherwise: never a full copy in its place.
source_dev="$(stat -f %d "$source_tree")"
trees_dev="$(stat -f %d "$trees")"
if [ "$source_dev" != "$trees_dev" ]; then
    die "$trees and $source_tree are on different volumes, so the tree can't be a clone. Set GP_TREES to a folder on the source's volume. Nothing was copied."
fi
device="$(df "$trees" | awk 'NR == 2 { print $1 }')"
if ! mount | grep -F "$device on " | grep -q "(apfs"; then
    die "$trees isn't on an APFS volume ($device), so the tree can't be a clone. Nothing was copied."
fi

free="$(free_gb "$trees")"
if [ "$free" -lt 60 ]; then
    echo "Warning: only $free GB free on this disk (under 60). A tree costs little at first but grows with every rebuild; finish trees soon and clear .done." >&2
fi

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    echo "Note: CARGO_TARGET_DIR is set ($CARGO_TARGET_DIR) here. The tree is cloned with $source_tree/target, and agents in the tree must build with CARGO_TARGET_DIR unset." >&2
fi
if [ ! -d "$source_tree/target" ]; then
    echo "Note: $source_tree has no target folder, so the tree's first build is a cold one." >&2
fi

base="$(git -C "$source_tree" rev-parse HEAD)"
base_line="$(git -C "$source_tree" log -1 --format='%h %s' "$base" | cut -c1-100)"
stamp="$(date +%Y%m%d-%H%M%S)"
left_out="$done_dir/$name.left-out-$stamp"

echo "Cloning $source_tree and its target into $tree"
start=$(date +%s)
python3 -I -B "$scripts_dir/hold-cargo-locks.py" "$source_tree/target" python3 -I -B "$scripts_dir/clone.py" "$source_tree" "$tree"
end=$(date +%s)
echo "Cloned in $((end - start)) s, waiting included."

# The tree's own repository, in place of the copied .git.
mkdir -p "$left_out"
mv "$tree/.git" "$left_out/source-dot-git"
git clone --quiet --bare --local --no-tags "$common" "$tree/.git"
git -C "$tree" config core.bare false
hooks="$(git -C "$source_tree" config --get core.hooksPath || true)"
if [ -n "$hooks" ]; then
    git -C "$tree" config core.hooksPath "$hooks"
fi
if [ -f "$common/info/exclude" ]; then
    cp "$common/info/exclude" "$tree/.git/info/exclude"
fi
# A tree never pushes: the lead takes its commits.
git -C "$tree" remote set-url --push origin "no-push-from-a-tree:the-lead-takes-commits-with-tree-take"
git -C "$tree" config gp.tree.name "$name"
git -C "$tree" config gp.tree.base "$base"
git -C "$tree" config gp.tree.source "$source_tree"
git -C "$tree" config gp.tree.made "$(date +%s)"
git -C "$tree" branch "tree/$name" "$base"
git -C "$tree" symbolic-ref HEAD "refs/heads/tree/$name"
git -C "$tree" read-tree HEAD
git -C "$tree" update-index -q --refresh >/dev/null || true

# What differs from the base: moved aside, then the base's version put back.
moved=0
while IFS= read -r -d '' path; do
    mkdir -p "$left_out/files/$(dirname "$path")"
    mv "$tree/$path" "$left_out/files/$path"
    moved=$((moved + 1))
done < <(git -C "$tree" ls-files -z --others --exclude-standard)
changed=()
while IFS= read -r -d '' path; do
    changed+=("$path")
    if [ -e "$tree/$path" ] || [ -L "$tree/$path" ]; then
        mkdir -p "$left_out/files/$(dirname "$path")"
        mv "$tree/$path" "$left_out/files/$path"
    fi
    moved=$((moved + 1))
done < <(git -C "$tree" diff -z --name-only)
if [ "${#changed[@]}" -gt 0 ]; then
    git -C "$tree" checkout --quiet -- "${changed[@]}"
fi
left="$(tree_changes "$tree")"
if [ -n "$left" ]; then
    echo "$left" >&2
    die "The tree at $tree still differs from $base after the clean checkout (above). Look before using it."
fi

echo
echo "Tree:    $tree"
echo "Branch:  tree/$name, from $(git -C "$source_tree" rev-parse --abbrev-ref HEAD) at $base_line"
if [ "$moved" -gt 0 ]; then
    echo "Left out: $moved files that were uncommitted or untracked in $source_tree; they are in $left_out/files"
fi
echo "Free disk: $(free_gb "$trees") GB"
echo
echo "The agent starts every shell command with this line (the tree's own target/ is used; CARGO_TARGET_DIR must stay unset):"
settings=""
for var in RUSTC_WRAPPER SCCACHE_DIR GP_JOBS GP_ROM_DIR GP_PROFILE; do
    if [ -n "${!var:-}" ]; then
        settings="$settings $var=${!var}"
    fi
done
if [ -n "$settings" ]; then
    echo "  cd $tree && unset CARGO_TARGET_DIR && export$settings"
else
    echo "  cd $tree && unset CARGO_TARGET_DIR"
fi
echo "The first build there recompiles gp-cli once (about a minute; it reads its own path at compile time) and the crates whose files were left out."
