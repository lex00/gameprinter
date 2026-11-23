#!/usr/bin/env bash
# just release-plan: chant release cut LABEL --dry-run from a workstation.
#
# The cut records the archives the release run's build jobs write
# (dist/gp-TARGET.tar.gz, chant.config.ts's builds) and its dry run reads
# them from dist/ like the cut does, so on a checkout with no dist/ it stops
# with "artifact binaries has no dist/gp-...tar.gz" before printing the
# label, the green check or the CHANGELOG check. So for each archive dist/
# lacks, this writes a stand-in there for the dry run, says so, and removes
# it after, with the dist/SHA256SUMS the dry run writes over them. The plan's
# digests of those files are of the stand-ins, not of the release's archives.
# dist/ is ignored by git, so the stand-ins leave the tree clean for the cut's
# own check. An archive already in dist/ is used as it is.
#   bash scripts/release/plan.sh [LABEL]
set -euo pipefail

label="${1:-auto}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo"

# The targets of chant.config.ts's builds, the commented-out ones left out.
targets=()
while IFS= read -r t; do targets+=("$t"); done < <(
  grep -v '^[[:space:]]*//' chant.config.ts | sed -n 's/.*target: "\([^"]*\)".*/\1/p'
)
if [ "${#targets[@]}" -eq 0 ]; then
  echo "release-plan: chant.config.ts names no build target" >&2
  exit 1
fi

made=()
sums_saved=""
cleanup() {
  for f in ${made[@]+"${made[@]}"}; do rm -f "$f"; done
  if [ -n "$sums_saved" ]; then
    mv -f "$sums_saved" dist/SHA256SUMS
  elif [ "${#made[@]}" -gt 0 ]; then
    rm -f dist/SHA256SUMS
  fi
  rmdir dist 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p dist
for t in "${targets[@]}"; do
  case "$t" in *windows*) f="dist/gp-$t.zip" ;; *) f="dist/gp-$t.tar.gz" ;; esac
  if [ ! -e "$f" ]; then
    printf 'stand-in for %s, written by just release-plan; the release run builds the real one\n' "$f" >"$f"
    made+=("$f")
  fi
done
if [ "${#made[@]}" -gt 0 ]; then
  if [ -e dist/SHA256SUMS ]; then
    sums_saved="$(mktemp)"
    cp dist/SHA256SUMS "$sums_saved"
  fi
  echo "release-plan: the archives come from the release run's build jobs; dist/ here lacks ${made[*]}, so the dry run records stand-ins for them (removed after) and the digests it shows for them and SHA256SUMS are not the release's" >&2
fi

if [ -z "${GH_TOKEN:-}" ] && command -v gh >/dev/null; then
  GH_TOKEN=$(gh auth token 2>/dev/null || true)
  export GH_TOKEN
fi
bash scripts/release/chant.sh release cut "$label" --dry-run
