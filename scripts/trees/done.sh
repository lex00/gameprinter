#!/usr/bin/env bash
# done.sh NAME (`just tree-done NAME`): puts away a taken tree. Refuses
# while the tree has uncommitted changes or while any of its commits since
# its base isn't on the onto branch (work): one tree-take recorded, whose
# commit there makes the same change, or else one with the same patch-id.
# Then moves the folder to $GP_TREES/.done/ (a rename on the same disk:
# instant, nothing deleted) and deletes its branch tree/NAME there, which
# lives only in the tree's own repository. Deleting .done is the owner's or
# the lead's one cleanup command; `just tree-list` prints it. macOS only.
# Settings: scripts/trees/common.sh.
set -euo pipefail
# shellcheck source=scripts/trees/common.sh
source "$(dirname "$0")/common.sh"

name=${1:-}
check_name "$name"
refuse_inside_tree
tree="$(tree_dir "$name")"
base="$(tree_base "$tree")"

changes="$(tree_changes "$tree")"
if [ -n "$changes" ]; then
    echo "Tree $name has uncommitted changes:" >&2
    echo "$changes" >&2
    die "Not put away: they would be lost with the folder."
fi

on="$(git -C "$tree" symbolic-ref --quiet --short HEAD || echo "a detached HEAD")"
if [ "$on" != "tree/$name" ]; then
    die "Tree $name is on $on, not tree/$name. Not put away: look at it by hand."
fi

tip="$(fetch_tip "$tree" "$name")"
# A commit tree-take recorded as taken counts when the commit it made is
# on the branch and makes the same change (its patch-id differs when the
# lines around the change differ there); one with no record needs a
# commit with the same patch-id.
missing=""
while read -r mark sha; do
    [ "$mark" = "+" ] || continue
    made="$(recorded_take "$tree" "$sha")"
    if [ -n "$made" ]; then
        echo "${sha:0:9} was taken as ${made:0:9} (tree-take's record; the same change, other lines around it)"
        continue
    fi
    missing="$missing+ $sha"$'\n'
done < <(git -C "$onto_dir" cherry "$onto_branch" "$tip" "$base")
missing="${missing%$'\n'}"
if [ -n "$missing" ]; then
    echo "These commits of tree/$name are not on $onto_branch: no take of them is recorded there, and no commit there has the same patch:" >&2
    while read -r _ sha; do
        echo "  $(git -C "$onto_dir" log -1 --format='%h %s' "$sha" | cut -c1-90)" >&2
    done <<< "$missing"
    die "Not put away. Take them first: just tree-take $name"
fi
count="$(git -C "$tree" rev-list --count "$base..tree/$name")"

mkdir -p "$done_dir"
dest="$done_dir/$name"
if [ -e "$dest" ]; then
    dest="$done_dir/$name-$(date +%Y%m%d-%H%M%S)"
fi
mv "$tree" "$dest"
git -C "$dest" checkout --quiet --detach
git -C "$dest" branch --quiet -D "tree/$name"

echo "Tree $name put away in $dest; its $count commits are all on $onto_branch, and its branch tree/$name (was ${tip:0:9}) is deleted."
echo "Clear every put-away tree with the command just tree-list prints."
