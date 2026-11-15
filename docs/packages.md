# Packages

A package is a published version of a folder, a game's (kind `game`) or a theme's (kind `content`), in one format for both: its files, each stored once as a blob named by the BLAKE3 hash of its bytes, and a manifest that names them. The package id is the BLAKE3 hash of the manifest's bytes, so one id pins every byte of every file. The code is `crates/gp-package`, which depends on `blake3` alone, so `gp`, the web player and the registry reader in the page read packages with the same code. It came from game-printer's docs/packages.md; the format is the same, and game-printer's local package store, its mirrors (`mirrors.toml`) and `gp theme publish` are replaced by registries ([Run a registry](../site/src/content/docs/registry.md)).

A game's package starts as its folder in the games checkout (`games/donkey-kong`, which is also the game's template), with `package.toml` at its root. `gp registry publish` turns a folder into a package and writes it to a registry; `gp game fetch` reads one back into a working copy and lifts each ROM entry it finds in the ROM folder. Given `--id`, it fetches that exact package; without it, it reads the id from the registry's index (`index/NS/NAME/VERSION.toml`) for the version named, or for the newest version not yanked when none is named, and says in its output which id it used and that it came from the index.

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

Every key but the list files (`catalog.toml`, `by-game/` and `yanks.toml`, which `gp registry reindex` rewrites; [their format](formats/registry-lists.md), with a by-game line's optional `recheck`) is written once: a key that is there holds the same bytes, since its name is their hash. A publish writes the blobs first, then the manifest, then the version file, so a reader never sees a version with parts missing. Every read is checked: a blob against its hash and its size in the manifest, a manifest against the id it was asked for by. A mismatch stops the read.

A reader refuses a registry with no `yanks.toml`, since it can't tell which versions were withdrawn, so `gp registry init` writes an empty one (signed by the signing key when the registry is signed, with the same seven-day expiry a reindex gives it). A publish onto an older registry that has none writes one when its key may sign the list files, and otherwise says that `gp registry reindex TARGET --key SIGNING_KEY` writes it.

Blobs are stored as they are, not compressed: a theme is mostly PNG and WAV files, which are compressed already, and a plain blob is checked against its key with no decoder, which the browser needs.

[Where a registry can live](registry-backends.md) has each target and what makes a key write-once on it.

## Signing

A manifest is signed with an SSH key in the namespace `game-printer-package` (`ssh-keygen -Y sign`), and the signature is stored at `packages/<id>.sig`. Signing is off unless the registry's operator opts in, and a reader checks signatures only for a registry it holds a root key for (`--root` for `gp`, the source's `root` line for the page). Hashes are checked either way. [Where a registry can live](registry-backends.md#signing-is-opt-in) says how to turn it on.

## What a package may hold

A game's package holds no byte of the game's ROM. Two commands check that before a version is published, and the gate runs the first on every patch:

- `gp package scan DIR` sorts every file and field of the folder into ours, reference, from the ROM and needs rewrite, and rebuilds what comes from the ROM from the person's own copy ([the package scan](package-scan.md)).
- `gp template served DIR` checks the files a page serves: no line of 24 characters or more of the lifted files, and no run of 32 of the ROM's bytes, raw or written as numbers.

A registry in a git repo also compares every file of a package with each ROM it names before it commits, and refuses on 32 matching bytes or a ROM it can't find.

A game made with pictures of its own (a new game, from a template whose `source.how` is `build` or `authored`) names no ROM, so there is none of its own to compare it with; until now both checks passed such a package unread. Its `play/page.json` says so (`rom.kind = "none"`, [the page's format](web.md#a-game-with-its-own-pictures)), and then:

- `gp registry publish`, on any target, refuses it when it names a ROM (`[[rom.entry]]`), has a `roms.toml` or a lift (`play/lift/`), or when any file of it starts with an iNES header, holds a run of 32 bytes of any ROM in `$GP_ROM_DIR` (raw or written as numbers), or is a file lifted into `$GP_HOME/work/assets/` byte for byte. The game's own build is left out: a ROM its template builds from source (`how = "build"`, its hash in `accepts`, Moss Hop's) and that ROM's lifted folder are its own. A publish it takes says what it was compared with.
- `gp template served` compares such a package the same way.

Its `[page]` table may give `about` (what the game is, a line of up to 600 characters) and `made_by` (who made it, up to 120), which the page shows as text.

## Submitting to a registry you can't write to

`gp registry submit DIR --name NS/NAME --version V --title T --to OWNER/REPO [--kind game] [--for GAME]` offers a game or theme to the registry in the GitHub repo `--to` names (required; `lex00/gp-registry` takes no one else's themes), as a pull request (issue #9). The owner still decides: nothing reaches the registry until they merge.

| Step | What happens |
|---|---|
| Stage | DIR is copied to `$GP_HOME/work/registry-submit/NS-NAME-V/stage`, leaving out every file and folder whose name starts with `.` (named in the output), since publish takes every file it is given |
| Clone | The registry is cloned fresh beside it (`registry/`), on a branch `submit/NS-NAME-V`, with no `origin`, so the registry commands below commit and never pull or push |
| Grant | A namespace registry.toml doesn't list is granted in the branch (`gp registry grant`) when the registry is unsigned; the pull request says so, and merging it grants the namespace. A signed registry's root key is the owner's, so the submission stops and asks the person to have the owner grant it |
| Publish | `gp registry publish` into the clone, the same code with every check it runs: each file compared with each ROM the package names, a game with its own pictures compared with every ROM in `$GP_ROM_DIR` and every lifted folder, the namespace key for a signed registry (`--key`), and each check another change adds to publish. Its verdict on what the page will offer goes in the pull request |
| List | `gp registry reindex --keep NAME@V`, so the branch holds the package's lines in `catalog.toml` and `by-game/`. A signed registry's lists are signed by the owner's key, so then it isn't listed and the pull request says the owner reindexes after merging |
| Fork | The person's fork, `--fork OWNER/REPO` or LOGIN/REPO by their GitHub login (`gh api user`), made with `gh repo fork --default-branch-only` when it isn't there |
| Push and open | The branch is pushed to the fork (replacing an earlier push of the same submission) and `gh pr create` opens the pull request on the registry, titled `Submit NAME@V`, its body the package, the checks it passed, the grant and the commits. A pull request already open for the branch is kept and named |

When a check fails, the submission stops at that step and prints the check's own words, then that nothing was pushed to the fork and no pull request was opened, that the registry and the fork are as they were, and that the same version can be sent again once DIR is fixed, since it was never published. It exits 1 and names the clone it was checked in. `--dry-run` runs every step up to the fork in the clone, commits included, and sends nothing. It needs `gh` logged in to GitHub; `$GP_GH` names another program in its place, which the tests use with two bare repos on disk standing in for the registry and the fork. From MCP it is an ask-first tool (`registry submit`, a yes unless `--dry-run`).

## Publishing an arcade game

The owner's rule for an arcade game (issue #3): it is published only when every clip of its corpus scores exact on state and every screen frame each clip compares is the same on picture. The two are apart: a clip "exact" in a score record is exact on the game's state alone, and its picture is its `screen` counts, the frames whose sprites and background both match the original's. `gp registry publish` takes a game package (`--kind game`) for an arcade board (a `[[rom.entry]]` with `platform = "arcade"`, a template whose `[source]` platform is `arcade`, or a `play/page.json` whose player is `arcade` or whose `rom.kind` is `zip`) only when its `evidence/` holds the score record `gp corpus score --write` keeps for its model and corpus as they are, `evidence/MODEL/CORPUS.json` ([fidelity](fidelity.md)), and that record says:

- every clip exact on state, none that couldn't be scored, and as many clips as the corpus has now;
- every clip the same on picture: of its compared screen frames, as many the same as were compared, and more than none compared (a clip with no `screen` counts fails);
- the package's own `mapping.toml` and `fidelity.toml` (their hashes), its scored romset (`role = "scored"`) as the ROM, and for a game with hooks the module it ships.

It doesn't score anything itself. The record sits in the game's folder and is committed with it, so the pass is the one for the package's commit and goes into the registry with the package. Anything else stops the publish (a dry run too) before a file is written, saying what is missing and how to get there:

```text
lex00/tapper is an arcade game, and an arcade game is published only when every clip of its corpus scores exact on state and every screen frame is the same on picture, but it holds no score record for its model and corpus as they are (evidence/b3:.../b3:....json isn't there). Score it in the game's folder with `gp corpus score GAME_DIR --run publish --write GAME_DIR` (for a game with hooks, add `--module PLAY_DIR/module.wasm`, the module the package ships, which build-page.sh or `gp game build` makes) until every clip is exact on state and every screen frame is the same on picture. That writes the record into GAME_DIR/evidence/; commit it with the game, and publish the folder again. Nothing was published
```

A record for another model or corpus (one changed after the score) is said so, and a record with clips not exact on state names up to five of them. A refusal says the state and the picture apart, and names up to five clips that differ on picture. Tapper's record on games main (2026-11-15) would be refused so, its model having no pictures yet:

```text
... but its score record evidence/.../....json has 54 of 54 clips exact on state; picture: 0 of 336 screen frames the same, 54 of 54 clips differ on picture (differ on picture: ..., and more). ...
```

A publish it takes adds a line such as `lex00/tapper is an arcade game: all 54 clips of its corpus tapper 0.2.0 score exact on state and all 336 of their screen frames are the same on picture, on its model as it is (score record evidence/.../....json, by gp ...)`. The rule reads each clip's `screen` counts, so a record from before 0.3.0 (no `results.picture`) is judged the same way. For a game with hooks (`[hooks]` in package.toml) the record must name the hooks module it was scored through ([record format](formats/scoring-corpus.md#score-records) 0.2.0 on), and that must be the module the package ships, `play/module.wasm`: score with `--module PLAY_DIR/module.wasm`. A record from before records named the module is refused for such a game and taken for a game without hooks. An NES game, a theme and a game with its own pictures are published as before.

The package also names the MAME romsets it needs: `build-page.sh` writes them into `play/page.json` as `rom.sets`, each set's MAME name, title, identity and the files its zip must hold by name, size and SHA-1, from `gp roms sets` (MAME's own list, compiled into gp-arcade; a game's `roms.toml` gives each set's name as `system` and its identity as `sha256`). [web.md](web.md#what-the-page-reads-from-the-games-package) says what the page does with them.

## Tests

| Test | What |
|---|---|
| `gp_package::tests` | A manifest round trip and its id; the same files in another order give the same id; bad paths and texts refused; a blob checked by size and hash |
| `gp_package::store::tests` | A package through a folder store and back; two paths with the same bytes as one blob; a key written once; a blob or manifest changed on disk refused |
| `gp_package::index::tests` | The themes index round trip; a bad index refused; keys from a newer gp passed over |
| `registry::command_tests::an_arcade_game_is_published_only_when_every_clip_scores_exact` (gp-cli) | A made-up arcade package refused with no score record, with a record of a clip not exact (naming it) and with a record scored with another mapping, each with nothing written; refused with every clip exact on state but a screen frame different (saying "1 of 1 clips exact on state; picture: 39 of 40 screen frames the same" and naming the clip), and with no screen frames compared; published once its record has every clip exact on state and every screen frame the same on the package as it is; with hooks, a record from before records named the module and one naming another module than play/module.wasm refused and one naming it taken; refused again after a clip's presses change |
| `gp_package::registry::tests` | A package published, listed and read back; a wrong root, an edited file, a stranger's signature, a changed blob or manifest and an expired list refused; a version published once; attestations written once; a registry with no keys read with no signatures and its hashes still checked; a by-game line's recheck ([list files](formats/registry-lists.md#tests)) |
| `registry_submit::tests::a_submission_forks_pushes_and_opens_a_pull_request` (gp-cli) | A made-up theme submitted to a bare repo standing in for the registry, through a stand-in gh: a dry run forks and opens nothing; the real run makes the fork, pushes one branch with the grant, the publish and the listing, opens the pull request with the head `friend:submit/...`, leaves `.git` out of the package, and leaves the registry's own branch as it was |
| `registry_submit::tests::a_failed_check_stops_before_anything_leaves` (gp-cli) | A package publish refuses (it says it has its own pictures and has a `roms.toml`) stops at the publish checks with publish's words and the plain account, and gh is never asked to fork or open anything |
| `registry_submit::tests::a_stage_leaves_out_hidden_files` (gp-cli) | `.git`, `.gitignore` and a hidden folder inside are left out of the stage and named |
