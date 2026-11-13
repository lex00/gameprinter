# Asset stubs and the slot catalog

A model refers to its assets through slots (docs/model.md, "Asset slots"). The simulation reads some of them, such as a level's grid and its spawn tables. Everything else is presentation, which today means tiles and palettes. A stub is a placeholder with the same shape as a presentation slot's asset, so the game plays exactly as before while its art is missing or being made. The slot catalog gives an agent a record per slot, and `assets_place` puts a replacement into a working copy and checks that the simulation didn't change.

This is Milestone 3 work ([the roadmap](../ROADMAP.md)), adopted by the user on 2026-10-28 from a research spike whose snapshot, with the prior art and the measurements on the stubbed ROM, is game-printer's docs/research/asset-stubs.md (history, not carried here).

Brought from game-printer's docs/stubs.md. In gameprinter `gp assets stub` refuses an arcade lift (Tapper's) in words: no model slot names an arcade lift's pictures yet (`gp_interp::Lifted` reads NES maps), so there are no presentation slots to stub.

## Run it

```sh
just mosshop
gp assets lift target/mosshop/mosshop.nes --map assets/mosshop.toml -o $GP_HOME/work/assets/mosshop --force
gp assets stub templates/mosshop --lifted $GP_HOME/work/assets/mosshop -o /tmp/mosshop-stubs --original-license Apache-2.0
gp interp run templates/mosshop --baseline baselines/mosshop/stomp-then-pit.toml --assets /tmp/mosshop-stubs
```

The last command prints the same digest as it does on the lifted assets. Through the agent tools, `surface_describe` returns the catalog and `assets_place` replaces a tiles slot or one entity's sprite ([agent tools over MCP](mcp.md)).

| Command or tool | What it does |
|---|---|
| `gp assets stub MODEL --lifted DIR -o OUT` | Copies the lifted directory to `OUT`, outside any git work tree, with every presentation slot of a format it can stub replaced by a stub, and writes `OUT/slots.toml`. `--original-license SPDX` records the originals' license (Moss Hop's are Apache-2.0); without it they're `LicenseRef-template-source` when the model directory holds an open-tier `template.toml`, and `LicenseRef-own-copy` otherwise. An asset any simulation slot names is never stubbed. `--force` writes over an earlier stub or lift. `--json` prints the catalog |
| `gp model schema slots` | The JSON Schema of `slots.toml`, kept in `schema/slots.json` |
| MCP `surface_describe` | Its `slots` is the catalog: one record per slot |
| MCP `assets_place` | Replaces a tiles slot's image, or the tiles one entity's sprite uses, in a working copy |

## Where the line is

`gp_interp::lifted::simulation_slots` is the one definition. Any slot a level names as its `grid`, `tileset`, `layout`, `spawn_objects` or a spawn's `table` is simulation, and every other slot is presentation. The interpreter reads only the simulation slots' files, so a presentation slot can't change a state hash. Stubs, themes and `assets_place` draw the line with this function too.

| Slot kind | Moss Hop | Stub |
|---|---|---|
| `tiles` | presentation | Each 8x8 tile shows its number in hex on a grid line (index 1 for tiles 0 to 255, index 2 after); the rest is index 0 |
| `palettes` | presentation | Each palette is `[0x0F, h, 0x10+h, 0x30+h]`, the hue `h` from the role whose sprites use it (player green, enemy red, and so on), gray when none or several do |
| `metatiles` | simulation (its `solid` property) | None yet; copied |
| `level-grid`, `level-objects`, `spawn-objects`, `table` | simulation | Never: a new level is a model change, held to the contract |
| `font`, `screen-text` | none | None yet; copied |

A slot can mix the two by field: a metatile's four tile numbers are art, its properties are simulation. The catalog lists them as `art_fields` and `simulation_fields`.

Stubs are made from the asset map's counts and the model's roles only, never from a ROM or a lifted file, and the same inputs give the same bytes. Nothing commits them, because the pre-commit hook and `gp template check` refuse binary files and a stub can always be made again. The license on every stub is `CC0-1.0` (the user's decision of 2026-10-28), so any fork, sold or not, may keep them.

A commercial template can have stub art with no ROM, but its simulation slots still come from the person's own copy through `gp template extract` or `gp game fetch` ([templates](templates.md)), or are replaced by new levels as model edits.

## The catalog

Each record is worked out from the model, the contract and the asset map when it's asked for. So it can't go stale, and it holds no game data. The status and provenance come from `slots.toml` beside the assets.

| Field | Meaning |
|---|---|
| `name`, `kind`, `role`, `from`, `description`, `item_roles` | The slot as the model declares it |
| `reads` | `simulation` or `presentation` |
| `contract_level` | The level `assets.<name>` resolves to in the contract |
| `format`, `files` | The asset map format, and the slot's files in the assets directory |
| `shape` | What a replacement must keep. Tiles: `tile_size`, `tiles`, `tiles_per_row`, `image_size`, `colors`. Palettes: `palettes`, `colors_per_palette`, `color_space`, `layout`. Metatiles: `metatiles`, `names`, `art_fields`, `simulation_fields`. Levels: `size`, `legend`, `cell_size`. Tables: `rows`, `columns`. Object lists: `entries`, `bytes` |
| `used_by` | Each entity that draws from the slot, with its `role` and `palette`; for a tiles slot also its `tiles`, `width`, `image_size` (the image `assets_place` takes for it) and `flip_when_facing_left` |
| `accept` | The checks a replacement must pass |
| `status` | `original`, `stub` or `replaced`, from `slots.toml`; without one, `original` when the assets are there and `empty` when they aren't |
| `provenance` | The slot's entry in `slots.toml`, below |

The Moss Hop record for `tiles`, as `surface_describe` gives it after `assets_place` (trimmed):

```json
{
  "name": "tiles", "kind": "tiles", "role": "graphics",
  "reads": "presentation", "contract_level": "style",
  "format": "chr-2bpp", "files": ["tiles.png"],
  "shape": { "tile_size": [8, 8], "tiles": 512, "tiles_per_row": 16, "image_size": [128, 256], "colors": 4 },
  "used_by": [
    { "entity": "player", "role": "player", "tiles": [16, 17, 18, 19], "width": 2, "image_size": [16, 16], "palette": 4, "flip_when_facing_left": true },
    { "entity": "slug", "role": "enemy", "tiles": [20, 21, 22, 23], "width": 2, "image_size": [16, 16], "palette": 5, "flip_when_facing_left": true }
  ],
  "status": "replaced",
  "provenance": { "origin": "generated", "license": "CC0-1.0", "sha256": "...", "generator": "..." }
}
```

## slots.toml

Format 0.1.0, one per assets directory: beside a stub's files, and in a working copy's `assets/`. Its schema is `schema/slots.json`.

```toml
format = "0.1.0"
model = "mosshop"

[slots.tiles]
status = "replaced"
origin = "generated"
license = "CC0-1.0"
sha256 = "..."
generator = "..."
note = "player's sprite, tiles [16, 17, 18, 19]"
```

| Key | Meaning |
|---|---|
| `format`, `model` | The format version and the model's name |
| `status` | `original`, `stub` or `replaced` |
| `origin` | `source` (lifted), `stub`, `generated` or `uploaded` |
| `license` | An SPDX expression. `CC0-1.0` for stubs. Originals are `LicenseRef-template-source` (the template source's terms) when the model directory holds an open-tier `template.toml`, which `template_fork` copies, and `LicenseRef-own-copy` (from the person's own copy, never shared) otherwise; `gp assets stub --original-license` overrides both |
| `sha256` | The digest of the slot's files: `sha256sum` lines sorted by name, hashed again, as the template manifest's `lifted_sha256` |
| `generator`, `author`, `note` | Optional: what made it, who, and one line on it |

Unknown keys are refused, as in the model. No asset map may name an asset `slots`, so a lifted file can't take its place.

## Placing an asset

`assets_place` works on tiles slots so far. Its arguments are in [agent tools over MCP](mcp.md#assets_place). It refuses, before writing anything, when:

| Check | Refused when |
|---|---|
| Slot | It isn't a tiles slot, it has no `from`, the simulation reads it, or a simulation slot fills from the same asset |
| Baselines | The model's source has no baselines, so no run could show the simulation unchanged |
| License | It isn't an SPDX-style expression, or it's `LicenseRef-own-copy` |
| Image | It isn't an indexed PNG of exactly the slot's `image_size`, or with `entity`, the sprite's `image_size`; a pixel's index is above 3; a pixel past the last tile isn't 0 |

Then it writes and checks, in this order:

| Step | What happens |
|---|---|
| Copy's assets | On first use the working copy gets its own `assets/`, copied from the lifted assets the server was given into a temporary directory with a `slots.toml` (every slot original, or the state the source directory had), then renamed into place |
| Write | The new sheet goes in by a temporary file and a rename. With `entity`, only that sprite's tiles change, laid out row by row as the sprite lists them |
| Pack | Refused if the copy's assets don't encode into their spans (`gp assets pack` code) |
| Simulation | Refused if any baseline of the model's source gives a different state digest after than before. The digest is the one `gp interp run` prints |
| Model | Refused if `gp model check` fails with the copy's assets |
| State | The slot becomes `replaced` in `slots.toml`, with its origin, license and new digest |

When a step after the write refuses or fails, the old image and the old `slots.toml` go back. The reply lists the tiles that changed and the packed bytes that differ, with the slot's new state, the number of baselines run and each one's digest. `run_preview`, `validate` and the model checks use a copy's own assets once it has them. Working copies live in the server's work directory, outside any git work tree, so nothing placed reaches the repo.

The simulation check can't fail for a slot that passes the slot check, since the interpreter doesn't read presentation slots' files. It stays as the guard against a slot classed wrongly, and against a future simulation that reads more than `simulation_slots` names.

## Tests

| Test | What it checks |
|---|---|
| `crates/gp-assets/src/stub.rs` | A stub tile shows its number and the pattern table; stub tiles pack like lifted ones and are the same bytes every run; palettes by role |
| `crates/gp-mcp/src/catalog.rs` | `slots.toml` round-trips and refuses unknown keys and other formats; the digest is `sha256sum` style |
| `crates/gp-mcp/src/place.rs` | Images held to their size and index range, including 8-bit indexed PNGs |
| `crates/gp-assets/src/stub.rs`, `no_asset_may_be_named_like_the_state_file` | A map naming an asset `slots` is refused |
| `crates/gp-cli/tests/assets.rs`, `stubs_keep_the_simulation` | On Moss Hop built from source: stubbed slots, the simulation's files copied byte for byte, the stubs pack, the same digest on stubs and lift, an existing stub refused without `--force` |
| `crates/gp-cli/tests/mcp.rs`, `an_agent_replaces_the_player_sprite` | Over MCP stdio: the catalog in `surface_describe`, a wrong size refused with no `assets/` made, a simulation slot refused, the player's sprite placed (tiles 16 to 19 changed, at most 64 packed bytes), `slots.toml` recording it and the originals as `LicenseRef-template-source`, the same `run_preview` state hash and the same `gp interp run` digest as the lift, and the copy's assets packing into a ROM |
| `crates/gp-cli/tests/model.rs`, `schema_files_are_current` | `schema/slots.json` is what `gp model schema slots` prints |

All of them run in `just check`, on made-up data or Moss Hop.

## Limits

- Only tiles and palettes have stubs, and only tiles slots can be placed. The other kinds of art are designed in the research snapshot and wait on their slots. Core 0.3.1 gives every metatile and palette a role, and a theme can restyle by role on top of stubs ([themes](formats/theme.md)).
- A stub labels every tile, so Moss Hop's sky, tile 0, is a field of `00` labels.
- The catalog's `shape` is the ROM's: a replacement must fit the NES pattern table. Larger art for the Bevy and web builds needs presentation shapes in the model (open-questions row, M4).
- The consistency critic is deterministic metrics, with no image model behind it, and it judges what can be measured (palette coherence, size, contrast against the backdrop, style) rather than whether art looks right ([host.md](host.md#critic)).
- The readability check is built for theme stacks ([themes](formats/theme.md)); it doesn't run on stubs alone yet.
- Two calls on one working copy at once aren't locked against each other, and `slots.toml`'s digests aren't compared with the files (open-questions rows, M3).

## Generation and publishing

`assets_generate` makes placeholder art from a slot's shape and an entity's role, offline and deterministic, and places it with `assets_place`'s checks ([mcp.md](mcp.md)). `assets_place` takes palettes slots as well as tiles. `build_publish` refuses any slot whose license names `LicenseRef-own-copy`, or that is an original in a copy of a commercial template, because that art comes from the person's own copy.
