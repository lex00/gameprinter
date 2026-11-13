# Capturing presentation

Brought from game-printer's docs/capture.md (port wave 4) for `gp sprites capture` and `gp sound capture`; gameprinter's docs/capture.md is the dense and carry clips. What it says of the Bevy generator is game-printer's: that isn't in gameprinter. `gp sound capture` reads the NES's APU. `gp sprites capture` reads an arcade game too (port wave T): see "An arcade game" below. Links to pages gameprinter doesn't carry yet name game-printer's docs.

A model's presentation is everything a runtime draws ([the model](model.md#presentation)). For each part of it, this page says how capture finds the part for any NES game. It also says what is still written by hand and what would make it automatic. Super Mario Bros. is the worked example; nothing here is particular to it.

Mechanics and their parameters are captured as [mechanics](mechanics.md) says, and the facts a person already wrote are read back from their evidence by [derivation](derivation.md), which counts the presentation facts it can't derive yet.

Capture has three sources, and each part below uses one or more of them.

| Source | What it gives | Where |
|---|---|---|
| The asset map and the disassembly's labels | Where each table is in the ROM, its format, and the routine that reads it, so a lift copies the data and the code says what it means | `assets/<source>.toml`, `gp assets lift`, [labels](formats/labels.md), `just smb-asset-labels` |
| Traces | What the console drew on each frame: the sprite table (OAM: each sprite's Y, tile, attribute byte and X), palette RAM, the name tables, the scroll | The frame trace of a recording ([format](formats/recording.md)), `gp trace`, `gp track` ([tracks](formats/tracks.md)), the inspector |
| Measurement | Facts of the console and the display it was made for | The NESdev wiki, `gp-nes`, `sources/nes.toml` |

Two NES facts come up again and again. A sprite's attribute byte holds its palette in bits 0 and 1 (sprite palette 4 plus that number, in the model's numbering), a horizontal flip in bit 6, a vertical flip in bit 7 and the priority in bit 5. A name table is 32 by 30 tiles from $2000, followed by 64 attribute bytes from $23C0, each giving the palettes of a 4 by 4 tile block, two bits for each 2 by 2 quarter.

## Summary

| Part | Model | Found today by | Manual today | Automatic with |
|---|---|---|---|---|
| Tiles | `tiles` slot | Asset map | The map entry | Already lifted; CHR banks found from the mapper |
| Palettes, per level | `palettes` slots, a level's `present.palettes` and `background` | Asset map and labels | Which palette set each level uses (a script reads the area type) | Palette RAM on each `level.started` in a trace, matched to the lifted palettes |
| Sprite frames | `sprite-set` slot, a sprite's `frame` | `gp sprites capture`: the tables `sprites.toml` names and the recordings' sprites | Where the tables are, and the RAM that names a model state | Tables found by the catalog's table shapes |
| Sprite layout | A set frame's pieces | `gp sprites capture`, from the sprite table of each recorded frame | Nothing | |
| Sprite colors | A set's palettes, a frame's `colors` | `gp sprites capture`, from palette RAM by level and state | Nothing | Matching set colors to lifted palettes, so a level's palettes come from the ROM |
| Animation | A sprite's `clips` and `blink` | `gp sprites capture`: frames in order, timing, game clock, held clips and blinks by model state and condition | The RAM conditions in `sprites.toml` (a jump, a climb, invulnerability) | Speed steps (`by_speed`) measured from `vx` |
| Color cycles | `present.cycles`, a level's `cycles` | Asset map and labels | The palette, color and period | Palette RAM changing on a steady period |
| HUD layer | `screen-text` slot, `present.hud.layer` | Asset map (update strings) | Which string is the HUD, and its size | Name table rows that stay put while the view scrolls |
| Font | `font` slot | The asset map's `charset` | The charset | Digit tiles from counters bound in RAM |
| HUD fields | `present.hud.field` | The status routine, read by hand | Positions, digits, palettes, the counter each shows | Name table cells that change when a bound counter does |
| Level labels | A level's `present.label` | The level list (a script) | The text | The HUD's level field in a trace |
| Screen | `present.screen` | Measurement | Written once per platform | A platform table the model reads |

## Tiles

| | |
|---|---|
| Asset map and labels | A `chr-2bpp` entry per pattern table or bank. For a CHR ROM game it's the CHR region; for CHR RAM, the PRG data the game copies there (Mega Man 2's `mm2-chr-uploads`) |
| Traces | A trace doesn't hold CHR, but its PPU control byte says which pattern table sprites and the background use |
| Manual today | The entry, and which tiles slot is the background's and which the sprites' |
| Automatic with | The mapper's banks listed from the ROM header, and the pattern table split read from `ppu.ctrl` in a trace |

## Palettes and the background color

| | |
|---|---|
| Asset map and labels | `palette` entries at the labels of the game's palette data (Super Mario Bros.' `GroundPaletteData` and its siblings, `PlayerColors`), and a `table` for background colors (`BackgroundColors`) |
| Traces | Palette RAM ($3F00 to $3F1F, the trace's `palette` field) at any frame: the set the game had loaded |
| Manual today | Which set each level uses. For Super Mario Bros., `scripts/model/smb-levels.py` reads the area type and the header's background control, as `GetAreaPalette` does |
| Automatic with | Palette RAM sampled a few frames after each `level.started` in a recording through the state mapping, then matched to the lifted palette assets (a set the ROM doesn't hold whole, such as Super Mario Bros.' snow palettes written over the ground set, shows as a match of its parts) |

Sprites take the second half of their level's set, so the underground's enemies are blue. A palette a state brings, such as fire Mario's, is a sprite's `colors`.

## Sprites

`gp sprites capture <template> --recording ID...` captures every sprite frame of every entity, by default for any game ([sprites.toml](formats/sprites.md), [the sprite set](formats/sprite-set.md)). It is `crates/gp-sprites`, and nothing in it names a game.

### An arcade game

On a package for Midway's MCR board (Tapper), capture reads each frame of an arcade recording through gp-arcade's sprite list: each sprite's place in the game's units (the screen is 256 by 240 of them), its picture's number in the graphics region `gfx2`, its colour bank and flips. The picture is found by its number among the arcade lift's `mcr-sprites` (32 by 32 pixels, 16 by 16 in the game's units), so a set frame's piece is a whole sprite, and a frame such as Tapper's bartender is several of them. Colour banks 0 to 3 are palettes 4 to 7. A position the state mapping reads is taken in the sprite RAM's own units, as the game's object slots hold them, and placed where the board would draw a sprite with those bytes, `(x - 3, 241 - y)`. The board draws a frame's sprites from that frame's sprite RAM, so `oam_lag` doesn't apply.

The arcade asset map names the set with a `sprite-set` entry and no region, as an NES map does; its pictures are already named `sprites` there, so the set needs another name, given in `sprites.toml` as `asset` and `slot`:

```toml
# assets/tapper.toml
[[asset]]
name = "sprite_set"
format = "sprite-set"

# sprites.toml
schema = 1
asset = "sprite_set"
slot = "sprite_set"
```

The board's colours are in its palette RAM, not NES colour numbers, so no palettes are captured. A picture numbered past the lift's sprites (the board wraps those) counts as a sprite with no lifted tile. `--apply` is refused: no model slot takes an arcade lift's pictures yet, so the model is only proposed.

| Part | How it's found for any game |
|---|---|
| Table frames | Each table `sprites.toml` names, in one of four general shapes (rows of tile numbers, tile and attribute records with a layout table, OAM records, animation sequences), read from the lifted files. Rows tables are bound to entities through an index column by object ID; sequences by object ID too |
| Seen frames | On each recorded frame the state mapping places every entity (its fixed fields, and its object slots with their kinds), and each sprite of the sprite table goes to the entity whose hitbox it's on, or that it touches through a sprite of the same palette already given (a boss's back half). Sprites are 8x8 or 8x16 by PPUCTRL |
| Tiles | A sprite's tile is found through the cartridge: CHR ROM through the mapper's banks (NROM, MMC1, MMC3 from the trace's registers), CHR RAM by its pixels against every lifted tiles asset. A tiles asset the model has no slot for gets one |
| One frame each | A frame is its tiles, places, flips and palettes from its top left; where that corner sits from the position is kept per frame (its most usual place), so a pixel of drawing lag makes no new frame. Frames are made relative to the way the entity faces: the mapping's `facing`, a rule, or the way it moved |
| States and conditions | Rules over RAM give the model state; without rules a slot's raw state byte groups frames for the report. Clip conditions come from the mapping (`on_ground`, `vx`), rules (`jumped`, `climbing`, `invulnerable`) or motion. Frames while a state freezes the world, of a slot whose state ends its life, near the screen's edges, or in no level of the model are left out and counted |
| Clips | Per state and condition: frames ordered by what follows what, the usual run as `every`, the game clock when changes keep to it, `hold` when no run ever changes picture (a walk that stops in the air), a `blink` when empty frames come in short runs. Pictures that change every frame are a transition the model can't choose yet, reported and left out |
| Palettes | Palette RAM per level as the set's palette sets; a palette a state shows in most of its frames that its level's set doesn't have is the state's own colors |
| Coverage | Per entity: model states with frames, table frames never seen, seen frames no table holds, timings and palette variants; recorded as `sprites.<template>.frames`, `.states`, `.states_covered`, `.unobserved`, `.observed_only` for the scorecard |

A frame's place is kept from the mapped position, so it is right in the rebuilt game only where the model's body is where the mapping puts it. A body that stands lower or higher than the game's position draws its frames into the floor or above it by the same amount. The usual cause is a body that touches tiles with a box other than the game's: Super Mario Bros.' walking enemies stood 3 or 4 pixels below Enemy_Y until they had the game's tile box (`tile_hitbox`, foot at (8, 24) from the position). `gp interp slots`'s y line measures it; fix it in the model's tile box, with the routine that looks for ground as evidence. Leave the mapping and the frames alone.

The set goes to the lifted assets. The model, with each captured entity's sprite and states, is written beside it as a proposal. With `--apply` it is written over the template's instead, and its evidence names the recordings and `sprites.toml`. A sprite the model already has stays first: its frames move into the set, a captured clip goes after its clips where none of them holds, and before them only where the first that holds shows other frames and capture saw it at least 30 times (falling off a ledge keeps the walk). A sprite capture agrees with is left as it was, so Moss Hop's model doesn't change. A hand-written frame that differs from a captured one only by a flip is that frame, and one no recording or table showed gives way to the captured one, so capturing an applied template again gives the same model and the same set (`just sprites-roms` checks this).

To draw on the set by hand, `gp assets open` takes an entity's frames from a copy of the game to a pixel editor and back, laid out as an [image format config](formats/image-config.md) says ([sprite editing](sprite-editing.md)).

Run on 2026-10-29 (recordings named in each template's evidence):

| Game | Frames | States with frames | Entities with no frame | Table frames never seen | Seen frames in no table |
|---|---|---|---|---|---|
| Moss Hop | 2 | 2 of 2 | 0 of 2 | 0 (no tables) | 0 |
| Super Mario Bros. | 161 | 34 of 42 | 9 of 37 | 11 | 2397 |
| Mega Man 2 (Wood Man's stage only) | 23 | 3 of 9 | 7 of 10 | 5 | 143 |
| Super Mario Bros. 2 | 28 | 6 of 15 | 5 of 11 | 0 (no tables yet) | 0 |

## Sprite frames

| | |
|---|---|
| Asset map and labels | `byte-rows` of tile numbers at the drawing routine's table (Super Mario Bros.' `EnemyGraphicsTable`, six tiles a row; `PlayerGraphicsTable`, eight) |
| Traces | `gp track` follows each object (sprites of one palette that touch) and lists the tiles it used |
| Manual today | The row each entity and state shows, read from the offsets table (`EnemyGfxTableOffsets` over 6) |
| Automatic with | Each bound track's tiles matched against the rows: the row whose tiles the object shows most is its frame, and a model state is bound to the rows seen while the state's RAM holds its value |

Games that store frames as lists of sprites with their own offsets (metasprites: Y, tile, attribute, X), as Mega Man 2 does, need an asset format that lifts those records into a sprite's `tiles` and `cells`. The open questions hold that format.

## Sprite layout

A sprite's `cells` are its OAM layout: each tile's flips, palette and place. They are how a game that saves tiles by mirroring half a sprite draws it right, and how a falling podoboo turns over.

| | |
|---|---|
| Asset map and labels | Rarely data. Super Mario Bros. sets flips in code: `MirrorEnemyGfx` mirrors the right column of the bloober, the piranha plant and the podoboo, and of a shell; `ChkForPlayerAttrib` mirrors the right tile of the standing Mario's feet |
| Traces | Every frame's OAM holds the answer: for a tracked object, each sprite's tile, attribute byte and position relative to the object's box |
| Manual today | All of it, from reading the routine |
| Automatic with | A layout proposed from the tracks: for each distinct set of tiles an object shows, the attribute bits and offsets of its sprites, made relative to the facing (an object moving left with every sprite flipped is the facing flip, not a layout). The lift would then write `cells` as a proposal for a person to confirm |

## Animation

A clip says which frames show while a condition on the entity's state holds, and how fast they change, driven by its own timer, its speed or the game's frame counter.

| | |
|---|---|
| Asset map and labels | The timing tables and the routine. For Super Mario Bros., `PlayerAnimTmrData` (2, 4 or 7 frames a step by speed, chosen in `GetPlayerAnimSpeed`), `PlayerGfxTblOffsets` (the walk, skid and jump rows), and `CheckForSecondFrame` (an enemy's second frame while the frame counter's bit 3 is clear) |
| Traces | The tile sequence of each track, frame by frame, and the RAM bound to it: its speed, its state, whether it stands |
| Manual today | All of it: the conditions (`ground`, `moving`, `skidding`, `rising`, `speed_at_least`), the rows and the timing |
| Automatic with | For each bound entity, the frames it shows grouped by the values of its bound bytes: a frame change that lines up with the frame counter is a `game` clock; one whose period follows the speed byte is `by_speed`, with thresholds where the period changes; one tied to a state byte is a state's sprite. `gp track` already binds position bytes; binding speed and state bytes the same way is the step this needs |

Presentation keeps its own state (each body's clip, picture and timer), which `gp-present` steps after each frame. The game keeps the same in RAM (`PlayerAnimTimer`, `PlayerAnimCtrl`), and since state mapping schema 6 a mapping loads them into a start (`present.mario[0].frame` and `.timer`, [the format](formats/state-mapping.md#a-bodys-animation)). Capture doesn't write `clock = "body"` clips yet: Super Mario Bros.' Mario clips are written by hand from `ProcessPlayerAction`, and a capture with `--apply` would put its own clips before them where it saw other frames 30 times or more ([open question](open-questions.md)).

## Color cycles

| | |
|---|---|
| Asset map and labels | A `table` at the list of colors (`ColorRotatePalette` for Super Mario Bros.' question blocks and coins), and the routine that writes it (`ColorRotation`: every eight frames, into background palette 3's second color) |
| Traces | Palette RAM over a run: an entry that steps through values on a steady period while nothing else changes |
| Manual today | The palette, the color and the period |
| Automatic with | A scan of palette RAM in a recording for entries that change periodically, giving the palette, the color, the period and the list; the list found in the ROM by its bytes then gives the label and the table to lift |

Cycles apply to background palettes. A sprite palette that flashes (a star) would need masks for sprites too; it's filed as an open question.

## The HUD

The HUD is a layer of fixed tiles, a font, and fields bound to counters and the level's label.

| Part | Asset map and labels | Traces | Manual today | Automatic with |
|---|---|---|---|---|
| Layer | A `ppu-strings` entry, when the game writes its status bar as update strings (Super Mario Bros.' `TopStatusBarLine`). The writes include the attribute bytes, so the palettes come with it | The name table rows above the split that stay put while the view scrolls (Super Mario Bros.' sprite 0 hit marks the split) | Which entry, and the rows it covers (`size`) | Rows found from the sprite 0 hit's scanline and the scroll, and their tiles matched to an update string in the ROM; a game that draws its HUD from code instead needs a name table snapshot as the layer |
| Font | The `charset` on a text asset in the map, written from the disassembly's text encoding | Tiles in the HUD rows that change with a counter | The charset | The digit tiles found from a counter bound in RAM: the tile shown when it holds each value |
| Fields | The status routine's table (`StatusBarData`: PPU address and digit count of the score, coins and time; `WriteBottomStatusLine` for the world and level) | Name table cells that change on the frames a counter changes | Position, digits, palette, `divide`, and the counter each shows | Cells matched to counters bound through the state mapping, their run length giving `digits`, and the attribute bytes the palette. A digit that never changes beside them (Super Mario Bros.' score's last 0) gives `divide` |
| Level label | The level list (`WorldNumber`, `LevelNumber`) | The level field's text in a trace | The text (written by `scripts/model/smb-levels.py`) | Read from the HUD in a trace once the fields are known |

## The screen

| | |
|---|---|
| Measurement | The NES puts out 256 by 240 pixels. On an NTSC television a pixel is 8/7 as wide as it's tall and the top and bottom 8 lines are usually hidden (the NESdev wiki's Overscan and PPU pages); PAL shows more lines and wider pixels |
| Manual today | Written in each model's `present.screen` |
| Automatic with | A platform table, per region, that a model takes by its source (`sources/nes.toml` already says which region a ROM is), so a model writes the screen only when it differs |

## Sound

Sound is model data since core 0.4.3, and [sound](sound.md) has all of it. In short, for any NES game:

| Part | Model | Found by | Manual today | Automatic with |
|---|---|---|---|---|
| Notes and instruments | `songs` and `sound-effects` slots (`gp-songs`) with a `capture` recipe | The APU register log of a recording (its event stream or a replay), applied to the APU model: notes from triggers, pitch and volume; instruments from clustered envelopes; tempo and loop from timing | The recording and frames to capture | Every store recording of a game, split by level through the state mapping |
| Music and effects | Each song's `kind` | gp gaps' sound requests (queues and play-sound routines, named by the disassembly's constants), effects claiming the channels they start notes on | Nothing for queues named by channel; a game asking for both through one routine is filed | Per-request classification from how many channels a request's notes take and for how long |
| Bindings | `present.sound` | Effect names against model events, music names against rule states, the level from the frames the player is in control | Writing the found bindings into the model | The same through the state mapping's level and the model's event stream on the same recording |
| Driver data | `origin = "driver"` songs | A registered driver's decoder on the lifted music (`gp sound decode`), cross-checked with the capture | The song's slots and loop for a game's chained songs | The slots from the requests the capture saw |
