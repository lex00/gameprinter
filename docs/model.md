# The game model

A game model is an engine-neutral description of a game, written as TOML, that a runtime runs: the interpreter ([interpreter](interpreter.md)) and the web build both read it. This page says how to read and write one. Moss Hop's model is in `templates/mosshop/`, written by hand from the game's source in `games/mosshop/mosshop.s`; a commercial game's model is in its package in the games checkout, such as Donkey Kong's in `games/donkey-kong/model.toml`. The page came from game-printer's docs/model.md; the examples from Super Mario Bros., Super Mario Bros. 2 and Mega Man 2 went with those games, the version history before features is cut, and the Bevy generator is gone.

## Run it

```sh
just check    # builds Moss Hop, lifts its assets, and runs gp model check and gp model fmt --check on templates/mosshop
```

| Command | What it does |
|---|---|
| `gp model check DIR` | Checks `DIR/model.toml` against the schema, and `DIR/fidelity.toml` and `DIR/profiles.toml`, if they are there, against the model ("Profiles" below). Prints the facts by source, the contract's rules by level, the features used and the profiles |
| `gp model check DIR --assets LIFTED` | Also checks each asset slot filled from the lifted directory's asset map against the asset it names |
| `gp model check DIR --root ROOT` | Reads evidence refs relative to `ROOT`. The default is the package folder for a game's model, else the git work tree the model is in, else the model's directory |
| `gp model fmt DIR` | Rewrites `model.toml` and `fidelity.toml` in canonical form. `--check` writes nothing and fails if either isn't in it |
| `gp model schema WHICH` | Prints a JSON Schema, such as `core` or `platformer`. `gp self schemas DIR` writes every format's schema to a directory, and `--check` fails when one there is stale |

| Command | Exit 1 | JSON fields with `--json` |
|---|---|---|
| `check` | Any problem | `ok`, `dir`, `model`, `facts`, `facts_by_source`, `features` (the features the model uses, derived from its fields), `contract` (facts per level), `profiles` (their names, `retro` first), `refs_skipped` (refs into a pinned download that isn't there), `problems` (each with `file`, `path` and `message`), and with `--assets`, `assets_checked` |
| `fmt --check` | A file isn't canonical | `dir`, `check`, and `changed`, the files that weren't canonical |

Every command exits 2 when it can't run, as when a directory or file is missing.

Not here yet: `gp model diff A B`, which listed every fact that differs between two models by model path.

## Versions

The model core schema is versioned, and genre extensions are versioned on their own. A version bumps only for a breaking change, once per release. An optional part added since is a named feature, which bumps nothing. [features.md](features.md), written by `gp self features` (`just docs`), lists every one the model, the state mapping, the contract and the asset map know.

| Piece | Version | JSON Schema | Rust |
|---|---|---|---|
| Model core schema | 0.4 | `model-core.json` | `gp_model::model` |
| Profiles format | 0.2.0 | `profiles.json`, [its format](formats/profiles.md) | `gp_model::profile` |
| Platformer extension | 0.4 | `platformer.json` | `gp_model::platformer` |
| Top-down extension | 0.1 | `topdown.json` | `gp_model::topdown` |
| Fidelity contract format | 0.2.0 | `contract.json` | `gp_contract`, [its format](formats/contract.md) |
| State mapping | schema 22 | `mapping.json` | `gp_interp::mapping` |
| Canonical state | computed from its layout | n/a | `gp_sim::layout` |

The Rust types are the definition. Parsing goes through them, and the JSON Schemas are generated from them (`gp_model::schema`), so a model the parser takes always fits the schemas. The parser is the stricter of the two: it also refuses inexact decimals and numbers out of range. Some checks are beyond JSON Schema (references between parts, evidence for every fact), and `gp model check` does those.

A model names the versions it was written for: `model.schema` for the core and `model.extensions` for each extension. A reader takes the same major and minor version. A patch, as in "0.4.23", is from before features: it is read as "0.4" and says nothing a reader needs. A breaking change bumps the minor version while the major is 0.

### Features

Each optional option or component is a feature with a name, such as `launcher`, `input.keep_direction` or `contact.touch_edges`. The list this build knows is in `crates/gp-model/src/feature_list.rs` (the core and the platformer extension, with the version each appeared in kept as a note) and `crates/gp-interp/src/mapping_features.rs` (the state mapping).

| Part | How |
|---|---|
| Listing | A model may list the features it uses in `model.features`, a state mapping in `features`. With a list, each feature the file uses must be on it |
| Deriving | Without a list, the features are derived from the fields the file sets: each feature has paths over the file's TOML tree, and a path is met when the value there is set (not `false`, not empty). `gp model check --json` prints the derived list as `features` |
| Refusing | A reader refuses a file that lists a feature it doesn't know, by name, before it reads the file's fields: "the model uses the feature `rope.swing`, which this build doesn't know; update gp to read it" |
| Adding one | Add the option, add its row to `feature_list.rs` with a new name and `since` empty, and write the schemas again (`gp self schemas`). Bump nothing. Two lanes collide only if they pick the same name |
| Breaking | A change that makes old files read differently or be refused bumps the piece's version, once per release |

The canonical state's version is computed: a hash of the code that writes and reads it (`gp_sim::layout`), so a layout change changes it and nobody counts it.

An extension declares, in `gp_model::platformer::EXTENSION`, what a plugin extension will declare in its manifest:

| Declares | Platformer 0.4 |
|---|---|
| Name and version | `platformer`, `0.4` |
| Core versions it works with | `0.4` |
| Systems it adds | `platformer.player`, `platformer.enemies`, `platformer.contact` |
| Components it adds to entities | `run`, `jump`, `patrol`, `walk`, `fall`, `fly`, `hop`, `path`, `contact`, `climb`, `health`, `knockback`, `hits`, `shoot`, `carry`, `carryable`, `oscillate`, `leave_view`, `leap`, `launcher`, `shuttle`, `orbit`, `orbit_part`, `springboard`, `stream`, `invincible`, `locks`, `bounce` and `touch_points`; each added since 0.4.0 is also a feature |
| Fields it adds to levels | `kill_below`, `pit_event`, `goal`, `hazard`, `water`, `ice`, `hard_spawns`, `ladders`, `ground` |
| Parameters it adds to spawns | `patrol_end` |
| Events its rules can name | `player.died`, `player.hurt`, `enemy.stomped`, `enemy.kicked`, `item.collected`, `goal.reached`, `enemy.hurt_player`, `enemy.hit`, `enemy.defeated`, `enemy.stood_on`, `item.lifted`, `item.thrown` |
| Actions its transitions can take | `kill-player` |
| Its schema | The types for each component and field |

No name may be both a core field and an extension's, which a test checks. An entity's or level's keys that aren't core fields must belong to an extension the model declares. Extensions are compiled in.

## The files

A model directory holds `model.toml`, and optionally `fidelity.toml`, the [contract](formats/contract.md) ([fidelity](fidelity.md)), and `profiles.toml`, its profiles ("Profiles" below). A package may also hold a scoring corpus, `corpus.toml` with its clips ([scoring corpus format](formats/scoring-corpus.md)); the model doesn't name it, so choosing the clips again never changes the model's hash. Lifted assets aren't in it: slots name the asset map entry that fills them, and `gp assets lift` writes the files outside the repo.

`model.toml` has these top-level tables, each described below:

| Table | Holds |
|---|---|
| `model` | The header: name, versions, view, frame rate, system order, first level |
| `input` | Named actions and the buttons that trigger them |
| `assets` | Asset slots |
| `entities` | Entity types and their components |
| `levels` | Levels: grids, tilesets, spawns |
| `cameras` | Cameras |
| `rules` | The game's state machines |
| `counters` | Numbers the game keeps across levels |
| `present` | How the game is shown: the screen, the HUD and color cycles (presentation only) |
| `evidence` | Where each fact comes from |

### Numbers

Every number the simulation reads is fixed-point: a signed 64-bit integer counting 1/65536ths (16 fraction bits). Write a whole number as a TOML integer (`walk_speed = 1`, or `0x10` in hex) and a fraction as an exact decimal string (`gravity = "0.25"`). The decimal must be a multiple of 1/65536, so `"0.1"` is refused, and TOML floats are refused everywhere, so no float reaches game logic. NES games count subpixels in 1/16ths or 1/256ths, which 16 fraction bits hold exactly. The runtime protocol's `fixed(s64)` value carries the same integer.

Positions and sizes are in pixels and times in frames, so speeds are in pixels per frame. Positions grow right and down, and a position is an entity's top left corner. The one number that isn't game logic is `model.frame_rate`, frames per second for presentation. It's written as `"60"`, or as a ratio such as `"39375000/655171"` for the NTSC NES.

### Header

```toml
[model]
schema = "0.4"
name = "mosshop"
title = "Moss Hop"
source = "mosshop"
view = "side"
frame_rate = "39375000/655171"
systems = ["rules", "entity_states", "counters", "spawn", "platformer.player", "platformer.enemies", "platformer.contact", "exits", "camera"]
events = ["block.coin", "coin.touched", "player.fell"]

[model.extensions]
platformer = "0.4"
```

`name` is a name in the path grammar ([model path](formats/model-path.md)). `profile` is never written in a `model.toml`: applying a profile sets it ("Profiles" below). `source` is the source ID the model was made from, when there is one. `view` is `side` (gravity pulls down) or `top-down`. `start_level` names the level the game starts in, and a model with more than one level must give it. `events` is optional. It lists the model's own events (dotted names such as `ladder.grabbed`) that the core, counters and extensions don't already fire. An `ext` hook fires them (`ExtCx::fire_own`), and transitions and counter changes name them like any other event; Moss Hop's head hit and pickup fire `block.coin` and `coin.touched`. An event fired for an entity concerns only that instance's machine, so a hook moves its own instance by a transition the model states. Fired with no entity, an event reaches every instance.

`systems` is the order the systems run in each frame. It lists every system of the core and of each declared extension once, so the order is always stated, never a runtime's choice. The order decides what happens when two things land on one frame: in Moss Hop the player moves before the enemies and touches are checked after both, so a stomp and a fall into a pit on the same frame resolve the way `mosshop.s` does (its main loop, lines 157 to 160).

| System | From | Each frame |
|---|---|---|
| `rules` | Core | The game's state machines count time and take `after` transitions |
| `entity_states` | Core | The same for each entity's own machine |
| `counters` | Core | Counters with an `every` change count frames and change |
| `spawn` | Core | Entities within a level's `spawn_ahead` of the view appear, and those past `despawn_behind` go |
| `exits` | Core | The player takes an exit whose `take` it meets |
| `camera` | Core | Cameras follow |
| `platformer.player` | Platformer | The player runs and jumps against tiles, then falls into a pit or reaches the goal |
| `platformer.enemies` | Platformer | Other entities patrol, walk, fly, follow a path, hop and fall |
| `platformer.contact` | Platformer | The player's touches with other entities: stomps, hurts, kicks, collecting, defeats while invincible; then the hitters that move first move, and the hits |

Events don't wait for a system. A transition or a counter change on an event happens when the event fires, so the systems after it in the same frame see the new state. Freezing is decided when a frame begins: a frame that begins in a state with `freeze` runs only the state machines (`rules` and `entity_states`), and a state entered or left during a frame freezes or frees the world from the next frame. That is what Moss Hop's main loop does, checking `STATE` once at its top, and the interpreter's comparison with its baselines fails when freezing takes effect mid-frame instead. A state's `stops` freezes only the systems it names, the same way; the ones in its `stops_at_once` also stop for the rest of the frame the state is entered in, from the system after the one that entered it ("Rules and counters"). A player that dies or reaches the goal touches nothing more in that frame. The header isn't a fact, so the order has no evidence entry; say where it comes from in the model's description or the docs.

### Input

```toml
[input]
device = "nes-pad"

[input.actions]
jump = "a"
left = "left"
right = "right"
run = "b"
```

Components read actions by name, never buttons. `nes-pad` has the buttons `a`, `b`, `select`, `start`, `up`, `down`, `left` and `right`.

Feature `input.keep_direction` (`keep_direction = true` under `[input]`) latches the pad's directions as a game filters them: when a direction the game saw the frame before is still held, it sees only those, and a direction newly held waits until they are let go; with none of them held it sees every direction held. Buttons pass as they are, and a press is new against the latched pad. Donkey Kong's pad routine ($F549) does this into $15, so Up added while walking left at a ladder walks on and climbs the frame after Left is let go.

Feature `input.turn` (`turn = { counter, actions }` under `[input]`) is for players who take turns: while the counter is k, the listed actions read controller k + 1, and the others controller 1. A runtime gives the model each controller's byte (`gp_sim::systems::step_pads`; the runtime protocol's `buttons` holds controller 2 in its second byte). Donkey Kong's player routine reads the pad at $15 + 2 times $52 ($D175), so player 2 walks, climbs and jumps from controller 2, while Start and Select come from controller 1 ($CF42, $CE8B).

### Asset slots

```toml
[assets.metatiles]
kind = "metatiles"
role = "tileset"
item_roles = ["sky", "terrain", "terrain", "terrain", "goal", "goal"]

[assets.metatiles.from]
source = "mosshop"
asset = "metatiles"
```

The model refers to assets only by slot. `kind` says what the slot holds, and each kind matches an asset map format.

| Kind | Asset map format |
|---|---|
| `tiles` | `chr-2bpp` |
| `palettes` | `palette` |
| `metatiles` | `metatiles` |
| `level-grid` | `level-grid` |
| `table` | `table` |
| `level-objects` | `ppu-strings` (a level drawn from a name table string) |
| `spawn-objects` | None here: the object formats it took were the dropped games' |
| `rows` | `byte-rows` |
| `font` | `ppu-strings` or `chr-2bpp`: the text encoding (`charset`), which tile stands for each character |
| `screen-text` | `ppu-strings`: tiles a game writes to places on the screen, such as a status bar's fixed text |
| `sprite-set` | `sprite-set`: sprite frames as layouts of tiles placed in pixels from the entity's position, with the sprite palettes each level showed. `gp sprites capture` writes it from the game's sprite tables and recordings ([its format](formats/sprite-set.md)); the map entry has no ROM span |
| `songs` | `gp-songs` ([its format](formats/songs.md)): songs as notes per channel with their patterns, instruments and tempo in frames, written by `gp sound capture` |
| `sound-effects` | The same, each effect tied to the event it marks |

The check still lists the object-list formats of the dropped games (`smb-area-objects`, `smb2-level-objects`, `mm2-screens` and their spawn formats) for these two kinds, but gp-assets has no decoder for them and the interpreter no builder. `level-objects` and `spawn-objects` slots hold one list per entry, and a level names the entry it uses. The check compares the slot's kind with the format's kind; a runtime refuses a format it has no builder for. A game whose levels are in another format adds its decoder to `gp-assets` and its builder to the interpreter, and its models need no other change. A `spawn-objects` slot says which entity each object kind number places, by entity name:

```toml
[assets.enemy_objects]
kind = "spawn-objects"
role = "spawns"
entities = { walker = [0x06], shell = [0x00] }
```

An object whose number isn't listed places nothing in the model. Each number places one entity at most.

`from` names the asset map's `source` and the asset's `name` in it, which is the recipe that fills the slot. A commercial game's package ships its slots with only this recipe, and the person's own copy fills them. `item_roles` gives each item in the asset a role, for themes and the readability check: one per metatile of a `metatiles` slot, one per palette of a `palettes` slot.

A `songs` or `sound-effects` slot has no asset map entry: its songs come from a recording, so its recipe is `capture`, the recording, its frames and the capture's name, which `gp sound capture` fills under `$GP_HOME/work/sound/` ([sound](sound.md)). Its role is `music.<mood>` or `sfx.<event>`.

```toml
[assets.music]
kind = "songs"
role = "music.level"
capture = { recording = "782c557d", frames = "1..2400", name = "level-1" }
```

`gp model check --assets DIR` reads `DIR/assets.toml`, the map a lift copies into its directory. Each slot whose `from.source` is that map's is checked against the asset it names:

| Check | Fails when |
|---|---|
| The asset | The map has no asset of that name, or it has another format |
| A slot's `item_roles` | There isn't one role per metatile of a tileset, or one per palette of a palettes slot |
| A level's `size` | It isn't the grid's width and height |
| A level's `solid_property` | The tileset has no property of that name |
| A spawn's `columns` | The table has no column of that name |
| A level's `layout` and `spawn_objects` | The object-list asset has no entry of that name |
| A sprite's `tiles` and `palette` | A tile or palette is past the end of the asset |

### Entities

```toml
[entities.slug]
role = "enemy"
size = [16, 16]

[entities.slug.hitbox]
x = 2
y = 4
w = 12
h = 12

[entities.slug.patrol]
step = 1
period = 2
```

| Field | Meaning |
|---|---|
| `role` | One of the entity roles below |
| `size` | Width and height |
| `hitbox` | The box that touches other entities and tiles, from the entity's position |
| `tile_collision` | Optional, default false: stops against solid tiles and stands on them |
| `tile_hitbox` | Optional: the box that touches tiles, when it isn't `hitbox`. Its bottom rests on the ground, so it sets where the body stands: a game that looks for ground below a small contact box needs it |
| `sprite` | Optional, presentation only: `slot` (a `tiles` slot), `tiles` (tile numbers, row by row) or `frame` (a row of a `rows` slot holding them, `{ slot, row }`, so a commercial game's frames stay in its lifted assets, or a frame of a `sprite-set` slot, which brings its own layout), `width` (tiles per row), `palette` (an index into the palettes the entity is drawn with, a level's `present.palettes` or else the model's palettes slot), `flip_when_facing_left`, `flip_in_place` (feature `sprite.flip_in_place`: mirrored where it is, its left edge kept at `offset`, as a console flips a sprite, for every picture of the sprite whatever its width, where without it a mirrored picture turns about the hitbox's middle; Donkey Kong's hammer, one tile wide when up and two in front, keeps its handle on Mario's hand both ways), and `cells`, `colors` and `clips` ("Presentation" below: the tiles' layout, colors of its own and animation). A state's `set` can change it, such as a squashed slug's tiles |
| `machine` | Optional: the entity's own state machine (below) |
| `ext` | Optional: the name of a hand-written behavior behind the `ext` boundary, for what the components can't say. It runs in the system of the entity's role: `platformer.player` for the player, `platformer.enemies` for the rest |
| `ext_params` | Optional, with `ext`: the numbers the hook reads, by name (`ext_params = { swim_speed = "1.5", period = 0x20 }`). Each is a fact with evidence like any other, and a state's `set` can change them. A hook reads them with `ExtCx::param`, which fails on a name the model doesn't give, so a hook's numbers live in the model rather than in its code |
| `ext_tables` | Optional, with `ext`: the tables the hook reads, by name, each a column of one of the model's `table` asset slots as `<slot>.<column>` (`ext_tables = { positions = "orbit_positions.offset" }`). The interpreter reads the column's rows from the lifted assets when it compiles the model, and a hook reads them with `ExtCx::table`, which fails on a name the model doesn't give. Indexed data a hook needs (a game's position or timing tables) is lifted from the original this way rather than written into the hook |
| `ext_rows` | Optional, with `ext` (feature `entity.ext_rows`): more tables the hook reads, by name, each a `rows` asset slot whose rows' numbers in order are one table (`ext_rows = { runs = "ground_runs" }`), for a hook that reads by address a span the model lifts as rows. A name can't be in `ext_tables` too |
| `hook_through_stops` | Optional (feature `entity.hook_through_stops`), default false, with `ext`: the entity's hook still runs, after the system's frozen work, on a frame its system (`platformer.enemies`, or `platformer.player` for a player) doesn't run but the `counters` system does: a state's `stops` lists the system, or a freeze frees the counters at once (`frees_at_once`). It runs in body order, nothing else of the entity runs, and the hook sees `ExtCx::system_stopped` set, so it can do only what the game still does then. A freeze that stops the counters stops the hook too. Donkey Kong's oil drum keeps the game's frame timers ($38, $3B and $40), which count whenever the timer routine $F4AC runs: through a death, a clear and a smash, and on the frame Start unpauses, while the fires stand still |
| `hook_frame_start` | Optional (feature `entity.hook_frame_start`), default false, with `ext`: the entity's hook also runs its `frame_start` (`gp_sim::Ext::frame_start`) at the start of every frame, after the frame's input is taken and before the first system, frozen or not, in body order (`gp_sim::systems::begin_frame_ext`). Its `step` runs as before. Donkey Kong's sound driver steps there, as the game's NMI runs it first ($C890 calls $FA48): it takes the requests the last frame left, from the rules and the hooks alike, and plays through the Start pause |
| `hook_first` | Optional (feature `entity.hook_first`), default false, with `ext`: the entity's hook runs at the start of the `platformer.player` system, before the player moves, and not in `platformer.enemies`. A hook there can set what the player reads this frame (`ExtCx::hold`, an input action held or let go for the rest of the frame). Donkey Kong's attract demo ($EBDA) puts its script's direction in $56 just before Mario's routine reads it |
| `hook_last` | Optional (feature `entity.hook_last`), default false, with `ext`, not with `hook_first`: the entity's hook runs once at the end of every frame, after every system, in body order, whether the frame was frozen, a state stopped systems or a level started; it doesn't run in `platformer.enemies`. A game's idle loop runs after the frame's work whatever it was: Donkey Kong's ($C7E1) steps its random register $18-$1F once a pass while it waits for the NMI, after every draw of the frame |
| `no_slot` | Optional, default false: the entity takes none of a level's `spawn_slots`, for an object a game keeps outside its enemy slots |
| `extra_slots` | Optional, default 0: how many more of a level's `spawn_slots` the entity takes besides its own, the lowest free ones after it, while it is there. They hold nothing of it: with `slot_rest` they keep what their last body left |
| `despawn_behind` | Optional: its own `despawn_behind`, in place of the level's |
| `despawn_behind_wrap` | Optional: `[from, behind]`, off by default. While the view's left edge is `from` or more into its 256-pixel page, `behind` stands in for `despawn_behind`, for a game whose distance test carries into the page byte |
| `despawn_ahead` | Optional: its own `despawn_ahead`, in place of the level's |
| `description` | Optional, for people |

Every other key is a component from a declared extension (see "Platformer extension").

An entity's `machine` is a state machine like the game's rules, run once per instance. A state's `set` gives the entity's fields while in that state, written as the entity's own are: a table merges into the entity's key by key and any other value replaces it. A state can set any field but `machine`, `role`, `ext` and `description`, and `gp model check` refuses a `set` after which the entity wouldn't parse. Moss Hop's slug flattens when stomped and goes 30 frames later:

```toml
[entities.slug.machine]
initial = "walking"

[[entities.slug.machine.states.walking.transition]]
on = "enemy.stomped"
to = "flat"

[entities.slug.machine.states.flat.set.contact]
otherwise = "nothing"
stomp_depth = 0

[entities.slug.machine.states.flat.set.patrol]
period = 2
step = 0

[[entities.slug.machine.states.flat.transition]]
after = 30
do = ["remove"]
to = "flat"
```

`Entity::in_state` in `gp-model` gives an entity as it is in a state. An entity's machine sees the events that concern that instance.

| Event | Concerns |
|---|---|
| `player.died`, `player.hurt` | The player |
| `enemy.stomped`, `enemy.kicked`, `enemy.hurt_player`, `enemy.hit`, `enemy.defeated`, `enemy.stood_on` | The entity it happened to |
| `item.collected`, `item.lifted`, `item.thrown` | The item and the player |
| A model's own event, fired for an entity | That entity |
| Any other | Every instance |

Its transitions can also take `remove`, which takes that instance out of the level.

### Levels

```toml
[levels.main]
grid = "level"
tileset = "metatiles"
solid_property = "solid"
kind_property = "kind"
tile_size = 16
size = [32, 15]
kill_below = 208
pit_event = "player.fell"

[levels.main.goal]
x = 468

[[levels.main.spawn]]
entity = "player"
x = 24
y = 176

[[levels.main.spawn]]
entity = "slug"
table = "slugs"
y = 176

[levels.main.spawn.columns]
patrol_end = "end"
x = "start"

[levels.main.tile_kinds]
2 = "coin_block"
5 = "coin"
```

A level is built from a grid or from objects, or is empty: `empty = true` (feature `level.empty`) gives `size` cells with no tiles and needs no slot, the placeholder `gp game new` writes until the level is lifted. `layout` may name an entry of a `ppu-strings` slot (a level-objects slot may be filled from a `ppu-strings` asset): the string's updates are drawn on the first name table, and the level is its 32 by 30 cells of 8 pixels, each cell the tile number drawn there (0 where nothing is), whose kind `tile_kinds` gives by that number (empty when it isn't listed). Donkey Kong draws each stage from such a string. `grid` names a `level-grid` slot and `tileset` a `metatiles` slot; a tile is solid when its tileset property `solid_property` is nonzero, and with `kind_property` its kind is what `tile_kinds` gives that property's value. Otherwise `layout` names an entry of a `level-objects` slot, `{ slot = "area_objects", entry = "area-6" }`, whose objects build the tiles; what each object kind builds is the object format's. `size` counts grid cells, each `tile_size` pixels on a side.

A spawn places one entity at `x` and `y`, or one per row of a `table` slot, with `columns` naming the column each field is read from. Each of `x` and `y` is given once, as a number or a column. `params` holds parameters extensions define (`patrol_end`), as numbers or, through `columns`, from the table. A level has exactly one player spawn. A position given as a number lies inside the level, and a `patrol_end` given as a number is at or right of the spawn's `x`. A spawn's `drawn` (default false) makes it appear when the level's `draw` draws the tile column its x is in, rather than by `spawn_ahead`, and only if one of the level's `spawn_slots` is free then; it needs the level's `draw`. A `layout`'s own placements are `drawn` too. A spawn's `at_edge` (a number of pixels) places it, as it appears, that many pixels past the view's last column along the level's axis instead of at its x (or y), which is then only where the view reaches it; `appear_moving` (default false) lets it move on the frame it appears even in a level with `appear_still`. A game that starts a group of enemies at the screen's edge, a set distance apart, uses both.

| Field | Meaning |
|---|---|
| `spawn_objects` | Optional: an entry of a `spawn-objects` slot, placing more entities by the slot's `entities` |
| `spawn_ahead` | Optional: entities other than the player appear when their spawn's x is within this many pixels right of the view (y below it, when the level's `camera` follows y only). Without it, everything appears when the level starts |
| `despawn_behind` | Optional: entities other than the player go when their right edge is this many pixels left of the view (their bottom above it, along y). Without it they stay |
| `despawn_ahead` | Optional: entities other than the player go when their left edge is this many pixels right of the view's right edge (their top below its bottom, along y) |
| `respawn` | Optional, default false: an entity that leaves by `despawn_behind` or `despawn_ahead` waits at its spawn, in its initial state, and appears again once its spawn has been out of the view's reach (more than `spawn_ahead` past either edge) and comes back within it |
| `despawn_at` | Optional: what `despawn_behind` and `despawn_ahead` measure. `edge`, the default, is the entity's far edge behind the view and its near edge ahead of it; `position` is its position in whole pixels both ways |
| `despawn_after_move` | Optional: `despawn_behind` and `despawn_ahead` are tested at the end of the `platformer.enemies` system, after everything has moved, rather than in `spawn`. Off, an entity whose own step takes it past the line goes a frame later |
| `spawn_slots` | Optional: at most this many entities other than the player, and other than those with `no_slot`, are in the level at once. A spawn the view reaches while all are taken waits; a `drawn` one is left out for good |
| `spawn_in_order` | Optional, default false: entities appear by `spawn_ahead` in the order of the level's spawns (the `drawn` ones aside), one for each slot free when the frame's `spawn` begins. One waits for those before it, and one whose spawn is left of the view's last column when its turn comes never appears and uses its turn |
| `appear_still` | Optional, default false: an entity that appears by the view first moves on the next frame, as a game that fills an empty enemy slot instead of running it does |
| `slot_rest` | Optional, default false, with `spawn_slots`: the slots are numbered and keep what their last entity left. An entity that appears by the view, a shot or a hook takes the lowest free one; one that appears by the view starts with the speed up and down and the fraction of its place across that the slot's last entity had when it left. They are 0 when the level is placed |
| `draw` | Optional: the level is drawn ahead of the view in sets of columns, and a spawn with `drawn` appears as its column is drawn. `ahead`: the columns drawn past the view's far edge when the level is placed. `step`: each time the view's near edge passes a whole `step` (counted from the level's left edge) a set of `step` pixels of columns starts, a whole number of tiles. `every`: the frames from one column of a set to the next, the first drawn on the frame the set starts. `pause`, default 0: the frames after a set starts on which nothing but `drawn` spawns appears |
| `scroll_stop` | Optional, with `draw`: a column that stops the scroll when it is drawn. `at`: the column's place along the level's axis, in pixels. `removes`, optional: entity kinds whose active bodies go as the column is drawn, with no event. From the next frame no camera moves in the level |
| `loop_back` | Optional, with `draw`: a level that loops. `checks` lists places, each `{ x, y }`. On the first frame with a slot free after the draw finishes the set ending at a check's column `x`, a player who isn't standing at height `y` goes back `back` pixels, with the view and the draw. Every other entity goes and waits at its spawn, except a `drawn` spawn behind the draw, which stays gone. The last `keep` columns drawn go back with the view, and columns the draw reaches again are as built. `back` is a whole number of the draw's steps and of tiles |
| `kind_property`, `tile_kinds` | Optional: what kind each tile is (below). Without them a tile is solid or empty |
| `tile_surfaces` | Optional (feature `level.tile_surfaces`): for a cell value (a grid's metatile number, or the tile number a PPU string draws), the pixel row inside the cell where a `top` or `ladder` tile of that value stands, 0 being the cell's top. A body lands when its bottom reaches that row, and a body standing on ground follows the surface under it as it moves across, up or down at most 2 pixels a move: it keeps its height while any column of its tile hitbox still stands on that surface, and once the hitbox has wholly left it, moves to the highest surface the hitbox covers. So the step comes at the same place walking either way, as Donkey Kong's girder check (a 4-pixel foot box) does. With `jump.feet`, a body in the air lands only where its feet find a surface. Donkey Kong's girders step a pixel every 24, drawn as 8-pixel tiles whose number is the step |
| `origin` | Optional: the level's place in a stage of rooms, in pixels. A state mapping compares the state's `stage_x` and `stage_y`, a position plus the origin, with a game that counts positions across the stage |
| `camera` | Optional: how the cameras move while this level plays: `axis` and `lead` instead of each camera's, `step` (the most pixels a camera moves in a frame) and `hold_player` (with `step`: while a camera is more than a step from its target, the followed entity doesn't move). `keep_in_view` (feature `level.camera.keep_in_view`): the followed entity's bounds in the view across in this level, in place of each camera's `keep_in_view` and `keep_in_view_rows`; Donkey Kong's walk stops at each stage's own edges ($D99D reads them by $53 from $C1B4), x 21 to 229 on 25m and 13 to 237 on 50m. `keep_in_view_rows` (feature `level.camera.keep_in_view_rows`): with it, other bounds by the entity's row, as a camera's `keep_in_view_rows`; 75m's are x 17 to 229, and 109 to 229 on its fifth girder |
| `counters` | Optional: values counters take when the level starts, such as `{ time = 400 }` |
| `halfway` | Optional (feature `level.halfway`): `from` and `at`, a checkpoint. When `restart-level` runs in this level and the first camera's x is at least `from`, the level starts again here instead of the level last started: the camera at `from`, the player at `at`, and the other entities as the view reaches them |
| `next` | Optional: the level the `next-level` action goes to |
| `present` | Optional, presentation only: `palettes` (a `palettes` slot the level is drawn with, background palettes and sprite palettes both), `background` (its background color, a row of a `table` slot whose first column is a color number, `{ slot, row }`), `label` (its name as the HUD shows it, such as `"1-1"`) and `cycles` (colors that change over time, "Presentation" below). The simulation never reads it, and a theme may replace it |
| `exit` | Optional: other ways out, each with `to` (a level), `via` (`pipe`, `vine`, `warp` or `edge`), `take` (what takes it), `enter` (where the player comes out) and `description`. With `next`, these make the progression graph that Structural checks compare |

An exit's `take` says what takes it, and `enter` where the player comes out:

```toml
[[levels.main.exit]]
to = "bonus"
via = "pipe"
take = { at = { x = 912, y = 140, w = 32, h = 4 }, hold = "down", standing = true }
enter = { x = 24, y = 32 }
```

| Field | Meaning |
|---|---|
| `take.at` | A box in the level, in pixels. The player's hitbox must overlap it |
| `take.hold` | Optional: an input action that must be held, such as `down` on a pipe's top or `right` into a side pipe |
| `take.standing` | Optional, default false: the player must be standing |
| `start` | Optional, default false: taking it starts `to` as a level, as `next-level` does, rather than entering it as a room of the level being played. With it, the player is at its spawn in `to`, so there's no `enter` |
| `enter` | Optional: the player's position in `to`, its top left corner. Without it, the player's spawn there |
| `take.if` | Optional: a counter condition, as a transition's `if`, that must hold too: a door that needs a key |
| `keep` | Optional, `x` or `y`: that coordinate keeps its place in the view across the exit (the old position less the old view's corner, plus the new view's), as a room change that scrolls does; `enter` gives the other |
| `scroll` | Optional: frames the exit takes. The world is frozen for them, the player held where it comes out, as while a room change scrolls the view |

An exit without `take` is only in the progression graph. The platformer section below says how the `exits` system takes the others.

A cell has one of these kinds. A layout's builder gives each cell its kind from the format, and `tile_kinds` names what a format's other classes or a grid's `kind_property` values are, keyed by the value in decimal (Moss Hop's `tile_kinds = { "2" = "coin_block", "5" = "coin" }`).

| Kind | What it does |
|---|---|
| `empty` | Nothing |
| `solid` | Stops movement from every side |
| `top` | Stops only an entity moving down onto its top row: jump-through blocks, logs, clouds |
| `ladder` | Climbed by `climb`; a ladder cell with no ladder above stands like `top` for a climber with `stand_on_top` that isn't climbing |
| `hazard` | Touches the player as the level's `hazard` says |
| `water` | An entity whose hitbox's middle is in one falls by the level's `water` gravity and fall limit |
| `ice` | An entity standing on one runs by the level's `ice` acceleration and deceleration |
| `coin_block`, `item_block`, `brick` | Solid like `solid`; a head hit (platformer `jump.head`) tells them apart: a block with a coin in it, one with an item, and a brick. Without `tile_kinds` they are `solid`. Moss Hop's ? block is a `coin_block` |
| `coin` | Stops nothing, and a pickup (platformer `jump.pickup`) takes it. Without `tile_kinds` it is `empty`. Moss Hop's coins are these |

A tile can change while the game runs: an `ext` hook asks for it with `ExtCx::set_tile`, or a head hit's `becomes` changes it (after its `hold`). The changed cells and the held ones are in the canonical state and last until the level is placed again (a start, a restart or an exit).

Feature `stages` is for a model whose levels aren't the game's stages one for one. The model gives it in a top-level `[stages]` table with `order`, `stand_in` and `counter`. `order` lists the original's stages in the order it plays them, each a level's name or the name of a stage the model has no level for yet; `stand_in` names the level that plays a stage with no level of its own. `counter` holds the index in `order` of the stage being played: the model's rules step it as the game steps its own stage number (a counter with `max` and `wrap`, and a loop counter that changes `on = "<counter>.wrapped"` when the game goes round), and a state mapping can compare it with the game's byte. With `[stages]`, `next-level` and `restart-level` start the level that plays the counter's stage, so a turn hand-over that swaps the counter starts the other player's stage. Donkey Kong's model has `order = ["main", "m50", "m75", "m100"]`, `stand_in = "main"` and the counter `stage`. `skip` (feature `stages.skip`, optional) lists stages of `order` the game passes over: when the rules step the counter onto one, it steps on again the same way, through its wrap past the last, so no level plays it. Donkey Kong (JU) has no 50m and steps $53 from 1 (25m) to 3 (75m); the page loads Original Edition's model for a JU ROM with `skip = ["m50"]` (gp-web-lift's `skip_missing`, from the levels the ROM has). Without `[stages]`, `next-level` goes to the level's `next`.

### Cameras

```toml
[cameras.main]
follow = "player"
axis = "x"
size = [256, 240]
lead = [120, 0]
clamp_to_level = true
```

The camera keeps the followed entity `lead` pixels from the view's top left corner along `axis` (`x`, `y` or `both`), inside the level with `clamp_to_level`. Each `lead` lies within the view. `scroll_back = false` stops it moving back left or up; the default is true.

With `by_move = { from, full, short_by }` the camera moves by the followed entity's move instead. Each frame it reads where the entity was in the view on the frame before, in whole pixels from the view's corner, and how many whole pixels it moved forward (right or down) since. Under `from` it doesn't move. From `from` it moves that many less `short_by`, but at least a pixel. Once the entity's move reaches `full`, the camera moves the whole move. A move back, or none, doesn't move it, so it needs `scroll_back = false`. A level start still places it by `lead`. The place it read is world state, `cameras.<name>.seen_x` and `.seen_y`, which a state mapping can read from the original; a translation without them takes it from the loaded positions.

With `by_move`'s `push_hold` (frames) the camera doesn't move on the frames after its followed entity is pushed out of a wall (by `run.walls` or `jump.land_depth`): the count is set after the frame's move, taken down by one at the start of each camera step, and the camera holds while it isn't 0, so 16 holds it on the next 15 frames. The frames left are in the canonical state and the state view (`cameras.<name>.hold`). Under `hold_with_counters` the count is a frame timer. It is taken down on every frame the `counters` system runs, neither frozen nor stopped by a state, whether a state stops the camera or not (on a frame that stops the camera, in the shared `run_frozen`), and on no other.

`by_move`'s `keep_move` makes the camera measure its entity's move from where the entity was when the camera last noted it, and on a frame a state stops the entity's system (`stops`) while the camera runs, it moves as if the entity had made its last move again. The last move and where it was measured from are in the canonical state (`cameras.<name>.move_x`, `.move_y`). A camera that a state stops still keeps where its entity is in the view, for the next frame's move.

Under `keep_in_view = [lo, hi]` the followed entity's whole-pixel place in the view across stays from `lo` to `hi`, checked after the camera's step every frame, whether the camera moved or not. Past either, the entity is put back at it, keeping the fraction of its place, and its speed's whole steps across become 0 unless only the direction away from that edge is held (its `run` actions). Under `keep_in_view_touch` a place at `lo` counts as past it, so the speed goes on the frame the entity reaches that edge. With `keep_in_view_turn` (feature `camera.keep_in_view_turn`) an entity in the air at either bound, or past it, whose speed across goes toward that bound turns around instead: its speed across is reversed and its place put back at the bound, its fraction kept. Donkey Kong's jump ($D547) asks $D990 whether Mario's sprite is at the screen's edge (x 16 or less, or 224 or more) and flips his way ($56) before it moves him, so a jump into the edge comes back; on the ground he stops there.

Under `keep_in_view_rows = [{ rows = [y0, y1], bounds = [lo, hi] }]` (feature `camera.keep_in_view_rows`) the bounds are another pair while the followed entity's row is from `y0` to `y1`: its whole y on the ground or climbing, and in the air the height it took off from when `jump.fatal_drop` keeps it (mechanic word 3), else its y. The first row that holds gives them. A row's bounds are a wall: with any rows the bounds also hold the entity as the player's system ends, so the enemies and contacts test the place it is held at. Donkey Kong's $D990 refuses a move left on 25m's sixth girder ($59 = 6, sprite y 40 to 67 by $E016's thresholds) at sprite x 104, so Mario never walks into Kong; $59 is kept through a jump, which turns there. The model's row is `{ rows = [45, 72], bounds = [109, 229] }`.

### Rules and counters

Moss Hop's death, from its model:

```toml
[rules.game]
initial = "playing"

[[rules.game.states.playing.transition]]
to = "dying"
on = "player.died"

[rules.game.states.dying]
stops = ["platformer.enemies", "platformer.contact", "camera"]

[[rules.game.states.dying.transition]]
to = "playing"
after = 90
do = ["change:lives:-1", "restart-level"]

[rules.game.states.dying.transition.if]
counter = "lives"
cmp = ">="
value = 1

[[rules.game.states.dying.transition]]
to = "game_over"
after = 90

[[counters.deaths.change]]
on = "player.died"
```

A rule is a state machine. The current state's first transition whose event fires (`on`) or whose time is up (`after`, frames since entering the state), and whose condition holds (`if`), moves it to `to`, taking the actions in `do`. A state with `freeze` stops the world on every frame that starts with the game, or any entity, in it (see "Header" for when that takes effect); state machines and counters keep counting. `if` compares a counter with a number, and `cmp` is one of `<`, `<=`, `==`, `!=`, `>=` and `>`.

These optional keys of a state set its part freeze and timing, in the game's states and an entity's alike.

| Key | What it does |
|---|---|
| `stops` | A list of systems of `model.systems` that move things. A frame that starts in the state doesn't run them, as `freeze` stops all of them, and the others run. Moss Hop's `dying` stops the slugs, their touches and the camera while the player hops off the screen |
| `stops_at_once` | Systems of those it stops (any that move, with `freeze`) that also stop for the rest of the frame the state is entered in, if they haven't run yet. Moss Hop's `game_over` stops the player at once |
| `holds` | Counters whose `every` changes wait while a frame starts in the state: the frames count up to the change, which is made on the first frame after the state that comes round to it |
| `keep_time` | Entering the state keeps the frames counted in the state before, so its `after` counts from when the earlier state was entered: a run of states that are parts of one pause, each ending at its own time from the pause's start |
| `seen_every` | An entity's state only: a camera `by_move` following the entity in this state keeps its place in the view only on frames whose number is a multiple of this, the frames it is drawn |
| `stops_machines` | Feature `state.stops_machines`, `true` or `false` (default): a frame that begins in the state doesn't run `entity_states`, so the entities' own machines wait: no time counted, no transition, no `step_word`. The game's machines still run. Donkey Kong's Start pause runs only the Start check, so Mario's death spin waits with everything else |
| `frees_at_once` | Feature `state.frees_at_once`, a game machine's state: systems that move things which run for the rest of the frame on which the state is left, from the system after the one that left it, though the frame began frozen in it. Donkey Kong's unpause clears $0516 in the Start check and the frame timers count on that frame, so its `paused` state frees `counters` |

An entity state's `ground` sets what entering it does to the entity's standing. `leave` takes it off the ground as the state is entered, though nothing moves it. `land`, from the frame after the state is entered, stands an entity that is off the ground on the ground under it, if there is some, whatever its systems do.

An entity state's `enter_speed`, `[across, up and down]`, sets the entity's speed as the state is entered, in pixels a frame. With `enter_keeps_fraction` only the whole pixels of the speed up and down are set and the fraction below a pixel it had stays. The speed across is set whole, and a run's `vx_frac` stays as it was. Platformer's `run.off`, for a state's `set`, makes the run do nothing, so the speed across stays as it is and no input reaches it.

A transition with `after` can also have `align`: once its time is up it is taken only on a frame whose number, the frame count from power-on, is a multiple of `align`, for a game that steps a change on a frame counter's phase; a start from the original's state gives the model's frame count the game's counter.

Feature `state.step_word`, an entity's state only: on each frame the entity is in the state, frozen or not, its mechanic word 0 (the step counter `run.step_cycle` and the climb cycles share) goes up by one, back to 0 at this number, before its transitions are tested. Feature `transition.word`, in an entity's machine with `after`: `word = { index, at_least, below }`, and the transition is taken only on a frame on which that mechanic word is in the range. Donkey Kong's death spin calls $D9E6 on every frame, so $88 counts on while the world is frozen, and the spin ends as $88 comes round to 4 at least 121 frames in: `step_word = 15` and `word = { index = 0, at_least = 4, below = 5 }` with `after = 121`.

Two more triggers are features: `transition.align_to` and `transition.below`. `align_to`, a counter with an `every` change, with `after`: once the time is up the transition is taken only on a frame on which that counter's first `every` clock has just come round (its count is back at 0), so a state lasts to a beat the clock keeps. `below` is for an entity's machine only and takes no `on`. With it the transition is taken once the entity's y is at or past this (with `after` too, once both hold), for a death that ends once the body has fallen below the screen however high it was hit.

`transition.pressed` is a trigger of its own, without `on`: an input action's name, and the transition is taken on a frame on which that action is newly pressed (held, and not on the frame before). With `after` as well, only once that many frames have passed in the state. Donkey Kong's Start pauses the game, and unpauses it only once the pause's 64-frame timer ($0517) is out, so its `pause` machine has `pressed = "pause"` from `running` and `pressed = "pause"`, `after = 64` from `paused`.

Feature `transition.held` is the same trigger on every frame the action is held, newly or not; a transition has one of `pressed` and `held`. Donkey Kong's title sets its idle timer to 64 on each frame Select is held ($C95D), so the demo waits from Select's release.

A change on an event can have `of`, a list of entities: only an event about an entity of one of them changes the counter (points by the kind of enemy defeated), and `after`, a number of frames: the change is made that many frames after the event, counted on every frame from the next one, frozen or not, in the order the events fired (points a game adds once its floating number has shown). A change still waiting is in the canonical state; a level start drops it.

A counter starts at `start` (default 0) when the game does, takes a level's `counters` value when that level starts, and otherwise keeps its value when a level restarts. Each entry of `change` changes it by `by` (default 1), either when the event `on` fires or every `every` frames in the `counters` system while nothing freezes the world (on every frame, with `runs_when_frozen`):

```toml
[counters.coins]
max = 99
wrap = true
change = [{ on = "item.collected" }]

[counters.time]
min = 0
change = [{ every = 24, by = -1 }]
```

| Field | Meaning |
|---|---|
| `min` | Optional: it never goes below this. Reaching it fires `<counter>.reached_min` |
| `max` | Optional: it never goes above this; a change past it stops at it |
| `wrap` | Optional, needs `max`: a change past `max` goes on from `min` (or 0) instead, and fires `<counter>.wrapped` |
| `runs_when_frozen` | Optional, default false: its `every` changes go on counting while the world is frozen (a rule state's `freeze`, an exit's scroll), as the `counters` system does for every counter when nothing freezes it. Needs an `every` change. A color rotation that keeps turning while the player dies is one |
| `restarts_clock` | Optional (feature `counter.restarts_clock`, default false): a level start that sets the counter (the level's `counters`) also starts its `every` clocks over, for a game that clears its frame timers when a level starts |
| `high_of` | Optional (feature `counter.high_of`): another counter's name. This counter keeps the highest value that one has had: whenever that counter changes, or an action sets it, this one is raised to it if lower. Donkey Kong's top score ($21-$23) follows the score so, from $D03E after every award |
| `shift` | Optional, default none: `bits` and `taps`. The counter is a linear feedback shift register: each frame the `counters` system runs, its low `bits` bits move one bit down and the top bit becomes the exclusive-or of the old bits at `taps`. With `runs_when_frozen` it steps through freezes, as a register the NMI rotates every frame does |

| Events | From |
|---|---|
| `level.started` | Core: a level starting, the first or after `restart-level` or `next-level` |
| `<counter>.reached_min` | Core: a counter with `min` reaching it |
| `<counter>.wrapped` | Core: a counter with `wrap` going past `max` |
| `player.died` | Platformer: a touch that kills, falling to `kill_below`, or `kill-player` |
| `player.hurt` | Platformer: a touch that hurts |
| `enemy.stomped` | Platformer: a stomp |
| `enemy.kicked` | Platformer: a touch that kicks |
| `item.collected` | Platformer: a touch that collects |
| `goal.reached` | Platformer: the player reaching the goal |
| `enemy.hurt_player` | Platformer: a touch that hurt the player, for the entity that touched |
| `enemy.hit` | Platformer: an entity with `health` hit by one with `hits`, for the entity hit |
| `enemy.defeated` | Platformer: the same, out of hit points |
| `enemy.stood_on` | Platformer: the player landing on an entity with `contact.stand` |
| `item.lifted`, `item.thrown` | Platformer: a `carry` lifting or throwing an entity, for the entity and the player |
| `<model event>` | The model's own (`model.events`), fired by a hook, a head hit, a pickup or `emit:` |

| Actions | From |
|---|---|
| `restart-level` | Core: every entity back to its spawn, counters kept but for the level's `counters`. From a level with a `halfway` the view has reached, that level starts again from there (`level.halfway`) |
| `next-level` | Core: the current level's `next` starts; from a level without one it does nothing |
| `enter-level` | Core (feature `action.enter_level`): `next-level`, with every counter and its clocks left as they were and each player keeping its mechanic words, for a game that draws the next stage and its objects before it sets the stage up; a later `next-level` starts the level in full. The counters still count on the frame it is taken, where a level start stops the moving systems for the rest of the frame. Donkey Kong: Original Edition draws the next stage's picture, HUD and Kong two frames after its clear scene's $73 ($838A, $CB60, $EBA6, clear_wait 15), where it enters the level with its sprites hidden (`present.hidden`) until $6D draws the objects ($CC34, clear_wait 11), and sets the stage up at $62 ($CC47, clear_wait 0) |
| `reset-game` | Core: every counter back to `start`, and `start_level` starts |
| `remove` | Core, an entity's machine only: that instance leaves the level |
| `change:<counter>:<by>` | Core: the counter changes by `by`, a whole number, as an event's change would (bounds, wrapping and their events included) |
| `set:<counter>:<value>` | Core (feature `action.set`): the counter is put at `value`, a whole number, held within its `min` and `max`, firing no event. Donkey Kong's stage setup ($CB94-$CBA2) puts the bonus timer back at 5000 and its slow timer at 13 while the clear scene still runs |
| `clear:<entity>` | Core (feature `action.clear`): every active body of the entity leaves the level at once, with no event. Donkey Kong's death wait redraws the screen on the 13th divider wrap after the spin and its barrels are gone |
| `respawn:<entity>` | Core (feature `action.respawn`): every active body of the entity that a spawn of the level placed goes back to that spawn's place at rest, its machine's state kept. With `stages`, the spawns are those of the level that plays the stage the counter names, so after a goal has stepped the stage they are the next stage's. Donkey Kong draws Mario at his start on the 21st divider wrap of a death's wait, still dying, before the restart, and at the next stage's start on the 26th wrap of a clear's scene ($CADE) |
| `remove-body:<counter>` | Core (feature `action.remove_body`): the body whose ID plus one the counter holds leaves the level, if there is one, and the counter goes to 0, firing no event, for a body a hook names by its ID |
| `add:<to>:<from>` | Core (feature `action.add`): counter `to` changes by counter `from`'s value as it is, as a change would (bounds, wrapping and their events included); `from` is unchanged. Donkey Kong adds the bonus timer to the score 90 frames after a stage is cleared ($CB0F calls $CC24) |
| `emit:<event>` | Core (feature `action.emit`): the event fires, an event of the core, a counter, a declared extension or the model's own. A death that a rule ends once the body reaches a depth fires the model's own event there, and the game's rule takes the life on it |
| `become:<entity>` | Core, an entity's machine only: that instance is replaced by a new instance of `<entity>` at its position, facing the same way, in its initial state |
| `kill-player` | Platformer: `player.died` fires. Moss Hop's rule takes it on `player.fell`, the pit event |

Feature `cost.frame` is the original's CPU cycles for a frame's work, for a game that steps its random generator in the time the frame's work leaves ([cycles](cycles.md), "The cost model"). Its settings are `base` and `[[cost.terms]]`, under a top-level `[cost]`. A term has `cycles`, `tests` that must all hold, and with `per` (an entity) counts once for each active body of it that passes `per_tests`. A test reads one thing and holds when it is `at_least` and `below` its bounds. It reads a `counter` as the frame started (`at = "start"`, the default) or as it ends (`at = "end"`), or an action `held` this frame or `held_before` it (1 or 0). It may also read an action's bits on controller 1's `pad` as pressed this frame (feature `cost.frame_pad`), or a game machine's `state` as the frame started (`machine:state`, 1 or 0). In `per_tests` it may read a body's `ext` `word`. The `pad` test sees the bits as a controller routine reads them. Neither `keep_direction` nor a turn's controller has changed them yet, and a hook's held actions haven't either. `routine` names the code the term stands for. The simulation sums the terms at the end of the frame, before the `hook_last` hooks, in whole cycles; a hook that runs a routine of the original's adds what its branches cost (`ExtCx::add_cycles`), and a hook reads the sum (`ExtCx::frame_cycles`). Nothing else reads it. Donkey Kong's idle loop steps its random register in the cycles the sum leaves before the next NMI.

### Roles

Every entity and asset slot has a role, and every metatile and palette has one too (`item_roles`). `gp model check` refuses a model where one is missing or isn't in the table below. Themes and the readability check key on roles and their groups, and so will mechanic search.

| Roles | For |
|---|---|
| `player`, `enemy`, `boss`, `projectile`, `npc`, `collectible`, `power-up`, `hazard`, `interactive`, `goal` | Entities, and items within an asset |
| `terrain`, `platform`, `background`, `sky`, `water`, `hud`, `menu`, `font` | Items within an asset, such as metatiles |
| `graphics`, `palettes`, `tileset`, `layout`, `spawns`, `font`, `hud`, `behavior` (a table an `ext` hook reads), `music.<mood>`, `sfx.<event>` | Asset slots |

Entity and item roles fall in groups (`gp_model::validate::ROLE_GROUPS`), which a theme can name instead of a role:

| Group | Roles |
|---|---|
| `actors` | `player`, `enemy`, `boss`, `projectile`, `npc` |
| `things` | `collectible`, `power-up`, `hazard`, `interactive`, `goal` |
| `world` | `terrain`, `platform`, `background`, `sky`, `water` |
| `interface` | `hud`, `menu`, `font` |
| `sound` | `music.<mood>`, `sfx.<event>` |

### Presentation

Presentation is what a runtime draws: the class of change that leaves every state hash equal. The simulation never reads it, `gp_interp::lifted::simulation_slots` puts its slots on the presentation side, and a theme may replace any of it. Its parts are a sprite's layout, colors and animation, and a level's palettes, label and color cycles. The model's top-level `present` adds the screen, the HUD and color cycles for every level. [Capture](capture.md) says how each is found for a game.

A sprite's `cells` give each tile its own flips, palette and place, in the order of its tiles: the sprite as a layout of tiles, as a console's sprite attribute bytes give it. A game that saves tiles by drawing one half of a sprite mirrored says so here, and so does one that turns a sprite over while it falls.

```toml
[entities.squid.sprite]
slot = "sprite_tiles"
frame = { slot = "enemy_frames", row = 10 }
width = 2
palette = 7
flip_when_facing_left = true
cells = [{}, { flip_x = true }, {}, { flip_x = true }, {}, { flip_x = true }]
```

| `cells` item | Meaning |
|---|---|
| `flip_x`, `flip_y` | Optional: the tile mirrored left to right, top to bottom |
| `palette` | Optional: a palette for this tile instead of the sprite's |
| `at` | Optional: the tile's column and row, in 8-pixel tiles from the picture's top left, instead of its place in rows of `width` |

`colors`, `{ slot, palette }`, gives the sprite's `palette` the four colors of one palette of a `palettes` slot.

`clips` animate it. Each frame, the first clip whose `when` holds is drawn; when none does, the sprite's own frame is. A state's `set` replaces the list, so `clips = []` stops a walk cycle.

```toml
[[entities.player.sprite.clips]]
when = { ground = true, moving = true }
frames = [{ row = 12 }, { row = 13 }, { row = 14 }]
by_speed = [{ at_least = "1.75", every = 2 }, { at_least = "0.875", every = 4 }, { every = 7 }]

[[entities.walker.sprite.clips]]
clock = "game"
every = 8
frames = [{ row = 3 }, { row = 2 }]
```

| Clip field | Meaning |
|---|---|
| `when` | Optional: `ground` (standing or in the air), `moving` (moving sideways at all), `skidding` (moving against the way it faces), `rising` (moving up), `speed_at_least` (its sideways speed, either way), `jumped` (in the air after a jump, where walking off a ledge is in the air without one), `invulnerable` (in frames where no hurt counts, as after a hit) and `climbing` (on a ladder or vine). Each one given must hold |
| `frames` | The pictures, shown in turn. A frame is a `row` of the slot the sprite's `frame` names, `tiles`, or else the sprite's own frame, with optional `flip_x` and `flip_y` (the whole picture, on top of the facing flip) and `cells` (else the sprite's) |
| `every` | Frames each picture shows for |
| `by_speed` | Instead of `every`: the first step whose `at_least` the entity's speed reaches gives the time, picked again at each change of picture, so a faster walk steps faster |
| `clock` | `own` (the default): a timer per entity that starts with the clip. `game`: the frames the game has run, so every entity showing the clip changes together. `body`: one timer per entity that runs down every frame whatever clip shows, as a game keeps one animation timer per object. On each frame the clip draws its picture and then, if the timer has run out, steps to the next and sets the timer again from `every` or `by_speed`. It starts at the picture shown before when that is one of its own (a walk after a fall off a ledge goes on where it was), else at its first. A state mapping can load the timer and the picture into a start |
| `while_frozen` | Optional: keeps changing while a `freeze` state holds the world, which stops clips otherwise |
| `hold` | Optional: never changes picture. It keeps the picture the entity showed when the clip started, when that is one of its frames, and else shows its first: a walk cycle that stops in the air |

A sprite's `blink = { when = { invulnerable = true }, every = 2 }` hides it on every other run of `every` frames, counted by the game's frames, while `when` holds, whatever clip it shows: a hurt entity's blink.

More sprite options:

| Field | Meaning |
|---|---|
| `offset` | `[x, y]`: the picture's top left in pixels from the entity's position, in place of standing it on the entity's bottom edge. A mirrored picture turns about the middle of the hitbox, as a sprite-set frame does. The NES shows a sprite one line below the Y its table gives, so a picture drawn from a ROM's tile may need `[0, 1]` |
| `behind` | Drawn behind the level's picture: its pixels show only where the level shows the clear color, as the NES's sprite priority bit does. A state's `set` can turn it on and off. The screen measure still counts its hidden pixels as the sprite layer's, as the NES's sprite pixels are |
| a clip's `speed_before` | `by_speed` reads the speed the entity had on the frame before, as a game that picks its animation's time before it moves |
| `replaced` | `{ flip_y }`: the entity is drawn as the picture of the body it replaced showed last, mirrored top to bottom with `flip_y`. A body a defeat leaves (`health.defeat.becomes`) looks like the enemy it was. A body with nothing to copy, such as one a start carried in, draws the sprite's own frame, and a sprite with `replaced` may have no tiles, so it draws nothing then |
| `leaves` | A sprite (its `slot`, `frame`, `width`, `palette`, `cells` and `offset`) that a body replacing this one draws, unturned, in place of this one's last picture: a game that has a frame of its own for the defeated enemy |
| `front` | Drawn over every body without it, whatever their order, as a console draws its first sprites over the rest |
| a clip's `when.word` | `{ index, at_least, below }`: one of the entity's `ext` hook words is in a range |
| a clip frame's `offset` | The picture's top left for this frame, in place of the sprite's `offset`, for a game that places each picture of a turn apart |

More, each a feature, found drawing Donkey Kong:

| Field | Meaning |
|---|---|
| a clip's `when.words` (`clip.when_words`) | More word tests, `[{ index, at_least, below }]`, each of which must hold with `word`. Donkey Kong's barrels roll one way or the other by their girder (word 2) |
| a clip's `when.counters` (`clip.when_counter`) | `[{ name, at_least, below }]`: the model's counters in ranges, each of which must hold. A picture the game picks by a timer or a flag the model keeps as a counter: Donkey Kong's oil drum burns while `drum` is above 0, Mario swings his hammer by `swing` and Pauline calls while `pauline_body` is $D7. Presentation reads the counters; nothing in the simulation reads the clip |
| a clip's `when.slot_counters` (`clip.when_slot_counter`) | `[{ name, at_least, below }]`: the counter named `name` followed by the number of the numbered slot the body is in, in its range; a body in no slot fails it. A picture the game keeps per object slot: Donkey Kong's barrels draw the tile in `$72,X`, which the model keeps as `barrel_tile0` to `barrel_tile6`, so each barrel turns $90 and $94 on its own moves down a ladder |
| a clip's or blink's `when.x_at_least`, `when.x_below`, and the same in `behind_when` (`clip.when_x`) | The body's x in whole pixels is at least `x_at_least` and below `x_below`, each when given: a picture the game picks by where the object is on the screen. Presentation only |
| a clip frame's `every` (`clip.frame_every`) | Frames this picture shows for, in place of the clip's `every`; every frame of the clip gives one, or none does. Donkey Kong's death spin steps 8 and 7 frames in turn |
| a clip frame's `slot` (`clip.frame_slot`) | The `rows` slot its `row` is read from, in place of the sprite's frame slot, for a game that keeps each picture in a table of its own: Donkey Kong's six poses, each behind a size byte |
| a clip frame's `colors` (`clip.frame_colors`) | Its colors, as the sprite's `colors` (a palettes slot and a palette in it), for a game that writes a sprite palette while something lasts: Donkey Kong's 100m turns the fires blue from a hammer's touch ($D89A, entry $46 of $C03C) until the hammer goes ($D7F2, entry $4E) |
| a sprite's `name_table` (`sprite.name_table`) | The game writes the picture into the background's name table rather than drawing sprites. It is drawn as any body, but the screen measure counts its pixels as the background's and the sprite limit doesn't count it. Donkey Kong is background tiles, his poses rewritten in place |
| a sprite's `behind_when` (`sprite.behind_when`) | A condition, as a clip's `when`: drawn behind the level's picture, as `behind`, on the frames it holds, for a game that sets a sprite's priority bit by what the object is doing or where it is. Donkey Kong's `$DFC3` gives a barrel rolling on the bottom girder ($68,X = 1) with its sprite left of x $30 attribute $23, so it rolls into the oil drum behind the drum's front: `behind_when = { words = [{ index = 0, at_least = 1, below = 2 }, { index = 2, at_least = 1, below = 2 }], x_below = 48 }`. A sprite gives `behind` or `behind_when`, not both. Presentation only |
| a sprite's `split` (`sprite.split`) | `{ keep = [left, top, width, height], x, y }`: the part `keep` of the picture, in pixels from its top left as drawn (after any flip), stays at the body's place and is drawn over the rest, and the rest of the picture is drawn moved by the values of the counters `x` and `y` (0 for one not given). While both are 0 the picture is drawn whole. For a game that moves one of an object's hardware sprites and leaves the others where its last draw put them: on the frame Mario walks off a girder's end with the hammer, Donkey Kong moves only his first sprite ($D70A/$D710 a pixel across, and on 25m $D72B the girder's step down) and draws none of the others ($D7BF), so the player has a `split` whose `x` and `y` are counters its hammer hook sets for that frame. Presentation only |
| `levels.*.present.sprite_order` (`present.sprite_order`) | Each entity's place in the console's sprite table in the level, by entity name: `at` (the first hardware sprite's number), `step` (sprites a body takes, default 4), `word` (the `ext` word holding a body's slot plus one; else its count among the entity's drawn bodies) and `turn` (`counter` and `count`: the slots are copied from the counter's slot on, around `count`). The sprite limit drops the sprites that come last on a crowded line. Original Edition's 50m copies its sprite buffer into the table in its own order ($895F): the moving ladders after the pies, and the pies from a slot that turns each frame ($0493, the counter `sprite_turn`) |
| `levels.*.present.effect_order` (`present.effect_order`) | Each effect's place in the same table, by the event that starts it: `at` and `step` (default 4). It takes the sprites from `at + step * k`, k the place among its phase's `until_counters` of the counter the phase took (else 0), and is painted over and under the bodies by that place as they are by theirs; without one an effect is painted under every body. Donkey Kong's points ($CFC6) take sprite 48 or 50 ($02C0, $02C8), so a 500 over the hammer (52) shows over its head: `effect_order."smash.done" = { at = 48, step = 2 }` |
| `present.screen.sprites_off` (`present.sprites_off`) | Rule states, each `<machine>.<state>`, in which the console's sprite layer is off: the level, the HUD's tiles and name-table bodies show, and no sprite. Donkey Kong turns its sprites off once a Start pause's 64 frames are out ($CFA1 sets bit 4 of $11, which the NMI writes to $2001 inverted), its state `pause.still` |
| `present.hidden` (`present.hidden`) | `[{ states, when, kinds, hud }]`: bodies not drawn while one of the rule states (`<machine>.<state>`) holds and every counter test of `when` (`{ name, at_least, below }`) does, though they are still in play. `kinds` names the entity types hidden, in any layer; without it the console's sprite layer is off, as `sprites_off` turns it off. Donkey Kong hides its sprites from a death's 13th divider wrap, which moves every sprite off the screen ($F08C), until the 21st draws the stage's objects again ($CC34), Kong for the frame the stage is drawn before him, and the oil drum's flame until the restart. `hud = true` hides the HUD too, its layer, fields and sprites, and with no `kinds` only the HUD: Donkey Kong's 100m ending blanks the status rows with its first update string ($C6E4, entry $0A of $C03C) |
| a `present.screens` screen's `from_line` (`present.screens_from_line`) | With `blank = true`: the screen line the picture goes off at. The lines above it show the frame as it would be without the screen (the level, its bodies and the HUD), the lines from it the backdrop alone, as a console whose picture is turned off partway down a frame. Donkey Kong's clear script turns it off on its $7A step ($CBB7 calls $D19A, which writes $2001 = $06 at scanline 34, after the NMI has drawn the frame's top), so the status rows and Pauline's head still show on that frame: `from_line = 35` on the `game.cleared` blank at clear_wait 20. The screen measure draws it. Presentation only |
| a `present.screens` screen's `text` (`present.screens_text`) | `[{ at, text, palette }]`: characters of the HUD's font (`present.hud.font`) written into the screen's name table over its layout or blank, from tile column and row `at`, each cell in background palette `palette` (default 0) in place of the attribute bytes'. Drawn into the screen's picture, so a runtime draws nothing new. Donkey Kong's turn screen in a two-player game is a cleared name table with PLAYER I or PLAYER II on row 15 ($CBCA writes $C6AA's string, $67 at $0345 for player 2): `text = "  PLAYER |  "`, the charset naming the numeral tiles $66 and $67 `|` and `#`. Presentation only |

A level whose `layout` is a `ppu-strings` entry is drawn as the string draws the name table: each cell's tile from the level's tiles slot (its only one, the one with role `tileset`, or the one no sprite uses) in the background palette the string's attribute bytes give it, color 0 the backdrop. Donkey Kong's 25m is `screen_25m`. The palettes the string writes ($3F00 to $3F1F) go over the level's own in the string's order, as a `present.screens` screen's do: Donkey Kong's 75m and 100m strings write their stage's first two background palettes and three sprite colors after the first set, and Original Edition's 50m string the same. Original Edition draws 50m with CHR bank 1 ($CB5D, $838A), so its layout slot `stage_screens_50m` gives `tiles = "tiles_50m"`, that bank's second pattern table, which a level drawn from it uses for every cell (feature `assets.layout_tiles`, presentation only).

A state's `set` merges into the sprite, so a state that names only a `row` keeps the sprite's slot. When that is a sprite-set, the state draws the set's frame of that number, and its `cells` do nothing; `gp model check` reports `cells` on a sprite-set frame for that reason.

A clip on the game's clock (`clock = "game"`) counts from the world's frame count when the animation starts, so a world loaded partway through a game, with the frame count a mapping carries, shows it in the game's phase.

A frame of a `sprite-set` slot is laid out in pixels from the entity's position, so it is placed from there rather than stood on the bottom edge, and a mirrored one turns about the middle of the hitbox. A frame with no tiles draws nothing, which is how a blink's off frame is written.

A level's sprites take the second half of its palette set (`present.palettes`), so an enemy is drawn in the level's own colors. A color cycle steps one background color through a list:

```toml
[[present.cycles]]
palette = 3
index = 1
colors = "color_rotation"
every = 8
counter = "color_step"
shift = -1
```

`palette` is a background palette (0 to 3) and `index` a color in it (1 to 3). `colors` is a `table` slot whose first column lists the colors, and `every` is the frames each shows for. The model's `present.cycles` apply to every level whose own `present.cycles` is empty.

By default the color is picked by the frames the world has run, so a clip that starts in the middle of a game starts the cycle at its first color. A game that keeps the rotation's place in a variable is carried over with `counter`: a counter with `max` one less than the list's length, `wrap` and an `every` change, whose value picks the color instead of the clock, plus `shift` (default 0, modulo the list's length; -1 when the game's counter has already stepped past the color on screen). A mapping then maps the counter and its `ticks` like any other, so a translation loads the game's phase. `every` is ignored when `counter` is given.

The top-level `present` table also holds the screen and the HUD. Moss Hop's HUD draws its coins and lives as sprites:

```toml
[present.screen]
size = [256, 240]
pixel_aspect = "8/7"
overscan = [0, 8, 0, 8]

[present.hud]
tiles = "tiles"
font = "font"

[[present.hud.field]]
at = [3, 2]
counter = "coins"
digits = 2
sprite = true

[[present.hud.sprite]]
at = [16, 16]
tiles = "tiles"
tile = 58
```

| Field | Meaning |
|---|---|
| `screen.size` | The picture the console puts out, in pixels (256 by 240 for the NES). Without `screen`, the first camera's view |
| `screen.pixel_aspect` | A pixel's width over its height on the display the game was made for (`"8/7"` for the NTSC NES); default `"1"` |
| `screen.overscan` | Pixels a television hides at the left, top, right and bottom; a runtime shows the rest |
| `screen.sprite_limit` | Optional: `per_line`, the hardware sprites the console draws on one line (8 on the NES), `sprite_width` (default 8) and `flicker` (default false), set when the game turns its sprites' order each frame so the ones past the limit take turns. A runtime draws the limit through `gp_present::view::limit`: each body is as many sprites as its picture is wide, and on each line the sprites past the limit are cut from that line, the bodies taken by their place in the console's sprite table (a level's `present.sprite_order`) and else in body order. The screen measure draws the model with it. Without it nothing is hidden |
| `screen.sprite_x_wraps` | Optional (`present.sprite_x_wraps`): the console's sprite x is a byte, so each hardware sprite of a body (8 pixels across) whose left column is left of the screen is drawn 256 pixels on, at the right edge, as the NES draws a sprite placed at x below 0. The screen measure draws it; for a game that never scrolls. Donkey Kong's hammer swung left of Mario at the screen's left edge puts its head at x 255 |
| `hud.tiles`, `hud.font` | A `tiles` slot and a `font` slot: the tile for each character the fields show. A font is a charset, from a `ppu-strings` asset or a `chr-2bpp` asset that gives one |
| `hud.layer` | Optional: an entry of a `screen-text` slot, the HUD's fixed tiles and palettes as the game writes them to the screen |
| `hud.size` | Optional: columns and rows of tiles the HUD covers from the top left; default the screen |
| `hud.field` | Text drawn from the game's state at a tile column and row (`at`), in a background palette (`palette`, default 0): a `counter` padded to `digits` with `pad` (default `"0"`) and shown divided by `divide` (default 1, for a score whose last digit is fixed in the layer), the level's `label` (`level_label = true`) or fixed `text`. Its characters are name table cells: each covers what the layer had there, its color 0 the backdrop, so a field of blanks clears the layer's tiles (Donkey Kong's second player's label, which the stage setup blanks). With `sprite = true` its characters are sprites in sprite palette `palette`, which show the layer through their color 0, and the screen measure counts them in the sprite layer: Moss Hop draws its coins and lives in the sprite table |
| `hud.field.when` | Optional (feature `present.hud.field_when`): counter tests, `[{ counter, cmp, value }]`, all of which must hold for the field to be drawn. Donkey Kong draws player 2's label and score only in a two-player game, and each player's score in its own place whoever's turn it is |
| `hud.sprite` | Optional: sprites fixed on the screen, drawn over the fields: `at` (the tile's top left in screen pixels), `tiles` (a `tiles` slot), `tile` (its number), `palette` (a sprite palette, 0 to 3, default 0) and `behind` (as the NES's priority bit: its pixels show only where the layer draws nothing). Moss Hop has three, beside its counters |

A counter's `digits` and `pad` set how wide its number is on the screen and what fills the left of it, so a game's own width is a value of the field. A number too long for `digits` shows its last digits.

A HUD is drawn over the view in screen positions. Its role is `hud`, so a theme can restyle it like any other part. A HUD sprite's pixels are the sprite layer's in the screen measure, as the game's are.

Feature `present.screens` shows a whole screen in place of the level, its bodies and the HUD while one of its rule states (`<machine>.<state>`) holds: `layout`, an entry of a `screen-text` slot, drawn as the game writes the name table, with the palettes it writes over the first level's, and `sprite`, tiles fixed on it (`at`, `tiles`, `tile`, `palette`), each moved `step` pixels for each 1 of a `counter`. Donkey Kong's title is the string its title routine draws ($C807 with 8) and the cursor, tile $A2 at x 56, 16 lines down for each line of the menu (`cursor`). A `screen-text` slot may name the `tiles` slot its layouts are drawn with (feature `assets.screen_tiles`, presentation only), in place of the level's, for a screen the game draws with another pattern table or CHR bank. The title of Donkey Kong: Original Edition is one, drawn with CHR bank 1 (`assets.title.tiles = "title_tiles"`).

The model's `present.sound` says which song plays in each level and rule state and which effect each event starts ([sound](sound.md)).

```toml
[present.sound]
music = [
  { slot = "music", song = "level_theme", level = "main" },
  { slot = "music", song = "level_end", state = "game.clear" },
]
sfx = [{ event = "enemy.stomped", slot = "sfx", effect = "stomp" }]
```

| Field | Meaning |
|---|---|
| `music` | A song of a `songs` slot (`slot`, `song`) and exactly one of `level` (a level it plays in) or `state` (a rule state, `<machine>.<state>`, whose song takes over from the level's while it holds) |
| `sfx` | An effect of a `sound-effects` slot (`slot`, `effect`) played when `event` (an event of the model) fires |

Sound is presentation, so no state hash depends on it and a runtime that has no audio ignores it. Its role group is `sound`, which a theme's restyled-audio tier keys on.

The features `present.cells` and `present.effects` say how the cells the game changes are drawn and add short-lived pictures that events start. The level's picture shows the cells as they were built; a runtime draws each changed cell over it. Moss Hop's coins and ? block:

```toml
[present.cells.kinds]
empty = 0
solid = 7
```

A fuller one, with a bump and an effect:

```toml
[present.cells]
held = 0
kinds = { empty = 0, solid = 196 }

[present.cells.bump]
speed = -2
gravity = "0.3125"
offset = [0, 1]
sprites.solid = { slot = "sprite_tiles", tiles = [135, 135, 135, 135], width = 2, palette = 7, cells = [{}, { flip_x = true }, { flip_y = true }, { flip_x = true, flip_y = true }] }

[[present.effects]]
event = "block.coin"
at = "hit"
offset = [5, -16]
phases = [
  { slot = "sprite_tiles", frames = [[96, 96], [97, 97], [98, 98], [99, 99]], every = 2, width = 1, palette = 6, cells = [{}, { flip_y = true }], motion = { speed = -5, gravity = "0.3125", max_fall = 6 }, until_speed = 5, while_frozen = true },
  { slot = "sprite_tiles", frames = [[247, 251]], width = 2, palette = 6, rise_every = 2, with_camera = true, length = 46, while_frozen = true },
]
```

| Field | Meaning |
|---|---|
| `cells.kinds` | By tile kind: the metatile a cell changed to that kind is drawn with, by its number in the level's tileset (a grid level's `metatiles` slot, or a layout's metatile sets, whose top two bits pick the set). A coin taken (`empty`), a block used (`solid`), a brick broken. A kind not listed keeps the built picture |
| `cells.held` | The metatile a cell shows while a head hit holds it (platformer `jump.head`'s `hold`) |
| `cells.bump` | While a cell is held, its next picture rises and falls: `speed` and `gravity` (each frame the place goes by the speed, then the speed by the pull, at most `max_fall`), from the cell's top left plus `offset`. `sprites`, by the kind the cell will have, is the bump's picture as a sprite's tiles (`slot`, `tiles`, `width`, `palette`, `cells`); a kind without one draws the cell's metatile with color 0 clear. A cell that stays as it was (a bumped brick) draws its own kind's |
| `effects[].event`, `at`, `offset` | The event that starts it, and where: `hit`, the cell a head hit took on that frame, or `entity`, the event's entity's position; `offset` is the picture's top left from there |
| `effects[].phases` | Shown in turn. Each has its pictures (`frames`, lists of tiles laid out as a sprite's, by `width`, `palette` and `cells`, shown `every` frames each by the game's frames), an optional `motion` (as the bump's), `rise_every` (up a pixel on each frame whose count is a multiple), `with_camera` (moves across as the camera does), and its end: `until_speed` (the frame its speed's whole part reaches it), `length` (frames), `until_counter` (a counter's name: the frame after one on which the counter is 0 or less, as a game that hides a sprite by a timer it reads before the frame's timers step; feature `present.effects.until_counter`) or `until_gone` (the frame the event's entity is gone, for an effect at its entity, and the next phase starts on that frame; feature `present.effects.until_gone`). `while_frozen` keeps it going while a state freezes the world, as a game object the freeze doesn't stop |

An effect takes its first frame on the frame its event fires, as a game that makes an object in its frame moves and draws it then. Cells, bumps and effects are drawn by `gp_present` (`Scene::cells` over the level and its tints; bumps and effects after the bodies in `Scene::bodies`), never read by the simulation, and leave every state hash equal. A model with no palettes slot draws a changed cell in its new kind's color.

## Platformer extension

The options below were each found in a game's code, most of them in Super Mario Bros.'s before that game was dropped; they stay in the extension because they are general, and the interpreter runs them. Where Donkey Kong or Moss Hop uses one, the row says so.

| Component or field | Keys | Meaning |
|---|---|---|
| `run` (entity) | `walk_speed`, `run_speed` and `run_action` (together, optional), `acceleration`, `run_acceleration`, `deceleration` and `skid_deceleration` (optional), `air_control` | Moves with the actions `left` and `right`. Without `acceleration` the top speed is reached at once, and without `deceleration` the entity stops at once. `run_acceleration` replaces `acceleration` while `run_action` is held. `skid_deceleration` slows it while the direction held is against its motion on the ground |
| `jump` (entity) | `action`, `speed`, `gravity`, `max_fall`, `gravity_held`, `coyote_frames`, `buffer_frames`, `bands`, `move_first` (the last five optional) | `action` newly pressed while standing sets the vertical speed to `speed` (negative is up). `gravity` is added every frame, and the fall is capped at `max_fall`. `gravity_held` replaces it from takeoff until the action is let go or the entity stops rising, for jumps whose height depends on the hold; pressing again in the air doesn't bring it back. `bands` make the jump depend on the horizontal speed at takeoff: the first band whose `below` is above that speed gives its own `speed`, `gravity` and `gravity_held` for the whole jump, and a speed at or above every band's uses the values above. Bands go in order of `below` |
| `patrol` (entity) | `step`, `period` | Moves `step` pixels once every `period` frames, between its spawn's `x` and its `patrol_end`, turning at each end. Moss Hop's slugs |
| `walk` (entity) | `speed`, `turn_at_ledges` (optional), `walls_at_top` (optional) | Walks at `speed` pixels a frame, starting toward the player, turning at walls, and at ledges with `turn_at_ledges`. With `walls_at_top` a wall is met only at the tile hitbox's top row. One of `walk` and `patrol` |
| `fall` (entity) | `gravity`, `max_fall`, `move_first`, `feet` and `stands_still` (all but the first two optional) | Falls as a jump does, for an entity that doesn't jump. With `feet` the ground is looked for at those columns, as `jump.feet`. With `stands_still` a body on the ground takes no gravity and doesn't move up or down: it only looks for the ground and falls from the frame after it has gone, and placed on the ground it takes no speed from its numbered slot (`slot_rest`). A body that `stands_still` placed in a numbered slot also takes the fraction of the height the slot's last body left. One of `fall` and `jump` |
| `fly` (entity) | `speed`, `heading` (optional: `player`, the default, `left` or `right`), `keep_speed` (optional) | Moves sideways at `speed` through tiles: a bullet or a swimming fish. With `keep_speed` it keeps the speed across it has (one a start carries, or `shoot.vx`) and takes `speed` the way it faces only while that is 0. One of `walk`, `patrol`, `fly` and `path`; a flier may `fall` too, for a projectile thrown in an arc |
| `hop` (entity) | `speed`, `pattern`, `every` and `phase` (optional), `land_first` (optional), `land_while_stopped` (optional) | Jumps whenever it's standing: its vertical speed becomes `speed` (negative). With `pattern`, its hops take the pattern's speeds in turn instead, from the first. With `every`, it hops only on frames whose number plus `phase` times its index is a multiple of `every`. Needs `fall`. With `land_first` the landing comes before the move: "How a runtime runs these parts" says how. With `land_while_stopped` too, that landing still happens on a frame the enemies system is frozen or stopped, and the body doesn't move |
| `path` (entity) | `axis` (`x` or `y`), `distance`, `speed`, `pause` and `wait_near` (optional), `every` and `phase` (optional), `wait_near_wrap` (optional) | Moves from its spawn by `distance` (negative: left or up) at `speed` and back, waiting `pause` frames at each end, with no gravity and through tiles. With `wait_near` it doesn't leave the spawn end while the player's x is less than that many pixels from its own; with `wait_near_wrap` that whole-pixel distance is taken modulo it first. With `every` it moves only on frames whose number plus `phase` (below `every`) is a multiple of `every`; its waits count every frame |
| `contact` (entity) | `stomp_depth` and `bounce` (together, optional), `keep_on_stomp` (optional), `otherwise` | A falling player whose bottom row lands within the top `stomp_depth` rows of the hitbox stomps the entity: `enemy.stomped` fires, the player's vertical speed becomes `bounce`, and the entity is removed unless `keep_on_stomp`, which leaves it to its machine. Any other touch does `otherwise`: `kill-player`, `hurt-player` (fires `player.hurt`), `collect` (removes it and fires `item.collected`), `kick` (the entity turns to face away from the player and `enemy.kicked` fires) or `nothing`. Moss Hop's slug: `stomp_depth = 7`, `bounce = -3`, `keep_on_stomp`, `otherwise = "kill-player"` |
| `contact` | `damage`, `stand` (optional) | `damage` (default 1): what a `hurt-player` touch takes from the player's `health`. `stand`: a falling player whose bottom row is within `stomp_depth` of the hitbox's top lands there instead of stomping, rides it while it moves, and `enemy.stood_on` fires on landing |
| `contact` | `every`, `phase` (optional) | The touch is tested only on frames whose number less `phase` is a multiple of `every` |
| `contact` | `invincible` (optional) | What a touch does while the player is invincible: one of `otherwise`'s touches, or `defeat`. By default `defeat` for an entity whose touch would hurt or kill the player, or kick it, or that can be stomped, and `otherwise` for the rest |
| `contact` | `touch_edges` (optional) | The touch's test takes each hitbox a pixel wider and taller, so two boxes that only meet at their edges touch, for a game that counts equal edges as a touch |
| `contact` | `sides` (optional: `left`, `right`, `top`, in pixels) | A touch at a side of the entity stops the player as a wall does: its speed across becomes 0 and it moves one pixel away, unless it is moving away. The player is at the left side while its right edge is less than `left` past the entity's left edge, and otherwise at the right side while its left edge is less than `right` before the entity's right edge. A falling player whose bottom edge is less than `top` past the entity's top edge isn't at a side. The touch is tested before the entity moves this frame. With the camera system earlier in the order, the frame's scroll stands and a camera's `push_hold` starts at once |
| `contact` | `in_turn` (optional) | A state's `stops_at_once` of `platformer.enemies` set off by this touch stops the enemies from the toucher's numbered slot on, and the bodies in earlier slots move that frame. A toucher without a numbered slot stops them all |
| `contact` | `near` (optional: `within`, `right`, in pixels), and an entity's `touch_points` (a list of `[x, y]` from its position) | The touch is tested after the entity moves, in its own turn, in place of the hitboxes. A player's point touches while it is less than `within` from the entity's position on both axes. An entity `right` or more from the view's left edge touches nothing |
| `climb` (player) | `speed`, `up`, `down`, `release`, `stand_on_top`, `grab_down`, `hold_still` (the last four optional) | Climbing `ladder` tiles, as "How a runtime runs these parts" says |
| `climb` (features `climb.up_cycle`, `climb.down_cycle`, `climb.by_feet`, `climb.down_holds`) | `up_cycle`, `down_cycle` (lists of whole pixels), `by_feet`, `down_holds` (all optional) | `up_cycle` and `down_cycle` replace `speed`: each frame up (or down) is held, on a ladder or at one it could take, the step counter (mechanic word 0, shared with `run.step_cycle`) advances by one, back to 0 at the list's length, and the body moves that entry's pixels. A climb starts only on a count that moves, and moves on that frame. Both lists are as long as any other cycle of the entity. Donkey Kong's $D9E8 and $D9E6 with $24/$49 up and $49/$49 down: a pixel every 3 frames up, 6 in 15 down. `by_feet`: the ladder is the row under the tile hitbox. Up moves only while that row is in a ladder, a solid tile or a cell with a surface (a `top` tile, or a ladder with a `tile_surfaces` row), so a broken ladder's gap stops the climber, still climbing; down moves only while the new row is. After a move that puts that row on a surface (a top tile's or a solid tile's top, or the surface row), the climb ends standing. Down starts a climb from standing when the cell under the one the feet are in is a ladder. `down_holds`: down held while standing and not climbing keeps the body still; its run gets no input. `down_through` (feature `climb.down_through`, pixels, with `by_feet`): down newly pressed also starts a climb with the feet in a ladder cell (a ladder's foot), a down move tests nothing on the way and goes on until the feet reach a surface row, the height wraps at this many pixels, and neither `kill_below` nor the goal touches the climber. Donkey Kong's ladder routine looks for a girder only on the floor Mario's y + 8 is in, so Down at a ladder's foot takes him through the bottom girder, round the 8-bit y and onto the top girder. `down_through_step` (feature `climb.down_through_step`, with `down_through`): the press starts it only while the entity's last move across stepped it up a surface. Donkey Kong's walk tests the ground ($D20B) after the move across and before the step up, so after a step up the slope $5A is 0 and the climb routine ($D432) skips its ladder-top test: the one-frame ladder-zip |
| `climb` (feature `climb.grab`) and a level's `ladders` (feature `level.ladders`) | `grab`, `[x0, y0, x1, y1]` from the entity's place; `ladders = { places, shapes, shape_step }`, two `byte-rows` slots | In a level with `ladders`, a climb up starts only where the `grab` box meets a ladder's box, edges included, below the box's top row, instead of by the cell at the entity's middle, and only from the ground; standing on a ladder's top, up does nothing. Down keeps its own test. With `grab_snap` (whole pixels), a climb up by a ladder box starts with the entity's x at the box's left plus this, as Donkey Kong's $D2B9 places Mario's sprite at the ladder's x less 4 (`grab_snap = 1`, his place being his sprite + 5). Each row of `places` is a ladder's x, y and shape; the shape divided by `shape_step` (default 1) is a row of `shapes`, the box's left, top, right and bottom from that x and y. Donkey Kong's ladder routine ($D28B, $D8AD) tests Mario's feet box, his sprite +8..+10 across and +17 down, against each ladder's box from the stage's list ($C0E3 and the shapes at $C10B for 25m) |
| A level's `ground` (feature `level.ground`) | `ground = { runs, shapes, shape_step, rise, feet }`: two `byte-rows` slots, `feet` the first and last columns from a body's x | The level's ground as one-row boxes, in place of the tile surfaces. Each row of `runs` is a run: x, y, shape, the step across from one box to the next (a signed byte) and the count, each box `rise` (default 1) pixels above the last, x and y wrapping as bytes; the shape divided by `shape_step` is a row of `shapes`, the box's left, top, right and bottom from its place. A body on the ground keeps its height while its feet (`feet` from its x, on the row under its tile hitbox) meet a box, edges included; once they meet none it moves to the nearest row a pixel or two up or down whose box they meet, and a body whose feet meet a box on that row stands there. Off the boxes the tiles decide. Donkey Kong's walk tests Mario's feet box ($C186) against the girder pieces of his row ($D91A, the runs at $C08C and boxes at $C0C3) after each move across and steps a pixel when it meets none ($D8EB) |
| A level's `ground` `boxes` (feature `level.ground_boxes`) | `boxes = true` | Each row of `runs` is one box: its first three bytes x, y and shape, with no step across or count. The walk takes no step where the feet meet none, so a fall off a ledge (`jump.ledge_fall`) drops its `speed` on its first frame, not its `first`. Donkey Kong's ground on every stage but 25m: $D2E2 reads the stage's rows through the pointer at $C46B and tests Mario's feet box against each ($D8AD; 75m's at $C20C and $C252), where 25m's $D91A steps a pixel first ($D8EB) |
| A level's `ground` `land` (feature `level.ground_land`) | `land` (pixels, optional) | A jump falling (its vertical speed 0 or more) lands on the first box, in the boxes' order, that its feet meet on a row from `land` pixels above the row under its tile hitbox down to that row, and is put on it, wherever it was the frame before; the tiles don't land it. A ledge fall (`jump.ledge_fall`) lands the same way and keeps its height. Donkey Kong's jump tests Mario's feet box, his y + 10 to + 12, against the girder pieces in their list's order ($D5CC, $D2CB, $D91A) and puts him on the first it meets ($D5D3); his fall off a girder's end dies where the box meets one ($D6BC): `land = 2` |
| `run` (0.4.0 keys) | `start` (`speed`, `frames`), `air_speed`, `wall_snap` (all optional) | `start`: the first `frames` frames walked from standing go at `speed` (a frame that takes off isn't one). `air_speed`: the speed in the air while a direction is held, with `air_control`. `wall_snap`: holding a direction into a wall the tile hitbox touches clears the position's fraction |
| `run` | `move_step`, `cap_whole`, `skid_by` (`speed`, the default, or `last_motion`), `skid_turn_below`, `decel_bands` (a list of `from` and `rate`), `decel_hold_from`, `run_hold_frames`, `run_with_motion` (all optional, all off by default) | The step, cap, skid, band and hold options that "How a runtime runs these parts" describes |
| `run` | `takeoff_in_air`, `face_on_ground`, `air_run_from` (all optional, all off by default) | The air options that "How a runtime runs these parts" describes: the frame a jump starts is a frame in the air, the facing changes only on the ground, and in the air the speed alone says whether the run counts |
| `run` | `walls` (`left`, `right`: lists of points; optional) | Walls are found at points after the vertical move instead of the tile hitbox stopping the move across |
| `run` | `push_masks` (optional, off by default) | On the frame after a wall point or the feet's depth pushes the entity out to one side, a direction held toward that side changes its speed as nothing held would, in the air too; the facing and the run's counts still see it held. The side is world state (`pushed`) |
| `run` | `skid_turn_with` (a list of input actions), `speed_before_walls` (both optional, off by default) | `skid_turn_with`, with `skid_turn_below`: the actions named steer for the quick turn as a direction does, so one held with no direction, or with a direction that isn't the way the entity last moved, turns a slow skid. `speed_before_walls`: the run's speed tests and the jump's bands read the speed the run last left, before a wall stopped it, kept as world state (`seen_speed`) |
| `run` (features `run.step_cycle`, `run.air_step_cycle`) | `step_cycle`, `air_step_cycle` (lists of whole pixels, optional) | `step_cycle`: on each walking frame on the ground (a direction held), the entity's step counter (mechanic word 0) advances by one, back to 0 at the list's length, and the entity moves the entry at the counter that way, instead of by `walk_speed`. With nothing held it doesn't move and the counter stays. The counter also advances on every frame in the air and of a `jump.land_hold`, so a jump shifts the phase. `air_step_cycle`, with `step_cycle` and without `air_control`: in the air the entity moves across by this list, the way held on the takeoff frame (none held, it doesn't move across), by a counter of its own (mechanic word 1) that each such frame advances first. Donkey Kong's walk routine steps one counter through 15 frames and moves on the bits of $DB/$36 (`[1, 1, 0]` five times), and its jump moves Mario a pixel every other frame by a toggle (`[1, 0]`). An entity with an `ext` hook can't have them, since the hook keeps its words |
| `run` (feature `run.lone_direction`) | `lone_direction` (optional) | Left or right moves the entity only held alone: with its climb's up or down action held too, the run gets no direction. Donkey Kong's walk ($D1CF) and hammer routine ($D6DE) move Mario only while the latched pad byte $56 is exactly 1 or 2 |
| `run` (feature `run.face_way`) | `face_way` (optional) | The facing is a way the game keeps from the pad: the frame a jump takes off (with `takeoff_in_air`) turns the entity to a lone direction held, a landing hold (`jump.land_hold`) keeps the facing, a turn at the view's edge in the air (`camera.keep_in_view_turn`) turns it with the speed, and a level's restart keeps it. Donkey Kong's pad routine $D175 writes $57 from Left or Right held alone on every frame but the jump's and the walk-off fall's ($CEC1-$CED3), the jump's edge turn writes it ($D55F-$D56E), the stage setup leaves it, and $F070 mirrors his pictures by it |
| `jump` (0.4.0 key) | `release_cap` (optional) | Once `action` isn't held, a rise faster than this is cut to it, every frame after the move |
| `jump` and `fall` | `max_fall_from_half` (optional, off by default; `max_fall` must then be whole) | The fall limit acts only once the vertical speed's whole part is at least `max_fall` and its fraction at least one half, and then sets it to `max_fall`, so a fall goes up to a half past the limit on some frames |
| `jump` | `no_gravity_on_ground`, `land_keeps_fraction`, `land_depth`, `feet` (a list of columns), `head` (`point`, `above`, `row_from`, `keep_fraction`, `hits`: each `tile`, `speed`, `event` and `becomes`, the last two optional) (all optional, all off by default) | A frame that starts standing adds no gravity; a landing keeps the height's fraction; a landing happens only near a tile's top; the ground is looked for at given columns; one point over the head decides what a rise into a tile does. "How a runtime runs these parts" describes them. Moss Hop's player has a `head` at `[8, 0]` whose hit on a `coin_block` makes it `solid` and fires `block.coin` |
| `jump` | `keep_fall_gravity`, `start_fall_gravity`, and a head hit's `hold` (frames) (all optional, all off by default) | The gravity of a fall that isn't a jump is the last jump's, kept after landing, or the start's before the first; a head hit keeps its cell solid for `hold` frames before it changes |
| `jump` | `collide_y` (`[from, to]`, optional) | The entity meets tiles (landing, the head point, the feet's depth and the wall points) only while its whole y is at least `from` and below `to` |
| `jump` | `pickup` (`points`, `tile`, `becomes`, `event`; optional) | Points that take a cell of a kind by touching it, change it and fire an event. Moss Hop's player takes `coin` cells at `[8, 8]` and fires `coin.touched` |
| `jump` (features `jump.takeoff_fraction`, `jump.land_hold`) | `takeoff_fraction` (a fraction, from 0, below 1), `land_hold` (frames) (both optional) | `takeoff_fraction`: a takeoff sets the height's fraction to this, before the takeoff frame's move. Donkey Kong's arc routine starts its fraction at $F0 (0.9375), takes $0158 off the height a frame and adds a fall speed starting at $08 and growing by $11: `speed = -353/256`, `gravity = 17/256` with it. `land_hold`: for this many frames after the frame a jump lands, the entity stands: it neither walks, jumps nor takes a ladder, and its step counter still advances. Mechanic word 2 counts the frames left. Donkey Kong keeps Mario in the jump state 4 frames after landing. An entity with an `ext` hook can't have a hold |
| `jump` (feature `jump.ledge_fall`) | `ledge_fall`: `speed`, `first` (whole pixels, optional) and `kills` (optional) | A fall that isn't a jump drops straight down with no gravity: `first` pixels (or `speed`) on the frame the ground leaves, then `speed` a frame. Its run gets no input and its speed across is 0, and the step counter (`run.step_cycle`) advances each frame of it. With `kills` its landing kills the player (`player.died`). Donkey Kong's state 8 ($D697): 3 then 2 a frame, and any landing kills |
| `jump` (feature `jump.fatal_drop`) | `fatal_drop` (pixels) | A jump that lands this many pixels or more below the height it took off from kills the player (`player.died`) as its `land_hold` ends, or as it lands without one. Mechanic word 3 keeps the takeoff height, and the test reads only it and the height, so a state loaded in the jump or the hold decides as the run would. Donkey Kong sets $95 to $FF once Mario is 16 px below his takeoff and kills him after the 4-frame hold |
| `jump` (feature `jump.fatal_drop_in_air`) | `fatal_drop_in_air` (flag) | With `fatal_drop`, mechanic word 3 keeps the height after the takeoff frame's move, and the drop is tested on every later frame in the air, on the height the frame's move reaches before a landing puts the body on the ground. Once it holds, the word is -1000 (`FATAL_MARK`) and the landing kills. Donkey Kong's jump routine keeps $95 after the arc's first step ($D5EA) and sets it to $FF at $D5BF-$D5CA, before the landing test at $D5CC. A marked word is in no row of `camera.keep_in_view_rows` |
| `walk` | `ledge_at` (optional, off by default) | With `turn_at_ledges`, the ledge is looked for after the move, at this many pixels across the entity whichever way it faces, in place of the leading bottom corner before the move, and the walker turns where it stands |
| `walk` | `turn_after_move` (optional, off by default) | A wall doesn't stop the move: the walker moves, kept in the level, and if the leading edge of its tile hitbox's top row is then in a solid tile it turns where it stands |
| `walk` | `bump` (optional, off by default) | `every`, `phase` (optional) and `view` (optional, `[first, last]`): walkers whose hitboxes touch, edges included, turn each other once, until a test finds them apart. Each is tested in its turn against the earlier bodies, at their places before the frame's moves, on frames where the frame less `phase` is a multiple of `every`; `every = 0` never, for a state that leaves the bumping. With `view`, a body takes part only with its x that far from the view's left edge |
| `health` (entity) | `counter` (the player's) or `points` (any other's), `invulnerable`, `shield`, `damage_from`, `defeat` (optional) | Hit points. The player's are a counter; a hurt that counts takes `contact.damage` from it and starts `invulnerable` frames in which no hurt counts. Any other entity's are its own, taken by entities with `hits`; `shield` makes hits do nothing, and `damage_from` gives the damage by the hitter's entity. `defeat`, with `becomes`, `rise` and `speed`: a defeat by a hit or a touch leaves a body of `becomes` in the defeated one's place and numbered slot, `rise` pixels higher |
| `knockback` (player) | `speed`, `rise`, `frames` | A hurt that counts knocks the player back for `frames` frames: `speed` a frame against the way it faces, its run input ignored, and on the first frame its vertical speed becomes `rise` |
| `hits` (entity) | `damage` (default 1), `roles` (default `enemy` and `boss`), `keep`, `every`, `phase` and `move_first` (optional) | Touching an active entity whose role is in `roles` and that has `points` of `health` takes the damage from it and fires `enemy.hit`; out of hit points it leaves and `enemy.defeated` fires. The hitter leaves unless `keep`. With `every`, hits are tested only on frames whose number less `phase` is a multiple of it. With `move_first` the hitter moves in `platformer.contact`, just before its hits are tested, and not in `platformer.enemies`, so it hits the others where they were before they move |
| `shoot` (entity) | `entity`, `action` or `every`, `ahead`, `max`, `vx`, `vy`, `unmirrored`, `after_move`, `whole` and `in_slots` (the last six optional) | Spawns `entity` when `action` is newly pressed, or every `every` frames, while fewer than `max` of it are active: `ahead` from this entity's position (x mirrored facing left), facing the same way. A spawner is an entity with `every`. The new body takes `vx` (mirrored for a shooter facing left) and `vy`; `unmirrored` keeps `ahead` across unmirrored; a player with `after_move` shoots after it moves this frame; `whole` starts the body from the shooter's whole-pixel place. `in_slots`: the `max` shots take numbered slots in turn, nothing is shot while the slot whose turn it is holds one, and a new shot sets only the whole parts of its place and its speed down, keeping the fractions its slot's last shot left |
| `carry` (player) | `action`, `lift_frames`, `offset`, `throw`, `throw_moving` and `moving_above` (together, optional) | `action` newly pressed while standing on a `carryable` entity lifts it: the player holds still `lift_frames` frames and the entity rides at `offset` from the player's position (or its own `carryable.offset`), running none of its components. Pressed again it throws: the entity takes `throw` as its speed (x mirrored facing left), or `throw_moving` when the player's speed across is `moving_above` or more. `item.lifted` and `item.thrown` fire |
| `carryable` (entity) | `offset` (optional) | A `carry` can lift it |
| `invincible` (player) | `while` (`counter`, `cmp`, `value`, as a transition's `if`) | While the condition holds, each touch does the entity's `contact.invincible` instead of a stomp or its `otherwise` |
| `bounce` (entity) | `point`, `speed`, `snap` and `from_y` (optional) | After the entity moves, the tile at `point` from its position is looked at (not above `from_y`). One that isn't empty, water or a ladder, met while moving down and not on the frame after a bounce, bounces it: its speed up and down becomes `speed` with its fraction kept, and its whole height is rounded down to a multiple of `snap`. One met while moving up, or on the frame after a bounce, removes it |
| `oscillate` (entity) | `axis` (`x` or `y`), `center`, `force`, `max_speed`, `low_at` and `low_center` (together, optional), `starts_at_rest`, `move_first` and `rest_creep_every` (optional) | Swings around a center `center` pixels from where it's placed along `axis` (or `low_center` when it's placed at or past `low_at`): each frame its speed along the axis moves by `force` toward the center, up to `max_speed` either way, and it moves by it, through tiles. With `starts_at_rest` it takes no speed from its numbered slot (`slot_rest`), and with `move_first` it moves by the speed it had before the force is added. One of `walk`, `patrol`, `fly`, `path` and `oscillate`, and no `fall` on y |
| `leap` (entity) | `speed`, `gravity`, `max_fall`, `every`, `from`, `rise` (`speed`, `height`), `interval`, `waits`, `below` (all but the first two optional) | Leaps again and again from a start (`from`, a y, or where it's placed): at `speed` (negative is up), pulled by `gravity` on frames whose number is a multiple of `every`, at most `max_fall`, through tiles. With `rise` it first rises at `rise.speed` until `rise.height` above the start. It starts again from the start when its wait is over (`interval` frames times one of `waits`, drawn at random) or once it's `below` pixels under the start. No `fall`, and no `path` or `oscillate` on y |
| `shuttle` (entity) | `axis`, `step`, `every` (optional), `turn_after`, `sway` (`step`, `half`, optional) | Moves back and forth by a counter: on frames whose number is a multiple of `every` it counts a move, turns once the count passes `turn_after`, and moves `step` along `axis`, right or down first, through tiles; with `sway` each move also goes `sway.step` across, toward the far side while the frame number divided by `half` is odd. No `fall` |
| `orbit` (entity) | `spin`, `angles` (default 32), `parts`, `part`, `place`, `skip`, `in_view`, `place_on_appear`, `angle_in_slot` (the last six optional) | A pivot: on its first frame it moves by `place` and spawns `parts` of the entity `part`, or with `place_on_appear` on the frame it appears; then (with `in_view`, only while its x is inside the first camera's view) its angle moves by `spin` steps of the `angles` in a turn each frame, a step on when it lands on an angle in `skip`. With `angle_in_slot`, in a level with `slot_rest`, a new pivot takes its whole angle from its slot's speed up and down, wrapped to `angles`, and a pivot that leaves puts its whole angle there |
| `orbit_part` (entity) | `within`, and the entity's `ext_tables` `offsets`, `rows` and `signs` | A part of an orbit: it finds its pivot (the nearest body with an `orbit` within `within` pixels of where it was placed) and which part it is, and each frame sits at the pivot plus a quarter-turn table's offsets for the pivot's angle; it leaves with its pivot. An entity with an `orbit_part` and no hook may have `ext_tables` |
| `launcher` (entity) | `fires`, `counter`, `at`, `slot_step`, `mask`, `hard_mask`, `slots`, `table`, `next`, `rearm`, `offset`, `from_x`, `near`, `near_wrap`, `behind` (`slot_step`, `hard_mask`, `offset`, `from_x` and `near_wrap` optional) | An object that fires the entity `fires` on its own timer and a shift counter's bits, aimed at the player's side. The level's launchers keep `table` entries: one that appears takes the entry the counter `next` names, ending the launcher there and keeping its timer, and `next` moves on; one left of `from_x` fires nothing. Each frame after the other enemies, the first `slots` numbered slots, the last first: one holding a shot gives it its turn, a free one reads a byte of `counter` (from bit `at`, `slot_step` lower per slot, and'ed with `mask`, or `hard_mask` in a level with `hard_spawns`) that may pick an entry, whose timer counts down or at 0 fires a shot at the launcher plus `offset` and rearms to `rearm`. On its first turn a shot goes if it is off the view (`behind` left) or within `near` of the player, modulo `near_wrap`; otherwise it faces the player's side. Needs `no_slot`, numbered slots (`slot_rest`), a `fly` on the fired entity and no `ext` hook on either |
| `springboard` (entity) | `steps` (each `frames`, `move` and `offset`, the last two optional), `speed`, `boost` (`speed`, `action`, `from_step` and `held`, the last two optional), `throw_late` (optional) | A body that holds a player standing on it for its steps' frames, moving the player by each step's `move` and itself to each step's `offset` below its place, then throws the player up at `speed`, or at `boost.speed` once `boost.action` was newly pressed (or held, with `held`) from step `from_step` on. With `throw_late` the thrown player stands one more frame. No movers beside it (`fall`, `walk`, `patrol`, `fly`, `path`, `oscillate`, `leap`, `shuttle`, `hop`), no `ext` hook, not on the player |
| `leave_view` (entity) | `margin`, `axis` (optional, `x` by default), `far_margin` (optional), `bottom_margin` (optional) | Leaves the level on the first frame its position along `axis`, in whole pixels from the first camera's near edge, is under `margin` or at or past the camera's size less `margin`. With `far_margin` that is the margin at the far edge instead. With `bottom_margin` it also leaves at or past the view's height less that, whatever the axis |
| `kill_below` (level) | a number | A player whose top edge reaches this y dies |
| `pit_event` (level) | one of `model.events` | With `kill_below`: a player whose top edge reaches `kill_below` from above it fires this event once instead of dying, and goes on moving, so the model's rules say what a fall does. Moss Hop's `player.fell`, on which its rule takes `kill-player` |
| `goal` (level) | `x`, `under` (optional) | A player whose left edge reaches `x` wins. With `under`, a player whose top edge is at or below that y passes under the goal instead |
| `hazard` (level) | `touch` (`kill-player` or `hurt-player`), `damage` (optional) | What a `hazard` tile does to a player whose hitbox touches it |
| `water` (level) | `gravity`, `max_fall` | Falling in `water` tiles |
| `ice` (level) | `acceleration`, `deceleration` | Running on `ice` tiles |

The feature list names more options than this table describes, among them `stream`, `locks` (`power.locks`), `contact.ride`, `contact.after_move`, `head.busy_while`, `head.above`, `level.hard_spawns`, `leap.draw` and the level's `ladder_*`, `pickup*` and `ground_*` features. `crates/gp-model/src/feature_list.rs` says what each does.

### How a runtime runs these parts

This is the precise reading, for the interpreter. Speeds are pixels a frame, positive right and down, and every number is fixed point.

A frame goes like this. The runtime reads the input, settles whether the world is frozen (a frame is frozen when it starts with the game or any entity in a `freeze` state, or while an exit's `scroll` frames last), then runs the systems in `model.systems` order. On a frozen frame `platformer.player`, `platformer.enemies`, `platformer.contact`, `spawn` and `camera` do nothing, `rules` and `entity_states` count time as usual, and `counters` doesn't count its `every` changes. A current state's `stops` skips the systems it names the same way while the rest run; a skipped `camera` with `by_move` still notes its entity's move and place in the view, and its `keep_in_view` still holds the entity in it. Before the systems, the counter changes waiting for their `after` count a frame on frozen and unfrozen frames alike. Those with none left are then made. In `platformer.contact`, after the touches, the bodies whose `hits` have `move_first` take their turn as `platformer.enemies` would give it, then the hits are tested; `platformer.enemies` passes them over.

`run` runs in `platformer.player`, as this table says.

| Held | On the ground | In the air |
|---|---|---|
| A direction along the motion, or any direction from standing still | Speed moves toward the top speed in that direction by `acceleration` (at once without it), and a speed past the top speed drops to it | The same with `air_control`; without it the speed is kept |
| The direction against the motion | Speed moves toward that direction by `skid_deceleration`, or by `acceleration` without it | As along the motion, with `acceleration` |
| Neither | Speed moves toward 0 by `deceleration` and stops there | Speed is kept |

Without `deceleration` the speed drops to 0 at once when neither direction is held, in the air too. Moss Hop's player stops in mid-air when Right is let go.

The top speed is `run_speed` while `run_action` is held and `walk_speed` otherwise, and the acceleration `run_acceleration` (when given) while `run_action` is held and `acceleration` otherwise. Right wins when both directions are held. Then the position moves by the speed, and with `tile_collision` a hitbox (or `tile_hitbox`) that ends up in a solid tile is pushed back out and that speed set to 0.

The step, cap, skid, band and hold options change this table, each only when it's given. Any game that keeps its speed in two bytes, or chooses its friction by the last way moved, has them. A runtime runs them in this order each frame:

| Step | What happens |
|---|---|
| The speed | With `move_step`, the speed is `vx` plus `vx_frac`: `vx` is whole steps and `vx_frac` the rest, from 0 up to a step. Speeds compared with a threshold below are the whole steps, as a size |
| The run | `run_hold` goes down by one if it's above 0. The run action counts while it's held, and with `run_with_motion` only while the direction held is the way the entity last moved (`moved`; with nothing held, only before it has moved at all). A frame it counts on the ground, not a frame that jumps, sets `run_hold` to `run_hold_frames`. While `run_hold` is above 0 the run still counts, with `run_with_motion`'s condition. So the run is kept `run_hold_frames` less one frames after it's let go |
| The band | While the run doesn't count, the last band whose `from` the speed is at or above gives its `rate` in place of `acceleration`, `deceleration` and `skid_deceleration`. With `decel_hold_from`, the last band's rate applies whatever the speed while `decel_hold` is set. Then, on the ground and not on a frame that jumps, `decel_hold` is set when the speed is at or above `decel_hold_from`, and cleared when it's below and a direction is held the way it last moved |
| The double | With `skid_by = "last_motion"`, the rate in use doubles while the facing the frame starts with isn't the way it last moved, including before it has moved at all. `skid_deceleration` isn't used |
| The quick turn | With `skid_turn_below`, on the ground and not on a frame that jumps, a direction held that isn't the way it last moved, with the speed below `skid_turn_below`, sets the speed to 0 and `moved` to the way it faced; the frame's rate then applies the new way. With `skid_turn_with`, an action it names held steers too: with it held, no direction held, or one that isn't the way it last moved (before it has moved at all, no direction is that way), turns it the same way, the rest below a step (`vx_frac`) cleared too, and with nothing held after it the speed stays 0 |
| The change | A direction held: with `cap_whole`, the rate is added that way whatever the speed, and then whole steps past the top speed are set to the top speed with the fraction kept (going right, whole steps at or above it; going left, whole steps below its negative). Without, as the table above. Neither held, on the ground, with `move_step`: the speed changes by the rate toward 0 while its whole steps aren't 0, and is left as it is once they are, so a stopped entity keeps its fraction |
| The move | With `move_step`, `vx` becomes the speed rounded down to a step and `vx_frac` the rest, and the position moves by `vx`. With any of `skid_by = "last_motion"`, `skid_turn_below`, `run_with_motion` or `decel_hold_from`, `moved` becomes right or left by the sign of `vx`, and stays as it was when `vx` is 0 |
| The speed seen | With `speed_before_walls`, the speed the band, the hold, the quick turn and `air_run_from` compare is `seen_speed` as the frame starts instead of the speed, and so is the speed the jump's bands read at takeoff. After the change, on the ground and in the air while a direction is held, `seen_speed` becomes the new speed (whole steps, without its sign), before the wall points or the level's sides stop the entity. In the air with no direction held it is kept |

With `face_both`, both directions held where the facing turns are a facing of their own: the body's `facing` is `both` from the next frame until one direction alone is held there, and `skid_by = "last_motion"` compares that with the way it last moved. The way held is `both` too, against the way it last moved, for `run_with_motion`, `decel_hold_from` and `skid_turn_below`, so a slow skid can leave `moved` at `both`. Right still wins for the speed. A body facing both ways draws as facing left.

`vx_frac`, `moved`, `run_hold` and `decel_hold` are model paths of an entity with `run`, which a state mapping can read and load like `vx`. `moved` is `none`, `right` or `left`, or `both` with `face_both`. `seen_speed` is a path of an entity with `run.speed_before_walls`.

The three air options, each only when it's given, are for a game that sets its jump state before it moves the player across, or turns its player only in the ground routine.

| Option | What happens |
|---|---|
| `takeoff_in_air` | On a frame the entity is standing and takes off (the jump `action` pressed, or a remembered press), the run takes the air's path for the whole frame: with nothing held the speed is kept, a direction held uses the air's rules, `air_speed` and `air_run_from` included, and the ground's skid rate doesn't apply. The counts that already leave out a frame that jumps (`start`, `run_hold`, `decel_hold`, the quick turn) are as before |
| `face_on_ground` | The facing changes only on the ground (with `takeoff_in_air`, not on the takeoff frame). In the air a direction held still moves the speed, so with `skid_by = "last_motion"` the rate doubles in the air only if it did on leaving the ground |
| `air_run_from` | In the air, the run counts (its top speed and `run_acceleration`) while the speed as the frame starts (whole steps, without its sign) is at least this, and doesn't count below it, whatever the run action and `run_hold`. Below it the walking rates and the bands apply as usual, `decel_hold` included |

Under `run.walls` the move across doesn't stop at tiles (it still stays in the level). After the frame's `run` and `jump`, the points are tested in order, every `left` point and then every `right` point, each from the entity's position in whole pixels. The first in a cell that stops movement impedes the entity, unless its speed across moves it away from that side (a `left` point with a speed above 0, a `right` one below 0): the speed across becomes 0 and the entity moves one pixel away from that side, its fraction kept. Only the first point found counts. A machine state's `set` can change `walls`.

`jump` runs in `platformer.player` after `run`.

- A takeoff happens when `action` is pressed on this frame and not the last, and the entity is standing or stopped standing (without jumping) within the last `coyote_frames` frames. A press in the air within `buffer_frames` frames before the entity lands counts as a press on the frame it's standing again.
- At takeoff the band is chosen by the horizontal speed, without its sign, at the start of the frame (with `run.speed_before_walls`, `seen_speed`): the first band whose `below` is above it, or the jump's own values if none is. The vertical speed becomes the band's `speed`, and the held phase starts.
- The held phase ends once `action` isn't held or the vertical speed is 0 or more, and it stays ended until the next takeoff. The vertical speed then takes `gravity_held` in the held phase and `gravity` otherwise, is capped at `max_fall`, and the position moves by it. Landing on a solid tile stops the fall, and hitting one above sets the vertical speed to 0.
- With `no_gravity_on_ground` a frame that starts standing and doesn't take off adds no gravity, so the vertical speed stays 0 and the entity doesn't move up or down; the ground under it is still looked for, and when it's gone the entity is in the air and gravity starts on the next frame. Under `land_keeps_fraction` a landing sets the whole part of the height to the tile's top and keeps the fraction the move left, and a takeoff clears the fraction.
- With `keep_fall_gravity` the entity keeps a fall gravity of its own (`fall_gravity` in its state): placed at a level start, through an exit or as a new body, it is `start_fall_gravity` (or `gravity` without it); at each takeoff it becomes the band's `gravity` (or the jump's own, when no band holds). Outside the held phase gravity is that value, not the band's or the jump's, so a jump falls as before, and a fall that isn't a jump (off a ledge, or the ground leaving) takes the last jump's. The held phase still takes `gravity_held`, and water still takes the level's values.
- Under `feet` the tile hitbox's bottom row looks for ground only at those columns (x from the entity's position, whole pixels), for landing and for standing alike. With `land_depth` a body moving down lands only while its bottom row is less than `land_depth` pixels into the tile; deeper, the body doesn't land and keeps its speed. It moves one pixel back the way it last moved (`moved`: left after moving right, right otherwise) unless its speed takes it that way already. The speed across becomes 0 and no wall point is tested that frame.
- Under `collide_y` none of this meets a tile while the body's whole y, after the frame's move, is outside `[from, to)`: there is no landing and no head bump, the body doesn't sink into a wall, and no wall point is tested.
- Under `run.push_masks` a wall point's push or the feet's marks the side the body was pushed to (a push away from the wall's side, which doesn't move it, marks it too), and the next frame's run, before it marks again, changes the speed as if a direction held toward the wall weren't held: on the ground it slows by `deceleration`, in the air too, and at a speed of 0 the rest below a step (`vx_frac`) stays.
- With `head` the tile hitbox no longer stops a rise. After the vertical move, while the vertical speed is below 0, the entity's y is at least `above` and `y mod tile_size` is at least `row_from`, the cell at `point` (from the entity's position, whole pixels) decides: if it stops movement, the first hit listing its kind as `tile` sets the whole part of the vertical speed to `speed` (with `keep_fraction`, plus the speed's fraction as it was; without it, exactly `speed`). The same hit changes the cell to `becomes` and fires `event` for the entity. With `hold` the cell is `solid` at once instead and is held: each frame that isn't frozen takes one off its count, from the frame after the hit's, and after `hold` frames, at the start of the next, it becomes `becomes` (or its kind before the hit, without one). Another body that hits the cell meanwhile gets the `solid` hit's speed and event; the cell keeps its count and its next kind. A cell that stops movement and that no hit lists sets the whole part to 0. The entity's position isn't changed. A machine state's `set` can give the entity another `head`. A hit's `event` is the core's, an extension's or the model's own.
- Under `busy_while` (feature `head.busy_while`), a counter condition, every cell that stops movement is hit as a `solid` one while it holds: the `solid` hit's speed, no event, no change to the cell. A hit's `above` (feature `head.above`), `tile`, `becomes` (default `empty`) and `event`: when the cell above the one hit is `tile` as the hit is made, it becomes `becomes` and `event` fires for the entity, before the hit's own event, as a game that takes a coin sitting on a bumped block does.
- With `pickup` on `jump`, after the frame's moves and before the wall points, while the entity meets tiles, the first of its `points` (from the entity's position, whole pixels, in order) in a cell of kind `tile` changes the cell to `becomes` (`empty` by default) and fires `event` for the entity, and the wall points aren't tested that frame. It takes one cell a frame. In a machine state, `set` can change `pickup` as well.
- Under `max_fall_from_half` (on `jump` or `fall`) the cap is different: after gravity is added, a speed whose whole part is at least `max_fall` and whose fraction is at least one half becomes `max_fall`, and any other speed is kept. With a gravity of 0.4375 and `max_fall = 4` the speed goes 4, 4.4375, 4, 4.4375. It applies to the water's `max_fall` too.

With `move_first` (on `jump` or `fall`) the order is the other way round: the position moves by the old vertical speed, then gravity is added and the cap applied. Landing and bumping go by the direction it moved.

`walk` and `fall` run in `platformer.enemies`.

- A walking entity starts facing the player (left when the player's x is less than its own) and each frame moves `speed` pixels that way. With `tile_collision`, if its hitbox ends up in a solid tile it moves back and turns. With `turn_at_ledges`, if it was standing and the cell below its leading bottom corner isn't solid after the move, it moves back and turns.
- An entity with `fall` that isn't standing adds `gravity` to its vertical speed, caps it at `max_fall` and moves by it, landing on solid tiles. A standing one has vertical speed 0.

`fly`, `hop`, `path` and the movers after them run in `platformer.enemies`.

- `fly` picks a direction once, at spawn, from `heading` (`player` means toward the player then). Then it goes `speed` pixels that way each frame. It doesn't turn by itself (a `kick` or `become` can), and tiles don't stop it.
- A `hop` entity falls as `fall` says, and on any frame that ends with the entity standing (the frame it lands, or its first when placed on the ground) the vertical speed becomes `hop.speed`, so it rises from the next frame. With `every`, only a frame whose number plus `phase` times the body's index in the world is a multiple of `every` counts. With `pattern`, the speed is the pattern's entry at the body's count of hops modulo its length, and the count goes up by one. The count is the body's mechanic word 0 (the first of the four `ext` words), so an entity with a pattern has no `ext` hook, which `gp model check` refuses.
- `hop.land_first` puts the landing before the entity moves. At the start of its turn, before its walk, if the row of pixels below its tile hitbox (or its bottom row, sunk into a tile) is in a tile that stops it, and the vertical speed's whole part is not -2, -1 or 0, the entity is put on that tile's top with its height's fraction kept and takes the hop's speed. Then it moves by its `fall` with no landing, so a fall into the ground sinks into it until the next frame's check.
- `oscillate` follows `path` and precedes falling: the center is the spawn's coordinate on the axis plus `center` (plus `low_center` instead when the spawn's coordinate is at or past `low_at`); the force is `force` when the position is less than the center and minus `force` otherwise; the speed becomes the old speed plus the force, kept within `max_speed` either way; the position moves by the new speed. `move_first` moves the position by the old speed first, the force still chosen by the position before the move. `starts_at_rest` lets a body placed in a numbered slot keep the slot's fraction across but not its speed up and down. With `rest_creep_every`, before any of this a body with no speed along the axis has its position's fraction cleared; if the position is then less than the spawn's coordinate, it moves 1 pixel forward on a frame whose number is a multiple of `rest_creep_every`, and the force and the move are skipped.
- `leap` runs after `oscillate`. The body's mechanic word 0 is the frames left of its wait and word 1 its phase (0 before the leap, 1 in it). Under `waits`, a wait at 0 or less first draws the next (an entry of `waits` by the world's generator, times `interval`), puts the body at the start and sets the phase to 0; then the wait counts down by one. In phase 0 with a `rise`, a body below the start's height less `rise.height` takes `rise.speed` as its speed and moves by it, and nothing else happens that frame; otherwise the phase becomes 1 and the speed `speed`. Then `gravity` is added (capped at `max_fall`) on a frame whose number is a multiple of `every`, and the body moves by its speed. Under `below`, a body more than `below` under the start goes back to it in phase 0. `move_first` makes the body move by the speed it had before gravity is added; `max_fall_from_half` makes the limit act from one half past it; `keep_fraction` lets a body put back at the start keep the fraction below a pixel of its y; and `beat` (a counter with an `every`) makes word 0 count that counter's ticks instead of frames (the wait is taken down on the frames its first `every` comes round, a wait is one entry of `waits` with no `interval`, and a new body waits one tick before its first leap). An entity with a `leap` has no `ext` hook.
- `launcher` runs in `platformer.enemies` after every other enemy and after the leaving half of spawning, and on a frozen frame through the shared `run_frozen` (timers only). The slots that hold a shot of a launcher, or the kind its shot becomes when defeated, as the frame starts are fired into by none that frame. The fired kind moves only in its launcher's turn. A launcher's mechanic word 0 is its timer and word 1 its entry; a shot's word 0 is 1 once aimed. A level's start sets each `next` counter to 0, and a launcher never leaves by the view.
- `shuttle` runs after `leap`: on a frame whose number is a multiple of `every`, mechanic word 0 goes up by one; past `turn_after` it becomes 0 and word 1 (the way: 0 right or down) flips; the body moves `step` that way (facing that way on x). With `sway` it also moves `sway.step` across, plus while (frame number / `half`) is odd and minus otherwise.
- `orbit` and `orbit_part` run after falling. A pivot with mechanic word 1 at 0 moves by `place`, sets its words to 0 and 1, spawns its parts at its position (each a new body at the end of the world's bodies, which this frame's loop doesn't reach), and does nothing else that frame. With `place_on_appear` the spawn system does this as the body appears, and with `appear_still` its parts are still that frame as well. Otherwise, with `in_view` and a camera, nothing happens while its whole-pixel x is at or left of the view's left edge or past its right edge (left plus the view's width less 1). Its angle state (word 0, 256ths of a step, wrapping at `angles` steps) takes `spin` in 256ths of a step (the fixed-point value's raw count shifted down 8); if the state's step is in `skip` it goes 256 more. A part with word 0 at 0 finds its pivot: of the alive bodies with an `orbit` whose whole-pixel distance across plus down from the part's spawn is at most `within`, the nearest (the first of equals); none, and it leaves. Word 0 becomes the pivot's index plus one and word 1 the number of alive bodies before it of its own entity with the same spawn. Each frame a part whose pivot isn't alive leaves; otherwise let `a` be the pivot's step, `H` and `Q` half and quarter of `angles`, and `row` its entry of `rows`, with `fold(a)` as `a` modulo `H` (or `H` less that when it's over `Q`). Then x is the pivot's plus `offsets[row + fold(a)]` and y the pivot's plus `offsets[row + fold(a + Q)]`, each negated unless bit 0 (x) or bit 1 (y) of `signs[(a / Q) mod 4]` is set.
- `springboard` runs between `orbit_part` and `leave_view`. Its mechanic word 0 is the frames of the press run so far (0 at rest), word 1 the player it holds plus one, word 2 is 1 once the boost counts. At rest it looks for the first player, in body order, that stands on it: active, on the ground, not climbing, with a vertical speed of 0 or more, and riding it (`contact.stand`) or with its tile hitbox's bottom row on the springboard's hitbox top as placed (its spawn y plus the hitbox's y) and the two overlapping across. None, and it sets its own `on_ground` and stops. Found, it starts a press (words 0, the player plus one, 0) and runs its first frame at once. Each frame of a press, frame `n` falls in the step whose frames cover it; the player's y moves by the step's `move`, its vertical speed becomes 0 and it stands; the springboard's y becomes its spawn y plus the step's `offset`; from step `boost.from_step` on, the boost's action newly pressed (held, with `held`) sets word 2. On the press's last frame the player's vertical speed becomes `boost.speed` with word 2 set and `speed` otherwise, the words go back to 0 and the springboard stands; without `throw_late` the player leaves the ground then (in the air after a jump, no held gravity, no coyote time, no remembered press). Otherwise word 0 goes up by one and the springboard doesn't stand (`on_ground` false, which a sprite clip's `when = { ground = false }` shows).
- In `platformer.player`, a player that a springboard holds (an active springboard's word 0 above 0 and word 1 its index plus one) still runs `run`, so its speed changes, but doesn't move across and doesn't take off; it doesn't jump or fall. In a model with a springboard that throws late, a player that starts a frame standing with a vertical speed below 0, not taking off this frame, runs `run` on the ground and then leaves the ground where it is, as a thrown player does, instead of jumping and falling; it rises from the next frame. A player that takes off there jumps as usual.
- `leave_view` sits between falling and the entity's hook: with a camera, the position is the whole pixels from its corner along the axis and the view is its size; without one, from 0 and the level's size. The entity leaves when the position is under `margin` or at or past the size less `margin`. `bottom_margin` adds a second way out. The entity also leaves when its whole height is at or past the view's height less that, measured from the view's top (or from 0 with no camera).
- On a `path`, the start is at the spawn, heading out. Each frame it moves `speed` toward its far end (spawn plus `distance` on `axis`) and stops there, waits `pause` frames, comes back the same way and waits `pause` frames at the spawn end. With `wait_near`, the wait at the spawn end lasts at least until the player's x is `wait_near` pixels or more from its own.

An entity's `machine` runs in `entity_states`.

- Each instance starts in `initial` when it spawns, and `restart-level` starts every instance again. `after` counts frames since the instance entered the state, counted in `entity_states`, frozen frames included.
- An event moves an instance's machine at most once (the first transition that matches), as the "Events" table above says which instances it concerns. A transition to the state it's in starts its time again.
- While in a state, the instance's fields are its entity's with that state's `set` merged in (`Entity::in_state`). Its position doesn't change when they do.
- `remove` takes the instance out of the level until the level starts again; it doesn't spawn again from `spawn_ahead`.
- `become:<entity>` removes the instance as `remove` does and adds an instance of `<entity>` at its position, facing the same way, in its initial state and with no speed, as a new entity of the level.

Machines, events and counters:

- A transition first moves the machine to `to`, then takes its `do` actions in order. An event an action fires is handled at once, so it sees the new state: a `kill-player` on the way out of `playing` finds the game in the new state, not `playing`.
- When an event fires, the counters that change on it change first, in name order, then the game's machines in name order, then each entity's machine. Each machine moves at most once for that event, by its current state's first transition that matches; it can move again for another event in the same frame.
- A machine's time in its state is the number of runs of its system (`rules` or `entity_states`) since the frame it entered the state, not counting that frame. A transition with `after = N` is taken on the Nth run.
- A counter change past `max` stops at `max`; with `wrap` it goes on from `min` (or 0) by what's left over and fires `<counter>.wrapped`. A change below `min` stops at `min`. `<counter>.reached_min` fires when a change brings the counter to `min` from above, not when it's already there.
- `level.started` fires when a level starts: before the first frame's systems at power-on, and when `restart-level`, `next-level` or `reset-game` starts one. The level's `counters` are set before it fires.

These run in `platformer.player` for each player, in this order. A player held by a level's `hold_player` camera does none of it, its hook included. A player lifting (its `carry`'s `lift_frames` counting down) does none of it but its hook. Otherwise:

- `shoot` spawns, as the component says, from the position at the frame's start.
- `climb` starts when the entity isn't climbing and either `up` is held with the hitbox's middle in a ladder cell or `down` is held while standing with a ladder cell just below the hitbox's middle bottom. The entity moves across so the hitbox's middle is the cell's middle, with speed 0 and not standing. It moves `grab_down` pixels down when it started from `down`. Climbing, `release` newly pressed lets go (it falls from this frame); `hold_still` held keeps it still; `up` moves it up `speed` and lets go once the hitbox's middle leaves the ladder, standing on the ladder's top with `stand_on_top`; `down` moves it down, and it lets go when the middle leaves the ladder or the hitbox's bottom row is in a solid tile. While climbing it neither runs nor falls. With `up_cycle` or `down_cycle` each move is the cycle's next entry (the step counter advanced first) and the grab frame moves too; with `by_feet` the row under the tile hitbox decides where the ladder holds and ends, as the `climb` row above says.
- Knockback replaces `run` while its frames last; then the jump and falling, as before, with the level's `water` gravity and fall limit while the hitbox's middle is in water, and the jump's `release_cap` after the move.
- A hitbox touching a `hazard` tile kills or hurts as the level's `hazard` says, before pits and the goal.
- `carry` lifts or throws on its action, and a carried entity is put at its offset from the player.

A hurt, from a touch or a hazard, goes like this. No further touch applies to the player that frame. If it's invulnerable from an earlier hurt, nothing else happens. Otherwise its `health` counter changes by minus the damage (bounds and `reached_min` as any change), its invulnerability and knockback start (climbing ends), then `player.hurt` fires for the player and `enemy.hurt_player` for the toucher. So a transition on `player.hurt` sees the counter after the hurt.

In `platformer.enemies` a carried entity runs only its hook; others count down their invulnerability and `shoot` before their movers. In `platformer.contact`, carried entities touch nothing; a `stand` entity holds up a falling player as the component says, and a player that's no longer over the entity it rode stops riding it. After the player's touches, each active entity with `hits`, in order, hits the first entity it touches that it targets and that has `points`, as the component says.

`camera` with a level `step` moves each axis at most `step` a frame (a level start places it at once), and with `hold_player` sets the hold for the next frame while any axis was more than a step away. With a level `camera.axis` of `y`, `spawn` measures `spawn_ahead`, `despawn_behind` and `despawn_ahead` along y.

An exit with `keep` places the player at `enter` (or its spawn) and the cameras on it, then sets the kept coordinate to its place in the old view plus the new view's corner and places the cameras again. An exit with `scroll` freezes the next `scroll` frames. A player keeps its climbing across an exit and drops what it carries.

`ext`: the named hand-written behavior runs once a frame in its system, after the entity's components there, and can read and change that instance. A runtime without it runs the entity on its components alone and reports the behavior as missing. Its numbers are the entity's `ext_params` as its state makes them, and its tables are its `ext_tables`, lifted from the original's asset map, so a hook holds no numbers of its own. The rows come from the lifted assets when the model is compiled. A commercial game's lifted assets live under `$GP_HOME/work` and are never committed, so this ships no game data; tests and docs use made-up tables.

`exits`: each frame the world isn't frozen, the player takes the current level's first exit, in order, whose `take` it meets: its hitbox overlaps `take.at` (they share at least one pixel), `take.hold` is held, and it's standing if `take.standing`. An exit with `start` starts `to` as `next-level` would. The level is placed from its spawns with its counter values set and `level.started` fired, and `restart-level` starts it from then on. Taking any other:

- The level becomes `to`. Its entities are placed as at a level start (its spawns, its spawn objects, `spawn_ahead` counted from the new view); the old level's entities go.
- The player keeps its own state (its machine's state and time) and is put at `enter`, or at its spawn in `to` without `enter`, with no speed.
- Each camera is placed on the player as at a level start: `lead` from the view's corner, kept inside the level, and with `scroll_back = false` the view doesn't go left of there.
- Counters keep their values, the level's `counters` aren't set, and `level.started` doesn't fire. The game's machines don't change.
- `restart-level` afterward starts the level last started by power-on, `next-level` or `reset-game` (so dying in a bonus room starts its level again), and `next-level` goes to the current level's `next`.
- Nothing moves for the rest of that frame, as after a level start: the moving systems after `exits` don't run. Entities waiting with `spawn_ahead` appear at once by the new view, as at a level start, those left of it included; the next `spawn` takes away any past `despawn_behind`.

The original's entering animations (sinking into a pipe, rising out of one, climbing a vine) aren't modeled: the player is at `enter` at once.

A level can be built from objects. With `layout`, the level's tiles and which are solid are what the object format's decoder builds from that entry's objects; `size` is in cells of `tile_size`. `spawn_objects` places, for each object whose kind number its slot's `entities` lists, one of that entity at the object's position, as the format says. These spawns come after the level's own `spawn` list, in the objects' order.

`spawn`: at level start the player spawns, and so does every other spawn when there's no `spawn_ahead`. When `spawn_ahead` is set, each other spawn appears once its x is less than the view's right edge plus `spawn_ahead`, once per level start. Under `despawn_behind`, an entity other than the player whose right edge is more than `despawn_behind` left of the view's left edge leaves the level and won't spawn again until the level starts again.

In a level with `spawn_slots`, `spawn_in_order`, `appear_still` or `draw`, `spawn` runs in the order a game that keeps its enemies in slots runs its enemy loop and then its area parser:

1. `drawn` spawns whose column is already drawn appear in order across while a slot is free; the rest never do. This happens only when a level has just been placed, since the draw then covers `ahead` at once.
2. Unless the draw's `pause` holds them, the other spawns within `spawn_ahead` appear while a slot is free, and `spawn_in_order` makes them follow the order and the turns the field says.
3. When no set is being drawn and the view's near edge has passed another whole `step`, the next set starts. Each frame of a set (`step` over the tile size, times `every`, frames in all) draws its next column every `every` frames, and the `drawn` spawns in it appear while a slot is free; the rest never do.

The draw is in the canonical state: the steps reached and the frames left of the set. A level is placed (a start, an exit, a translated start) with its draw at the view and no set under way.

Under `slot_rest`, before a body takes a slot each numbered slot whose body has left (no longer active) keeps that body's place-across fraction and vertical speed and is free; the body takes the lowest free number, and one that appears by the view starts at its whole x plus the kept fraction with the kept vertical speed. A body that becomes another entity (`become:`) leaves its slot to the new body. A spawn's `at_edge` places the body at the view's near edge plus its width less one, plus `at_edge`, along the level's axis, and its home there too, before it faces the player. `appear_moving` keeps it out of the frame's still list. The slots are in the canonical state.

## Top-down extension

`topdown` 0.1 is the genre extension for games seen from above. A model declares `extensions = { topdown = "0.1" }`, lists `topdown.player` in `model.systems`, and sets `view = "top-down"`. The extension adds the following.

| Component or field | Keys | Meaning |
|---|---|---|
| `move8` (entity) | `speed`, `actions` (optional: `left`, `right`, `up`, `down`) | Moves `speed` pixels a frame across while left or right is held and `speed` pixels down or up while down or up is held, both at once on a diagonal. Solid tiles stop each axis on its own, so it slides along a wall it meets at an angle. No gravity |
| `class` (entity) | `name`, and optional `armor`, `shot_power`, `magic` | A player class chosen before play, with its stats as stated facts. Only a player has one. In 0.1 the simulation reads the class's own components, not these stats |
| `class_select` (level) | `classes`, `next`, `pick` | The class select screen: player entities with a `class` in menu order, the action that moves the cursor and the one that picks. A rebuild starts as one class by pressing `next` that class's index times, then `pick` |
| `topdown.player` (system) | | Each player with `move8` moves across, then down, against solid tiles |

The camera's `axis = "both"` is the core's. Health that drains is a core counter with `every`. Version 0.1 has no generators or shots. It also leaves out keys and doors, magic, and several players under one camera. The interpreter runs `move8` and `topdown.player`; it reads `class` and `class_select` but they change nothing yet. Its systems and names never overlap the platformer's, which a test checks.

## Evidence

Every fact carries a source and a confidence. A fact is any value in the model except the header, the evidence table and `description` fields; a list of plain values (`size = [16, 16]`) is one fact, and a list of tables is walked into.

```toml
[evidence."entities.player.jump"]
source = "stated"
confidence = 100

[[evidence."entities.player.jump".refs]]
at = "games/mosshop/mosshop.s:49"
quote = "JUMP_SPEED   = $FB00"
```

The evidence table is keyed by [model path](formats/model-path.md). An entry covers the part it names and everything under it, and the most specific entry covering a fact is its evidence, the same way contract rules resolve. So one entry can cover all of `levels` while another covers `levels.main.kill_below` alone. `gp model check` fails on any fact no entry covers, and on an entry whose path names nothing.

| Field | Meaning |
|---|---|
| `source` | `observed` (measured by the pipeline from recordings), `inferred` (proposed by an agent) or `stated` (said by a person, in words or in the game's own source) |
| `confidence` | A percentage, 0 to 100 |
| `refs` | What supports it. Observed and stated evidence needs at least one |
| `refs.at` | `path:line` for a file in the repo or package (the path has a `/` or a `.` in it), or any other reference, such as `recording:68fdf735` |
| `refs.quote` | Optional: text that line must contain |
| `refs.in` | Optional: a pinned download the file is in, as `<name>@<revision>`, for sources that can't be in the repo, such as a commercial game's disassembly. `at` is then a path within the download |
| `note` | Optional, for people |

`gp model check` reads every `path:line` ref, relative to the package folder for a game's model, else the git work tree the model is in (or `--root`), and fails when the file or line is missing or the line doesn't contain the quote. A ref with `in` is read from that pinned download under `$GP_HOME/work/`, once the download's hash matches its pin (`crates/gp-cli/src/pinned.rs`). The revision must be the pinned one, given by at least 7 characters. When the download isn't there the ref is skipped, the report says how many were and which command downloads them, and `--json` gives the count as `refs_skipped`. No download is pinned here: game-printer's three pins were the disassemblies of the dropped games, so a ref with `in` is refused by name until a game adds its pin. Moss Hop's refs point at lines of `games/mosshop/mosshop.s`, so a change to the game's source that moves or changes a constant fails the check until the model is updated with it.

People's statements outrank agent inferences. A fact stated in the source and also checked against a recording is `stated`, with both refs.

## Canonical form

A model has one canonical text, which `gp model fmt` writes. Tables come in the schema's order, and an entity's or level's keys in alphabetical order. Whole numbers are integers and fractions the shortest exact decimal string, and defaults are left out. An entity state's `set` values are written the same way as the entity's own fields. Any two tools write a model's text the same way, byte for byte. A change to one fact then changes one line, so models diff and merge cleanly in git. Comments don't survive, so notes go in `description` fields and evidence `note`s. Models are kept canonical: `gp game new` writes the skeleton's model and contract in canonical form, the game gate's Model step runs `gp model fmt --check` on a game a patch touches and merges a conflicting model through its canonical text ([gate](gate.md)), and `just check` runs it on Moss Hop. Donkey Kong's model.toml was hand-laid from its skeleton until 2026-11-05, when its 214 lines of comments became descriptions and evidence notes of the parts they were about and the file was written canonically.

Not here yet: `gp model diff`, which compared canonical forms, so writing a value another way (`"-5.0"` for `-5`, `0x10` for `16`) was no change.

## Editing with types

The model is typed in Rust (`crates/gp-model`), and the JSON Schemas are generated from those types. An editor that reads JSON Schema for TOML gives autocomplete, hover docs and error underlines while you edit `model.toml`:

- VS Code: install the Even Better TOML extension (tamasfe.even-better-toml).
- Zed: its TOML support reads the same config; no extension is needed.
- Other editors: use Taplo's language server (`taplo lsp stdio`).

All of them read `.taplo.toml` at the repo root, which maps files to schemas in `schema/` by name. It maps every `model.toml` to `schema/model-platformer.json` (the core with the platformer extension merged in) and `fidelity.toml` to the contract schema, and mapping, profiles, corpus, clips, package and roms files to theirs. `gp self schemas schema` writes those files; the file names carry no version, so `.taplo.toml` never changes for a bump. A games checkout opened on its own needs a `.taplo.toml` there pointing back at this repo's `schema/` (schema/README.md has one to copy), or a first-line `#:schema <path>` comment, which `gp model fmt` would remove from a model.

The mapping is a config file instead of per-file directives for that reason: `gp model fmt` rewrites a model to canonical text and drops comments, so a directive would not survive it. A config file changes no file and no model hash. The schema checks shapes and types; references between parts and evidence coverage are still `gp model check`.

`crates/gp-cli/tests/schema_mapping.rs` validates every file the rules cover, in templates/ and in the games checkout beside the repo when there is one, and checks that the merged model schema refuses an unknown component.

## Profiles

A profile is a set of switches, each a named set of overrides of a model's mechanic parameters and options, such as Donkey Kong's "smooth walk" or Moss Hop's "slow slugs". The user asked for profiles on 2026-10-30, as a toggle between the original's physics and updated ones: "we should have that toggle too probably but we can just call it retro vs normal". They are switches a player turns on one at a time, in groups such as "Engine fixes" and "Assist".

The model as written is the profile `retro`, every switch off. It is always the default. Comparisons and scores run it, as do refinements and background jobs. A template may name switches and presets in `profiles.toml` beside `model.toml` ([its format](formats/profiles.md)). Donkey Kong's, shortened:

```toml
format = "0.2.0"

[[groups]]
name = "timing"
title = "Timing"
description = "What the console's frame timing decides, off"

[[groups.switches]]
name = "rng_per_frame"
description = "The random numbers step once a frame, not once for each pass of the loop the game waits for the next frame in, as many as the frame's work leaves time for"
off = ["entities.idle.ext_params.idle_loop"]

[[groups]]
name = "fixes"
title = "Engine fixes"

[[groups.switches]]
name = "smooth_walk"
description = "Mario walks and drifts in a jump by his exact speed each frame, not in whole pixels on a 15-frame counter"
off = ["entities.player.run.step_cycle", "entities.player.run.air_step_cycle"]

[[groups.switches]]
name = "high_landings_survive"
description = "A jump that lands 16 pixels or more below where it started doesn't kill Mario"

[groups.switches.set]
"entities.player.jump.fatal_drop" = 256

[presets.normal]
switches = ["rng_per_frame", "smooth_walk", "high_landings_survive"]
```

`off` removes a path, so it goes back to its default: an option off, a list empty. `set` gives a path a new value of the type the model has there. A switch can only name a concrete path the model has, and only a mechanic's parameter or option: a path [classed](mechanics.md) `parameter` or `mechanic` that names its mechanic, under `entities`, `cameras`, `levels`, `counters`, `rules` or, for `input.keep_direction` only, `input` (the last two by the profiles feature `switch.rules`, so a rule's timing, such as a death's wait, and the pad's latch can be switches too). Anything else, such as content and presentation or an entity's `role`, is refused. No two switches may change the same part, so any of them combine. A preset names a set of switches. The name `retro` is taken. Every switch has a description for the settings screen and must change something. `gp model check` checks each path, that switches stay apart, and that the model each switch, each preset and all switches together make is valid, and reports a problem in `profiles.toml` with the path it's about.

Every template keeps a preset `normal` beside `retro`, the game with no lag and none of the console's quirks: it turns on every switch of a `timing` group, and a model with nothing to turn off writes `switches = []` (Moss Hop does). `gp model check` doesn't hold a model to this; the gp-model test `every_template_keeps_to_the_settings_standard` does, over the tool repo's `templates/` (`Profiles::standard_problems`), so today it covers Moss Hop only.

A profile's name is `retro`, the name of a preset or of a switch, or the names of switches and presets joined by `+` (such as `normal+quick_restart`). A profile is applied only when a game is built or played (`gp_model::profile::Profiles::apply`). It writes the overrides of every switch on into a copy of the model and sets `model.profile` to the switches' names, sorted and joined by `+`. The model's canonical text and its hash are the base model's alone, so a profile never changes a recording's bundle. Nor does it change a score or a refinement. The state hash covers the set of switches on: in a profile other than `retro` it is BLAKE3 of `gp-profile `, the set's name, a zero byte and the canonical state, where `retro`'s is BLAKE3 of the canonical state alone, as before profiles. A preset and the same switches named one by one hash the same. The canonical state is the same in every profile, so a state loads in any of them, but runs with two sets of switches never share a hash.

| Where | How to choose a profile |
|---|---|
| The interpreter | `gp interp run --profile normal`, or `--profile normal+quick_restart` |
| The web build and the page's PLAY tab | The game's settings screen (Escape or F1, the right stick's press, or the page's button), which lists the modes and each group's switches with their descriptions ([settings](settings.md)). The web build makes a profile's bundle with `gp_web::profile_bundle` and the screen's list with `settings_catalog`. A `profile=NAME` in the preview's URL wins over the kept setting (`scripts/web/page/preview.js`) |

Donkey Kong's switches are in two groups. The timing group's `rng_per_frame` steps the random numbers once a frame. The engine fixes each turn off one of the NES engine's quirks the model took from the game's code:

| Switch | What it turns off or changes |
|---|---|
| `smooth_walk` | `run.step_cycle` and `air_step_cycle`: whole-pixel steps on a 15-frame counter |
| `smooth_jump` | `jump.takeoff_fraction`: the height fraction set at takeoff |
| `no_landing_hold` | `jump.land_hold`: the 4 frames after a landing |
| `even_climb_down` | `climb.down_cycle`, set to the up cycle's pace |
| `walk_off_survives` | `jump.ledge_fall`: the straight drop off a girder's end that always kills |
| `walk_while_down` | `climb.down_holds`: Down held with Left or Right stops the walk |
| `edge_stops_jumps` | The camera's `keep_in_view_turn`: a jump at the screen's edge turns around |
| `high_landings_survive` | `jump.fatal_drop`, set to 256 |
| `climb_down_stops_at_floor` | `climb.down_through`: the climb down through the floor and round the screen |
| `quick_restart` | The death spin's and wait's transitions, cut short |
| `free_directions` | `input.keep_direction`: the latched pad |

The preset `normal` turns every one on. Moss Hop has none of the quirk options and no timer, so it has one group, an assist: `slow_slugs` (`entities.slug.patrol.period` 4, from 2) and `easy_stomp` (`entities.slug.contact.stomp_depth` 12, from 7), the preset `assist` of both, and `normal` with no switches.

## Changes

Features added and breaking changes, newest first. An added feature needs nothing of a model, a runtime or a plugin that doesn't use it; a runtime that doesn't run one refuses a model that uses it (`gp_sim` does it for a runtime built on it).

| Change | What | What a model, runtime or plugin must change |
|---|---|---|
| Features (2026-10-30) | Versions stop counting options. Core and platformer are 0.4, a model naming a patch such as 0.4.23 reads as 0.4, and each option added from core 0.4.1 and platformer 0.4.1 on (to core 0.4.24 and platformer 0.4.29) is a named feature. The state mapping stays at schema 22, its last counted number, with its additive steps as features. The canonical state's version is computed from its layout, so every model state hash changes once. Schema files lose their versions: `model-core.json` and the rest | A model: nothing; it may list its features in `model.features`. A runtime: read `model.features` and refuse a name it doesn't know; recordings of model runs replay as stale once. A plugin: name new options as features, and bump only for a breaking change |

Before features, from core 0.1.0 and platformer 0.1.0 on 2026-10-28 to core 0.4.24 and platformer 0.4.29 on 2026-10-30, each version bumped for each added option, nearly all of them found in Super Mario Bros.' code. That history is in game-printer's docs/model.md; each option's first version is kept as the `since` note of its row in `feature_list.rs`.

## Moss Hop

`templates/mosshop/model.toml` has 184 facts: 183 `stated`, with refs to the lines of `games/mosshop/mosshop.s` and the baselines they come from, and one `inferred`. It uses 17 features, among them the head hit on its ? block and the pickup of its coins, and its profiles are `retro`, `assist` and `normal`.

`just check` runs `gp model check templates/mosshop --assets` on the lifted assets and `gp model fmt --check` on it, then runs the model in the interpreter against each of the eight baselines in `baselines/mosshop` (`gp interp compare`, in the recipe `_interp-mosshop`). Each baseline's input is played on the model and the run compared with what the baseline holds.

Not here yet: game-printer's `crates/gp-cli/tests/model.rs`, which held each group of facts to one recording on the NES runtime (the jump arc to `jump-height.toml` frame by frame, the run speeds to `finish-level.toml` and `walk-and-turn.toml`, the goal and `kill_below` to the frames the win and the death come on).

Some details aren't modeled, all below what the contract checks. A slug moves on frames when the NMI count is even, which `patrol` doesn't say (its evidence note says so). The player stays between x 0 and 496 (`WORLD_MAX_X`), and the dying player is drawn upside down.

## The ext crate

A model's `ext` names a hook: hand-written Rust for what the components can't say. A game's hooks live with the game, in a crate in its package that depends on `gp-sim` and implements its `Ext` trait; Donkey Kong's is `games/donkey-kong/ext`. This repo holds no game's hooks: `crates/gp-exts` has only a demo hook, and `crates/gp-ext-game` is the empty place the page's build puts a game's crate into.

```rust
pub fn exts() -> gp_sim::Exts
```

`exts()` returns every hook by the name the model uses (Donkey Kong's `dk_throw`, `dk_barrel`, `dk_hammer`, `dk_drum` and the rest). The interpreter takes them with `with_exts` on its runtime (`gp_interp::runtime`). A model with hooks runs in its game's module (`gp interp run --module FILE`), and `--allow-missing-ext` runs it without one, its hooked entities doing only what their components say ([interpreter](interpreter.md)).

A hook's numbers are the model's `ext_params` and its tables the model's `ext_tables` and `ext_rows`, each with evidence, so the crate holds no numbers of its own. The game's random numbers come from its own register where the model keeps one as a counter (Donkey Kong's `random`, stepped by `dk_idle`); otherwise a hook draws from the world's generator.

## Decisions

| Question | Decision |
|---|---|
| Schema language | Rust types with serde, with the JSON Schemas generated from them by `schemars`. One definition, so the published schema can't drift from the parser, and semantic checks live in Rust |
| Numbers | One fixed-point format for the whole model text: 16 fraction bits in an `i64`, written as integers or exact decimal strings. Floats are refused at parse time |
| Where evidence lives | In one table keyed by model path, covering downward like contract rules, so values stay plain and one entry can cover a whole part |
| Extensions | An entity's and level's non-core keys belong to declared extensions; each extension lists what it adds |
| Model text | One canonical text per model, written by `gp model fmt`; templates and packages are kept in it |
| Model directory | `model.toml` and `fidelity.toml`, with `profiles.toml` beside them; lifted assets stay outside, named by slot |
