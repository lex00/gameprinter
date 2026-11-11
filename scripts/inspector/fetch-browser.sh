#!/bin/bash
# Downloads the headless browser the inspector's end-to-end test drives,
# pinned by version and SHA-256, and prints its executable's path:
#   scripts/inspector/fetch-browser.sh GP_HOME
#
# The browser is Chrome Headless Shell 156.0.8078.4, the build that
# playwright-core 1.64.0 (editor/package-lock.json) is made for, from the
# same CDN Playwright's own installer uses. Playwright's installer doesn't
# check what it downloads, so this script does: the zip's SHA-256 must match
# the pin below before anything is unpacked. It goes to
# GP_HOME/work/chrome-headless-shell/<version>/, outside the repo, and a
# finished download is reused.
#
# Only macOS on ARM64 is pinned so far; another platform exits 2 and says
# what to add (docs/inspector.md).
#
# Exit codes: 0 ready, 1 a download failed or its hash differs from the pin,
# 2 couldn't run (no pin for this platform, a missing tool).

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: scripts/inspector/fetch-browser.sh GP_HOME" >&2
  exit 2
fi
home="$1"
version="156.0.8078.4"
cdn="https://cdn.playwright.dev/builds/cft/$version"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    platform="mac-arm64"
    sha256="94e4337b70cbc5638f0935fbc46b9f242bc7416f657e3fbe924baf348404b53d"
    exe="chrome-headless-shell-mac-arm64/chrome-headless-shell"
    ;;
  Linux-x86_64)
    # GitHub's hosted runners, for the page checks in just check.
    platform="linux64"
    sha256="c3e50596aef52c58f8fda3c6b3411c33d34c88b177b3ed1c2243359fd3b8ba20"
    exe="chrome-headless-shell-linux64/chrome-headless-shell"
    ;;
  *)
    echo "fetch-browser: no pinned Chrome Headless Shell for $(uname -s) $(uname -m)." >&2
    echo "  Add its URL ($cdn/<platform>/chrome-headless-shell-<platform>.zip) and SHA-256 to $0." >&2
    exit 2
    ;;
esac

for tool in curl shasum unzip; do
  command -v "$tool" >/dev/null || { echo "fetch-browser: needs $tool" >&2; exit 2; }
done

dir="$home/work/chrome-headless-shell/$version"
if [ -x "$dir/$exe" ] && [ -f "$dir/.sha256" ] && [ "$(cat "$dir/.sha256")" = "$sha256" ]; then
  echo "$dir/$exe"
  exit 0
fi

mkdir -p "$dir"
zip="$dir/chrome-headless-shell-$platform.zip"
url="$cdn/$platform/chrome-headless-shell-$platform.zip"
if [ ! -f "$zip" ]; then
  echo "fetch-browser: downloading Chrome Headless Shell $version for $platform (about 100MB)" >&2
  if ! curl -sSfL --retry 2 -o "$zip.partial" "$url"; then
    echo "fetch-browser: download failed from $url" >&2
    exit 1
  fi
  mv "$zip.partial" "$zip"
fi
actual="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
if [ "$actual" != "$sha256" ]; then
  echo "fetch-browser: $zip has SHA-256 $actual, expected $sha256" >&2
  echo "  move it aside and run again; if it still differs, the pinned build changed upstream: stop and review before changing the pin" >&2
  exit 1
fi
unzip -q -o "$zip" -d "$dir"
[ -x "$dir/$exe" ] || { echo "fetch-browser: $exe isn't in the zip" >&2; exit 1; }
echo "$sha256" > "$dir/.sha256"
echo "$dir/$exe"
