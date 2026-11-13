# Packages

A package is a published version of a folder, a game's (kind `game`) or a theme's (kind `content`), in one format for both: its files, each stored once as a blob named by the BLAKE3 hash of its bytes, and a manifest that names them. The package id is the BLAKE3 hash of the manifest's bytes, so one id pins every byte of every file. The code is `crates/gp-package`, which depends on `blake3` alone, so `gp`, the web player and the registry reader in the page read packages with the same code. It came from game-printer's docs/packages.md; the format is the same, and game-printer's local package store, its mirrors (`mirrors.toml`) and `gp theme publish` are replaced by registries ([Run a registry](../site/src/content/docs/registry.md)).

A game's package starts as its folder in the games checkout (`games/donkey-kong`, which is also the game's template), with `package.toml` at its root. `gp registry publish` turns a folder into a package and writes it to a registry; `gp game fetch` reads one back into a working copy and lifts each ROM entry it finds in the ROM folder.

## The manifest

```text
gp-package 1
kind content
name me/my-theme
file 22c8973a...653f 8766 checks.json
file 93e74fb7...a7e2 176 look/frame-46-0.png
file 497e5ced...d326 6256 theme/theme.toml
```

| Line | Means |
|---|---|
| `gp-package 1` | The format |
| `kind` | `game` or `content`, as `gp registry publish --kind` gives it (`content` by default) |
| `name` | The package's name, `NS/NAME` |
| `file HASH SIZE PATH` | A file: the BLAKE3 hash of its bytes (64 hex digits), its size in bytes, and its path, relative with `/` between parts. Never absolute, never with `.`, `..`, an empty part, a backslash or a control character |

The files are sorted by path, each path once, and nothing else is in the text: `Manifest::read` refuses a manifest that `Manifest::write` wouldn't write the same, so a set of files has one manifest and one id. Two paths with the same bytes are one blob.

## Hook code

A game whose model has hooks keeps their Rust code under `ext/`. The hook hash is the BLAKE3 of the `ext/` files' lines of the manifest (hash, size and path, the `ext/` taken off), so it changes with any byte of the hook code and with nothing else. A player built with that code is built with this hash, and a recording names it. `gp package hooks DIR` prints it for a package folder.

## Where packages are kept

A registry holds the same keys on every target it can live on (a folder, a git repo, an S3 bucket):

| Key | What |
|---|---|
| `blobs/<hash>` | A file's bytes, as they are |
| `packages/<id>` | A manifest |
| `packages/<id>.sig` | The manifest's SSH signature, when the publish was signed |
| `index/<ns>/<name>/<version>.toml` | The version file: the id, kind, title, size, the publish time, the signer and, for content, the games it is for |

Every key but the list files (`catalog.toml`, `by-game/` and `yanks.toml`, which `gp registry reindex` rewrites) is written once: a key that is there holds the same bytes, since its name is their hash. A publish writes the blobs first, then the manifest, then the version file, so a reader never sees a version with parts missing. Every read is checked: a blob against its hash and its size in the manifest, a manifest against the id it was asked for by. A mismatch stops the read.

Blobs are stored as they are, not compressed: a theme is mostly PNG and WAV files, which are compressed already, and a plain blob is checked against its key with no decoder, which the browser needs.

[Where a registry can live](registry-backends.md) has each target and what makes a key write-once on it.

## Signing

A manifest is signed with an SSH key in the namespace `game-printer-package` (`ssh-keygen -Y sign`), and the signature is stored at `packages/<id>.sig`. Signing is off unless the registry's operator opts in, and a reader checks signatures only for a registry it holds a root key for (`--root` for `gp`, the source's `root` line for the page). Hashes are checked either way. [Where a registry can live](registry-backends.md#signing-is-opt-in) says how to turn it on.

## What a package may hold

A game's package holds no byte of the game's ROM. Two commands check that before a version is published, and the gate runs the first on every patch:

- `gp package scan DIR` sorts every file and field of the folder into ours, reference, from the ROM and needs rewrite, and rebuilds what comes from the ROM from the person's own copy ([the package scan](package-scan.md)).
- `gp template served DIR` checks the files a page serves: no line of 24 characters or more of the lifted files, and no run of 32 of the ROM's bytes, raw or written as numbers.

A registry in a git repo also compares every file of a package with each ROM it names before it commits, and refuses on 32 matching bytes or a ROM it can't find.

## Tests

| Test | What |
|---|---|
| `gp_package::tests` | A manifest round trip and its id; the same files in another order give the same id; bad paths and texts refused; a blob checked by size and hash |
| `gp_package::store::tests` | A package through a folder store and back; two paths with the same bytes as one blob; a key written once; a blob or manifest changed on disk refused |
| `gp_package::index::tests` | The themes index round trip; a bad index refused; keys from a newer gp passed over |
| `gp_package::registry::tests` | A package published, listed and read back; a wrong root, an edited file, a stranger's signature, a changed blob or manifest and an expired list refused; a version published once; attestations written once; a registry with no keys read with no signatures and its hashes still checked |
