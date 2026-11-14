#!/bin/bash
# The release check for the public page (docs/web.md, "Deploying the
# public page"): everything that must hold before the page is uploaded.
#   scripts/web/release-check.sh [--full] [--out DIR] [--game GAME_DIR]
#
# 0. Takes a snapshot of the page's source: a shallow clone of its repo
#    (lex00/gp-registry for the deployed page) at the commit its branch is
#    on now, into DIR.registry, and prints that commit. The page check reads
#    the clone for the whole run (--registry-folder), the page's installs and
#    the check's expectations alike, so a publish during the run changes
#    nothing. A clone that fails ends the run (exit 2): the check never falls
#    back to the live registry. A page whose source is its own address
#    (GP_PAGE_SOURCE=local) has nothing to clone.
# 1. Builds the page from GAME_DIR (default ../games/donkey-kong, beside
#    this repo) at this checkout's tip into a folder of its own,
#    $GP_HOME/work/page-release/<commit>-games-<games commit>, each commit
#    with -dirty-<digest of the changes> when that repo has uncommitted
#    changes (a new folder for each, so no file is left from an older
#    build). build-page.sh's last step, `gp template served`, checks that
#    no file of the page or of its play files holds the ROM's bytes,
#    lifted files, pictures made from them, or a game image.
# 2. The page's headless check (check-page.mjs) under the page's _headers.
# 3. The lift under Node (lift.test.mjs) on the page's own lift files, and
#    the page check's theme line rule (theme-line.test.mjs).
# 4. `gp package scan` on the game's package: it must have no needs-rewrite
#    item, and must have been compared with the ROMs.
# 5. The page's size, its files and their SHA-256, also written to
#    DIR.manifest.txt (beside the page, so it isn't uploaded).
# It runs every step, prints each result and its time, and ends with PASS
# or FAIL.
#
# On PASS it records the page: $GP_HOME/work/page-release/passed/DIGEST,
# where DIGEST is scripts/web/page-digest.py's over the page's folder.
# scripts/web/deploy.sh uploads a folder only when its digest has such a
# record, so what goes up is what passed, file for file.
#
# The modules and gp-web-prep's output are reused when none of the files
# they were made from changed (build-preview.sh and build-lift.sh,
# GP_WEB_REUSE=1), and so is a package scan with nothing to rewrite.
# Reused results are printed with "kept". --full reuses nothing: it makes
# and checks everything again, and keeps the results for the next run.
# deploy.sh always runs --full. The page check runs every section each
# time either way (check-page.mjs says why).
#
# GP names the gp to run (default target/debug/gp); WEB_TOOLS,
# GP_PAGE_SOURCE and GP_PAGE_CONTENT are as for build-page.sh, and
# GP_PAGE_CONTENT is lex00/turnips when it isn't set, as on the deployed
# page. Exit codes: 0 passed, 1 a check failed, 2 couldn't run.
# The last line on success or failure is "bundle: DIR".

set -uo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
home="${GP_HOME:-$HOME/.gameprinter}"
gp="${GP:-$repo/target/debug/gp}"

full=0
out=""
game="$repo/../games/donkey-kong"
while [ $# -gt 0 ]; do
  case "$1" in
    --full) full=1; shift ;;
    --out) [ -n "${2:-}" ] || { echo "release-check: --out needs a folder" >&2; exit 2; }; out="$2"; shift 2 ;;
    --game) [ -n "${2:-}" ] || { echo "release-check: --game needs a folder" >&2; exit 2; }; game="$2"; shift 2 ;;
    *) echo "usage: scripts/web/release-check.sh [--full] [--out DIR] [--game GAME_DIR]" >&2; exit 2 ;;
  esac
done
[ -f "$game/package.toml" ] || { echo "release-check: $game is no game package (no package.toml)" >&2; exit 2; }
game="$(cd "$game" && pwd)"
# A repo's commit, and the digest of its uncommitted changes (tracked and
# new, by content) when it has any.
version() {
  local id changes
  id="$(git -C "$1" rev-parse --short=8 HEAD)" || return 1
  if [ -n "$(git -C "$1" status --porcelain)" ]; then
    changes="$(cd "$1" && { git diff HEAD --binary; git ls-files -o --exclude-standard -z | xargs -0 shasum -a 256; } | shasum -a 256 | cut -c1-8)"
    id="$id-dirty-$changes"
  fi
  echo "$id"
}
games_repo="$(git -C "$game" rev-parse --show-toplevel)" || { echo "release-check: $game isn't in a git checkout" >&2; exit 2; }
id="$(version "$repo")-games-$(version "$games_repo")" || exit 2
[ -n "$out" ] || out="$home/work/page-release/$id"
if [ $full = 1 ]; then export GP_WEB_REUSE=save; else export GP_WEB_REUSE=1; fi
# The theme the page installs with the game: the deployed page's, Turnips,
# unless GP_PAGE_CONTENT says otherwise (set and empty for none).
export GP_PAGE_CONTENT="${GP_PAGE_CONTENT-lex00/turnips}"
reuse="$repo/scripts/web/reuse.py"

fail=()
now() { perl -MTime::HiRes=time -e 'printf "%.1f", time'; }
start=$(now)
t0=$start
took() { perl -e "printf '%.1f', $(now) - $1"; }
step() {
  [ -n "${current:-}" ] && echo "time  $current: $(took "$t0") s"
  t0=$(now)
  current="$1"
  echo; echo "== $1"
}

step "a snapshot of the page's source"
source_name="${GP_PAGE_SOURCE:-registry}"
source_url="$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))[sys.argv[2]]["url"])' "$repo/scripts/web/sources.toml" "$source_name" 2>/dev/null)" \
  || { echo "release-check: scripts/web/sources.toml has no source named $source_name" >&2; exit 2; }
registry_folder=()
case "$source_url" in
  .) echo "ok: the page's source is its own address ($source_name), so there is no registry to clone" ;;
  https://raw.githubusercontent.com/*/*/*)
    rest="${source_url#https://raw.githubusercontent.com/}"
    source_repo="$(echo "$rest" | cut -d/ -f1-2)"
    source_branch="${rest#*/*/}"
    mkdir -p "$(dirname "$out")" || exit 2
    snapshot="$(cd "$(dirname "$out")" && pwd)/$(basename "$out").registry"
    # An earlier run's snapshot for this build is cleared first, as
    # build-page.sh clears the folders it makes.
    rm -rf "$snapshot"
    if ! GIT_TERMINAL_PROMPT=0 git clone --quiet --depth 1 --single-branch --branch "$source_branch" "https://github.com/$source_repo.git" "$snapshot"; then
      echo "FAIL: couldn't clone $source_repo to take a snapshot of the page's source (is the network down?), so the page can't be checked. The check doesn't read the live registry instead."
      echo "bundle: $out"
      exit 2
    fi
    echo "ok: $source_repo is at commit $(git -C "$snapshot" rev-parse HEAD) on $source_branch; the page and the check read this copy for the whole run ($snapshot)"
    registry_folder=(--registry-folder "$snapshot") ;;
  *)
    echo "FAIL: the page's source $source_url isn't a GitHub repo's raw address, so the check can't take a snapshot of it. The check doesn't read the live registry instead."
    echo "bundle: $out"
    exit 2 ;;
esac

step "build the page and check it for game data ($out)"
mkdir -p "$out" || exit 2
GP="$gp" "$repo/scripts/web/build-page.sh" "$game" "$out"
s=$?
case $s in
  0) echo "ok: built; no file holds a ROM's bytes, lifted files or pictures made from them" ;;
  1) fail+=("the page holds game data (gp template served)") ;;
  *) fail+=("the page could not be built or checked (build-page.sh exit $s; gp template served needs the ROMs in GP_ROM_DIR)") ;;
esac
if [ ! -f "$out/index.html" ]; then
  echo "FAIL: no page"; echo "bundle: $out"; exit 2
fi
out="$(cd "$out" && pwd)"
built="$(python3 -I "$repo/scripts/web/page-digest.py" "$out")" || exit 2

step "the page in a headless browser, under the page's _headers"
browser="$("$repo/scripts/inspector/fetch-browser.sh" "$home")" || { echo "no headless browser"; exit 2; }
if [ -z "${PLAYWRIGHT_FROM:-}" ] && [ ! -d "$repo/scripts/web/node_modules/playwright-core" ]; then
  npm ci --prefix "$repo/scripts/web" --silent --no-audit --no-fund || exit 2
fi
page_full=""
[ $full = 1 ] && page_full="--full"
node "$repo/scripts/web/public/check-page.mjs" --dist "$out" --browser "$browser" --shots "$out.shots" $page_full ${registry_folder[@]+"${registry_folder[@]}"}
s=$?
[ $s -eq 0 ] || fail+=("the page's headless check (exit $s; shots in $out.shots)")

step "the lift under Node, on the page's lift files, and the page check's theme line rule"
# build-page.sh splits the lift: the module stays with the page, the
# folder it reads goes to the play files. The test reads them together.
lift="$out.lift-test"
mkdir -p "$lift/lift" "$lift/work" || exit 2
cp -R "$out/game/lift/gp-web-lift.wasm" "$out.play/lift/"* "$lift/lift/" || exit 2
GP_WEB_LIFT_OUT="$lift" GP_WORKER_OUT="$lift/work" node --test "$repo/scripts/web/public/lift.test.mjs"
s=$?
[ $s -eq 0 ] || fail+=("the lift's tests (lift.test.mjs, exit $s)")
node --test "$repo/scripts/web/public/theme-line.test.mjs"
s=$?
[ $s -eq 0 ] || fail+=("the page check's theme line rule (theme-line.test.mjs, exit $s)")

step "gp package scan on the game's package"
# A scan reads the package's folder, the ROMs in $GP_ROM_DIR and gp. Only
# a complete scan with nothing to rewrite is kept.
gp_bin="$(cd "$(dirname "$gp")" && pwd)/$(basename "$gp")"
name="$(basename "$game")"
scan_in=(--file "$gp_bin" --listing "${GP_ROM_DIR:-$home/roms}" --tree "$game")
if [ $full = 0 ] && from="$(python3 -I "$reuse" find "package-scan-$name" --extra "$name")"; then
  json="$(cat "$from"/out/*.json)"
  echo "kept  $name: the scan of $(sed -n 's/.*"made": "\([^"]*\)".*/\1/p' "$from/inputs.json"), none of the files it read changed"
else
  json="$("$gp" --json package scan "$game" 2>/dev/null)" || true
  printf '%s\n' "$json" > "$out.scan-$name.json"
  if python3 -I -c 'import json, sys; d = json.load(open(sys.argv[1])); sys.exit(0 if d["rewrite"] == 0 and d["complete"] else 1)' "$out.scan-$name.json" 2>/dev/null; then
    python3 -I "$reuse" save "package-scan-$name" --extra "$name" "${scan_in[@]}" --out "$out.scan-$name.json" >/dev/null
  fi
fi
python3 -I - "$name" "$json" <<'PY' || fail+=("package scan: $name has needs-rewrite items or wasn't compared with its ROMs (above)")
import json, sys
t, raw = sys.argv[1:]
try:
    d = json.loads(raw)
except Exception:
    print(f"{t}: no scan result"); sys.exit(1)
print(f"{t}: needs rewrite {d['rewrite']} items; compared with {', '.join(d['compared']) or 'no ROM'}")
for part in d["parts"]:
    for f in part.get("files", []):
        if f.get("rewrite"):
            print(f"  {f['file']}: {f['rewrite']} {f.get('unit', '')}")
if not d["complete"]:
    print(f"  not compared: {', '.join(d['not_compared'])} (not in GP_ROM_DIR)")
sys.exit(0 if d["rewrite"] == 0 and d["complete"] else 1)
PY

step "page size and files"
python3 -I - "$out" "$out.manifest.txt" <<'PY' || fail+=("page over a Workers static assets limit")
import hashlib, os, sys
root, manifest = sys.argv[1:]
rows, total = [], 0
for d, _, names in os.walk(root):
    for n in names:
        p = os.path.join(d, n)
        b = open(p, "rb").read()
        rows.append((os.path.relpath(p, root), len(b), hashlib.sha256(b).hexdigest()))
        total += len(b)
rows.sort()
with open(manifest, "w") as f:
    for r, n, h in rows:
        f.write(f"{h}  {n:>9}  {r}\n")
for r, n, h in rows:
    print(f"{h[:16]}  {n:>9}  {r}")
big = max(rows, key=lambda r: r[1])
print(f"{len(rows)} files, {total/1048576:.2f} MiB; largest {big[0]} ({big[1]/1048576:.2f} MiB); manifest in {manifest}")
# Workers static assets: 20000 files, 25 MiB each.
ok = len(rows) <= 20000 and big[1] <= 25 * 1048576
print("ok: within the Workers static assets limits (20000 files, 25 MiB a file)" if ok else "FAIL: over a limit")
sys.exit(0 if ok else 1)
PY
# Nothing may have changed the page while it was checked.
checked="$(python3 -I "$repo/scripts/web/page-digest.py" "$out")" || exit 2
[ "$checked" = "$built" ] || fail+=("the page's files changed during the check (digest $built after the build, $checked now)")

echo "time  $current: $(took "$t0") s"
echo "time  the release check: $(took "$start") s"
echo
if [ ${#fail[@]} -eq 0 ]; then
  passed="$home/work/page-release/passed"
  mkdir -p "$passed" || exit 2
  printf 'bundle %s\nid %s\ngame %s\nfull %s\nwhen %s\n' "$out" "$id" "$game" "$full" "$(date '+%Y-%m-%dT%H:%M:%S')" > "$passed/$checked"
  echo "PASS: the page can be deployed (digest $checked, recorded in $passed)"
  echo "bundle: $out"
  exit 0
fi
echo "FAIL: ${#fail[@]} problem(s)"
for f in "${fail[@]}"; do echo "  - $f"; done
echo "bundle: $out"
exit 1
