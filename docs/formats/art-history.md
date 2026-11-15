# The art history (asset-edits.jsonl, asset-redo.jsonl, asset-proposals/)

A working copy's record of every `assets_edit` and editor save ([sprite editing](../sprite-editing.md)), kept in the copy's folder. `crates/gp-mcp/src/edit_log.rs` is the definition; `gp_mcp::sprite_ops::digit` and `value` write and read a pixel.

| File | What it holds |
|---|---|
| `asset-edits.jsonl` | Placed and accepted edits, oldest first, one JSON object a line: undo takes from here |
| `asset-redo.jsonl` | Edits undone, newest last: redo takes from here, and a new edit empties it |
| `asset-proposals/<seq>.json` | Edits proposed (`place: false`) and edits rejected, one file each, with who decided |

## An entry

| Key | Meaning |
|---|---|
| `seq` | The edit's number in the copy; `<copy>:<seq>` names it to accept, reject and undo |
| `slot`, `entity`, `tile` | The slot it changed, and the entity whose sprite it was (or the level tile kind, with `entity` empty) |
| `ops`, `palette_ops`, `prompt` | What was asked |
| `before`, `after` | The first frame edited, as rows of pixels (below) |
| `state_before` | The slot's state before the edit (its `slots.toml` record, [stubs](../stubs.md)), put back by undo of the newest edit |
| `frames`, `pictures` | The frames of the sprite set it edited, and those whose whole picture was given (an editor's save) |
| `tiles` | Each picture it changed: `tile`, its number in the slot's sheet, and `before` and `after`, its pixels in one string, row by row. On an NES tiles slot that is an 8 by 8 tile (64 digits); on an arcade `board-sprites` slot a 32 by 32 sprite (1024 digits), numbered by its index in the slot's sheet |
| `palettes` | A palettes slot before and after |
| `editor`, `sprite`, `status`, `decided_by` | The editor a save came from, a resize's tile lists, the status (`proposed`, `placed`, `accepted`, `rejected`, `undone`) and who decided |

## Pixels are hex digits

A pixel is one hex digit: `0` to `9`, then `a` to `f` for 10 to 15. Upper case reads too; the history writes lower case. An NES sprite's colors are 0 to 3, so its rows are the digits they always were. A Midway MCR board sprite has 16 values, which take the rest:

```json
{"seq":4,"slot":"sprites","entity":"tip","before":["0000..."],"after":["00c0..."],
 "tiles":[{"tile":77,"before":"0000...","after":"00c0..."}],"pictures":["board-sprite.4d"], ...}
```

Until 2026-11-14 a row was the digits 0 to 9 and nothing else. Files written then read the same now, since 0 to 9 mean the same in hex; no rewrite is needed.

The files carry no format version, and none was added: an entry refuses keys it doesn't know (`deny_unknown_fields`), so a version key would make every older gp refuse every history. What an older gp does with a newer file instead:

- An NES history is byte for byte what it wrote, so it reads and works as before.
- Hex letters come only in an arcade board sprite edit (its `pictures` are `board-sprite.CODE`), which only a gp that writes hex makes. An older gp lists such an edit (`assets_history`) and refuses to accept, undo or redo it, writing nothing: those go through its NES slot target, which says "slot `sprites` is board-sprites; assets_place takes tiles and palettes slots so far". Its row reader would stop at the first letter too, with "`c` isn't a color index" (or "not 64 color digits" for a tile).
