#!/usr/bin/env bash
# take.sh NAME (`just tree-take NAME`): puts tree NAME's commits on the
# shared branch. Shows the tree's commits since its base, refuses while the
# tree has uncommitted changes, then cherry-picks base..tip onto the onto
# branch (work) in the shared checkout, one commit at a time in order.
# At the first conflict it names the conflicting files, runs
# `git cherry-pick --abort` (the branch is left as it was before that
# pick; the picks before it stay), and stops. Never a merge commit, never a
# file copy, never a force. A commit whose patch is on the branch already
# (taken before) is skipped. Each pick is recorded in the tree's git config
# (gp.tree.taken, "TREE-SHA WORK-SHA") for tree-done. macOS only. Settings: scripts/trees/common.sh.
set -euo pipefail
# shellcheck source=scripts/trees/common.sh
source "$(dirname "$0")/common.sh"

name=${1:-}
check_name "$name"
refuse_inside_tree
tree="$(tree_dir "$name")"
base="$(tree_base "$tree")"

on="$(git -C "$tree" symbolic-ref --quiet --short HEAD || echo "a detached HEAD")"
if [ "$on" != "tree/$name" ]; then
    die "Tree $name is on $on, not on tree/$name. Its commits are taken from tree/$name; ask the agent to switch back to it."
fi

changes="$(tree_changes "$tree")"
if [ -n "$changes" ]; then
    echo "Tree $name has uncommitted changes:" >&2
    echo "$changes" >&2
    die "Nothing taken. The agent commits (or drops) them first."
fi

if ! git -C "$tree" merge-base --is-ancestor "$base" "tree/$name"; then
    die "tree/$name no longer grows from its base $base (the agent rebased or reset it). Nothing taken; look at the tree by hand."
fi
if ! git -C "$onto_dir" rev-parse --quiet --verify "refs/heads/$onto_branch" >/dev/null; then
    die "$onto_dir has no branch $onto_branch. Nothing taken."
fi

# The tree's own commits: since its base, and not in the onto branch's
# history. After the agent rebases tree/NAME on the onto branch (to clear a
# conflict), the onto branch's commits are in tree/NAME too; they are left
# out here, so only the agent's commits are shown, checked and picked.
tip="$(fetch_tip "$tree" "$name")"
own=("$tip" "^$base" "^refs/heads/$onto_branch")

merges="$(git -C "$onto_dir" rev-list --merges "${own[@]}")"
if [ -n "$merges" ]; then
    die "tree/$name has merge commits since its base ($merges). Nothing taken: a take is cherry-picks only."
fi

count="$(git -C "$onto_dir" rev-list --count "${own[@]}")"
if [ "$count" -eq 0 ]; then
    echo "Tree $name has no commits that aren't on $onto_branch since its base $(git -C "$tree" log -1 --format='%h %s' "$base" | cut -c1-80). Nothing to take."
    exit 0
fi

echo "Tree $name: $count commits since its base $(git -C "$tree" log -1 --format=%h "$base") that aren't on $onto_branch"
echo
git -C "$onto_dir" --no-pager log --stat --reverse "${own[@]}"
echo

# The shared checkout: on the onto branch, nothing half done, nothing
# staged (a cherry-pick commits the whole index), and no uncommitted change
# in a file the picks touch.
if [ "$(git -C "$onto_dir" symbolic-ref --quiet --short HEAD || true)" != "$onto_branch" ]; then
    die "$onto_dir has $(git -C "$onto_dir" rev-parse --abbrev-ref HEAD) out, not $onto_branch. Nothing taken."
fi
for state in CHERRY_PICK_HEAD MERGE_HEAD REVERT_HEAD rebase-merge rebase-apply; do
    if [ -e "$(git -C "$onto_dir" rev-parse --path-format=absolute --git-path "$state")" ]; then
        die "$onto_dir is in the middle of a git operation ($state). Nothing taken."
    fi
done
staged="$(git -C "$onto_dir" diff --cached --name-only)"
if [ -n "$staged" ]; then
    echo "$onto_dir has staged changes (an agent mid-commit?):" >&2
    echo "$staged" >&2
    die "Nothing taken: a cherry-pick would refuse with these in the index. Wait for that commit and run tree-take again."
fi
touched="$(git -C "$onto_dir" log --format= --name-only --no-renames "${own[@]}" | sort -u)"
dirty="$({ git -C "$onto_dir" diff --name-only; git -C "$onto_dir" ls-files --others --exclude-standard; } | sort -u)"
both="$(comm -12 <(echo "$touched") <(echo "$dirty") | sed '/^$/d')"
if [ -n "$both" ]; then
    echo "$onto_branch in $onto_dir has uncommitted changes in files the picks touch:" >&2
    echo "$both" >&2
    die "Nothing taken. Those edits are someone's work in progress; take the tree after they commit."
fi

if ! git -C "$onto_dir" merge-base --is-ancestor "$base" HEAD; then
    echo "Warning: the tree's base $base isn't in $onto_branch's history. Taking its commits all the same (only base..tip, never the whole tree)." >&2
fi

# `git cherry`: + for a commit to take, - for one whose patch is on the
# branch already.
taken=0
skipped=0
while read -r mark sha; do
    subject="$(git -C "$onto_dir" log -1 --format=%s "$sha" | cut -c1-90)"
    if [ "$mark" = "-" ]; then
        echo "Skipped ${sha:0:9} (its patch is on $onto_branch already): $subject"
        skipped=$((skipped + 1))
        continue
    fi
    if ! git -C "$onto_dir" cherry-pick "$sha" >/dev/null 2>&1; then
        conflicts="$(git -C "$onto_dir" diff --name-only --diff-filter=U)"
        if [ -e "$(git -C "$onto_dir" rev-parse --path-format=absolute --git-path CHERRY_PICK_HEAD)" ]; then
            git -C "$onto_dir" cherry-pick --abort
        fi
        echo >&2
        echo "Stopped at ${sha:0:9}: $subject" >&2
        if [ -n "$conflicts" ]; then
            echo "It conflicts in:" >&2
            echo "$conflicts" >&2
        else
            echo "git refused the pick without a conflict (an empty patch, or a local change in the way)." >&2
        fi
        echo "$onto_branch is as it was before that pick: $taken commits taken before it stay, at $(git -C "$onto_dir" log -1 --format='%h %s' | cut -c1-80)." >&2
        die "Send the agent back to rebase tree/$name on $onto_branch's tip, or resolve it by hand."
    fi
    made="$(git -C "$onto_dir" rev-parse HEAD)"
    # The pair, for tree-done: the same change taken with other
    # surrounding lines has another patch-id.
    git -C "$tree" config --add gp.tree.taken "$sha $made"
    echo "Took ${sha:0:9} as ${made:0:9}: $subject"
    taken=$((taken + 1))
done < <(git -C "$onto_dir" cherry HEAD "$tip" "$base")

echo
echo "Took $taken commits onto $onto_branch ($skipped on it already). $onto_branch is at $(git -C "$onto_dir" log -1 --format='%h %s' | cut -c1-80)."
echo "When the tree is finished: just tree-done $name"
