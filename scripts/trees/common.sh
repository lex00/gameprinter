# shellcheck shell=bash disable=SC2034
# Shared by the tree scripts (new.sh, take.sh, list.sh, done.sh): a tree
# per agent, an APFS clone of a warm checkout and its target folder
# (docs/agents.md, "A tree per agent"; issue #21). macOS only: the clone
# is clonefile(2) through `cp -c`, which only APFS has.
#
# Settings, all optional:
#   GP_HOME            default ~/.gameprinter
#   GP_TREES           where trees live, default $GP_HOME/trees; taken trees
#                      move to $GP_TREES/.done
#   GP_TREE_SOURCE     the checkout tree-new clones, default the checkout
#                      these scripts are in (the work tree when the lead
#                      runs `just tree-new` there)
#   GP_TREE_ONTO_DIR   the checkout tree-take picks into, default the same
#   GP_TREE_ONTO       the branch that checkout must have out, default work
#
# Sourced, not run. Never deletes anything: a finished tree is moved to
# .done, and deleting .done is the owner's or the lead's one command.

if [ "$(uname -s)" != "Darwin" ]; then
    echo "The tree scripts work on macOS only: a tree is an APFS clone (cp -c), which this system doesn't have." >&2
    exit 1
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gp_home="${GP_HOME:-$HOME/.gameprinter}"
trees="${GP_TREES:-$gp_home/trees}"
done_dir="$trees/.done"
source_tree="${GP_TREE_SOURCE:-$here}"
onto_dir="${GP_TREE_ONTO_DIR:-$source_tree}"
onto_branch="${GP_TREE_ONTO:-work}"
scripts_dir="$here/scripts/trees"

# Every git call reads paths plainly, without octal quoting.
git() {
    command git -c core.quotepath=false "$@"
}

die() {
    echo "$*" >&2
    exit 1
}

# A tree name: letters, digits and dashes.
check_name() {
    local name=$1
    if [ -z "$name" ]; then
        die "Name the tree: letters, digits and dashes."
    fi
    if ! [[ "$name" =~ ^[A-Za-z0-9-]+$ ]]; then
        die "A tree name is letters, digits and dashes only; '$name' isn't."
    fi
}

# Whether DIR is a tree that tree-new made (its own git config says so).
is_tree() {
    local dir=$1
    [ -n "$(git -C "$dir" config --get gp.tree.name 2>/dev/null || true)" ]
}

# These scripts never run inside a tree: no clones of clones, and a tree
# never takes another tree's commits.
refuse_inside_tree() {
    local dir
    for dir in "$here" "$source_tree" "$onto_dir"; do
        if [ -d "$dir" ] && is_tree "$dir"; then
            die "$dir is a tree made by tree-new ($(git -C "$dir" config --get gp.tree.name)). Run the tree commands from the checkout trees are cloned from, not from a tree: no clones of clones."
        fi
    done
}

# Free space on the volume holding DIR, in whole GB.
free_gb() {
    df -g "$1" | awk 'NR == 2 { print $4 }'
}

# The tree NAME's folder, which must exist and be a tree.
tree_dir() {
    local name=$1
    local dir="$trees/$name"
    if [ ! -d "$dir" ]; then
        die "There is no tree $name in $trees."
    fi
    if ! is_tree "$dir"; then
        die "$dir isn't a tree made by tree-new (its git config has no gp.tree.name)."
    fi
    echo "$dir"
}

# A tree's base commit, recorded by tree-new.
tree_base() {
    git -C "$1" config --get gp.tree.base
}

# The tree's uncommitted changes, as `git status --short` prints them;
# empty when it is clean. Ignored files (target/) don't count.
tree_changes() {
    git -C "$1" status --short --untracked-files=all
}

# Brings the tip of the tree's branch into the onto checkout's object
# store (no ref is made) and prints its id.
fetch_tip() {
    local dir=$1 name=$2
    git -C "$onto_dir" fetch --quiet --no-tags "$dir" "refs/heads/tree/$name"
    git -C "$onto_dir" rev-parse FETCH_HEAD
}

# The lines a commit adds and removes, with their files, and no context:
# two commits that make the same change print the same, wherever their
# surrounding lines differ.
change_lines() {
    git -C "$1" show --format= --no-color --no-renames -U0 "$2" | grep -E '^(\+|-)' || true
}

# Whether tree-take recorded the tree's commit SHA as taken (gp.tree.taken
# in the tree's git config, "SHA ONTO-SHA"), the commit it made is on the
# onto branch, and the two make the same change: prints the onto commit
# then, else nothing.
recorded_take() {
    local tree=$1 sha=$2 made
    made="$(git -C "$tree" config --get-all gp.tree.taken 2>/dev/null | awk -v s="$sha" '$1 == s { print $2 }' | tail -1)"
    [ -n "$made" ] || return 0
    git -C "$onto_dir" merge-base --is-ancestor "$made" "refs/heads/$onto_branch" 2>/dev/null || return 0
    if [ "$(change_lines "$onto_dir" "$sha")" = "$(change_lines "$onto_dir" "$made")" ]; then
        echo "$made"
    fi
}

# Seconds as "3 h 12 min" or "12 min".
age_words() {
    local secs=$1
    local hours=$((secs / 3600))
    local mins=$(((secs % 3600) / 60))
    if [ "$hours" -gt 0 ]; then
        echo "$hours h $mins min"
    else
        echo "$mins min"
    fi
}
