#!/bin/bash
# Builds a released gp for this machine, or for TARGET: one binary that
# carries the files it reads at run time (crates/gp-cli/src/bundle.rs), in
# an archive with its SHA-256, ready to hand to a theme or game maker
# (issue #11):
#   scripts/release/build.sh OUT [--target TARGET] [PAGE_DIR]
#
# TARGET is a Rust target: aarch64-apple-darwin, x86_64-apple-darwin,
# x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu or
# x86_64-pc-windows-msvc. The default is $GP_RELEASE_TARGET, else this
# machine's. Building for another system needs that system's linker, so the
# release workflow builds each on its own runner. On Windows it runs in Git
# Bash (the shell GitHub's Windows runners have).
#
# OUT gets:
#   bundle/                      what the binary carries, laid out as a checkout:
#     editor/dist/               the editor (gp serve at /), from editor/dist
#     mechanic/                  the game's page (gp serve at /mechanic/GAME/)
#     scripts/web/page/          the preview's files
#     target/play-web/           the two players, built here with build-play.sh
#     templates/, baselines/     the templates gp mcp forks
#     page/                      the public page, a bundle that passed the release check
#     module-kit.zip             the module kit (gp game kit), for gp game build
#   gp-VERSION-COMMIT-TARGET/    gp (gp.exe on Windows), LICENSE, THIRD-PARTY.md, README.txt
#   gp-VERSION-COMMIT-TARGET.tar.gz   the archive (.zip for a Windows target)
#   gp-VERSION-COMMIT-TARGET.tar.gz.sha256
#   times.txt                    each step's time
#
# PAGE_DIR is the built public page to carry: default the newest bundle that
# passed the release check ($GP_HOME/work/page-release/passed), which
# `just web-page-release` makes. A gp with no page still builds, and says
# in gp self check that it carries none.
#
# It publishes nothing: no tag, no upload, no push. The editor must be
# built (just editor). Needs cargo with the wasm32-unknown-unknown target
# and TARGET, zip (or 7z or PowerShell, on Windows), and cargo-about for the third-party notices (skipped with a note
# when it isn't installed).
#
# Exit codes: 0 built, 2 couldn't build.

set -euo pipefail
trap 'exit 2' ERR

repo="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:?usage: scripts/release/build.sh OUT [--target TARGET] [PAGE_DIR]}"
shift
home="${GP_HOME:-$HOME/.gameprinter}"
host="$(rustc -vV | sed -n 's/^host: //p')"
target="${GP_RELEASE_TARGET:-$host}"
page=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target) target="${2:?--target needs a Rust target}"; shift 2 ;;
    --target=*) target="${1#--target=}"; shift ;;
    -*) echo "release: unknown option $1" >&2; exit 2 ;;
    *) page="$1"; shift ;;
  esac
done
exe=""
host_exe=""
case "$target" in *windows*) exe=".exe" ;; esac
case "$host" in *windows*) host_exe=".exe" ;; esac
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)"
commit="$(git -C "$repo" rev-parse --short=12 HEAD)"
if [ -n "$(git -C "$repo" status --porcelain --untracked-files=no)" ]; then
  commit="$commit-dirty"
fi
name="gp-$version-$commit-$target"

mkdir -p "$out"
out="$(cd "$out" && pwd)"
stage="$out/bundle"
if [ -e "$stage" ]; then
  echo "release: $stage exists; give an empty OUT" >&2
  exit 2
fi
mkdir -p "$stage"
: > "$out/times.txt"
now() { perl -MTime::HiRes=time -e 'printf "%.1f", time'; }
t0=$(now)
lap() { local t; t=$(now); echo "$1: $(perl -e "printf '%.1f', $t - $t0") s" | tee -a "$out/times.txt"; t0=$t; }

# The files gp reads from a checkout at run time.
if [ ! -f "$repo/editor/dist/index.html" ]; then
  echo "release: editor/dist isn't built; run just editor" >&2
  exit 2
fi
mkdir -p "$stage/editor" "$stage/scripts/web" "$stage/target/play-web"
cp -R "$repo/editor/dist" "$stage/editor/dist"
# tar, not rsync, so this runs in Git Bash on Windows too.
mkdir -p "$stage/mechanic"
tar -C "$repo/mechanic" --exclude '*.test.mjs' -cf - . | tar -C "$stage/mechanic" -xpf -
cp -R "$repo/scripts/web/page" "$stage/scripts/web/page"
cp -R "$repo/templates" "$stage/templates"
cp -R "$repo/baselines" "$stage/baselines"
lap "copy the page's files"

"$repo/scripts/web/build-play.sh" "$stage/target/play-web" > /dev/null
"$repo/scripts/web/build-play.sh" --arcade "$stage/target/play-web" > /dev/null
lap "build the players"

if [ -z "$page" ]; then
  best=""
  for f in "$home/work/page-release/passed"/*; do
    [ -f "$f" ] || continue
    b="$(sed -n 's/^bundle //p' "$f")"
    w="$(sed -n 's/^when //p' "$f")"
    if [ -f "$b/index.html" ] && [[ "$w" > "${bestw:-}" ]]; then best="$b"; bestw="$w"; fi
  done
  page="$best"
fi
if [ -n "$page" ]; then
  cp -R "$page" "$stage/page"
  echo "release: carrying the page $page"
else
  echo "release: no page that passed the release check here; the binary carries none (just web-page-release makes one)"
fi

# zip_here ZIP PATH: PATH, under the current folder, into ZIP, with
# whichever zip this machine has: zip on macOS and Linux; Git Bash on
# Windows has none, but 7-Zip or PowerShell.
zip_here() {
  if command -v zip > /dev/null; then
    zip -qrX "$1" "$2"
  elif command -v 7z > /dev/null; then
    7z a -tzip -bso0 -bsp0 "$1" "$2" > /dev/null
  else
    # Compress-Archive -Path . would put the folder's own name in front.
    local what="Get-Item -Force '$2'"
    [ "$2" = . ] && what="Get-ChildItem -Force ."
    powershell.exe -NoProfile -Command "$what | Compress-Archive -DestinationPath '$(cygpath -w "$1")'"
  fi
}

# The module kit, made by a gp built from this checkout for this machine.
cargo build -q --locked --profile fastdev -p gp-cli
kit="$out/kit"
"${CARGO_TARGET_DIR:-$repo/target}/fastdev/gp$host_exe" game kit --out "$kit" > /dev/null
(cd "$kit" && zip_here "$stage/module-kit.zip" .)
lap "make the module kit"

# The binary.
GP_BUNDLE_DIR="$stage" GP_RELEASE_COMMIT="$commit" \
  cargo build -q --locked --release -p gp-cli --features bundle --target "$target"
bin="${CARGO_TARGET_DIR:-$repo/target}/$target/release/gp$exe"
lap "build gp ($target, release, carrying $(du -sh "$stage" | cut -f1))"

dist="$out/$name"
mkdir -p "$dist"
cp "$bin" "$dist/gp$exe"
cp "$repo/LICENSE" "$dist/LICENSE"
if command -v cargo-about > /dev/null; then
  (cd "$repo" && cargo about generate --locked --offline --fail \
    -c scripts/release/about.toml -m crates/gp-cli/Cargo.toml --target "$target" \
    scripts/release/about.hbs -o "$dist/THIRD-PARTY.md")
else
  echo "release: cargo-about isn't installed, so THIRD-PARTY.md is missing" >&2
fi
cat > "$dist/README.txt" <<EOF
gp $version ($commit) for $target, the Game Printer tool.

Put gp$exe somewhere on your PATH and run:  gp self check
It says what works on this machine and what each missing piece is for.
Install steps and connecting an AI agent (gp mcp):
https://lex00.github.io/gameprinter/install/

gp is Apache-2.0 (LICENSE). The crates it is built from are listed with
their licences in THIRD-PARTY.md.
EOF
chmod 755 "$dist" "$dist/gp$exe"
chmod 644 "$dist/LICENSE" "$dist/README.txt" "$dist"/THIRD-PARTY.md 2> /dev/null || true
# A .zip for Windows, which opens one itself; a .tar.gz elsewhere. Either
# holds the one folder gp-VERSION-COMMIT-TARGET/.
if [ -n "$exe" ]; then
  archive="$name.zip"
  (cd "$out" && zip_here "$archive" "$name")
else
  archive="$name.tar.gz"
  (cd "$out" && tar -czf "$archive" "$name")
fi
# shasum is Perl's (macOS, most Linux); sha256sum prints the same line.
if command -v shasum > /dev/null; then
  (cd "$out" && shasum -a 256 "$archive" > "$archive.sha256")
else
  (cd "$out" && sha256sum "$archive" > "$archive.sha256")
fi
lap "archive"
echo "release: $out/$archive ($(du -h "$out/$archive" | cut -f1); gp itself $(du -h "$bin" | cut -f1))"
cat "$out/$archive.sha256"
