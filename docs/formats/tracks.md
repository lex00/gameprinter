# Tracks, version 0

Brought from game-printer's docs/formats/tracks.md (port wave 4). Links to pages gameprinter doesn't carry yet name game-printer's docs (`~/Documents/checkouts/lex00/game-printer/docs`).

`gp track` finds a game's objects from a recording, with no hand-written mapping: it follows each sprite object through the frames and finds the RAM bytes that hold its position. Bytes that repeat at a fixed stride become object slots. The output is a list of tracks and a proposed `[slots]` section ([state mapping](state-mapping.md#slots)). `crates/gp-track` implements it and `crates/gp-cli/src/track.rs` feeds it a recording's frame trace ([recording format](recording.md#frame-trace)), so it needs no ROM and no runtime.

Status: Milestone 2, step 7. The output is a proposal for a person or an agent to check against the game's code. Nothing reads it automatically.

```text
gp track <recording> [--frames A..B] [--json] [--propose-slots] [--against MAPPING] [--oam-lag 0|1]
```

| Option | Meaning |
|---|---|
| `--frames` | Frames to track, as for `gp events` (default `1..`). Frame 0 is the power-on state and has no input behind it |
| `--json` | The whole report and the score as JSON |
| `--propose-slots` | Print the proposed `[slots]` section instead of the summary |
| `--against` | A hand-written `mapping.toml`; scores the proposal against it (Scoring) |
| `--oam-lag` | Frames the sprite table lags the RAM (default 1, see Frames) |

## Frames

Frame `f` pairs the RAM of state `f` with the sprite table of state `f + 1`, and takes the scroll and sprite size from there too. A game builds its sprites in RAM during frame `f`, and the console copies them to the PPU at the start of the next one, so the sprites in state `f + 1` are drawn from the RAM of state `f`. The scroll comes from the PPU's `t` register and fine x. The controller is the input log entry state `f` ran with. `--oam-lag 0` pairs state `f` with itself. The trace needs the fields `ram` and `oam`; it uses `ppu.t`, `ppu.x` and `ppu.ctrl` when it has them.

## Tracks

Sprites that are not hidden (Y below 0xEF) and share a palette are one object when their boxes touch or overlap within one pixel. An object is followed to the next frame by the nearest object of its palette to where its last motion puts it, within 16 pixels (plus 4 for each frame it went undrawn), preferring one that shares a tile. An object may go undrawn for 4 frames and keep its track. A track has a spawn frame (its first) and a despawn frame (its last). It also has its palette, the tiles it used and the number of frames it was drawn in.

## Binding

A byte holds an object's x when, on almost every frame the object is drawn, the byte minus the object's screen x minus the scroll is the same number mod 256. Either edge of the object's box is tried. The scroll is that of the same frame or that of the next. A byte that holds the screen position itself (a renderer's copy) binds with no scroll, and that is used only when neither scroll binds anything. y is the same against the top or bottom edge with no scroll. Twelve frames spread over a track pick up to 24 candidate bytes for each option, and one pass over the track counts the frames each holds on. A byte is a binding when it holds on at least 0.9 of the track's frames; `score` is that share. Other bytes within 0.02 are listed as `ties`. The RAM pages that equal the sprite table on most frames are left out, since their bytes are the screen positions themselves. A track that stands still has no binding.

For x, `input` is the share of frames with Left or Right held on which the byte moved as the pad says.

## Slots

- The player is the x byte that moves the way the controller says: `input` at least 0.4, and at least 0.9 of its changes while a direction is held agree with it. The best `input` wins.
- A group is a run of two or more x bytes at one stride (1 to 16) that share an active flag offset. A flag byte fits an x byte when it is nonzero on at least 0.98 of the frames the object is drawn and zero on some frame it isn't. A run takes the offset that is clear on the largest share of undrawn frames, if that share is at least 0.4: an object that is still there off screen keeps its flag set, so the share is under 1 (0.43 for Moss Hop's slugs), and below 0.4 a scratch byte that happens to be 0 would be taken. A game that keeps all its objects in one array (Super Mario Bros.' enemies, fireballs and bubbles are neighbors) gives a longer run than one kind, so a run is cut to the longest prefix whose flag is within 0.02 of the clearest prefix's, and trailing slots drawn on under a tenth of the median slot's frames are dropped
- The y byte of a group is the offset from x that most of its tracks' y bindings have.
- The page byte (the high byte of x) is the byte at one offset from every slot's x that changes only on frames x wraps, and by one in the direction it wraps. With no wrap in the range it is named only when the slots sit in different pages: a byte that holds one small value per slot on 95% of the frames, the slots' values differ, the scroll puts each slot on screen by it on every frame it is drawn, and it is the nearest such byte to x (Moss Hop's two slugs, pages 0 and 1, never cross a page).
- The ID byte is a byte at one offset that stays the same through each object's life and is equal for two objects exactly when their palette and tiles look alike (same palette, at least a third of their tiles in common), on at least 0.75 of the pairs. It is left out if every object looks alike.
- The frame counter is the byte that goes up by one on the most frames, at least half of them.

A track's slot is the group slot its x byte is in, and its ID is read from the ID byte.

## Motion

`speed_x` and `speed_y` are the mean absolute change a frame between consecutive frames the track was drawn in. A change of more than 128 pixels is a wrap and left out, and a frame standing still counts as zero. `turns` counts reversals along x, and `period` is the mean number of frames between every second turn. Each position is `[frame, x, y]`. It comes from RAM where the track is bound (x with its page byte when there is one) and otherwise from the box's left and top on screen. A kind groups tracks by ID, or by palette alone when there is none. Its speed and period are weighted by frames.

## Report

`--json` gives `{id, frames, report, score}`. `report` has `first`, `last`, `camera` (which scroll the bindings were read against), `tracks`, `player`, `groups`, `clock` and `kinds`. Each byte is `{ram, score}` with `score` from 0 to 1. A track is `{track, spawn, despawn, seen, palette, tiles, x, y, slot, player, id, motion, positions}`.

## Proposed slots

`--propose-slots` prints the best group as a `[slots]` section in the state mapping's format, with each address's score in a comment. The other groups follow as comments. Check each address against the game's code before using it. A mapping's `[slots]` puts slot `i` at the base address plus `i`. A group at another stride is noted and can't be used as is.

## Scoring

`--against` reads a mapping as a set of (role, address) pairs, one for each slot. The pairs come from the `[slots]` section, or from `entities.<name>[i]` fields when there is none.

| Role | Address |
|---|---|
| `x`, `x_page`, `y` | Each slot's position bytes |
| `active`, `id` | Each slot's flag and object ID |
| `clock` | The frame counter |
| `player_x`, `player_page`, `player_y` | The `player` or `mario` entity's position |

The proposal is the player's pairs and the frame counter, plus the one group that overlaps the mapping most.

Moss Hop's mapping (`templates/mosshop/mapping.toml`) is the ground truth for the open tier. Its two slug page bytes never change in its baselines. No trace names them, so recall stays below 1. `cargo test` (so `just check`) runs `gp track` on two of its baselines and holds precision to 0.95 and recall to 0.75 (`crates/gp-cli/tests/detectors.rs`), the floors game-printer kept in budgets.toml. The tracker reads a frame's sprites as a list (`gp_track::sprites::Sprite`: box, size, tile, palette, flips); `sprites::from_oam` reads the NES's table, and an arcade board's sprite RAM needs its own adapter (wave 6).

## Limits

- Objects drawn by the background (blocks, platforms made of tiles) are not sprites and are not tracked.
- An object that stands still for its whole life, or isn't on screen, has no binding.
- Objects of one palette that touch are one object. A player and an enemy of its palette in contact are one track.
- A group needs two slots to be seen. A flag that never clears in the range is not found.
