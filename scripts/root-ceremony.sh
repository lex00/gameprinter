#!/bin/bash
# The official root key ceremony (docs/root-ceremony.md). Run it yourself, in
# your own terminal:
#
#   cd ~/checkouts/lex00/gameprinter && scripts/root-ceremony.sh
#
# ssh-keygen asks for the passphrase directly; this script never sees it.
# Safe to rerun if it stops: it reuses what was already made and never makes
# a second root key for a registry that has one.
#
# GP_CEREMONY_TEST_DIR rehearses the whole thing in a folder, with no bucket.

set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
gp="$repo/target/debug/gp"
bucket=gp-official-354867293429
url="https://$bucket.s3.us-east-2.amazonaws.com"
namespace=game-printer-package

if [ -n "${GP_CEREMONY_TEST_DIR:-}" ]; then
  keys="$GP_CEREMONY_TEST_DIR/keys"
  target="$GP_CEREMONY_TEST_DIR/registry"
  mkdir -p "$target"
else
  keys="$HOME/.gameprinter/keys/official"
  target="s3://$bucket"
fi
holding="$keys/root-to-store"
work="$(mktemp -d)"

finished=no
on_exit() {
  rm -rf "$work"
  if [ "$finished" != yes ]; then
    echo
    echo "The ceremony didn't finish. It's safe to run this script again."
    echo "If it stopped on an error, send that error to the maintainer first."
  fi
}
trap on_exit EXIT

stop() { echo; echo "Stopped: $*"; exit 1; }
yes_or_stop() {
  local a
  read -r -p "$1 [y/N] " a
  case "$a" in y|Y|yes|YES) ;; *) stop "you answered no." ;; esac
}

# Fetches a registry key as an outsider would, into a file. False if absent.
fetch() {
  if [ -n "${GP_CEREMONY_TEST_DIR:-}" ]; then
    [ -f "$target/$1" ] && cp "$target/$1" "$2"
  else
    [ "$(curl -s -o "$2" -w '%{http_code}' "$url/$1")" = 200 ]
  fi
}

verify_registry() {
  fetch registry.toml "$work/registry.toml" || stop "registry.toml can't be read back."
  fetch registry.toml.sig "$work/registry.toml.sig" || stop "registry.toml.sig can't be read back."
  ssh-keygen -q -Y check-novalidate -n "$namespace" -f "$1" -s "$work/registry.toml.sig" < "$work/registry.toml" >/dev/null \
    || stop "registry.toml isn't signed by this root key."
}

[ -x "$gp" ] || stop "$gp isn't built. Run: cargo build -p gp-cli"
if [ -z "${GP_CEREMONY_TEST_DIR:-}" ]; then
  aws sts get-caller-identity >/dev/null 2>&1 || stop "the AWS command line isn't signed in."
fi

already=no
if fetch registry.toml "$work/existing"; then already=yes; fi
if [ "$already" = yes ] && [ ! -f "$holding/root" ]; then
  [ -f "$keys/root.pub" ] || stop "the registry already has a root key, and this machine doesn't hold its public half."
  verify_registry "$keys/root.pub"
  finished=yes
  echo "This registry already has its root key, and it checks out:"
  ssh-keygen -lf "$keys/root.pub"
  exit 0
fi

cat <<TEXT
The official root key ceremony

This makes the root key, creates the official registry at
  $target
and has you store the key in your password manager.

You need a passphrase. It must NOT be stored in the same password manager as
the key: keep it on paper, or somewhere else entirely. If you lose either
one, the root key is gone for good.

TEXT
yes_or_stop "Do you have a passphrase, and somewhere for it apart from the password manager?"

mkdir -p "$keys" "$holding"
chmod 700 "$keys" "$holding"

if [ -f "$holding/root" ]; then
  echo "Using the root key already made in $holding."
else
  echo
  echo "Step 1 of 4. Making the root key. Type your passphrase when asked."
  ssh-keygen -q -t ed25519 -a 200 -f "$holding/root" -C root@gp-official
fi
if ssh-keygen -y -P '' -f "$holding/root" >/dev/null 2>&1; then
  rm -f "$holding/root" "$holding/root.pub"
  stop "that key has no passphrase, so it was deleted. Run this again and give it one."
fi

echo
echo "Step 2 of 4. Making the online keys (no passphrase; scheduled jobs use them)."
for k in signing lex00; do
  if [ -f "$keys/$k" ]; then
    echo "  $k: already made"
  else
    ssh-keygen -q -t ed25519 -N '' -f "$keys/$k" -C "$k@gp-official"
    echo "  $k: made"
  fi
done

echo
if [ "$already" = yes ]; then
  echo "Step 3 of 4. The registry already exists; checking it's signed by this key."
else
  echo "Step 3 of 4. Creating the registry. Type your passphrase twice more."
  "$gp" registry init "$target" \
    --name gp-official \
    --root-key "$holding/root" \
    --signing-key "$keys/signing" \
    --namespace "lex00=$keys/lex00"
fi
verify_registry "$holding/root.pub"
fetch namespaces/lex00.toml "$work/lex00.toml" || stop "the registry was only half made (no lex00 namespace). Send this to the maintainer."
echo "  read back from outside: signed by this root key"
cp "$holding/root.pub" "$keys/root.pub"

echo
echo "Step 4 of 4. Store the key in your password manager."
echo "  Attach both files to a new entry named \"Game Printer official root key\":"
echo "    $holding/root"
echo "    $holding/root.pub"
echo "  Put the passphrase somewhere else, not in that entry."
[ -n "${GP_CEREMONY_TEST_DIR:-}" ] || open "$holding"
echo
answer=""
while [ "$answer" != stored ]; do
  read -r -p "When the files are attached and saved, type stored: " answer
done

rm -f "$holding/root" "$holding/root.pub"
rmdir "$holding"

finished=yes
echo
echo "Done. The private root key is gone from this machine."
echo "Its public half is at $keys/root.pub. Fingerprint:"
ssh-keygen -lf "$keys/root.pub"
echo
echo "Tell the maintainer it's finished."
