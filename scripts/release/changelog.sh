#!/usr/bin/env bash
# The release's check (chant.config.ts, release.check): CHANGELOG.md must
# have a section for the version being cut, a heading "## gp VERSION" (or
# "## gp VERSION ..." with a date after it) and some text under it. The
# GitHub release's notes are that section. chant runs this before the cut
# records or tags anything, with the label in CHANT_RELEASE_LABEL, so a
# release with no notes is refused there:
#   CHANT_RELEASE_LABEL=0.1.2 scripts/release/changelog.sh
#
# Exit codes: 0 the section is there, 1 it isn't, 2 no label.
set -euo pipefail

label="${CHANT_RELEASE_LABEL:-}"
if [ -z "$label" ]; then
  echo "changelog: no CHANT_RELEASE_LABEL (chant release cut sets it)" >&2
  exit 2
fi
version="${label#v}"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
heading="## gp $version"
body="$(awk -v h="$heading" '
  $0 == h || index($0, h " ") == 1 { f = 1; next }
  f && /^#{1,2} / { exit }
  f' "$repo/CHANGELOG.md")"
if ! grep -q '[^[:space:]]' <<<"$body"; then
  echo "changelog: CHANGELOG.md has no section for $version (a heading '$heading' with the release's changes under it); add it in a commit to main, then start the release" >&2
  exit 1
fi
echo "changelog: CHANGELOG.md has the section '$heading'" >&2
