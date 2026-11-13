# Original templates

Brought from game-printer's docs/originals.md with the three originals, Badge In, Signal Failure and Swing State, which live in `templates/` beside Moss Hop. Rivet Rodeo, the original derived from Donkey Kong's template, is retired and not here. The Super Mario Bros. template the originals took mechanics from (`smb1-ntsc-prg0`) isn't carried either: its `inspired-by:` refs and `[[inspired_by]]` entries are provenance, read as data.

An original template is a game written directly as a model, with no source game behind it. Its levels and tilesets, tables and palettes are written by hand as data. Its tiles are generated stubs, and its runs are recorded on the model itself. This page covers the first three originals and the tooling they exercise: authored assets and model baselines recorded by search, reachability and composed mechanics with "inspired by" provenance, and the mechanic index that `registry_search` searches. It is the first and second bullets of Milestone 5 in [the roadmap](../ROADMAP.md). The templates are content for the tooling: they give the model and interpreter, composition and reachability, and the templates, stubs and MCP tools something new to run on that isn't Moss Hop or a commercial game.

## Run it

```sh
just originals         # build, check and replay all three (in just check)
```

| Command | What it does |
|---|---|
| `gp template build DIR` | Builds an original's assets from its `authored.toml`: the data image into `$GP_HOME/work/images/<id>.nes`, the lifted files and `slots.toml` into `$GP_HOME/work/assets/<id>` (or `-o`), where `gp interp` and `gp model check` look by default. Exits 1 when the committed asset map or the manifest's pins aren't what the build gives; `--write` updates them |
| `gp interp reach MODEL` | For each level (or `--level`), whether the goal can be reached by the player's own moves with the player alone (`gp_interp::reach`), and the jump's envelope. Exits 1 when a goal can't be reached |
| `gp interp reach MODEL --record` | Also searches each level with its enemies and rules for a run to the goal (`gp_interp::reach::playthrough`) and writes it as `MODEL/baselines/<level>.toml`. Exits 1 when no run is found |
| `gp interp baselines MODEL` | Replays every model baseline in `MODEL/baselines/` from its level and compares its digest and expectations. Exits 1 if any differs. `--write` rewrites the ones that differ |

| Command | JSON fields with `--json` |
|---|---|
| `template build` | `ok`, `id`, `dir`, `sha256` (the image's), `image`, `assets`, `asset_map_sha256`, `lifted_sha256`, `stale` (the pins that were out of date), `written` |
| `interp reach` | `ok`, `model`, `levels`, each with `level`, `reachable`, `stands`, `goal_path`, `moves`, `envelope` (`rise`, `airtime`, `walk_distance`, `run_distance`) and, with `--record`, `recorded` (the file, or null) |
| `interp baselines` | `ok`, `model`, `baselines`, each with `baseline`, `level`, `ok`, `problems` |

`gp template check`, `extract` and `pack` take an original as they take Moss Hop ([templates](templates.md)): `extract` builds the image itself, so no ROM is involved anywhere.

## The three templates

| | Badge In (`templates/badge-in`) | Swing State (`templates/swing-state`) | Signal Failure (`templates/signal-failure`) |
|---|---|---|---|
| Premise | The return-to-office mandate: badge in at the far end of each floor before the standup | A campaign: run the trail from a town hall to election night | The commute: make the train before the doors close |
| Levels | `lobby`, `open_plan`, `hot_desks` (48, 64 and 64 cells wide) | `town_hall`, `debate_stage`, `election_night` (56, 64, 72) | `platform_9`, `interchange`, `last_train` (56, 64, 72) |
| Player | Moss Hop's run and jump | Super Mario Bros.' run and jump, bands and held gravity included | Its own: acceleration, slowing, a held jump, coyote time and a jump buffer |
| Others | Middle managers (walk), a printer that jams and slides (a shell machine), reply-all chains (fly), coffee | Hecklers (hop, then `become` a walker), pundits (path with `wait_near`), attack ads (fly, aimed), pollsters (patrol from a spawn table), donations | Fare inspectors (walk, turning at ledges), pigeons (walk and hop), platform screen doors (path, a hazard), delay announcements (fly), free papers |
| Rules | Lives, a clock per floor (`clock.reached_min` costs a life), three floors to win | Approval as lives, days per stop, three stops | Patience as lives, a departure time per platform, three trains |
| Facts | 242, all stated | 331 | 259 |
| Mechanics from | Moss Hop and Super Mario Bros. | Super Mario Bros. and Moss Hop | None |

Each game lasts one to three minutes, in line with the positioning (game-printer's docs/positioning.md, not carried here). The satire is in the premise, the names and the descriptions, which are presentation: a fork can rename everything and keep the mechanics. Each template's `AGENTS.md` has its change recipes.

## Authored assets

A model fills its slots from an asset map's assets ([assets](assets.md)). A lifted game's map says where each asset is in its ROM. An original has no ROM, so its template holds the assets as data in `authored.toml`, and `gp_assets::authored` builds the image the asset map describes: the assets one after another in PRG and CHR, with the same byte formats a cartridge's assets have, behind an iNES header. The image holds data and no code, so it doesn't boot. Every tool that reads lifted assets (the interpreter, `gp model check --assets`, `gp assets stub`, `gp assets pack`, the MCP tools) then reads an original's as it reads Moss Hop's, because the lift that makes them is the same code.

```toml
format = "0.1.0"
source = "badge-in"
title = "Badge In"

[[asset]]
name = "tiles"
format = "chr-2bpp"
tiles = 256
generate = "stub"

[[asset]]
name = "metatiles"
format = "metatiles"
properties = ["solid"]
metatile = [
  { name = "air", char = ".", tiles = [0x00, 0x00, 0x00, 0x00], solid = 0 },
  { name = "carpet over concrete", char = "#", tiles = [0x01, 0x02, 0x03, 0x04], solid = 1 },
]

[[asset]]
name = "lobby"
format = "level-grid"
metatiles = "metatiles"
rows = [
  "................................................",
  # ... 15 rows, one character per 16x16 cell
]
```

| Format | Keys | Becomes |
|---|---|---|
| `chr-2bpp` | `tiles`, `tiles_per_row` (default 16), `generate = "stub"` | Stub tiles, each showing its number (docs/stubs.md), `CC0-1.0` |
| `palette` | `palettes`, lists of four NES color numbers | A palettes asset |
| `metatiles` | `properties` (names), `metatile`: each with `name`, `char` (the one character levels draw it with), `tiles` and a byte per property | A metatiles asset; the chars are the levels' legend |
| `level-grid` | `metatiles` (the asset whose chars it uses), `rows` | A level grid; every row the same width |
| `table` | `columns` (names), `rows` (one list of numbers per row, each 16 bits); `bits = 8` for bytes; `count` to pad with rows of zeros, and `set` (`[row, column, value]`) for a long table that is mostly zeros | A table, every column 16-bit, or 8-bit with `bits = 8` |
| `byte-rows` | `width`, `rows` (lists of bytes) | Rows of bytes, as a lifted game's `byte-rows` |
| `ppu-strings` | `charset` (as the map's), `string`: each a `name`, an optional `note` and `updates` of `ppu` with `tiles`, `text` or `repeat` and `tile`, and `vertical` | PPU update strings, each ended by the byte 0 |

A `chr-2bpp` asset may be drawn instead of stubbed. Its `file` names a text picture in the template directory, `tiles_per_row` tiles wide with one character a pixel (`0` to `3`, `.` for 0, `#` lines are comments). Its `charset` gives the tiles that stand for characters, as a lifted map's does. The `table` options, `byte-rows`, `ppu-strings`, `file` and `charset` came with authored format 0.1.1 (2026-11-03) (for game-printer's Rivet Rodeo, retired since), which fills the slots of a model written for a lifted game.

`source` is the template's id, which the model's slots name in `from.source`. The build refuses two assets with one name, and a char that two metatiles share or that isn't one character. It also refuses a metatile missing a property and a cell no metatile draws. Rows of different widths, a color past 0x3F and a table value past 16 bits are refused too.

The build writes the asset map to `assets/<id>.toml`, the repo's place for asset maps, and the template's manifest pins it the way Moss Hop's is pinned: `source.accepts` holds the image's headerless SHA-256, `recipe.asset_map_sha256` the map's and `recipe.lifted_sha256` the lifted files'. A change to `authored.toml` changes all three, `gp template build` reports them stale and `--write` rewrites them, as `just docs` rewrites CLI.md. `gp template check` builds the image and fails when the committed map isn't the one it builds or `accepts` doesn't hold its hash.

The build's `slots.toml` marks each slot filled from a stub `stub`, with origin `stub` and license `CC0-1.0`. Every other slot is `original`, with origin `source` and the template's own terms (`LicenseRef-template-source`). The slot catalog and `build_publish` therefore see the art as replaceable placeholders.

### The manifest

An original is an open-tier template (we own it outright) whose `source.how` is `authored`, and manifest format 0.1.1 adds `how = "authored"` and `inspired_by` to 0.1.0 manifests, so a reader of 0.1.1 reads every one of them.

```toml
[template]
format = "0.1.1"
id = "badge-in"
title = "Badge In"
tier = "open"

[source]
id = "badge-in"
platform = "nes"
how = "authored"
accepts = ["617ea299d89daa6f608fc26c263d2b71fafa2e6af19d45cd3db678e9a7d7e6e2"]

[recipe]
asset_map = "../../assets/badge-in.toml"
asset_map_sha256 = "..."
lifted_sha256 = "..."
```

`platform = "nes"` says whose formats the image uses and that the input device is the NES pad; nothing runs on a console. `gp template check` doesn't compare an original's files with its image, since the image is made from them and they would all match it; the 256-byte rule for files with no ROM to compare still applies (docs/templates.md, "The asset-free check").

## Model baselines

A ROM baseline (`baselines/<source>/`) is an input and the state hashes a console run of it gives. An original has no console run, so its baselines run on the model: `gp_interp::baseline`, one file per level in the template's `baselines/`.

```toml
format = "0.1.0"
model = "badge_in"
level = "lobby"
description = "Plays lobby from its start to its goal; found by gp interp reach --record"
recorded = "search"
frames = 445
holds = ["a@1-40", "left@1-8", "a@42-81", "b@42-81", "right@42-81"]
digest = "6a791ae1f864ab33011d009b25a20dae4b158adec2ea5ab2799b29c40357b8ef"

[[expect]]
frame = 445
path = "rules.game.state"
equals = "badged_in"
```

| Key | Meaning |
|---|---|
| `model`, `level` | The model's name and the level the run starts in: the model is run with `start_level` set to it, so every level has a baseline of its own |
| `recorded` | `search` or `hand`: how the input was made |
| `frames`, `holds` | The input, as `gp input-log generate --hold` takes it |
| `digest` | What `gp interp run` prints for that input from that level: the digest of every frame's state hash |
| `expect` | A model path's value, as `gp interp run` prints it, after `frame` frames |

### How they're recorded

`gp interp reach --record` searches for each run. From the level's power-on state it tries the same moves `reach` does (walking a step, jumps with and without running, held short or long, steered for part or all of the jump), each played as button frames in the whole world: every enemy, timer and rule runs. It goes breadth first over the places the player comes to stand, keeping the world as it is when the player stands there and the buttons that got it there, and stops at the first move that reaches the goal. A move that ends in a death, or in the game's rules freezing the world without the goal, is dropped. The run then goes on 30 frames so the level's end state is in it, and the file records its digest and, at the last frame, the level, every state machine's state and every counter. The search is the simulation (`gp_sim::step_with_ext`), so the buttons replay to the same run in the interpreter; a test checks that on made-up levels, and `gp interp run` on the lobby's holds prints the baseline's digest.

Recording all of a template's levels, with the reachability search beside it, takes 0.4 to 3 seconds in a release build; the nine runs searched between 616 and 946 places each. They hold the jump button a lot, as the search does, so they show that each level can be finished, not how a person plays it.

A baseline can be written by hand too: `recorded = "hand"`, the holds, and `digest = ""`. `gp interp baselines --write` fills in the digest and the end state. The same flag records new runs' results after a change meant to change play; for a change that moves the goal or a gap, `gp interp reach --record` finds new runs.

The digest covers the canonical state, so a canonical-state version bump (core 0.4.0 took it from 4 to 5; it is 18 now) changes every baseline's digest without changing a run. Re-record with `gp interp baselines --write` for all three originals and read `git diff templates/*/baselines`: only the `digest` lines should move. That was done on 2026-10-30 for the bumps since core 0.4.0, which had never been re-recorded: frames and end states were unchanged. `just originals` stops at the first original that fails, so the other two can fail unseen.

`run_preview` and `assets_place` read a template's model baselines when its source has no ROM baselines, so `assets_place` proves the simulation unchanged on an original the way it does on Moss Hop. They run each input from the model's start level. `validate` still holds a model to its contract against ROM baselines only (open-questions row, M5).

## Reachability

`gp interp reach` puts `gp_interp::reach` (from the mechanics research (game-printer's docs/research/mechanics.md, not carried here)) behind a command. With the player alone in a level it lists the places it can stand, whether the goal is one of them and the jump's envelope:

| Template | Rise | Airtime | Carried walking, running |
|---|---|---|---|
| Badge In | 48 px | 40 frames | 40 px, 80 px |
| Swing State | 71 px | 59 frames | 88 px, 142 px |
| Signal Failure | 63 px | 53 frames | 66 px, 106 px |

Every level of the three is reachable. Writing them found one that wasn't: Badge In's ninth floor had a four-cell gap up to a desk that the player alone can't clear, while the search with enemies in the level crossed it by bouncing off a reply-all chain. The gap is now three cells. Reachability alone and a run with enemies answer different questions, and an original's check runs both.

## Composition

Badge In and Swing State take their mechanics from Moss Hop and Super Mario Bros. by the composition rules in the mechanics research (game-printer's docs/research/mechanics.md, not carried here) ("Composing games from mechanics"). The composer is a person for now: no tool rescales or checks a composition yet (open-questions row, M5).

| Rule | How it was applied |
|---|---|
| One writer | Each entity has one of `walk`, `patrol`, `fly` and `path`, as the schema already requires |
| Interfaces met | Stomps assume a falling player, which both players have. The printer's sliding shell "kills enemies it touches" in Super Mario Bros.; the model has no enemy-to-enemy contact, so here it slides through them, an assumption reported rather than patched |
| One order | Both games use the system order Moss Hop states |
| Scale | All three sources use 16-pixel tiles, so pixel speeds copy unchanged. The originals run at 60 frames a second, the NES's 60.1 rounded, a change of about 0.16% in every speed per second, below the 3% the contracts allow |
| Ratios | A speed taken from a source keeps its ratio to the source player's walking speed. The goomba walks at 0.5 against Mario's 1.5, so Badge In's middle manager walks at a third of the associate's 1, rounded to 1/65536 as `0.3333282470703125`. The shell slides at twice the walk (2), the bullet at the walk (1), and Swing State's pollster at half the candidate's 1.5, Moss Hop's slug ratio, as 3 pixels every 4 frames. Every stomp depth is at least the player's `max_fall` (4), so a stomp can't be fallen through |
| Playable | `gp interp reach` on every level, and a recorded run with the enemies in |

Each taken value's evidence says where it came from and what was converted, with a ref of the form `inspired-by:<template>` (not a file, so a packed template's check doesn't need the other template), and the manifest's `inspired_by` records the same as data:

```toml
[[inspired_by]]
template = "smb1-ntsc-prg0"
took = ["entities.goomba.walk", "entities.goomba.fall", "entities.koopa_green.machine", "entities.bullet_bill.fly"]
into = ["entities.middle_manager.walk", "entities.middle_manager.fall", "entities.printer.machine", "entities.reply_all.fly"]
note = "Speeds kept at their ratio to the player's walk; the sliding shell kills instead of hurting; frames and sprites dropped"
```

game-printer's `gp template check` required one `into` for each `took`, each `into` path in this model and, when the other template was beside it in `templates/`, each `took` path in that one's model; gameprinter's `check` holds a ROM to the pins only, and doesn't read `inspired_by`. Only mechanics and their numbers are taken, as the research's clean-room table allows: no level or sprite of a commercial game is in an original, and neither is a name or text from one.

## The mechanic index

`registry_search` ([agent tools](mcp.md#registry_search)) indexes every template by the mechanics its model uses, worked out from the model each time it searches (`gp_mcp::registry::mechanics`), so the index can't go stale. A mechanic is named by its carrier's role and the component, with a suffix for the options that change how it plays:

| Kind | Names |
|---|---|
| Components | `<role>:run`, `jump`, `walk`, `fall`, `fly`, `hop`, `path`, `patrol`, `contact`, `machine`, `ext` |
| Options | `run.acceleration`, `run.deceleration`, `run.skid`, `run.run_button`, `run.air_control`; `jump.variable_height` (`gravity_held`), `jump.bands`, `jump.coyote`, `jump.buffer`, `jump.move_first`; `walk.turn_at_ledges`; `fly.aimed`; `path.wait_near`; `contact.stomp`, `contact.keep_on_stomp`, `contact.<touch>` (from the entity and its machine's states: `kick`, `collect`, `kill-player`, `hurt-player`, `nothing`); `become` |
| The game's | `game:timer` (a counter with `every`), `game:conditions` (a transition with `if`), `game:restart`, `game:level_chain`, `game:reset`, `game:exits`, `game:spawn_ahead`, `game:spawn_table`, `game:goal`, `game:pits`, `game:camera.no_scroll_back` |

A search can require mechanics (`mechanics`, as `role:mechanic` or a mechanic for any role), ask for the templates that took from one (`inspired_by`), and rank by likeness to an example (`like`, a template): each result's `similarity` is the share of mechanics the two have in common (Jaccard), and results come most alike first. Each result also lists its `mechanics`, its `inspired_by` entries and `taken_by`, which holds the templates whose provenance names it. On game-printer's six templates, `like = "badge-in"` ranked Signal Failure (0.63) and Swing State (0.62) above Moss Hop (0.37) and Super Mario Bros. (0.34), each list running from most alike to least.

## Tests

| Test | What it checks |
|---|---|
| `crates/gp-assets/src/authored.rs` | The same file builds the same image and map; the lift of the image is the authored data and packs back to it; a cell no metatile draws and a missing property are refused |
| `crates/gp-interp/src/reach.rs`, `a_playthrough_with_the_slugs_replays_to_the_goal` | A run found with Moss Hop's slugs in a made-up level replays to the goal from power-on |
| `crates/gp-interp/src/baseline.rs` | Holds from button frames round-trip through the input log; a baseline round-trips and refuses an expectation past its frames |
| `crates/gp-cli/src/template_build.rs` | The pins are rewritten in their own table only |
| `crates/gp-mcp/src/registry.rs` | The index names mechanics by role; likeness is the share in common |
| `crates/gp-cli/tests/mcp.rs`, `an_agent_finds_moss_hop_generates_art_and_checks_a_publish` | Over MCP: `inspired_by = "mosshop"` finds Badge In and Swing State, a `mechanics` filter, and `like = "mosshop"` ranks Moss Hop first with Badge In in its `taken_by` |
| `just originals` (in `just check`) | Each original builds with current pins, holds its build to its pins (`gp template check`), checks its model against its build and in canonical form, and replays its baselines |

## Limits

- The originals are platformers, the schema's one genre. Their art is stub tiles until someone draws it, and the stub tiles fill the NES pattern table's shape (docs/stubs.md, "Limits").
- No tool composes mechanics yet: the conversions above were done by hand, and the composition check (one writer, interfaces, order, ratios) isn't built.
- The search finds a run, not a good one. It holds the jump button whenever it can, and with `PLAYTHROUGH_LIMIT` (4,000 places) it may miss a run a level does have; a level with momentum puzzles needs the reachability search's own limits lifted first (open-questions, "Does the reachability search carry momentum and see enemies?").
- A model baseline holds what the interpreter does. A built module's run of an original isn't checked against it yet (conformance runs on Moss Hop and Super Mario Bros.).
- `validate` and the contract's metric checks of borrowed mechanics against their sources' profiles aren't run on originals: the contract says what's intended, and nothing measures it yet.
