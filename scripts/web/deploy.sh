#!/bin/bash
# Puts a built page on its address, a Cloudflare Worker that serves a folder
# (docs/web.md, "Deploying the public page"):
#   scripts/web/deploy.sh [PAGE_DIR] [--list]
#
# 1. With no PAGE_DIR, runs the full release check (release-check.sh
#    --full: every product made and every check run again, nothing
#    reused) and takes the page it built. Anything but PASS stops here.
# 2. Works out the page folder's digest (scripts/web/page-digest.py) and
#    refuses unless the release check recorded a pass for that digest
#    ($GP_HOME/work/page-release/passed/DIGEST): only a folder whose every
#    file is as it was when the check passed goes up.
# 3. Shows what would upload: the folder, the file count and size, the
#    worker, the commit and the pass it goes up under.
# 4. Asks you to type the worker's name, works out the digest again, then
#    runs
#      npx wrangler@WRANGLER deploy --assets PAGE_DIR --message ...
#    which needs `npx wrangler login` done once (it opens a browser).
# --list stops after step 3 with wrangler's --dry-run, which uploads nothing.
#
# Cloudflare keeps earlier versions: `npx wrangler rollback` from
# scripts/web/deploy returns to the last one. GP_WRANGLER replaces the
# wrangler command (default `npx --yes wrangler@WRANGLER`), for trying this
# script against a stand-in. GP and WEB_TOOLS are as for release-check.sh.
# Exit codes: 0 deployed (or --list shown), 1 the release check failed, no
# pass is recorded for this folder, or you declined, 2 couldn't run.

set -uo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
wrangler_version="4.157.0"
read -r -a wrangler <<< "${GP_WRANGLER:-npx --yes wrangler@$wrangler_version}"
project="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$repo/scripts/web/deploy/wrangler.toml")"
[ -n "$project" ] || { echo "deploy: scripts/web/deploy/wrangler.toml names no worker" >&2; exit 2; }
home="${GP_HOME:-$HOME/.gameprinter}"
passed="$home/work/page-release/passed"

list=0
page=""
for a in "$@"; do
  case "$a" in
    --list) list=1 ;;
    -*) echo "usage: scripts/web/deploy.sh [PAGE_DIR] [--list]" >&2; exit 2 ;;
    *) [ -z "$page" ] || { echo "usage: scripts/web/deploy.sh [PAGE_DIR] [--list]" >&2; exit 2; }; page="$a" ;;
  esac
done

if [ -z "$page" ]; then
  log="$home/work/page-release/last-release-check.log"
  mkdir -p "$home/work/page-release" || exit 2
  "$repo/scripts/web/release-check.sh" --full | tee "$log"
  status=${PIPESTATUS[0]}
  page="$(sed -n 's/^bundle: //p' "$log" | tail -n 1)"
  if [ "$status" -ne 0 ] || [ -z "$page" ]; then
    echo "deploy: the release check did not pass; nothing uploaded." >&2
    exit 1
  fi
fi

[ -d "$page" ] || { echo "deploy: $page isn't a folder" >&2; exit 2; }
page="$(cd "$page" && pwd)"
for f in index.html page.json _headers _redirects game/module-host.js; do
  [ -e "$page/$f" ] || { echo "deploy: $page has no $f, so it isn't a built page" >&2; exit 2; }
done
digest="$(python3 -I "$repo/scripts/web/page-digest.py" "$page")" || exit 2
if [ ! -f "$passed/$digest" ]; then
  echo "deploy: no release check passed on this page as it is now (digest $digest; passes are recorded in $passed). Run scripts/web/release-check.sh, or deploy with no PAGE_DIR to run it; nothing uploaded." >&2
  exit 1
fi
record_id="$(sed -n 's/^id //p' "$passed/$digest")"
record_when="$(sed -n 's/^when //p' "$passed/$digest")"

files="$(find "$page" -type f | wc -l | tr -d ' ')"
size="$(du -sk "$page" | cut -f1)"
commit="$(git -C "$repo" rev-parse --short=12 HEAD)"
message="gameprinter $commit, page $record_id"
echo
echo "Would upload to Cloudflare Workers (static assets):"
echo "  worker    $project (https://$project.<your account>.workers.dev)"
echo "  folder    $page"
echo "  files     $files, about $((size / 1024)) MiB (the list and hashes: $page.manifest.txt)"
echo "  passed    the release check on $record_id at $record_when (digest $digest)"
echo "  commit    $commit"
echo "  command   ${wrangler[*]} deploy --assets $page --message \"$message\""
if [ $list = 1 ]; then
  # wrangler's own dry run: reads the folder and runs its checks, uploads nothing.
  (cd "$repo/scripts/web/deploy" && "${wrangler[@]}" deploy --assets "$page" --dry-run) || exit 2
  echo "(--list: nothing uploaded)"
  exit 0
fi

printf 'Type the worker name (%s) to upload, anything else to stop: ' "$project"
read -r answer
if [ "$answer" != "$project" ]; then
  echo "Stopped; nothing uploaded."
  exit 1
fi
# The folder could have changed while the prompt waited.
now="$(python3 -I "$repo/scripts/web/page-digest.py" "$page")" || exit 2
if [ "$now" != "$digest" ]; then
  echo "deploy: the page changed since its digest was taken ($digest, now $now); nothing uploaded." >&2
  exit 1
fi
cd "$repo/scripts/web/deploy" || exit 2
exec "${wrangler[@]}" deploy --assets "$page" --message "$message"
