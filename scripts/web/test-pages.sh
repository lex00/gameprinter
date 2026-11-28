#!/usr/bin/env bash
# The checks of scripts/web/pages.sh with a made-up page folder and a
# made-up pass record in a GP_HOME of its own: it refuses a page no release
# check passed on, and writes the page's files for GitHub Pages and the
# workflow that publishes them. Nothing reaches the network.
#   scripts/web/test-pages.sh [WORK_DIR]
# The release check and the deploy are the games repo's (docs/web.md,
# "Deploying the public page").
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:-$(mktemp -d "${TMPDIR:-/tmp}/test-pages.XXXXXX")}
mkdir -p "$work"
work=$(mktemp -d "$(cd "$work" && pwd)/run.XXXXXX")
fail() { echo "test-pages: $*" >&2; exit 1; }
export GP_HOME="$work/home"
mkdir -p "$GP_HOME/work/page-release/passed"

page="$work/page"
mkdir -p "$page/game"
printf '<!doctype html>\n<head>\n<meta charset="utf-8">\n<title>Made Up</title>\n' > "$page/index.html"
cp "$page/index.html" "$page/game/index.html"
echo '{}' > "$page/page.json"
printf '/*\n  Content-Security-Policy: default-src '"'"'none'"'"'; connect-src '"'"'self'"'"' https:; frame-ancestors '"'"'self'"'"'\n  Cross-Origin-Opener-Policy: same-origin\n' > "$page/_headers"
echo '/p /index.html 200' > "$page/_redirects"
echo "made up" > "$page/game/module-host.js"
"$here/pages.sh" "$page" "$work/pages" > "$work/out" 2>&1
[ $? = 1 ] || fail "pages.sh took a page with no pass: $(cat "$work/out")"
[ ! -e "$work/pages/site" ] || fail "pages.sh wrote something for a page with no pass"
printf 'bundle %s\nroms none\n' "$page" > "$GP_HOME/work/page-release/passed/$(python3 -I "$here/page-digest.py" "$page")"
"$here/pages.sh" "$page" "$work/pages" > "$work/out" 2>&1 || fail "pages.sh failed: $(cat "$work/out")"
site="$work/pages/site"
[ -f "$site/p.html" ] && [ -f "$site/.nojekyll" ] && [ -f "$site/game/module-host.js" ] || fail "site/ lacks p.html, .nojekyll or the page's files: $(ls -A "$site")"
[ ! -e "$site/_headers" ] && [ ! -e "$site/_redirects" ] || fail "site/ holds _headers or _redirects, which Pages doesn't read"
for f in index.html p.html game/index.html; do
  grep -q '<meta http-equiv="Content-Security-Policy" content="default-src &#x27;none&#x27;; connect-src &#x27;self&#x27; https:">' "$site/$f" || fail "$f has no policy <meta> (less frame-ancestors): $(cat "$site/$f")"
done
grep -q 'path: site' "$work/pages/.github/workflows/pages.yml" && grep -q 'actions/deploy-pages@' "$work/pages/.github/workflows/pages.yml" || fail "no workflow publishing site/"
grep -q "isolation headers" "$work/out" || fail "pages.sh didn't say what breaks on Pages: $(cat "$work/out")"
"$here/pages.sh" "$page" "$work/pages" > "$work/out" 2>&1
[ $? = 2 ] || fail "pages.sh wrote over a site/ that isn't empty"

echo "test-pages: pages.sh held to a pass, with p.html, the policy as a <meta> and the workflow"
