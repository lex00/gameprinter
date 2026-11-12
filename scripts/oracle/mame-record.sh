#!/bin/bash
# Runs one capture in the MAME oracle image (docs/oracle-mame.md):
#   scripts/oracle/mame-record.sh IMAGE ROMSET LOG OUT [SHOT_EVERY]
# ROMSET is a MAME romset zip named for its system (rbtapper.zip). LOG is an
# input log in the gp-arcade-input format. OUT is a folder name under
# $GP_HOME/work/oracle/, or a path inside it; it must not exist yet.
# SHOT_EVERY writes a PNG every N frames (default 30, one a second for
# Tapper; 0 for none).
#
# The romset is mounted read-only into the container for this run only; it
# never goes into the image or the output. Everything the run writes is
# derived from the ROM, so the output must sit under $GP_HOME/work/oracle/,
# never in a repo. MAME's cfg and nvram folders are a tmpfs inside the
# container, empty at the start of every run and gone at its end.
#
# Exit codes: 0 captured, 1 the capture failed, 2 couldn't run.

set -euo pipefail

if [ $# -lt 4 ] || [ $# -gt 5 ]; then
  echo "usage: scripts/oracle/mame-record.sh IMAGE ROMSET LOG OUT [SHOT_EVERY]" >&2
  exit 2
fi
image="$1"
romset="$2"
log="$3"
out_arg="$4"
shot_every="${5:-30}"
gp_home="${GP_HOME:-$HOME/.gameprinter}"
oracle_dir="$gp_home/work/oracle"
script_dir="$(cd "$(dirname "$0")" && pwd)"

die() { echo "mame-record: $*" >&2; exit 2; }

[ -f "$romset" ] || die "no romset at $romset"
case "$romset" in
  *.zip) ;;
  *) die "the romset must be a zip named for its MAME system, like rbtapper.zip" ;;
esac
[ -f "$log" ] || die "no input log at $log"
case "$shot_every" in
  ''|*[!0-9]*) die "SHOT_EVERY must be a whole number" ;;
esac
command -v docker >/dev/null || die "docker isn't installed"
docker image inspect "$image" >/dev/null 2>&1 || die "no image $image: run just mame-build"

romset="$(cd "$(dirname "$romset")" && pwd)/$(basename "$romset")"
log="$(cd "$(dirname "$log")" && pwd)/$(basename "$log")"
system="$(basename "$romset" .zip)"

mkdir -p "$oracle_dir"
oracle_dir="$(cd "$oracle_dir" && pwd -P)"
case "$out_arg" in
  /*) out="$out_arg" ;;
  */*) out="$(pwd)/$out_arg" ;;
  *) out="$oracle_dir/$out_arg" ;;
esac
[ ! -e "$out" ] || die "$out exists already; pick a new name"
mkdir -p "$out/shots"
out="$(cd "$out" && pwd -P)"
case "$out/" in
  "$oracle_dir"/?*) ;;
  *) rmdir "$out/shots" "$out"; die "the output must be under $oracle_dir: it's derived from the ROM" ;;
esac
if git -C "$out" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  rmdir "$out/shots" "$out"
  die "$out is inside a git work tree; captures are derived from the ROM and stay out of repos"
fi

start=$SECONDS
set +e
docker run --rm --network none \
  --user "$(id -u):$(id -g)" \
  --tmpfs /work:rw,size=16m \
  -v "$romset:/roms/$system.zip:ro" \
  -v "$script_dir/mame-capture.lua:/oracle/mame-capture.lua:ro" \
  -v "$log:/oracle/input.log:ro" \
  -v "$out:/out" \
  -e GP_ORACLE_LOG=/oracle/input.log \
  -e GP_ORACLE_OUT=/out \
  -e GP_ORACLE_SHOT_EVERY="$shot_every" \
  "$image" "$system" \
  -rompath /roms \
  -autoboot_script /oracle/mame-capture.lua \
  -noreadconfig -nowriteconfig -noplugins -nocheat -noautosave \
  -skip_gameinfo \
  -video none -sound none \
  -nothrottle -frameskip 0 \
  -keyboardprovider none -mouseprovider none -joystickprovider none -lightgunprovider none \
  -cfg_directory /work/cfg -nvram_directory /work/nvram \
  -input_directory /work/inp -state_directory /work/sta -diff_directory /work/diff \
  -comment_directory /work/comments -share_directory /work/share \
  -snapshot_directory /out/shots -snapview native -snapsize auto -nosnapbilinear \
  > "$out/mame.log" 2>&1
rc=$?
set -e
took=$((SECONDS - start))

if [ -s "$out/error.txt" ]; then
  echo "mame-record: the capture script failed: $(cat "$out/error.txt")" >&2
  exit 1
fi
if [ "$rc" -ne 0 ] || [ ! -f "$out/capture.txt" ]; then
  echo "mame-record: MAME exited with $rc before the capture finished; see $out/mame.log" >&2
  exit 1
fi

# A digest of each output file, for comparing two runs.
(cd "$out" && find . -type f ! -name mame.log ! -name SHA256SUMS | LC_ALL=C sort \
  | xargs shasum -a 256 > SHA256SUMS)

frames=$(awk '$1 == "frames" { print $2 }' "$out/capture.txt")
mismatches=$(awk '$1 == "input_mismatches" { print $2 }' "$out/capture.txt")
echo "mame-record: $frames frames of $system in ${took}s into $out (input mismatches: $mismatches)"
