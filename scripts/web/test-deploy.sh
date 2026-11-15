#!/usr/bin/env bash
# The checks of scripts/web/deploy.sh's choice of worker and address and
# its guards, against a stand-in for wrangler (GP_WRANGLER) that records
# what it is asked and uploads nothing, with a made-up page folder and a
# made-up pass record in a GP_HOME of its own: nothing reaches Cloudflare
# and no address is asked for over the network.
#   scripts/web/test-deploy.sh [WORK_DIR]
# The upload and the preview's page check aren't run here: they need a
# real version's preview address (docs/web.md, "Deploying the public page").
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:-$(mktemp -d "${TMPDIR:-/tmp}/test-deploy.XXXXXX")}
mkdir -p "$work"
work=$(mktemp -d "$(cd "$work" && pwd)/run.XXXXXX")
fail() { echo "test-deploy: $*" >&2; exit 1; }

# The stand-in: one line per call, the folder it ran in and its arguments.
log="$work/wrangler.log"
stand_in="$work/wrangler"
cat > "$stand_in" <<EOF
#!/usr/bin/env bash
echo "\$PWD: \$*" >> "$log"
EOF
chmod +x "$stand_in"
export GP_WRANGLER="$stand_in"
export GP_HOME="$work/home"
unset GP_PAGE_WORKER GP_PAGE_SUBDOMAIN GP_PAGE_ADDRESS GP_PAGE_NO_ROMS

# A made-up built page, and a pass recorded for its digest as a release
# check with --no-roms records one.
page="$work/page"
mkdir -p "$page/game" "$GP_HOME/work/page-release/passed"
for f in index.html page.json _headers _redirects game/module-host.js; do echo "made up: $f" > "$page/$f"; done
digest=$(python3 -I "$here/page-digest.py" "$page") || fail "no digest"
printf 'bundle %s\nid made-up\ngame made-up\nfull 1\nroms none\nwhen 2026-11-15T00:00:00\n' "$page" > "$GP_HOME/work/page-release/passed/$digest"

run() { "$here/deploy.sh" "$@" > "$work/out" 2>&1; }

: > "$log"
run "$page" --list --worker acme-page --subdomain acme || fail "--list with a worker and subdomain failed: $(cat "$work/out")"
grep -q 'worker    acme-page (https://acme-page.acme.workers.dev)' "$work/out" || fail "the address isn't the worker's on the subdomain: $(cat "$work/out")"
grep -q 'made without ROMs (--no-roms)' "$work/out" || fail "a pass made without ROMs isn't said: $(cat "$work/out")"
grep -q "versions upload --name acme-page --assets $page --dry-run$" "$log" || fail "wrangler wasn't given the worker's name: $(cat "$log")"
grep -q '(--list: nothing uploaded)' "$work/out" || fail "--list didn't stop before the upload"
[ "$(wc -l < "$log" | tr -d ' ')" = 1 ] || fail "--list called wrangler more than once: $(cat "$log")"

: > "$log"
GP_PAGE_WORKER=harbor GP_PAGE_SUBDOMAIN=lights run "$page" --list || fail "--list from the variables failed: $(cat "$work/out")"
grep -q 'worker    harbor (https://harbor.lights.workers.dev)' "$work/out" || fail "the variables weren't used: $(cat "$work/out")"
grep -q 'versions upload --name harbor ' "$log" || fail "wrangler wasn't given the variable's name: $(cat "$log")"
GP_PAGE_WORKER=harbor run "$page" --list --worker acme-page || fail "a flag over a variable failed"
grep -q 'worker    acme-page (https://acme-page.intentius.workers.dev)' "$work/out" || fail "the flag didn't win over the variable: $(cat "$work/out")"

run "$page" --list || fail "--list with the defaults failed: $(cat "$work/out")"
grep -q 'worker    gameprinter (https://gameprinter.intentius.workers.dev)' "$work/out" || fail "the defaults aren't the owner's worker and address: $(cat "$work/out")"

run "$page" --list --worker acme-page --address https://games.acme.example/ || fail "--address failed: $(cat "$work/out")"
grep -q 'worker    acme-page (https://games.acme.example)' "$work/out" || fail "--address wasn't used: $(cat "$work/out")"

: > "$log"
for bad in "--worker Acme_Page" "--worker -acme" "--subdomain acme.example" "--address http://games.acme.example" "--address https://games.acme.example?x=1" "--worker"; do
  # shellcheck disable=SC2086
  run "$page" --list $bad
  [ $? = 2 ] || fail "$bad wasn't refused (exit 2): $(cat "$work/out")"
done
[ ! -s "$log" ] || fail "a refused run called wrangler: $(cat "$log")"

# A folder that changed after its pass goes nowhere.
echo "changed" >> "$page/page.json"
run "$page" --list --worker acme-page
[ $? = 1 ] || fail "a changed folder wasn't refused (exit 1): $(cat "$work/out")"
grep -q 'no release check passed on this page as it is now' "$work/out" || fail "the refusal isn't said: $(cat "$work/out")"
[ ! -s "$log" ] || fail "a changed folder reached wrangler: $(cat "$log")"

# --first: the typed name is the worker's, and wrangler deploy gets it.
# The address is one on this machine where nothing answers, so the check
# that no page is there yet asks nothing of the network.
addr="https://127.0.0.1:9"
echo "nope" | run --first --worker acme-page --address "$addr"
[ $? = 1 ] || fail "--first with a wrong typed name didn't stop: $(cat "$work/out")"
[ ! -s "$log" ] || fail "--first called wrangler after a wrong typed name: $(cat "$log")"
echo "acme-page" | run --first --worker acme-page --address "$addr" || fail "--first failed: $(cat "$work/out")"
grep -q "deploy --name acme-page --assets $GP_HOME/work/page-release/first-placeholder --message placeholder before the first page" "$log" || fail "--first didn't deploy the placeholder as acme-page: $(cat "$log")"
grep -q "^$(cd "$here/deploy" && pwd): " "$log" || fail "wrangler didn't run from scripts/web/deploy, where wrangler.toml is: $(cat "$log")"

# pages.sh: the same pass record, the page's files for GitHub Pages and the
# workflow that publishes them.
page2="$work/page2"
mkdir -p "$page2/game"
printf '<!doctype html>\n<head>\n<meta charset="utf-8">\n<title>Made Up</title>\n' > "$page2/index.html"
cp "$page2/index.html" "$page2/game/index.html"
echo '{}' > "$page2/page.json"
printf '/*\n  Content-Security-Policy: default-src '"'"'none'"'"'; connect-src '"'"'self'"'"' https:; frame-ancestors '"'"'self'"'"'\n  Cross-Origin-Opener-Policy: same-origin\n' > "$page2/_headers"
echo '/p /index.html 200' > "$page2/_redirects"
echo "made up" > "$page2/game/module-host.js"
"$here/pages.sh" "$page2" "$work/pages" > "$work/out" 2>&1
[ $? = 1 ] || fail "pages.sh took a page with no pass: $(cat "$work/out")"
[ ! -e "$work/pages/site" ] || fail "pages.sh wrote something for a page with no pass"
printf 'bundle %s\nroms none\n' "$page2" > "$GP_HOME/work/page-release/passed/$(python3 -I "$here/page-digest.py" "$page2")"
"$here/pages.sh" "$page2" "$work/pages" > "$work/out" 2>&1 || fail "pages.sh failed: $(cat "$work/out")"
site="$work/pages/site"
[ -f "$site/p.html" ] && [ -f "$site/.nojekyll" ] && [ -f "$site/game/module-host.js" ] || fail "site/ lacks p.html, .nojekyll or the page's files: $(ls -A "$site")"
[ ! -e "$site/_headers" ] && [ ! -e "$site/_redirects" ] || fail "site/ holds _headers or _redirects, which Pages doesn't read"
for f in index.html p.html game/index.html; do
  grep -q '<meta http-equiv="Content-Security-Policy" content="default-src &#x27;none&#x27;; connect-src &#x27;self&#x27; https:">' "$site/$f" || fail "$f has no policy <meta> (less frame-ancestors): $(cat "$site/$f")"
done
grep -q 'path: site' "$work/pages/.github/workflows/pages.yml" && grep -q 'actions/deploy-pages@' "$work/pages/.github/workflows/pages.yml" || fail "no workflow publishing site/"
grep -q "isolation headers" "$work/out" || fail "pages.sh didn't say what breaks on Pages: $(cat "$work/out")"
"$here/pages.sh" "$page2" "$work/pages" > "$work/out" 2>&1
[ $? = 2 ] || fail "pages.sh wrote over a site/ that isn't empty"

echo "test-deploy: the worker, subdomain and address from flags and variables, the owner's as defaults, bad ones refused, a pass without ROMs said, a changed folder refused, --first named; pages.sh held to a pass, with p.html, the policy as a <meta> and the workflow"
