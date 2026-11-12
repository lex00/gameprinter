# Sprite sets (the sprite-set asset)

Brought from game-printer's docs/formats/sprite-set.md (port wave 4). Links to pages gameprinter doesn't carry yet name game-printer's docs (`~/Documents/checkouts/lex00/game-printer/docs`).

A sprite set holds sprite frames as layouts of tiles, each tile placed in pixels from the entity's position with its own flips, palette and tiles slot, and the sprite palettes each level showed. `gp sprites capture` ([capture](../capture.md#sprites)) writes it into the lifted assets, as `<asset>.toml` beside the files `gp assets lift` writes; a model reads it through a slot of kind `sprite-set` ([the model](../model.md)). The asset map names it with `format = "sprite-set"` and no span: `gp assets lift` writes nothing for it, so a lift never replaces a capture, and `gp assets pack` encodes nothing. `crates/gp-assets/src/sprite_set.rs` is the definition. This is schema 1.

A commercial game's set holds tile numbers and places taken from its tables and recordings, so it stays in `$GP_HOME/work/assets/<source>/` like any lifted file; the model in the repo names its frames by number.

```toml
schema = 1
asset = "sprites"

[[set]]
levels = ["meadow", "cave"]
colors = [0x0F, 0x01, 0x02, 0x03, 0x0F, 0x04, 0x05, 0x06, 0x0F, 0x07, 0x08, 0x09, 0x0F, 0x0A, 0x0B, 0x0C]

[[frame]]
entity = "beetle"
from = ["observed", "table enemies 3"]
pieces = [{ tile = 1, slot = "sprite_tiles", x = 0, y = 4, palette = 5 }, { tile = 2, slot = "sprite_tiles", x = 8, y = 4, palette = 5 }]
```

| Key | Meaning |
|---|---|
| `set` | The sprite palettes (16 NES color numbers, four palettes of four) seen in the `levels` named; a set with no levels is every other level's. A model with a palettes slot draws sprites in its levels' own palettes and uses these only for the report and exports |
| `frame` | The frames, numbered from 0 in order: a sprite's `frame = { slot, row }` names one by its number |
| `frame.entity`, `frame.state`, `frame.from` | For people: what it was found for, and where (`observed`, `table <name> <index>`, `model`) |
| `frame.pieces` | Its tiles: `tile`, `slot` (the model's tiles slot), `x` and `y` (its top left in pixels from the entity's position), `palette` (model numbering: sprite palettes are 4 to 7), `flip_x`, `flip_y`. A frame with no pieces draws nothing |
| `frame.colors` | `[{ palette, colors }]`: palettes the frame brings, a state's own colors |

A runtime places a set frame's picture at its pieces' top left from the entity's position, and mirrors it about the middle of the entity's hitbox when it faces left (`gp-present`'s origins and mirror axis).
