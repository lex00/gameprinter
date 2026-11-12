# Interactive element catalog, version 0

Brought from game-printer's docs/formats/interactive.md (port wave 4). Links to pages gameprinter doesn't carry yet name game-printer's docs (`~/Documents/checkouts/lex00/game-printer/docs`).

`gp gaps interactive` writes what it finds in a recording of the original game as a catalog of interactive element kinds: the parts of a level that react to the player, such as blocks that give a coin when hit from below, bricks that break, coins that vanish when touched. Each record says what sets it off, what it does, what it becomes and how often it can, with the frames that show it. A later lane can build model kinds from the records, and the catalog itself changes no model. [Gaps](../gaps.md) says how the records are found.

The catalog is `interactive.json` under `$GP_HOME/reports/gaps/<template>/` (and under `<template>/<recording>/`, the first 12 hex digits of the recording's ID). `interactions.json` beside it lists every interaction the kinds are built from.

## File

JSON, one object.

| Key | Meaning |
|---|---|
| `schema` | 0 |
| `game` | The source's registry ID |
| `about` | `{template, recording, frames}`: the template, the recording's full ID and the frames read, `[first, end)` |
| `frames`, `play_frames` | Frames read, and those in play (the mapping's `start` or `slots.moving` holds) |
| `has_code` | Whether the code signals were read (a replay with the ROM). Without them `trigger.routine`, `effects.requests`, `code` and the kinds found by a handler or a sound request are missing |
| `interactions` | How many interactions the kinds hold |
| `kinds` | The records, most interactions first |
| `handlers` | Routines found as handlers: `[name, frames it fired in play, of those with a cell change]` |

## A record

| Key | Meaning |
|---|---|
| `kind` | `cell <before> -> <after>` for a background cell that changed, as eight hex digits for the four tile numbers of the 2x2 tile cell (top left, top right, bottom left, bottom right); `sound <effect> (no cell change)` for a sound request with no cell change near; `handler <routine> (no cell change)` for a handler routine that fired with neither |
| `cell` | `{before, after}`, the same tile numbers, or null |
| `count` | Interactions of this kind |
| `trigger` | What sets it off (below) |
| `effects` | What happens with it (below) |
| `result` | `{after, reverted, revert_frames}`: what the cell became, how many changed back to what they were within 60 frames, and after how many frames on average |
| `limits` | `{cells, max_hits, interval}`: distinct world cells; the most changes one cell had, counting changes at most 1,200 frames apart as repeats; the mean frames between them |
| `evidence` | Up to five frames of the recording that show it, spread over its interactions |
| `code` | For the trigger routine, what its code says (below), or null |

`trigger`:

| Key | Meaning |
|---|---|
| `relation` | The most common place of the player's box relative to the cell on the frame before: `below`, `above`, `left`, `right` (the player's side), `inside` (overlapping), `away`, or `none` when no cell changed |
| `share` | The share of interactions with that relation |
| `relations` | Every relation and its count |
| `player_v` | The player's mean speed over the two frames before, pixels a frame, `[x, y]` (y grows downward) |
| `routine`, `routine_share` | The handler routine most of its interactions fired within two frames before, and their share |
| `request`, `request_share` | The sound effect most of them requested within two frames before to three after, and their share |

`effects`:

| Key | Meaning |
|---|---|
| `counters` | For each mapped counter that changed within 2 frames before to 12 after: `[change, how many]`, its most common change |
| `spawned` | Objects that appeared nearby within four frames, `[name, how many]`: a mapped slot's model entity, `slot id 0x..` for a slot no entity lists, or `sprite pal<p> tiles <t..>` for a sprite object no mapped entity explains |
| `sound` | APU channels that started a note within three frames, `[channel, how many]` |
| `requests` | Sound effects requested, `[name, how many]` |
| `ram` | Rarely changing RAM bytes (under 2% of play frames) that changed with it, by label, `[name, how many]` |

`code` (from the ROM, when the trigger routine has a label):

| Key | Meaning |
|---|---|
| `routine` | Its name |
| `exercised` | Branch, jump and call targets in its first 512 bytes that ran in the recording |
| `unexercised` | Those that never ran, by name: the cases the code handles that no recording exercised |
| `compares` | The immediate values it compares a register with (CMP, CPX, CPY #), often the tile or object classes it branches on |

## Turning a record into a model kind

A record has what a model kind needs: the cell's look before and after, and the trigger (the side it's hit from, a handler, a sound). It also has the effects (counter changes, what spawns), whether it changes back and how often it can be hit. The tile numbers name the background tiles the nametable shows; the game's own class for the cell (a metatile number in a level buffer) is in `effects.ram` when the buffer is labelled, and `code.compares` lists the classes the handler tests. A kind found by a sound request or a handler alone is an interaction with no change on screen (a bump on a block that stays, a kick).

## Decisions

| Question | Decision |
|---|---|
| What a cell is | A 2x2 tile block of the nametable, the size most NES games use for a level cell, read from the trace, so any game works without knowing its level format |
| How kinds are keyed | By the cell's tiles before and after, which needs no game knowledge; the game's own metatile class is reported beside it when labels name the buffer |
