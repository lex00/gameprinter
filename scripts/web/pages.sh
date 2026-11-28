#!/bin/bash
# A built page made ready for GitHub Pages (docs/host-your-own.md, "Your own
# page on GitHub Pages"), for someone who would rather not use Cloudflare:
#   scripts/web/pages.sh PAGE_DIR OUT
#
# PAGE_DIR is a page a release check passed on as it is now (the same
# digest record the games repo's page/deploy.sh needs:
# $GP_HOME/work/page-release/passed/DIGEST; its page/release-check.sh
# --no-roms makes one without ROMs). OUT is the folder of
# the repo that will publish it, made if it isn't there; its site/ must be
# missing or empty. OUT gets:
#   site/                         the page's files, with p.html (a copy of
#                                 index.html: Pages serves /p from it, as
#                                 _redirects does on Cloudflare), .nojekyll,
#                                 and the Content-Security-Policy of the
#                                 page's _headers as a <meta> in each HTML
#                                 file, less frame-ancestors, which a <meta>
#                                 can't set. _headers and _redirects are left
#                                 out: Pages doesn't read them.
#   .github/workflows/pages.yml   publishes site/ to Pages on each push to
#                                 main (actions/upload-pages-artifact and
#                                 actions/deploy-pages).
# Then: commit OUT, push it to a GitHub repo, and set the repo's Settings,
# Pages, Source to "GitHub Actions". It prints the steps.
#
# What Pages can't do: it sends no headers of the page's choosing, so the
# page isn't cross-origin isolated (no Cross-Origin-Opener-Policy and
# Cross-Origin-Embedder-Policy). A game with its own module (every game
# with hooks, as every ported game has) is then refused with "this page wasn't
# served with its isolation headers, so it can't run a game module with its
# limits" (page/module-host.js); only a game that plays in the page's own
# interpreter (one `gp game new` made, with no hooks) plays. The policy as
# a <meta> covers what the page loads, but not frame-ancestors (any site
# may frame it), and Pages caches each file for ten minutes, so for a while
# after a push a visitor can get a new page.js beside an old file. Use
# Cloudflare (the games repo's page/deploy.sh) for a page that plays games
# with modules.
#
# Exit codes: 0 written, 1 no release check passed on PAGE_DIR as it is
# now, 2 couldn't run.
set -uo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
passed="$home/work/page-release/passed"
[ $# = 2 ] || { echo "usage: scripts/web/pages.sh PAGE_DIR OUT" >&2; exit 2; }
page="$1"
out="$2"
[ -d "$page" ] || { echo "pages: $page isn't a folder" >&2; exit 2; }
page="$(cd "$page" && pwd)"
for f in index.html page.json _headers game/module-host.js; do
  [ -e "$page/$f" ] || { echo "pages: $page has no $f, so it isn't a built page" >&2; exit 2; }
done
digest="$(python3 -I "$repo/scripts/web/page-digest.py" "$page")" || exit 2
if [ ! -f "$passed/$digest" ]; then
  echo "pages: no release check passed on this page as it is now (digest $digest; passes are recorded in $passed). Run the games repo's just page-release (--no-roms without ROMs); nothing written." >&2
  exit 1
fi
site="$out/site"
if [ -d "$site" ] && [ -n "$(ls -A "$site")" ]; then
  echo "pages: $site isn't empty; give a new OUT, or move its site/ away first" >&2
  exit 2
fi
mkdir -p "$site" "$out/.github/workflows" || exit 2

# The page's files, less the two Pages doesn't read.
rsync -a --exclude /_headers --exclude /_redirects "$page/" "$site/" || exit 2
cp "$page/index.html" "$site/p.html" || exit 2
: > "$site/.nojekyll"
python3 -I - "$page/_headers" "$site" <<'PY' || exit 2
import html, os, re, sys
headers, site = sys.argv[1:]
csp = None
for line in open(headers):
    m = re.match(r"\s+Content-Security-Policy:\s*(.+)$", line)
    if m:
        csp = m.group(1).strip()
if not csp:
    sys.exit("pages: the page's _headers has no Content-Security-Policy")
# A <meta> policy can't set frame-ancestors (browsers ignore it there and
# say so in the console).
csp = "; ".join(d.strip() for d in csp.split(";") if d.strip() and not d.strip().startswith("frame-ancestors"))
meta = f'<meta http-equiv="Content-Security-Policy" content="{html.escape(csp, quote=True)}">'
n = 0
for d, _, names in os.walk(site):
    for name in names:
        if not name.endswith(".html"):
            continue
        p = os.path.join(d, name)
        text = open(p, encoding="utf-8").read()
        if '<meta charset="utf-8">' not in text:
            sys.exit(f"pages: {p} has no <meta charset=\"utf-8\"> to put the policy after")
        open(p, "w", encoding="utf-8").write(text.replace('<meta charset="utf-8">', '<meta charset="utf-8">\n' + meta, 1))
        n += 1
print(f"the policy is a <meta> in {n} HTML files (less frame-ancestors)")
PY
cat > "$out/.github/workflows/pages.yml" <<'YML'
# Publishes site/ (a Game Printer page made by scripts/web/pages.sh) to
# GitHub Pages on each push to main. Set the repo's Settings, Pages, Source
# to "GitHub Actions" once.
name: pages
on:
  push:
    branches: [main]
  workflow_dispatch:
permissions:
  contents: read
  pages: write
  id-token: write
concurrency:
  group: pages
  cancel-in-progress: false
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: site
      - id: deployment
        uses: actions/deploy-pages@v4
YML
files="$(find "$site" -type f | wc -l | tr -d ' ')"
echo "Written for GitHub Pages: $files files in $site, and $out/.github/workflows/pages.yml"
echo "From the release check that passed on $page (digest $digest)."
echo
echo "To publish it:"
echo "  1. git -C $out init -b main (once), then git -C $out add -A and commit"
echo "  2. make a public GitHub repo, add it as origin, and push main"
echo "  3. in the repo's Settings, Pages, set Source to \"GitHub Actions\"; the workflow publishes on each push"
echo "  4. the page is then at https://OWNER.github.io/REPO/ (and /p?g=GAME for a game by its name)"
echo
echo "On Pages the page has no isolation headers, so a game with its own module (one with hooks, as every"
echo "ported game has) is refused; only a game with no hooks plays. Cloudflare (the games repo's just page-deploy)"
echo "serves the headers the page needs."
