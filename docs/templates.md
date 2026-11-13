# Templates

A template is a game's model with no game data in it, which someone can clone and turn into a working local copy from their own ROM. A commercial game's template ships the model, the contract and the slot definitions with every slot that comes from the original empty. It declares the ROM it needs by hash and fills the slots from the person's own copy with its recipe, an asset map. It came from game-printer's docs/templates.md. There a template was a folder of its own under `templates/`; here a game's template is its package folder in the games checkout (`$GP_GAMES_DIR`, or `games/` beside the tool repo), with `template.toml` beside `package.toml`. Donkey Kong is `games/donkey-kong` there. Moss Hop, the reference game, stays in the tool repo as `templates/mosshop`.

[Bring in a game](../site/src/content/docs/bring.md) starts a new one: `gp game new` writes the folder, its `template.toml` and pins that hold against the ROM.

## Run it

```sh
gp template check ../games/donkey-kong
gp template check templates/mosshop --rom target/mosshop/mosshop.nes   # in just check
gp package scan ../games/donkey-kong
gp template served ../games/donkey-kong
```

| Command | What it does |
|---|---|
| `gp template check DIR` | Holds a ROM to the template's pins: the ROM's headerless SHA-256 is in `source.accepts`, the asset map's SHA-256 is `recipe.asset_map_sha256`, and what the map lifts from the ROM has the digest `recipe.lifted_sha256`. The ROM is `--rom ROM`, or the first file in `$GP_ROM_DIR` whose hash the template accepts. For an arcade template the romset's identity is checked, and the arcade map and its lift when there is a `[recipe]` |
| `gp template check DIR --original` | For a template derived from another with stages of its own: each level's cells, ladders and things against the base's, lifted in memory from your ROM of the base, refused when too close, and any data line of the template's built files that the base's lifted files hold ([Originals](#originals)) |
| `gp template build DIR` | Builds an original template's assets (`source.how = "authored"`) from its `authored.toml`: a data image (to `$GP_HOME/work/images/`), its asset map and the lifted directory (`-o`, by default `$GP_HOME/work/assets/<source>`). Exits 1 when the asset map or the manifest's pins are stale, unless `--write` updates them |
| `gp template served DIR` | Checks that a package's open version holds none of the original's data, and with `--folder` that every file of a folder to be served holds none: no line of 24 characters or more of the original's lifted files, and no run of 32 of its ROM's bytes, raw or written as numbers. Exits 1 on any find and 2 when a ROM isn't found |
| `gp package scan DIR` | Sorts every file and field of the package into ours, reference, from the ROM and needs rewrite, and rebuilds the from-the-ROM part from your ROM ([the package scan](package-scan.md)) |

`check` prints each step (`rom`, `asset map`, `lifted`) with its value, and `FAILED:` before the one that doesn't hold. It exits 1 on any, and 2 when it can't run: no `template.toml`, a manifest that doesn't parse or a ROM it can't read. `--json` gives `ok`, `rom_sha256`, `asset_map_sha256`, `lifted_sha256` and `files`.

Not here yet: `gp template extract` (the lift into a work copy of the template), `gp template pack` (a template written with its asset map inside, as a folder or a zip), `gp template derive` and `just derive` (a derived template's files made from its base's), `gp template font` (the font check) and `gp template cites`. A package goes to people through a registry instead of a packed copy ([Run a registry](../site/src/content/docs/registry.md)).

## What a game's folder holds

| File | Holds |
|---|---|
| `template.toml` | The manifest, below |
| `package.toml` | The package: its name, version, tier, the tool it needs, the files a player loads, and a `[[rom.entry]]` for each ROM with its hashes, its asset map and that map's pins ([packages](packages.md)) |
| `roms.toml` | Each of the game's ROMs as a `[[source]]` entry: hashes and facts, never the game ([Sources](sources.md)) |
| `model.toml` | The model. Every slot names its recipe (`from`) and holds nothing |
| `fidelity.toml` | The contract |
| `assets/` | The asset maps, one for each ROM entry: where each asset is in the ROM and its format ([Asset lifting](assets.md#the-asset-map)) |
| `ext/` | The hand-written behaviours behind the model's hooks, as a Rust crate (`[ext] crate`). `gp game build` builds it into the game's module |
| `mapping.toml` | The state mapping from the original's RAM to model paths ([state mapping format](formats/state-mapping.md)) |
| `corpus.toml`, `inputs/` | The scoring corpus: its clips as presses from power-on ([scoring corpus format](formats/scoring-corpus.md)) |
| `profiles.toml`, `probes.toml`, `observe.toml` | The physics profiles and quirk switches, the probe recipes `gp mechanic probe` runs, and how the original is observed (such as its level, read from RAM) |
| `level-kit/`, `themes/` | The level kit for new stages, and the theme kit and themes |

A template never holds:

- a ROM, a dump, a save or a movie, whatever it's named;
- a lifted asset file, or a copy of one's lines;
- a run of the ROM's bytes in any form the package scan reads;
- a file over 1MB, or a binary file the pre-commit hook doesn't allow.

The pre-commit hook refuses most of these on commit (`scripts/romguard/rom_text.py`, [Sources](sources.md#rom-bytes-in-text)), and `gp package scan` and `gp template served` check the folder itself, since a package also goes out through a registry.

## The manifest

Donkey Kong's, in part:

```toml
[template]
format = "0.3.1"
id = "donkey-kong-oe"
title = "Donkey Kong: Original Edition"
tier = "commercial"
model = "model.toml"
contract = "fidelity.toml"
clips = "corpus.toml"
game = "donkey-kong"

[source]
id = "donkey-kong-oe"
platform = "nes"
how = "rom"
accepts = ["45440d8ef512b4660dce0c753e110004aef96fa30f2967f9260e946d808db07d"]

[recipe]
asset_map = "assets/donkey-kong-oe.toml"
asset_map_sha256 = "35b718d2..."
lifted_sha256 = "34712477..."

[slots]
font = "font"
ground_runs = "layout"
palettes = "palettes"
# ... every slot of the model, with its role

[ext]
hooks = ["dk_barrel", "dk_carry", "dk_conveyor"]   # ... every hook the model uses
crate = "ext"

[scored_against]
corpus = "donkey-kong-oe-25m"
version = "7.0.0"
hash = "b3:39891..."
path = "."
```

| Key | Meaning |
|---|---|
| `template.format` | The manifest format. Donkey Kong's is `0.3.1`, Moss Hop's `0.1.0`; each version only added keys |
| `template.id` | The template's name: lowercase letters, digits, `-` and `_` |
| `template.tier` | `commercial`: every slot is filled from the person's own copy, and the source is a ROM. `open`: we own the source game outright, as for Moss Hop |
| `template.game` | Optional: the game id the template's source is one of (`donkey-kong` for `donkey-kong-oe`) |
| `template.model`, `template.contract` | The model file (`model.toml`) and the contract (`fidelity.toml`) |
| `template.clips` | Optional: the clips, `corpus.toml` for a game with a corpus plan |
| `source.id`, `source.platform` | The source, as the game's `roms.toml` names it, and its platform (`nes` or `arcade`) |
| `source.how` | `rom`, the person's own dump in `$GP_ROM_DIR`; `build`, a ROM built from source in this repo, with `source.build` saying how (`just mosshop`); or `authored`, an original whose `authored.toml` builds the image its asset map describes |
| `source.accepts` | The headerless SHA-256s the template accepts, as `gp roms scan` prints them, or an arcade romset's identity. No other ROM is used, so a renamed or re-headered copy works and another revision doesn't |
| `recipe.asset_map` | The asset map, relative to the folder |
| `recipe.asset_map_sha256` | The SHA-256 of that file |
| `recipe.lifted_sha256` | The digest of the files the map lifts from an accepted ROM: the SHA-256 of `sha256sum`'s output for the lifted files, sorted by name, so `cd assets && find . -type f \| sed 's\|^\./\|\|' \| LC_ALL=C sort \| xargs sha256sum \| sha256sum` gives the same digest (`shasum -a 256` on macOS) |
| `slots` | Every slot of the model, with its role |
| `ext.hooks`, `ext.crate` | Every hook name the model uses, and the crate in the folder that implements them |
| `scored_against` | Optional: the scoring corpus the game's scores are against, by `corpus`, `version` and `hash`, with `path`, its folder relative to the template (default `.`). `gp corpus score --write` sets it |
| `loads` | Optional, a list: ROMs a person may load to play the model that aren't its source ([below](#roms-to-play)) |
| `derive` | Optional: `from`, the base template's folder, for a template made from another's. `gp template check --original` reads it |

`package.toml`'s ROM entries carry the same pins: the scored entry's `asset_map_sha256` and `lifted_sha256` are the ones `gp package scan` checks its rebuild against.

## Originals

An original is a game whose data we wrote: its `authored.toml` holds its levels, pictures and tables in the formats a cartridge's assets have, and `gp template build` lays them out into a data image and the asset map that describes it. The image holds data only and doesn't boot. Its headerless SHA-256 is the one the template accepts, and the same `authored.toml` always builds the same image and map, byte for byte.

An original made from another template's game (`[derive] from`) keeps its base's engine and must not keep its base's stages. `gp template check --original` measures each level against the base's, read from your ROM of the base and lifted in memory, never written: the share of cells the two have the same (a level is refused at 60%), the ladders and the things placed. It also reports any data line of 24 characters or more of the template's built files that the base's lifted files hold. The built files are read from where `gp template build` writes them.

## ROMs to play

A template's `[[loads]]` lists ROMs a person may load to play its model that are not its source, and `package.toml` lists the same ROM as a `[[rom.entry]]` with a `role` other than `scored`. Donkey Kong (JU) is one for Donkey Kong: Original Edition: Original Edition's upper half is JU's program, and the art and the 25m, 75m and 100m levels are the same. JU has no 50m and no stage-clear scene.

```toml
[[loads]]
source = "donkey-kong-ju"
accepts = ["aa408f5a..."]
asset_map = "assets/donkey-kong-ju.toml"
asset_map_sha256 = "..."
levels = ["main", "m75", "m100"]
assets = { title_screens = "stage_screens", title_tiles = "tiles", title_sprite_tiles = "sprite_tiles" }

[[loads.without]]
what = "50m"
levels = ["m50"]
blank = ["climb_tops_50m", "conveyor_pattern"]   # ... the assets only 50m uses
```

| What | How |
|---|---|
| Loading one | The ROM is lifted through its own asset map. Each of the model's slots takes the ROM's assets, and an asset the model names that the map lacks takes the one `assets` names. Each `without` is something of the model's the ROM's game hasn't: its levels are passed over (`stages.skip`), its parts taken out of the model, and the assets only it uses left blank. Nothing is filled from anywhere else, so a JU ROM plays JU's game: 25m, 75m, 100m |
| What refuses one | It is not in `source.accepts`, so every tool that finds or checks the template's ROM by those hashes passes over it or refuses it |
| The check | The gate's loads guard (`gate::loads_problems`) fails a patch when `template.toml`'s `[source]` id or `accepts`, or `corpus.toml`'s `[target]` game or ROM, names a ROM the package loads only to play |

`gp roms add` and `gp roms scan` still identify a ROM to play, by its entry in the game's `roms.toml`.

## Limits

- The pins are of files other work changes. A change to Moss Hop's source changes its ROM's hash in `accepts`, and a change to an asset map or to what `gp assets lift` writes changes `asset_map_sha256` or `lifted_sha256`. Each fails `just check` on Moss Hop until the manifest is updated in the same commit, as for the baselines' hashes. `gp template build --write` updates an original's.
- `template.model` and `template.contract` must be `model.toml` and `fidelity.toml`, the names `gp model check` reads.
- The checks find data pasted in. They can't prove that nothing in a model was derived from the game, which is what the model's evidence is for.
