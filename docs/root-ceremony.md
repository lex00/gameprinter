# The official root key ceremony

The official registry's root key is made once, by its owner, and stays offline. Every player trusts this key, and it can't be replaced without shipping a new page. Do these steps yourself, in your own terminal. Nobody else, and no tool or agent, should see the private key or its passphrase.

You need one removable drive for the key and a second for the sealed backup. Pick a long passphrase and write it on paper.

The official registry is `s3://gp-official-354867293429`. The bucket exists and is empty.

## 1. Make the root key on the removable drive

Change `ROOT_DRIVE` to your drive's name.

```sh
ROOT=/Volumes/ROOT_DRIVE/gp-official
mkdir -p "$ROOT"
ssh-keygen -t ed25519 -a 200 -f "$ROOT/root" -C root@gp-official
```

Give it the passphrase when asked. Never make this key on the internal disk.

## 2. Make the two online keys

The signing key re-signs the registry's list files every week. The `lex00` key signs your packages. They live on this machine and have no passphrase, because scheduled jobs use them. If one leaks, the root key can replace it.

```sh
KEYS=~/.gameprinter/keys/official
mkdir -p "$KEYS"
chmod 700 "$KEYS"
ssh-keygen -t ed25519 -N '' -f "$KEYS/signing" -C signing@gp-official
ssh-keygen -t ed25519 -N '' -f "$KEYS/lex00" -C lex00@gp-official
```

## 3. Sign the registry into existence

This asks for the root passphrase twice, once for `registry.toml` and once for the `lex00` namespace file, and writes both to the bucket. Run it once.

```sh
cd ~/checkouts/lex00/gameprinter
target/debug/gp registry init s3://gp-official-354867293429 \
  --name gp-official \
  --root-key "$ROOT/root" \
  --signing-key "$KEYS/signing" \
  --namespace "lex00=$KEYS/lex00"
```

Save the fingerprint it prints. Then print the public key, which is safe to share:

```sh
cat "$ROOT/root.pub"
```

## 4. Make the sealed backup

Copy the key to the second drive, then eject both.

```sh
BACKUP=/Volumes/BACKUP_DRIVE/gp-official
mkdir -p "$BACKUP"
cp "$ROOT/root" "$ROOT/root.pub" "$BACKUP/"
diff -r "$ROOT" "$BACKUP" && echo "backup matches"
diskutil eject /Volumes/BACKUP_DRIVE
diskutil eject /Volumes/ROOT_DRIVE
```

Seal the backup drive in an envelope and store it apart from the first drive. Keep the paper with the passphrase in a third place, never with either drive.

## 5. Hand back the public parts

Give the maintainer the output of `cat root.pub` and the fingerprint. They pin the public key in `scripts/web/sources.toml` and run `gp registry check` from outside. The private key is not needed again until a namespace is added or a signing key is replaced.
