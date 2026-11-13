# Sprite capture config (sprites.toml)

Brought from game-printer's docs/formats/sprites.md (port wave 4).

`sprites.toml` in a template directory tells `gp sprites capture` ([capture](../capture.md#sprites)) where a game's sprite tables are and how its RAM names the model's states. It is data: everything particular to a game is here, in its state mapping and in its asset map, and the capture that reads it is the same for every game. `crates/gp-sprites/src/config.rs` is the definition. This is schema 1. A template without the file is captured from its recordings alone.

```toml
schema = 1
facing_left = "slot[0x46] == 2"     # objects in slots, unless an entity says
skip = "ram[0x2A] != 2"             # frames left out whole

[[table]]
shape = "rows"
name = "enemy"
asset = "enemy_frames"
slot = "sprite_tiles"
width = 2
index = { asset = "enemy_sprites", column = "frame", divide = 6, attributes = "attributes" }

[entity.mario]
facing_left = "ram[0x33] == 2"
when = { jumped = "ram[0x1D] == 1", climbing = "ram[0x1D] == 3", invulnerable = "ram[0x079E] != 0" }
state = [{ name = "fire", when = "ram[0x0756] == 2" }, { name = "big", when = "ram[0x0756] == 1" }]
frames = { player = [12, 13, 14] }
```

Conditions are the runtime protocol's ([selectors and conditions](../runtime-protocol.md)) over `ram[A]`, and for an object in a slot also `slot[A]`, A plus the slot's number, as the state mapping's slot fields read it.

## Top level

| Key | Meaning |
|---|---|
| `schema` | 1 |
| `slot`, `asset` | The model's sprite-set slot and the asset it's filled from; `sprites` by default |
| `oam_lag` | Frames the sprite table lags RAM: 1 (the default) for a game that copies sprites at the start of the next frame |
| `margin` | Pixels around an entity's hitbox within which a sprite is the entity's (4) |
| `min_seen` | Fewest frames a frame must be seen in to count (2) |
| `facing_left` | When an object in a slot faces left, for entities with no rule of their own |
| `skip` | Frames to leave out whole: where the game is somewhere the model doesn't have |

## Tables

Each `[[table]]` has a `name` and a `shape`. The shapes are general; a new game names its tables in one of them.

| Shape | Keys | What it reads |
|---|---|---|
| `rows` | `asset` (a `byte-rows` asset), `slot` (the model's tiles slot its numbers index), `width` (tiles across), `palette` (model numbering, 4 by default), optional `index` | Each row is a frame of tile numbers, `width` to a row of the picture. `index` is a column of a `table` asset holding each object ID's first row (an offset, divided by `divide`); an ID's rows run to the next ID's first. `attributes` names a column of the same asset whose low two bits are each ID's palette |
| `pieces` | `frames` and `layouts`, each `{ asset, pointers }` (a `blobs` asset and its pointer table), `count_at`, `layout_at` | A frame is a blob whose head gives the sprite count and a layout number, and whose rows are `[tile, attribute]`; the layout's rows are `[y, x]`, signed, from the entity's position |
| `oam` | `frames` (`{ asset, pointers }`) | A frame is a blob whose first head byte is the count and whose bytes are `y, tile, attribute, x` per sprite |
| `sequences` | `asset` (`{ asset, pointers }`), `frames` (a table's name), `last_at`, `every_at` | An animation per object ID: the head's last step and frames per step, the bytes frame numbers in `frames` |

Tile numbers of `pieces` and `oam` tables are numbers in the pattern table sprites are drawn from; capture finds which lifted tile each is from the recordings.

## Entities

`[entity.<name>]`, for a model entity:

| Key | Meaning |
|---|---|
| `ids` | Its object IDs, beside the state mapping's `[[slots.kind]]` ones; an ID only listed here is placed by the slots' own fields |
| `frames` | Entries of tables that are its frames, by table name |
| `state` | `[{ name, when }]`: the model state it's in, the first whose condition holds. Without rules, a slot's raw state byte groups its frames for the report, and the value its lives start in is its own sprite |
| `when` | Conditions clips choose by: `ground`, `moving`, `rising`, `jumped`, `skidding`, `climbing`, `invulnerable`. Without one, `moving` and `skidding` come from the state mapping's `vx` (else the position over 5 frames), `ground` from its `on_ground`, `rising` from the position |
| `facing_left` | When it faces left, where the state mapping has no `facing`; else the way it last moved |
| `skip` | Its frames to leave out |
