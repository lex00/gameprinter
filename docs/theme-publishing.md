<!-- Brought from game-printer's docs/theme-publishing.md with the theme commands (wave 8
of the port). In gameprinter a game's folder is the games checkout's
games/GAME (game-printer's templates/GAME): read templates/GAME below as
games/GAME, templates/donkey-kong-oe as games/donkey-kong. A command
names a game by its folder or by its template id. `gp theme kit` writes
into the game's themes/kit/ unless given -o, and the THEMES tab of the
mechanic page always gives -o, so nothing is written into the games
checkout from there. The publishing index is themes/index/GAME.toml in
the games checkout. The notes on Rivet Rodeo, retired and deleted on
2026-11-11, are kept as the record of how the checks came to be. -->

# Publishing themes

A theme version is published as a package ([theme packages](theme-packages.md)), checked, signed and listed in its game's themes index, so a rebuild can list the themes for its game, download one into a cache and draw it with no restart. The user asked for it on 2026-11-04: "the rebuild needs to be able to scan for available themes and let the player pick one, with a download for cache". The code is `crates/gp-cli/src/theme_publish.rs`, `crates/gp-package` and `crates/gp-cli/src/theme_mirror.rs`.

## The package

A theme package holds only what the theme's maker made and what gp checked. There are two kinds:

| Kind | For | Holds |
|---|---|---|
| `look` | A game with no ROM art: games made here (`how = "authored"`, Rivet Rodeo, the public page's game) and games built from source (`how = "build"`, Moss Hop) | `theme/`, `look/` and `checks.json` |
| `source` | A game lifted from the player's ROM (`[source] how = "rom"`, Donkey Kong JU and Original Edition) | `theme/` and `checks.json`, no `look/` |

| Path | What |
|---|---|
| `theme/` | The theme folder as it is: `theme.toml`, its `.pix` art and `.tune` sound, prompts and sources |
| `look/` | A look package only. The look the runtimes draw: the theme drawn for the game's build by the same code `gp-web-prep` uses (`gp_look::looks`), with `look.txt` and the pictures, `present.txt` and sound role WAV files it changes ([themes found at run time](formats/theme.md#themes-found-at-run-time)) |
| `checks.json` | The check results: readability and Style, originality with every score, geometry, the package scan, and each model the theme is for with its result (below) |

A look is a theme drawn for one game (`look.txt` and the pictures and sounds it changes). It is drawn on the game's own assets, which for a game lifted from a ROM are the lifted art: a picture the theme leaves alone would hold the original's pixels. So a ROM game's theme is published as a source package. gp still draws its look on the maker's machine, over the maker's own lifted art, to run the checks and to record the look's digest, and the look is never packaged. The player's runtime draws the theme over the player's own lifted art (ROADMAP, ROM themes, step 8). No pixel drawn from the ROM leaves the maker's machine.

### checks.json

`checks.json` has the given template's theme check results (`readability`, `style`, `originality`, `geometry`), the package scan (`scan`: the file count and the ROMs the ROM-bytes check ran against, for every model checked), `kind`, and `models`: each model in the theme's `for` (else the model it was published for) with its result.

`gp theme publish` checks every model in `for` whose ROM is on this machine: the given template's model with `--assets` and `--rom` (or their defaults), and each other template of the game (the same `template.game`) with the first ROM in `$GP_ROM_DIR` its template accepts, over assets lifted from that ROM into a temporary folder. A model whose ROM isn't there, or whose assets can't be lifted from it, is recorded as `unchecked` with the reason.

| Key under `models.<model>` | What |
|---|---|
| `result` | `pass` for a model checked here; `unchecked` for one that couldn't be. A `fail` refuses the publish, so it is never in a package; the refusal's output has `models` with `fail` and the checks that failed (`failed`) |
| `why` | An unchecked model's reason, in words: "no donkey-kong-oe ROM on this machine: no file in DIR has a hash templates/donkey-kong-oe/template.toml accepts (gp roms add puts one there, docs/sources.md)", or that its assets couldn't be lifted, or that no template of the game in the repo has the model |
| `template`, `slots` | The template checked and its model's slot fingerprint ([the slot fingerprint](theme-kit.md#the-slot-fingerprint)) |
| `rom_sha256` | The SHA-256 of the ROM the checks ran with, as the template's `accepts` lists it |
| `look` | The look's digest: the BLAKE3 hash of the drawn look's file hashes and paths, a line each, sorted. Drawing is deterministic, so someone with the same ROM and gp draws the same digest and can confirm the maker's result. A hash holds nothing of the art it was taken from |
| `coverage` | The build's pictures the look changes and the sound roles it gives a sound of its own, against the build's pictures and the kit's sound roles |
| `readability`, `originality`, `geometry` | The index's lines for this model. A readability check that warns but doesn't refuse reads `pass with a warning:` and what it warned of, such as "pass with a warning: hazards (41% of its pixels stand out from the background)" |

## The index

Each game has its own index file, `themes/index/<game>.toml`. The game is the template's `template.game` when it has one, else the template's id: Donkey Kong JU and Original Edition both list their themes in `donkey-kong.toml`, and Rivet Rodeo (no `template.game`) in `rivet-rodeo.toml`. `gp theme publish`, `fetch` and `list` take a template's id or a game and read the same file. It is the source of truth: reviewed in git and changed only through the gate. It names packages by hash, never by URL. Each mirror holds a byte-for-byte copy at `index/themes/<game>.toml`, which the runtimes fetch.

```toml
format = "0.1.0"
game = "rivet-rodeo"

[[theme]]
id = "night"
title = "Night Shift"
author = "themePub test"
license = "CC0-1.0"
package = "272f442a637bb9be2908806f6a35d65692575a16f1dc6efd861437c73f9ae3f9"
model = "rivet_rodeo"
fits = "9f9feb568ada17223ada7cbaa688c7d4a691170ee19cf649fddfb038d37d3b4b"
kit = "248b434d83c71973f0aec0e10211acbdbb4e3310d3770dc91e2ef177b08b1f5f"
kind = "look"
models = "rivet_rodeo"
slots = "rivet_rodeo:<the slots of rivet-rodeo's kit.toml>"
results = "rivet_rodeo:pass"
looks = "rivet_rodeo:<the look's digest>"
preview = "93e74fb7bb97c0425aa77ebda3d5c86fbce0cc8482cf0097c368d972ae30a7e2"
signer = "themepub@example.invalid"
originality = "pass: art at most 0.540, shape 0.503, melody 0.000; no ROM bytes: rivet-rodeo against rivet-rodeo.nes, donkey-kong-ju against Donkey Kong (JU).nes"
readability = "pass: ok"
geometry = "pass: 8 slots and stages checked, 8 flags, none refused"
size = 2229885
```

| Key | Meaning |
|---|---|
| `id`, `title`, `author`, `license` | From theme.toml |
| `package` | The package id |
| `model` | The model it was drawn for |
| `fits` | The game version it fits: the BLAKE3 hash of the build's picture file names, a line each, in order (`gp_package::index::pictures_digest`). A runtime swaps pictures by number, so it offers a theme only when its own build's digest is the same. Empty for a source package, so a runtime from before source packages offers it nowhere rather than look for a look it hasn't got |
| `kit` | The kit version it fits: the BLAKE3 hash of the game's `themes/kit/kit.toml` when it was made |
| `kind` | `look`: the package holds a look gp drew, and `fits` says which build it fits. `source`: the package holds the theme's own files and no look, and `slots` says which versions it was checked on. Empty in a line written before the key was added, which is a look |
| `models` | The models the theme was checked on and passed, a space between: each model in `for` whose ROM was on the publishing machine |
| `slots` | Each checked model's slot fingerprint, `model:digest`, a space between: the `slots` of that model's kit.toml when it was checked ([the slot fingerprint](theme-kit.md#the-slot-fingerprint)). It changes with an art slot's size, frames or color limit, or a sound role, and not with the kit's words. `Theme::fits_model(model, digest)` is true when the model is in `models` with this fingerprint |
| `results` | Each model the theme is for (its `for`, else the model checked) with its result, `model:pass` or `model:unchecked`, a space between, as checks.json's `models` has them (a `fail` refuses the publish, so no line has one). A runtime will offer the theme on a model with `pass` and list it greyed out on an unchecked one (ROADMAP, ROM themes, step 9). `Theme::result_for(model)` reads it |
| `looks` | Each checked model's look digest, `model:digest`, a space between, the `look` of [checks.json](#checksjson) for a model the publisher checked and the one drawn by `gp theme recheck` for a model added later ([recheck](#recheck)). `Theme::look_for(model)` reads it. Empty in a line written before the key was added |
| `preview` | The hash of the preview picture, the look's largest picture (a file of the package). Empty for a source package, and for a look that changes no picture |
| `signer` | Who signed the manifest, as `allowed_signers` names them |
| `originality`, `readability`, `geometry` | Each check's result on the model the theme was published for: `pass: ...`, `pass with a warning: ...` (readability only) or `not checked: ...`. checks.json has each model's lines |
| `size` | The package's files together, in bytes |

`fits` and `slots` sit side by side. `fits` keys a drawn look to the build whose pictures it was drawn for, and the runtimes offer a published theme by it, as before. `slots` keys a theme to the versions of a game it was checked on, one fingerprint a version, so one theme can list Donkey Kong JU and Original Edition, each with its own result; the runtimes don't read it yet. `kind`, `models`, `slots`, `results` and `looks` are new keys of index format 0.1.0 and kit format 0.1.0: an older gp passes over them, so neither format's version changes.

A newer version of a theme replaces its line by id; the older package stays in the stores. The file is TOML of a fixed shape (keys of strings and whole numbers, a `[[theme]]` table a theme), so the runtimes read it with `gp_package::index` and no TOML parser, and a TOML parser reads the same values (the test `an_index_round_trips_and_toml_reads_it_the_same`). Keys a newer gp adds are passed over.

### One file per game

One file per game, not one index beside games.toml, for these reasons:

| Reason | Detail |
|---|---|
| The runtime's download | A rebuild fetches only its own game's file, a few KB, at every start. One index for every game (thousands, docs/research/model-data-storage.md) would grow with every theme of every game |
| Merges | Each publish changes one small file. Publishes for different games never touch the same file, so the gate's three-way merge has nothing to resolve |
| One static object | A mirror serves each game's file as one key, with no query or server code |
| games.toml stays small | games.toml is a line a game (its sources and pinned model package). Themes are many a game and change more often |

## The mirrors

Mirrors are listed apart from the index, in `mirrors.toml` ([theme packages](theme-packages.md#mirrors)), so losing a host changes one line and no index.

## Publish

```sh
gp theme publish THEME_DIR --game TEMPLATE [--assets DIR] [--rom FILE] [--key FILE] [--signer NAME]
                 [--gate TEMPLATE | --no-gate] [--index FILE] [--local | --dry-run]
```

| Step | What |
|---|---|
| Load | The theme folder (`gp_theme::stack::load`, a `same_as.toml` pointer followed), for the template's model |
| Id | The id must not be one a theme the game ships with already has: any `themes/<id>/` of the template and of the other templates of its game, unless the theme is that one. A copy of the paper theme that keeps `id = "paper"` is refused with "the id `paper` is taken: templates/donkey-kong-oe/themes/paper ships with the game under that id, and a player's list would show only one of the two. Give this theme an id of its own in its theme.toml, such as `paper-2`" |
| Models | Each model in the theme's `for`: the given template's, and each other template of the game whose ROM is in `$GP_ROM_DIR`, its assets lifted from that ROM. The others are recorded as not checked, with the reason ([checks.json](#checksjson)) |
| Checks | For each model checked, `gp theme check --original` with the theme on its own: readability and the Style checks, then originality (art, shape, logo, names, melody, ROM bytes) against that model's assets and ROM, then geometry (the theme kit's gameplay geometry, docs/theme-kit.md). A refusal on any model stops here, naming the model and the check ("mosshop_b (template moss-b): refused by its readability check"): nothing is written or sent |
| Look | For each model checked, the theme drawn as a look for its build (its own stack under it), as gp-web-prep draws looks. A look package holds the given template's; a source package records only each model's digest and coverage |
| Scan | The package scan ([theme packages](theme-packages.md#the-scan)): each file's kind, contents and size, and `gp template served` on the staged files against the ROM of each model checked and the ROM of the template it is derived from, each template once (Rivet Rodeo: its own image, which gp builds, and Donkey Kong's). A file holding a run of a ROM's bytes is refused with the models that ROM was checked for ("ROM bytes check for donkey_kong_oe: theme/notes.txt: ..."). A source package also refuses a file with the same bytes as a file any model's drawn look has |
| Dry run | With `--dry-run`, it prints the files the package would hold with their sizes, the check results, each model's result, the package id and the index line it would write, and stops. Nothing goes to the store, the mirrors or the index, and no key is needed |
| Store | The blobs and the manifest into `$GP_STORE` |
| Sign | `ssh-keygen -Y sign -n game-printer-package` over the manifest with `--key` (else `$GP_SIGNING_KEY`, else git's `user.signingkey`), checked against the allowed signers before anything is sent. Only maintainers sign and publish (the user, 2026-11-04): the allowed signers are the maintainers' keys. An artist runs `--dry-run` and hands the theme folder to a maintainer |
| Mirrors | For each mirror with a write endpoint or folder: the blobs it lacks, then the manifest, then its signature |
| Index | The theme's line in the game's index file, as a patch landed through `gp model gate` on the game `--gate` names (default `donkey-kong`); `--no-gate` writes the file in place instead, for a fork's own index or a test's |
| Index copies | Once the index has landed, its copy on each mirror |

A dry run of Donkey Kong's paper theme for JU, 2026-11-04, with both ROMs in `$GP_ROM_DIR`. It is from before 2026-11-06, when `templates/donkey-kong-ju` was a template and the theme was for both models (`donkey_kong` and `donkey_kong_oe`); since then the theme is in `templates/donkey-kong-oe/themes/paper` and for `donkey_kong_oe` alone, and `--game donkey-kong-oe` is the one game to name:

```text
$ gp theme publish templates/donkey-kong-oe/themes/paper --game donkey-kong-ju --dry-run
dry run: `paper` for donkey-kong (template donkey-kong-ju), a source package: the theme's own files and checks.json, no look drawn from the ROM's art
files (23, 267965 bytes):
  checks.json 9740
  theme/clear.tune 253
  ...
checks on donkey_kong_oe: readability pass with a warning: hazards (41% of its pixels stand out from the background); originality pass: art at most 0.383, shape 0.562, melody 0.125; no ROM bytes: donkey-kong-oe against Donkey Kong - Original Edition (USA, Europe) (3DS Virtual Console).nes, donkey-kong-ju against Donkey Kong (JU).nes; geometry pass: 9 slots and stages checked, 3 flags, none refused
checks on donkey_kong: readability pass: ok; originality pass: art at most 0.383, shape 0.562, melody 0.125; no ROM bytes: donkey-kong-ju against Donkey Kong (JU).nes; geometry pass: 8 slots and stages checked, 3 flags, none refused
package scan: 22 files, no problem
models: donkey_kong_oe checked: pass, 74 of 75 pictures, 14 of 14 sound roles; donkey_kong checked: pass, 71 of 72 pictures, 14 of 14 sound roles
...
  kind = "source"
  models = "donkey_kong_oe donkey_kong"
  results = "donkey_kong_oe:pass donkey_kong:pass"
...
nothing written: no store, no signature, no mirror, no index (--dry-run)
```

Original Edition is checked over assets lifted from its ROM in a temporary folder, and its look digest is the one a dry run through its own pointer (`templates/donkey-kong-oe/themes/paper --game donkey-kong-oe`) records over `$GP_HOME/work/assets/donkey-kong-oe` (`7c941b02...` both ways), so where the lift comes from doesn't change the result. With `$GP_ROM_DIR` holding only the JU ROM, the same dry run records:

```text
models: donkey_kong_oe not checked: no donkey-kong-oe ROM on this machine: no file in DIR has a hash templates/donkey-kong-oe/template.toml accepts (gp roms add puts one there, docs/sources.md); donkey_kong checked: pass, 71 of 72 pictures, 14 of 14 sound roles
  models = "donkey_kong"
  results = "donkey_kong_oe:unchecked donkey_kong:pass"
```

A publish that fails at a mirror stops before the index, so the index never names a package a mirror lacks; run it again, and only what is missing is sent. `--local` stops after the signature. Output goes to `$GP_HOME/work/theme-publish/GAME/THEME/` (the gate's patch).

## Recheck

```sh
gp theme recheck GAME THEME [--model MODEL] [--rom FILE] [--assets DIR]
                 [--gate TEMPLATE | --no-gate] [--index FILE] [--dry-run]
```

A theme published by someone without every ROM of its game lists the other models as unchecked. `gp theme recheck` lets someone who has one of those ROMs check the published package on its model and add the result to the index. The package and its signature are left as they are: a recheck adds a result to the theme's index line and signs nothing, and the line still names the publisher as `signer`. The index change lands through the gate, so a maintainer lands it, as for a publish.

| Step | What |
|---|---|
| Package | The theme's line in the game's index (`themes/index/<game>.toml`, or `--index`), then its package by id from the local store or the first mirror that has it, the manifest, the signature and every file checked as `gp theme fetch` checks them. Nothing is written to the store. A look package is refused: its look was drawn for one build, so it is published again instead |
| Models | `--model`, else the model whose template accepts `--rom`, else each model in the theme's `for` the index doesn't record as `pass`. The ROM is `--rom`, else the first file in `$GP_ROM_DIR` the model's template accepts. With no model or ROM named, a model whose ROM isn't here is passed over with the reason; a named one that can't be checked refuses the recheck |
| Checks | The same as publish's for each model ([publish](#publish)), on the package's own files: `gp theme check --original` over assets lifted from the ROM here (or `--assets`), the look drawn and its coverage, a drawn file in the package, and the ROM-bytes check of the package's files against the model's ROM and the ROM of the template it is derived from |
| Look | The digest of the look drawn here is compared with the one recorded for the model: the index's `looks`, else the package's checks.json. The same digest, or none recorded yet, adds the model. A different one is reported and not added: "the look drawn here differs from the maker's for donkey_kong_oe: the maker's ROM or gp differs (recorded 7c941b02fb72, drawn here ...); not added", with both ROMs' SHA-256 when they differ |
| Dry run | With `--dry-run`, it prints each model's checks and the index line it would write, and stops |
| Index | The theme's line with each model checked added to `models`, `slots`, `results` and `looks`, landed through `gp model gate` on the game `--gate` names, as a whole-file patch (`--no-gate` writes it in place), then its copy on each mirror |

A failed check, a different look, or a ROM-bytes find refuses the whole recheck: nothing is written, exit 1, and the JSON's `models` gives each model's result (`pass`, `fail` with the checks that failed, or `differs` with both digests).

On this machine, 2026-11-04, with a key made for the run and a private index: Donkey Kong's paper published for JU with only the JU ROM in `$GP_ROM_DIR` (`donkey_kong_oe` unchecked), then rechecked with both ROMs there:

```text
$ gp theme recheck donkey-kong paper --index FILE --no-gate
recheck of `paper` (Paper) for donkey-kong, package e2eb61125753 signed by ...; its files are left as they are
checks on donkey_kong_oe: readability pass with a warning: hazards (41% of its pixels stand out from the background); originality pass: art at most 0.383, shape 0.562, melody 0.125; no ROM bytes: donkey-kong-oe against Donkey Kong - Original Edition (USA, Europe) (3DS Virtual Console).nes, donkey-kong-ju against Donkey Kong (JU).nes; geometry pass: 9 slots and stages checked, 3 flags, none refused
donkey_kong_oe (template donkey-kong-oe, ROM Donkey Kong - Original Edition (USA, Europe) (3DS Virtual Console).nes): pass, 74 of 75 pictures, 14 of 14 sound roles; look 7c941b02fb72, the first look recorded for this model
index FILE written
added donkey_kong_oe for `paper` in the index of donkey-kong
```

The line then has `results = "donkey_kong:pass donkey_kong_oe:pass"`, and OE's look digest (`7c941b02...`) is the one a publish with both ROMs records. `--model donkey_kong` then draws JU's look again and reports "the same look as the one recorded" (`110d26c6...`). The patch for the gate is written in `$GP_HOME/work/theme-recheck/GAME/THEME/`.

## Fetch and list

```sh
gp theme fetch GAME THEME [--index FILE] [-o DIR]
gp theme list GAME [--index FILE] [--mirror]
```

`gp theme fetch` reads the theme's line in the index, takes the manifest from the local store or the first mirror that has it and checks it against the package id, checks its signature when the index names a signer, then takes each file, checks its size and hash, and keeps it in the local store. It writes the package into DIR, `$GP_HOME/themes/packages/GAME/THEME/` by default, with the theme in `theme/` (usable as `--theme DIR` on any command) and, for a look package, the look in `look/`. Any mismatch stops it before a file is written.

`gp theme list` prints each theme of the game's index with its title, author, licence, size, kind, package, each model's result, check results and whether the local store has it; `--mirror` reads the index copy on the first mirror instead of the repo's.

## In the game

A rebuild lists the themes published for its game on the settings screen's Theme line, downloads one into a cache when it is chosen, checks every file against its hash, and draws it from the next frame with no restart. The simulation never reads a theme, so the state hash goes on as before. The shared code is `gp_present::catalogue` (the index read and filtered, a package's look files checked against its id, and the Bevy build's cache) and the settings screen's marks (`gp_present::settings::ThemeMark`).

| Step | Web preview (`scripts/web/page/preview.js`) | Bevy build (`template/render.rs`, `menu.rs`) |
|---|---|---|
| Where the index comes from | `index/themes/<game>.toml` from the first mirror in preview.json's `catalogue` (`gp-web-prep` writes the game's id and the mirrors' read URLs from mirrors.toml), or `mirror=URL` in the page's URL | The same key from the data folder's `mirrors.txt` (a URL a line), then the mirrors the build was generated with (`MIRRORS` in `look.rs`), with curl |
| Offline | The copy kept in the cache the last time | The copy kept at `catalogue/<game>.toml` in the data folder |
| Then | The themes beside the bundle (`themes/index.json`) are listed as before | The looks beside the executable and in the data folder, as before |
| Which themes | Those whose `fits` is the digest of the build's picture list (`gp_catalogue`) | Those whose `fits` is `CATALOGUE_FITS`, written by `gp gen bevy` |
| The cache | Cache Storage (`gp-themes`), else IndexedDB (`gp-themes`, store `files`): keys `index/<game>`, `pkg/<id>`, `blob/<hash>`, `theme/<id>` | The data folder's `themes/<id>/`, with `package.txt` naming the package; written in a folder beside it and renamed into place when every file is checked |
| Hash checks | The manifest against the package id (`gp_package_look`), each file's size and BLAKE3 (`gp_blake3`) before it is kept, and each file again before it is drawn | The manifest and each file in `gp_present::catalogue::install` before any is written |
| Choosing one | Done downloads it, then draws it; the bar under the game says "Downloading ..." and then "Theme: ..., downloaded and checked" | Done starts the download on a thread; the game plays on in the old look until every file is checked |
| Removing one | "Remove it from the cache" under the Theme line: its record, its manifest, and each of its files no other downloaded theme holds | The same line: the look's folder, only when it has `package.txt` |

The Theme line shows a published theme's title, and the line under it its author and whether it is downloaded:

```text
Theme     < Night Shift >
  by themePub test, download 2.1 MB
```

A theme that shipped with the build (its own look, `original`, the template's other themes) has no line under it and can't be removed. On the public page, a template theme the visitor's ROM can't take (it doesn't load, or doesn't draw over the art read from the ROM) is named with its reason in a line above the game, from the lift's `looks_skipped` ([the public page](web.md#the-rom-in-the-browser)); the Theme line doesn't list it. A theme kept as the player's choice (`gp-theme:<model>`, `theme = ID` in `settings.txt`) is drawn at the next start from the cache, online or not.

The Bevy build reads the catalogue only when it plays live (not in a scripted run), and the web preview not in `test=`, `run=` or control mode, so the conformance runs read no network.

## Tests

`crates/gp-cli/tests/theme_publish.rs` builds Moss Hop from source and runs gp against a mock of an S3-compatible mirror on 127.0.0.1 (no call leaves the machine), with an SSH key made for the test:

| Test | What it checks |
|---|---|
| `gp_package::index::tests::a_theme_fits_each_model_whose_fingerprint_it_records` | A made-up theme checked on two models fits each with its own fingerprint, not one with the other's or a model it wasn't checked on; a recheck replaces a model's fingerprint; the keys read back from the index's text |
| `gp_package::index::tests::an_index_from_before_the_slot_keys_reads_with_them_empty` | A line without `kind`, `models` and `slots` reads, and fits no model by fingerprint |
| `gp_present::catalogue::tests` | A look downloaded through a made-up mirror, checked, into a cache folder; a theme drawn for other pictures not offered; a changed blob refused with the cached look kept; a downloaded look removed and a built one left |
| `gp_present::settings::tests::published_themes_are_marked_and_one_can_be_removed_from_the_cache` | The Theme line's marks ("download 2.1 MB", "downloaded"), the line under it passed over by the cursor, and "Remove it from the cache" |
| `a_theme_publishes_to_a_mock_mirror_and_fetches_back_checked` | Moss Hop's paper theme publishes: every PUT carries a SigV4 signature with the mirror's key id, the blobs go before the manifest, then its signature, then the index copy; the secret is in no request and no output; a second publish sends no blob; the index names the package, `kind = "look"` and Moss Hop's model with its kit's fingerprint; list and fetch into an empty store from the mirror alone give the same files; a blob changed on the mirror is refused by its hash, and a changed manifest by the package id |
| `gp_web_lift::tests::a_shared_theme_is_offered_on_both_models` | Two made-up versions of Rivet Rodeo's template, the second's paper theme a `same_as.toml` pointing at the first's (`for` naming both models): each lifts and offers paper, and the two looks are byte for byte the same. With the second model taken out of `for`, the second lists paper in `looks_skipped` with the reason |
| `gp_web_lift::tests::themes_the_lift_cannot_offer_are_named_with_why` | Rivet Rodeo's template lifted from the data image its authored.toml builds (no game's ROM), with two made-up themes: one whose restyle's hash isn't its file's (not read) and one whose tiles are 8 by 8 pixels where the game's are 128 by 128 (not drawn). The lift succeeds, offers the paper theme, and `result.json` names both with their reasons and the tool's messages |
| `gp_package::index::tests::each_model_a_theme_was_made_for_has_its_result` | `results` records a made-up theme's result for each model, a recheck replaces one, and a model it wasn't made for has none; `looks` records each model's look digest the same way |
| `a_dry_run_of_the_shared_rom_theme_lists_what_it_would_publish_and_writes_nothing` | A made-up repo holds Moss Hop's template twice as versions of one game lifted from a ROM (`moss-a`, model `mosshop`, Moss Hop's ROM; `moss-b`, model `mosshop_b`, a copy of that ROM with one code byte changed and its own copy of the asset map; both `game = "moss"`, `how = "rom"`), sharing one paper theme through `same_as.toml`. A dry run of moss-b's pointer with no ROM in `$GP_ROM_DIR` lists only `theme/` files and checks.json, checks `mosshop_b` (pass, with a look digest) and records `mosshop` as unchecked ("no moss-a ROM on this machine"), and gives the index line for `themes/index/moss.toml` (`kind = "source"`, `fits = ""`, no preview, `models = "mosshop_b"`, both results). No index, store or mirror is written |
| `every_model_whose_rom_is_here_is_checked_and_a_fail_on_one_refuses` | The same repo. With both ROMs in `$GP_ROM_DIR`, a dry run of moss-a's paper checks both models (each `pass` with a look digest, `mosshop_b` with moss-b's ROM hash and "moss-b against moss-b.nes"), and the index line has `models = "mosshop mosshop_b"` with both fingerprints. With only Moss Hop's ROM there, `mosshop_b` is unchecked with the reason. A copy holding 63 bytes of moss-b's ROM typed out as numbers, around the changed byte, is refused with both ROMs there (`mosshop` pass, `mosshop_b` fail by "ROM bytes check for mosshop_b"), and passes with only Moss Hop's |
| `a_rom_theme_publishes_its_own_files_with_no_look_and_records_the_models_checked` | The same repo: moss-a's paper publishes to the mock mirror as a source package with no `look/`, PNG or WAV; checks.json names the ROM it was checked against and its hash; the index records `mosshop` with its kit's fingerprint, its result `pass` and `mosshop_b` unchecked. A copy holding 64 of the ROM's bytes typed out as numbers is refused by the ROM-bytes check, and nothing more is sent |
| `a_recheck_adds_the_model_the_publisher_had_no_rom_for_and_reports_a_different_look` | The same repo. moss-a's paper is published with only Moss Hop's ROM in `$GP_ROM_DIR` (`mosshop_b` unchecked). A recheck dry run with both ROMs there gives the line with both models `pass` and leaves the index file as it was. The recheck adds `mosshop_b`: `results = "mosshop:pass mosshop_b:pass"`, both models with moss-b's fingerprint, and the look digest a publish with both ROMs records; the package id, signer and manifest bytes are unchanged. With every model passed, a recheck naming none is refused. Then moss-b's template and asset map accept a ROM with a stone block where the level has sky: `--model mosshop_b` with it draws a different look, is refused with "the look drawn here differs from the maker's for mosshop_b" (`differs`, with the recorded digest), and the index file is unchanged |
| `a_theme_with_the_id_of_a_bundled_theme_is_refused_by_name` | A copy of the paper theme from outside the templates, still `paper`, is refused for moss-b with the message naming the bundled folder and suggesting `paper-2`; renamed `paper-2`, its dry run passes |
| `a_theme_its_checks_refuse_publishes_nothing` | A theme the readability check refuses (every color one gray) and a theme holding a file of a kind a package may not hold: exit 1, nothing in the store, the index or the mirror |

`scripts/web/themes.test.mjs` plays a preview build in Chrome Headless Shell against a local static server standing in for R2, with a mirror folder `gp theme publish` wrote:

```sh
GP_WEB_PREP_FLAGS=--themes scripts/web/build-preview.sh OUT templates/rivet-rodeo ~/.game-printer/work/assets/rivet-rodeo
gp theme publish THEME --game rivet-rodeo --no-gate --index FILE     # with a folder mirror in $GP_HOME/mirrors.toml
node scripts/web/themes.test.mjs --dist OUT --mirror MIRROR --theme ID --browser CHROME_HEADLESS_SHELL --shots DIR
```

It checks that the index comes from the mirror and the theme is listed "download"; that choosing it downloads it (the manifest and each file from the mirror), draws it and doesn't restart the run; that a reload with the mirror down reads the index from the cache and draws the kept theme from it; that the settings screen removes it and nothing of it is left in Cache Storage; and that a file changed on the mirror is refused and the theme isn't drawn. On 2026-11-04 every check passed for a test theme of Rivet Rodeo (61 files downloaded, 2.1 MB package).

## Not done yet

- No runtime draws a source package yet: the public page and the Bevy build list only look packages, by `fits` (ROADMAP, ROM themes, steps 8 and 12).
- A source package has no preview picture. The plan's preview is the look drawn over a blank base, so it shows only the maker's art; `gp assets stub` stubs only tiles and palettes, so a stub base would still hold other slots' ROM art.
- The index records each model's slot fingerprint and result, but no runtime offers or greys out a theme by them yet (ROADMAP, ROM themes, step 9).
- A recheck that fails or draws a different look records nothing: the model stays `unchecked` in the index, and the reason is only in the recheck's output.
- The index's `originality` and `readability` lines are the publisher's model's; a model added by a recheck has its lines only in the recheck's output, since the package's checks.json is never changed.
- The settings screen doesn't list a theme it can't offer. The public page names a template theme the lift couldn't offer in a line above the game, but a published theme whose `fits` differs is still dropped with no word (`gp_present::catalogue::offered_fits`), and a restyle naming a slot the model lacks is still passed over with no word.
- No mirror is set up, and `themes/allowed_signers` lists nobody yet.
- The Bevy build's download is compiled (a generated Rivet Rodeo checks with it) but not played: `play` opens a window, and no headless run reads the settings screen. The web preview's is tested end to end.
- The Bevy WebGL2 build has no curl and no file system, so it lists no published themes.
- `scripts/web/themes.test.mjs` is run by hand; no `just` recipe runs it yet.
- A theme's preview picture is in the index by hash but not shown on the settings screen.
