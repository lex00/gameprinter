<!-- Brought from game-printer's docs/packages.md with the theme commands (wave 8 of the
port), which read and write this format. Paths under templates/GAME are a game's
folder in the games checkout in gameprinter (games/GAME). -->

# Theme packages

A package is one version of a model or a theme, in one format for both: its files, each stored once as a blob named by the BLAKE3 hash of its bytes, and a manifest that names them. The package id is the BLAKE3 hash of the manifest's bytes, so one id pins every byte of every file. This is the format decided on 2026-11-04 (docs/research/model-data-storage.md, option C, and docs/research/package-hosting.md, "Deduplication"). The code is `crates/gp-package`, which depends on `blake3` alone, so gp, the web preview and the generated Bevy games read packages with the same code.

Themes are the first packages ([theme publishing](theme-publishing.md)). A theme package is a look package (`theme/`, the `look/` gp drew, `checks.json`) for a game with no ROM art, or a source package (`theme/` and `checks.json` only) for a game lifted from the player's ROM, so that no picture drawn from a ROM's art is ever in a package. Model packages (`gp game publish`, `gp game fetch`) use the same crate when they are built.

## The manifest

```text
gp-package 1
kind theme
name night
file 22c8973a...653f 8766 checks.json
file 93e74fb7...a7e2 176 look/frame-46-0.png
file 497e5ced...d326 6256 theme/theme.toml
```

| Line | Means |
|---|---|
| `gp-package 1` | The format |
| `kind` | `theme` or `model` |
| `name` | The theme's or model's id |
| `file HASH SIZE PATH` | A file: the BLAKE3 hash of its bytes (64 hex digits), its size in bytes, and its path, relative with `/` between parts. Never absolute, never with `.`, `..`, an empty part, a backslash or a control character |

The files are sorted by path, each path once, and nothing else is in the text: `Manifest::read` refuses a manifest that `Manifest::write` wouldn't write the same, so a set of files has one manifest and one id. Two paths with the same bytes are one blob.

## The store and the mirrors

The local store and every mirror hold the same tree of keys, so a mirror is a copy of the local store's files, and any S3 bucket, folder or static host can serve it:

| Key | What |
|---|---|
| `blobs/<hash>` | A file's bytes, as they are |
| `packages/<id>` | A manifest |
| `packages/<id>.sig` | The manifest's SSH signature |
| `index/themes/<game>.toml` | A copy of a game's themes index |

The local store is `$GP_STORE` (`~/.game-printer/store`), beside the evidence store's `objects/`. Every key but the index copy is written once: a key that is there holds the same bytes, since its name is their hash. A write is staged beside the key and renamed into place. Blobs go first and the manifest last, so a manifest in a store always has its files. Every read is checked: a blob against its hash and its size in the manifest, a manifest against the id it was asked for by. A mismatch stops the read.

Blobs are stored as they are, not compressed: a theme is mostly PNG and WAV files, which are compressed already, and a plain blob is checked against its key with no decoder, which the browser needs. Compressing text blobs (zstd, as the hosting note suggests) is an open item for model packages.

## Mirrors

`mirrors.toml` in the repo lists the mirrors, tried in order; `$GP_HOME/mirrors.toml` is read first and adds to it (a person's own, or a test's mock). The repo's lists none yet: no mirror is set up (the user, 2026-11-04: R2 first, B2 optional).

```toml
format = "0.1.0"

[[mirror]]
name = "r2"
read = "https://packages.example.org"                  # anyone reads <read>/<key>
endpoint = "https://ACCOUNT.r2.cloudflarestorage.com"   # gp writes <endpoint>/<bucket>/<key>
bucket = "game-printer"
region = "auto"

[[mirror]]
name = "folder"
dir = "/srv/packages"      # a folder a static host serves, written as files
```

| Key | Meaning |
|---|---|
| `name` | The mirror's name: lowercase letters, digits and `-` |
| `read` | The base URL keys are read under, `http(s)://` or `file://`. A mirror without one is read from its `dir` |
| `endpoint`, `bucket`, `region` | The S3-compatible API gp writes through, path style. `region` is `auto` for R2 (the default) and the bucket's region for B2 |
| `dir` | A folder to write keys into |

gp writes through one client for every S3-compatible host: curl with its AWS Signature Version 4 (`--aws-sigv4 aws:amz:REGION:s3`). Before each PUT it asks HEAD, and a key that is there is not sent again. A mirror's write key is never in either file. It is `GP_MIRROR_KEY_<NAME>` (the name upper-cased, other characters as `_`), `ID:SECRET`, or the file `$GP_HOME/secrets/mirror-<name>`, which gp refuses while anyone but its owner can read it. curl reads the key from its standard input, never from its arguments, and every message from a request has the key and its secret taken out (`mirror::redact`). Reads need no key.

The code is `crates/gp-cli/src/mirror.rs`.

## Signing

A manifest is signed with an SSH key in the namespace `game-printer-package` (`ssh-keygen -Y sign`), as docs/research/package-hosting.md recommends for score records too. The signature is stored at `packages/<id>.sig`. Who may sign is `themes/allowed_signers` in the repo (`ssh-keygen`'s allowed signers format, empty until the first publisher adds a line), and `$GP_HOME/allowed_signers` first. Only maintainers sign and publish (the user, 2026-11-04), so the allowed signers are the maintainers' keys; an artist checks a theme with `gp theme publish --dry-run`, which needs no key, and hands it to a maintainer. A publish checks its own signature before anything is sent; a fetch checks it when the index names a signer, and stops on a mismatch.

## The scan

Before a package is stored, every file is checked:

| Check | Refused |
|---|---|
| Kind | A file not one of `toml`, `pix`, `tune`, `txt`, `md`, `json`, `png`, `wav` |
| Contents | A `png` without the PNG signature, a `wav` that isn't RIFF WAVE, a text file that isn't UTF-8 |
| Size | Over 1 MB, the pre-commit hook's limit |
| Game images | An iNES header at the start |
| Drawn files in a source package | A file with the same bytes as a file of the look gp drew over the game's art |
| ROM bytes | `gp template served` on the staged package: 32 or more of the ROM's bytes in a row, raw or written as numbers, a file identical to a lifted file, the bundle or a picture drawn from the lifted assets, and lines of the lifted files, against the game's ROM and the ROM of the game it is derived from, when each is on this machine |

## Tests

| Test | What |
|---|---|
| `gp_package::tests` | A manifest round trip and its id; the same files in another order give the same id; bad paths and texts refused; a blob checked by size and hash |
| `gp_package::store::tests` | A package through a store and back; two paths with the same bytes as one blob; a key written once; a blob or manifest changed on disk refused |
| `gp_package::index::tests` | The themes index round trip, read the same by a TOML parser; a bad index refused; keys from a newer gp passed over |
| `mirror::tests` | `$GP_HOME/mirrors.toml` first; a key redacted |
| `crates/gp-cli/tests/theme_publish.rs` | Publish to a mock S3 mirror on 127.0.0.1 and fetch back into an empty store; a source package for a made-up ROM game with no `look/` and no ROM bytes; a dry run that writes nothing; a bundled theme's id refused; a recheck that adds a model the publisher had no ROM for and reports a different look ([theme publishing](theme-publishing.md#tests)) |
