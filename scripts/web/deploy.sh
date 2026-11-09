#!/bin/bash
# Puts a built page on its address, a Cloudflare Worker that serves a folder.
#   scripts/web/deploy.sh PAGE_DIR [--list]
#
# --list shows what would upload and changes nothing. Needs `npx wrangler
# login` done once. Cloudflare keeps earlier versions: `npx wrangler rollback`
# from scripts/web/deploy returns to the last one.
# Exit codes: 0 done, 2 couldn't.
set -euo pipefail
trap 'exit 2' ERR
wrangler="4.157.0"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
page="$(cd "$1" && pwd)"
for f in index.html page.json _headers _redirects game/module-host.js; do
  [ -e "$page/$f" ] || { echo "$page has no $f, so it isn't a built page" >&2; exit 2; }
done
commit="$(git -C "$repo" rev-parse --short=12 HEAD)"
if [ "${2:-}" = --list ]; then
  exec npx --yes "wrangler@$wrangler" --cwd "$repo/scripts/web/deploy" deploy --assets "$page" --dry-run
fi
exec npx --yes "wrangler@$wrangler" --cwd "$repo/scripts/web/deploy" deploy --assets "$page" --message "gameprinter $commit"
