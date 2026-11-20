# Board art

An arcade game's model draws its stage and its bodies with its board's own pictures: the background tiles and sprites in the board's graphics chips, the screens and colours the program keeps in its tables. All of them come from the player's romset through the arcade asset map (`gp_assets::arcade`) when the game loads; the model names them by slot and holds none of them. This page is the model's side: five slot kinds, a level's `present.board` and an entity's `board_sprite`. It is written for Midway's MCR board with the 91490 CPU board and the 91464 sprite board (Tapper's), whose drawing is `gp_arcade::video`.

Code: `gp_model::model::{BoardLook, BoardSprite, BoardByte, PaletteWrite}` (parsed and checked in `gp_model::validate`), the arcade map's `mcr-screen` and `mcr-palette` formats in `gp_assets::arcade`, and the drawing in `gp_look::board`, which the screen measure (`gp-cli`'s `screen.rs`) and the mechanic page's REVIEW REBUILD use.

## Why it reads this way

The board draws a frame from three memories: video RAM (a 32 by 30 map of 16 by 16 tiles, two bytes a cell), sprite RAM (128 entries of four bytes: y, flags, picture, x) and palette RAM (64 colours of 9 bits). The model gives each of them from its own state and slots:

| Board memory | From the model |
|---|---|
| Video RAM | The level's screen (`present.board.screen`), a table of the program lifted from the romset: the stage as the model's level, not a copy of the original's RAM |
| Palette RAM | The level's colours (`present.board.palette`), a table of the program, and the bytes the program writes over it after loading it (`palette_writes`), each from a lifted table |
| Sprite RAM | One entry for each body of an entity with a `board_sprite`: its position as the entry's y and x, its picture and flags from its words or fixed numbers, at the entry its level's `present.sprite_order` gives |

The sprite's picture and flags are the bytes the board reads, not a table of frames made for the model. The game keeps each object as the four bytes the board reads (its program copies its object table into sprite RAM each frame), so a state mapping loads them into a body's word as they are, and a hook that runs the game's objects keeps them there. The copy is made as the frame starts, before the program moves its objects, so the board's picture of a frame shows the objects as they stood a frame before, while its tile layer is the frame's own (issue #42). The model's sprites are drawn the same way, a frame late: a frame's sprite RAM is made from the frame before's bodies (and which of them the game showed), the rest from the frame's own world (`gp_look::board::SpriteLag`). Only what draws frames in turn keeps the lag: the page's board drawing, the screen measure (`gp picture`, the lockstep's screen score) and `gp theme play`; a frame drawn alone, or after a jump or a loaded state, has no frame before and draws its own bodies. The state and the rules don't see it. NES models choose a sprite's picture by `clips` over the model's state; an arcade model can do the same later by fixing `picture` per clip, but the board's own bytes come first.

The stage is a lifted table rather than mapped state: a mapping could read video RAM each frame, but then the picture would be the original's and not the model's, and a model played without the original (FREE PLAY, the browser) would have no stage. The screen the program copies into video RAM is in its ROM, so the lift reads it from the player's romset like any other table.

Everything is then drawn by `gp_arcade::video::draw`, the board's own drawing, with the lifted tiles and sprites: the tile layer, the sprites from the last entry down (the last on top), a sprite pixel that hides lower sprites without drawing, the colour banks and the doubling to 512 by 480 are the board's, and nothing decodes a pixel a second time.

## Slot kinds

| Kind | Arcade map format | What |
|---|---|---|
| `board-tiles` | `mcr-tiles` | The background tiles (`gfx1`), 8 by 8, drawn doubled |
| `board-sprites` | `mcr-sprites` | The sprites (`gfx2`), 32 by 32 |
| `board-screen` | `mcr-screen` | A whole screen of the tile layer: 30 rows of 32 cells from `offset` in `maincpu`, two bytes a cell, low first (tile bits 0 to 9, flip x 10, flip y 11, colour bank 12 and 13, priority 14 and 15). Lifted as `NAME.toml`, `rows = [[...], ...]` of 30 rows of 32 cell words |
| `board-palette` | `mcr-palette` | The 64 colours, two bytes each from `offset` in `maincpu`, unpacked as Tapper's loader ($600F) unpacks them into the board's 9 bits (red bits 6 to 8, blue 3 to 5, green 0 to 2). Lifted as `NAME.toml`, `colors = [...]` |
| `board-font` | `mcr-font` | Background tiles (`gfx1`) that draw text, lifted as `mcr-tiles` are, with a `charset` as an NES map's (`[[0, "0123456789"]]`) whose numbers count from the asset's `first`: the tile a character is drawn with is `first` plus its number. The letters a theme or a new game draws text with on the board. `gp template font` checks it ([The font check](../templates.md#the-font-check)) |

A `rows` slot (`byte-rows`) gives `palette_writes` their bytes. Every board kind is presentation only: the simulation never reads it.

In the arcade map, `mcr-screen` and `mcr-palette` name `region = "maincpu"` and `offset`, and no `count` or `width`: their size is the board's. `[asset.at]` gives the place in a set whose program keeps the table elsewhere, as for `byte-rows`. The public page's lift takes a set whose program differs from the map's own set only when every program table has a place for it, a table kept at the same place giving its own offset (docs/web.md, "A set's tables").

```toml
# assets/tapper.toml (the arcade map)
[[asset]]
name = "palette"
format = "mcr-palette"
region = "maincpu"
offset = 0x616D

[[asset]]
name = "screen_saloon"
format = "mcr-screen"
region = "maincpu"
offset = 0x6221

[[asset]]
name = "digits"
format = "mcr-font"
region = "gfx1"
first = 0x30
count = 10
per_row = 10
charset = [[0, "0123456789"]]
```

## A level's look

```toml
[levels.main.present.board]
palette = "board_palette"   # a board-palette slot
tiles = "board_tiles"       # a board-tiles slot, which `screen` is drawn with
screen = "screen_saloon"    # a board-screen slot: the stage

[[levels.main.present.board.palette_writes]]
at = 0x60                   # palette RAM's byte 0x60
slot = "bar_colors"         # a rows slot
row = 0
column = 0
```

| Key | Meaning |
|---|---|
| `palette` | Required: the board's 64 colours |
| `tiles` | The background tiles; required with `screen` |
| `screen` | The stage. Without it every cell is tile 0 in colour bank 0 |
| `palette_writes` | Bytes the program writes into palette RAM after loading `palette`, in order: byte `column` of row `row` (both default 0) of a `rows` slot goes to palette RAM's byte `at` (0 to 127). As the board's palette RAM does, colour `at / 2` takes the byte as its low 8 bits, and as its ninth bit 1 when `at` is odd and 0 when even |
| `sprite_order` | (`present.sprite_order`, as on the NES) each entity's entry of sprite RAM: body k of the entity takes `at + step * k`, k being its slot (or its `word` less one, or its place among its kind's bodies). Entries run 0 to 127. A body with no entry, or whose entry another body took, takes the lowest free one, under the rest |

`looks` (other looks chosen by counters), `counters` and `repeats` (counters drawn as tiles) and `texts` (strings drawn as tiles) are below. The board draws a level only when it has `present.board`; a level without one is drawn as before. The board has one tile region and one sprite region, so a model names one `board-tiles` and one `board-sprites` slot.

## Looks chosen by counters

A game with several stages and colour sets picks them by its own counters: Tapper's setup ($5DF6) takes its stage from the pointers at $61ED by the levels cleared (E4A8), the bonus round ($2500) loads its own, and $570C writes a bar's two colours by the bar shown (E3B8). `looks` (feature `level.board.looks`) lists the level's other looks, each with the counter tests that choose it:

```toml
[[levels.main.present.board.looks]]
when = [{ counter = "state", cmp = "==", value = 16 }]
screen = "screen_bonus"
palette = "board_palette"
palette_writes = [{ at = 0x60, slot = "bonus_colors" }, { at = 0x6D, slot = "bonus_colors", column = 5 }]

[[levels.main.present.board.looks]]
when = [{ counter = "cleared", cmp = ">=", value = 2 }, { counter = "cleared", cmp = "<=", value = 4 }]
screen = "screen_sports"
palette_writes = [{ at = 0x60, slot = "bar_colors", row = 1 }, { at = 0x6C, slot = "bar_colors", row = 1, column = 1 }]
```

| Key | Meaning |
|---|---|
| `when` | Required: the counter tests, each `{ counter, cmp, value }` as a HUD field's `when` (`cmp` one of `<`, `<=`, `==`, `!=`, `>=`, `>`). The look is chosen while every one holds |
| `screen` | A `board-screen` slot in place of the level's `screen` |
| `clear` | `true`: the tile layer cleared, tile 0 everywhere in colour bank 0, in place of any screen (feature `level.board.looks.clear`), as a program that clears video RAM for an interlude. The counters, repeats and texts still draw over it. Not with `screen` |
| `palette` | A `board-palette` slot loaded in place of the level's. The level's `palette_writes` aren't written over it then, only the look's own, as a program that loads its colours again |
| `palette_writes` | More bytes written into palette RAM: after the level's own writes, or with `palette` after loading it |

A look that clears the screen, for a "get ready" message drawn on an empty layer:

```toml
[[levels.main.present.board.looks]]
when = [{ counter = "interlude", cmp = "==", value = 1 }]
clear = true
```

Each frame is drawn with the first look whose tests hold, or the level's own when none does. The choice is made as the frame is drawn, from the world's counters then: a hook that sets a counter (`ExtCx::set_counter`) changes the stage and its colours on that frame, and the level, its bodies and its spawns stay as they are. A level's look is drawn with the level's `tiles`, so a look's screen needs them.

This is the arcade side of what an NES model does with a level of its own (`levels.*.present.palettes`, `present.background`) and a counter's `when` (a HUD field's `when`, `present.hidden`'s): an NES game that changes stage changes the model's level, which places the level's bodies again; an arcade board that redraws its stage under the same play keeps its level and changes its look by a counter. The counter is the model's, so in free play the look follows the game the model plays, not a reading of the original's RAM; a mapping can load the counter at a clip's start (a presentation field) until the hooks keep it.

## Counters drawn as tiles

A board game's vblank writes its score, its lives and its level number into video RAM; the model keeps counters. `counters` and `repeats` (feature `level.board.counters`) draw a counter into the tile layer over the screen's cells, each cell's two bytes from bytes lifted from the player's romset, as `palette_writes` takes its bytes: never tile codes written into the model.

```toml
# Tapper's score ($05D5): digit d's cell is ($30 + d, $30); leading zeros
# are skipped, so those cells keep the screen's tiles.
[[levels.main.present.board.counters]]
counter = "score"
at = [3, 1]
digits = 6
tile = { slot = "score_codes" }               # the code of `add a,$30` ($05F6)
high = { slot = "score_codes", column = 4 }   # the byte of `ld (hl),$30` ($05FA)
when = [{ counter = "mode", cmp = "==", value = 0 }]

# The level number ($5D8C): the levels cleared plus 1, at F6FA and before it.
[[levels.main.present.board.counters]]
counter = "cleared"
plus = 1
at = [28, 27]
digits = 2
tile = { slot = "level_codes" }
high = { slot = "level_codes", column = 4 }

# The lives ($5D45): one picture for each life besides the one in play,
# leftward from F090, at most 8.
[[levels.main.present.board.repeats]]
counter = "lives"
plus = -1
at = [8, 2]
step = [-1, 0]
most = 8
tile = { slot = "lives_codes", column = 3 }
high = { slot = "lives_codes" }
```

A cell's two bytes are video RAM's, low first: the tile code (bits 0 to 7) and the high byte (tile bits 8 and 9, flip x and y, the colour bank, the priority), as `mcr-screen` describes them. Each of `tile` and `high` is `{ slot, row, column }`, a byte of a `rows` slot (row and column default 0); for a game whose code holds the numbers as immediates, the slot is a `byte-rows` asset whose row is the instruction's bytes.

| Key | Meaning |
|---|---|
| `counter` | Required: the model's counter |
| `plus` | Added to it before it is drawn (default 0): 1 for a level number kept as the levels cleared, -1 for the lives besides the one in play. Below zero draws as zero |
| `at` | Required: `[column, row]` of the first digit's cell (`counters`) or the first picture's (`repeats`), columns 0 to 31 and rows 0 to 29 |
| `digits` | (`counters`) Required, 1 to 10: a number too long for them shows its last digits |
| `tile` | Required: the tile code. For `counters`, digit 0's: digit d's is this plus d, as code that adds the digit to a code; with `table = true`, the first of ten codes in the row, digit d's at `column + d` |
| `table` | (`counters`) `tile` is a table of the ten digits' codes |
| `high` | Required: the cells' high byte |
| `zeros` | (`counters`) Leading zeros drawn as `0`. Without it they are left out, and those cells show the screen's own; the last digit is always drawn |
| `step` | (`repeats`) `[columns, rows]` from one picture to the next, default `[1, 0]` |
| `most` | (`repeats`) Required: the most drawn; cells past the board's edge are never drawn |
| `when` | Counter tests that must all hold for it to be drawn, as a look's |

A counter drawn this way is the model's own: it shows what the model's counter holds, so a game whose hooks don't keep the score yet shows the score its clip started with. This is the arcade form of an NES model's HUD (`present.hud.field`'s `counter`, `digits` and `when`, drawn in a `font`'s characters): an MCR board has no HUD layer of its own, so the digits are cells of the one tile layer, in its codes.

## Strings drawn as tiles

A game's text routine writes a message into video RAM a byte at a time: Tapper's `$0350` writes each byte of a string as a cell's tile code, a space as tile 0, and the colour byte `E38F` holds beside it, up to a zero byte. `texts` (feature `level.board.counters`) draws such a string while counters say so:

```toml
[[levels.main.present.board.texts]]
text = { slot = "attract_text", row = 0 }
at = [6, 2]
high = { slot = "attract_text", row = 4 }
when = [{ counter = "attract_screen", cmp = "==", value = 2 }]
```

| Key | Meaning |
|---|---|
| `text` | Required: where the string starts, a `rows` slot's row read from `column` up to a zero byte or the row's end. Each byte is a cell's tile code, a space (0x20) drawn as tile 0. The bytes are the player's romset's, as every slot's are |
| `at` | Required: `[column, row]` of the first character's cell |
| `high` | Required: the cells' high byte |
| `when` | Counter tests that must all hold for it to be drawn |

A theme names a string's cells `text-SLOT`, after its slot.

## An entity's board sprite

```toml
[entities.mug.board_sprite]
slot = "board_sprites"             # a board-sprites slot
picture = { word = 0, byte = 1 }   # byte 1 of ext[0]: sprite RAM's picture byte
flags = { word = 0 }               # byte 0 of ext[0]: its flags byte

[entities.tip.board_sprite]
slot = "board_sprites"
picture = { number = 0x4D }        # fixed by the model
```

| Key | Meaning |
|---|---|
| `slot` | Required: a `board-sprites` slot |
| `picture` | Required: the entry's picture byte (byte 2), the sprite's number; its ninth bit is bit 3 of the flags |
| `flags` | The entry's flags byte (byte 1), as the board reads it: bit 3 the picture's ninth bit, bits 0 and 1 the colour bank inverted, bit 4 a flip left to right, bit 5 a flip top to bottom. Default 0 |
| `flip_when_facing_left` | Mirrored left to right, on top of the flags' flip, while the body faces left |

### More sprites for one body

A board whose program copies two sprites for one object (a customer and the mug it holds, an object's second half) gives the body's other sprites as `extra` (feature `sprite.board.extra`), each drawn while its picture isn't 0:

```toml
[[entities.customer.board_sprite.extra]]
offset = [16, 0]                     # from the body's position, in its units
picture = { word = 1, byte = 3 }     # the second sprite's picture
flags = { word = 1, byte = 2 }       # and its flags
```

| Key | Meaning |
|---|---|
| `offset` | Its x and y from the body's position, in the position's units. Default `[0, 0]` |
| `picture` | Required: its picture byte. 0 draws nothing |
| `flags` | Its flags byte. Default 0 |
| `flip_when_facing_left` | As the body's own |

Once every body has its own entry, the k-th of a body's other sprites drawn (k from 0) takes the entry k+1 after the body's own, as the program copies it there, so it stacks with its body; when that entry is taken or past 127 it takes the lowest entry still free, in the bodies' order. A sprite is drawn over the entries below its own and under those above. The screen measure puts a box over it down to its body (`gp picture`), as it does the body's own sprite.

Each of `picture` and `flags` is `{ word = W, byte = B }`, byte B (0 the lowest, default 0) of the body's `ext[W]` (W 0 to 3), or `{ number = N }`, N 0 to 255. A state mapping that reads two bytes into one word (`ram = [flags, picture]`) puts the first at byte 0 and the second at byte 1, so one word holds both.

The body's position gives the entry's x and y. With the mapping's `board_units = true` (feature `platform.board_units`) the position is the board's own units, from the screen's top left with y down, and the board draws the 32 by 32 picture with its top left there, each unit two screen pixels: the entry holds the bytes that draw it there, `(x + 3, 241 - y)` (`gp_arcade::video::sprite_bytes`). Without it the position is the sprite RAM's own bytes, as the mapping reads them from the game's object slots (Tapper's model today), the entry holds them as they are, and the board draws the picture with its top left at `(x - 3, 241 - y)` units (`gp_arcade::video::sprite_place`). The two draw the same picture of the same frame, and a theme's picture of a sprite lands in the same place either way. An entity with a `board_sprite` is drawn only by the board; a body of an entity without one is still drawn by tile kind over the board's picture, placed the board's way.

A model that names none of this draws as before: an arcade model's bodies as boxes by tile kind, and the page's REVIEW REBUILD says so beside it.

## Themes

A large-art theme (docs/formats/theme.md, "Large-art rules") changes an arcade game's pictures with the same rules, the same reader and the same painter (`gp_present::art`) an NES game's theme is drawn with. Its rules name the board's pictures by names made from what the model already holds:

| Picture | Name | Size, in the board's picture's pixels |
|---|---|---|
| A screen | its `board-screen` slot, as the level's look or a `looks` entry names it: `screen_saloon` | 512 by 480 |
| A sprite | `sprite-ENTITY-CODE`: the entity with a `board_sprite` and the picture code its entry draws, in hex (0 to 1ff, the ninth bit from the flags): `sprite-mug-4d` | 32 by 32 |
| A counter's digit | `counter-COUNTER-D` for a `counters` entry, D from 0 to 9: `counter-score-7` | 16 by 16 |
| A counter's tile | `counter-COUNTER` for a `repeats` entry: `counter-lives` | 16 by 16 |
| A screen's stage | `SLOT-N` for a screen several of a level's looks show (its colours changing a stage at a time, such as a title lit letter by letter), N its look's place among them from 1, the base look first: `screen_attract_logo-3`. A theme that draws a stage draws it on that look; any other look of the screen takes the screen's own picture | 512 by 480 |
| A text's letter | `letter-C` for a letter or digit a `texts` entry writes (the text's byte is its tile's code): `letter-A`, `letter-7`; any other tile `letter-xNNN`, its code in hex: `letter-x02e`. One picture redraws the letter in every text | 16 by 16 |

A theme centres text lines with `#+ text_center X0 X1 [Y0 Y1]` (each line within columns X0 to X1, and rows Y0 to Y1 when given, centred between the columns by its letters) and words a text its own way with `#+ text_say SLOT WORDS...` (that text's cells drawn as those words in the theme's letters; a space with no picture is drawn by neither). Each drawn cell of a text keeps its text and place (`CellDrawn::text`).

Sizes and places in the rules are the board's picture's pixels (512 by 480), and `scale N` draws the canvas N times that. A screen can also be built from the theme's pieces on its own cells, `#+ room` lines that sort the screen's cells by tile code, colour bank, priority and colours read from the lift (`BoardArt::screen_cells`), so the theme holds no picture or layout of it (docs/formats/theme.md, "Rooms"). A `pic` line draws its art at the picture's place: a sprite's at the rectangle the board draws its entry in, a counter cell's at its cell, a screen's over the whole picture. `timed`, `also`, `held`, `face`, `near`, `elsewhere`, `prop` and `drift` work as on the NES (`also BUILD ART W H DX DY` draws another picture with a body while it shows BUILD, over it), a level being a screen's slot name; `#+ hud_over` changes nothing, since the board has no HUD of its own.

Each frame, the page's `gp_board.wasm` lays the frame out as the board draws it (`gp_look::board::BoardArt::themed`), makes a scene of those names, runs the theme's painter over it once a game frame, and leaves out of the board's own picture what the theme draws: a sprite drawn as art loses its entry of sprite RAM, a digit drawn as art shows the screen's own cell, and a screen drawn as art leaves every pixel no sprite drew clear (a counter's cells still drawn keep their pixels of colour other than 0). The page paints the theme's draws with the board's picture among them, over the theme's screen and under its sprites. A theme that names nothing the frame shows draws the board's picture as it is, pen for pen. A rule naming a picture the board hasn't got is passed over, as on the NES, and rules that name none of the board's pictures are refused as a theme for another game.

The names hold no lifted value: an entity's name and a code (one byte, below `gp template served`'s run of 32) are in the model, and the codes each entity shows are found on the person's own machine. `gp theme look sheet --game GAME` plays the game's recorded inputs (inputs/) on the model with the person's own romset and writes, under `$GP_HOME/work/theme-kits/MODEL/board` or a folder they name outside any repo, `names.toml` (each name the game showed, with how many frames) and a contact sheet of each entity's codes and each screen. Those are pictures of the ROM's game and never go in a repo. `gp theme look check` reads a theme against the board's names, refuses a name the game hasn't got and warns of a sprite code the sheet never saw; `gp theme look new` lists the names to draw.

## Tapper

Tried on a copy of Tapper's package (not the games repo) on 2026-11-14, with the saloon's screen at $6221, the palette at $616D, the intro's pokes ($540A to $5414) and the bars' colours ($5751) as `palette_writes`, and board sprites for the bartender, customers, mugs, taps, the tip and the dancers. On `tapper.l1.001` every play frame's stage is the original's but for the score, the lives and the level number, which the vblank writes into video RAM and the model doesn't keep; the sprites differ where the model's state holds no picture (the bartender and the customers are fixed pictures there, since no word of theirs is free) and where it has no body (the bars' end pieces at E356 to E375).

Tried again on a copy on 2026-11-14 with `looks`, `counters`, `repeats`, a presentation group for the bars' end pieces (docs/formats/state-mapping.md) and the customers' flags and picture read through `strides`: every bar draws its own stage and colours (the stages the setup picks from $61ED by the levels cleared, the bonus round's from $2546, the bar colours from $5751 and the space bar's pokes at $573E), and the score, player 2's score, the lives and the level number are drawn as the vblank draws them ($05D5, $5D45, $5D8C). Frames with the whole background the original's went from 0 to 86 of 336 on `tapper.l1.001` and 132 of 270 on `tapper.die.001`; the rest differ where the model's counters stand still (Tapper's hooks don't keep the score, the lives or the bonus round's counter yet) and where its bodies are drawn otherwise. All 54 clips stay exact.
