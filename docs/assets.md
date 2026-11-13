# Asset lifting

Asset lifting decodes a game's tiles, palettes, metatiles, level data and tables from its ROM into files that people and tools can edit, and encodes the files back into ROM bytes. The round-trip check lifts a ROM, encodes the files again and compares the result with the ROM byte for byte: any difference is a bug in a decoder, an encoder or the asset map. It came from game-printer's docs/assets.md. The commands, the general formats and the map came over; the maps and formats of other commercial games, and the commands that found assets in other dumps, did not.

Moss Hop's map is in the tool repo (`assets/mosshop.toml`). A game's maps are in its package folder in the games checkout, one for each ROM entry (`games/donkey-kong/assets/donkey-kong-oe.toml` and `donkey-kong-ju.toml`, `games/tapper/assets/tapper.toml`). The code is `crates/gp-assets`, with the commands in `crates/gp-cli/src/assets.rs`.

## Run it

```sh
gp assets lift target/mosshop/mosshop.nes --map assets/mosshop.toml -o ~/.gameprinter/work/assets/mosshop
gp assets pack ~/.gameprinter/work/assets/mosshop --check target/mosshop/mosshop.nes
```

`just check` builds Moss Hop and runs those two, then `gp model check templates/mosshop --assets` on the lifted files. A game's ROM goes the same way from `$GP_ROM_DIR`, its files to `$GP_HOME/work/assets/<source-id>/`.

| Command | What it does |
|---|---|
| `gp assets lift ROM` | Decodes every asset the ROM's asset map names into a directory, by default `$GP_HOME/work/assets/<source-id>/`. `-o DIR` names another. The map is found by the ROM's hash in `$GP_ASSETS`, else in the tool repo's `assets/` and each game's `assets/` in the games checkout; `--map FILE` gives it instead. A directory that already holds a lift (an `assets.toml`) is refused unless `--force`, since its files may have been edited |
| `gp assets pack DIR --check ROM` | Encodes the lifted files in `DIR` and compares them with the ROM. Exits 1 if any byte differs, listing each one by asset, offset and value. `--metrics PREFIX` adds `PREFIX.bytes_covered` and `PREFIX.differences` to the JSON |
| `gp assets pack DIR --base ROM -o FILE` | Writes a copy of the ROM with the encoded assets in place and lists the bytes that changed. `--force` replaces an existing file |

With `--json` the commands print these fields:

| Command | JSON fields |
|---|---|
| `lift` | `source`, `title`, `sha256` (the ROM's), `map` (the file used), `out`, `bytes_covered`, `files`, and `assets`, each with `name`, `format`, `file` and `bytes` |
| `pack --check` | `ok`, `rom_is_maps`, `source`, `rom`, `dir`, `bytes_covered`, `difference_count` and `differences`, and `metrics` with `--metrics` |
| `pack --base` | `source`, `base`, `out`, `sha256` (the new ROM's), `bytes_changed` and `changes` |

Each difference or change has these fields:

| Field | Meaning |
|---|---|
| `asset` | The asset's name |
| `part` | The asset, or which of its tables |
| `region` | `prg` or `chr` |
| `offset` | Where the byte is in PRG or CHR data |
| `file_offset` | Where the byte is in the ROM file, header included |
| `rom` | The ROM's byte (the base ROM's, for `pack --base`) |
| `encoded` | The byte the lifted files encode to |

Exit codes are the usual ones: 0 when the files match the ROM or were written, 1 when the round trip finds a difference, and 2 when the command can't run. A ROM that no map describes is exit 2, and so is a lifted file that doesn't parse or holds a value out of range.

Both commands refuse to write inside a git work tree, using the same check as `$GP_STORE` and `$GP_ROM_DIR` (`gp_evidence::paths::enclosing_git_work_tree`). They also refuse to write through a symlink, file or directory, which could point into a repo past that check. Assets lifted from a commercial game, and ROMs packed from them, must never be committed. Moss Hop's assets are ours, but they go through the same path, so the check that guards commercial games is the one that runs every day.

Not here yet: `gp assets hash` (writes a map's content section), `gp assets find` (finds a game's assets in another dump of it by content), `gp assets stub` (placeholder assets with each asset's shape and none of its content, which `gp_assets::stub` still makes in code) and `gp assets open`.

## What Moss Hop's map lifts

`assets/mosshop.toml` names five assets, 8,766 bytes of the 24KB ROM:

| Asset | Format | ROM bytes | Lifted file |
|---|---|---|---|
| `tiles` | `chr-2bpp` | CHR 0x0000, 8,192 bytes: the whole pattern table, 512 tiles. Tiles $00 to $27 and $30 to $3C are drawn in `games/mosshop/chr.s`; the rest are empty. Its `charset` names the digits from $30 and the cross at $3B, so the model's HUD font is filled from it | `tiles.png` |
| `palettes` | `palette` | PRG 0x06F9, 32 bytes (`palette`) | `palettes.toml` |
| `metatiles` | `metatiles` | PRG 0x0719, 36 bytes of tile numbers (`metatile_tiles`), and two properties of 9 bytes each, the solid flags (`metatile_solid`) and the kinds (`metatile_kind`) | `metatiles.toml` |
| `level` | `level-grid` | PRG 0x074F, 480 bytes: 32 by 15 metatiles (`level`) | `level.toml` |
| `slugs` | `table` | PRG 0x094D to 0x0954: each slug's patrol start and end, low and high bytes in four tables (`enemy_start_lo` and so on) | `slugs.toml` |

A `chr-2bpp` asset may give a `charset` for a game that draws text from its tiles and keeps no strings. A `font` slot is filled from that or from a `ppu-strings` asset's.

The names in parentheses are the labels in `games/mosshop/mosshop.s`. Code, the level's row pointer tables (`level_rows_lo` and `level_rows_hi`, which only point at the rows) and the interrupt vectors aren't lifted.

The map pins the built ROM's SHA-256, like the baselines. A change to the game's source that moves the data makes `gp assets lift` refuse the new ROM until the map's offsets and hash are updated. Not here yet: game-printer's test that checked every offset in the map against ld65's debug file (`mosshop.dbg`).

## Donkey Kong's maps

`games/donkey-kong/assets/donkey-kong-oe.toml` describes Donkey Kong: Original Edition, a CNROM board: 32KB of PRG in two 16KB halves and 32KB of CHR in four 8KB banks. Its upper half is Donkey Kong (JU)'s whole program with about 24 instructions turned into jumps into the lower half, so every span of JU's map is in it at its offset plus 0x4000. It names 75 assets: the pattern tables of CHR banks 0 and 1 (`chr-2bpp`), palettes, the stage screens, the title and 50m's screen as PPU update strings (`ppu-strings`), and each stage's ground runs, ladders, heights, Kong's poses, the demo script, the sound driver's data and the stretches of code the hooks walk for their cycle costs as `byte-rows` and `table` assets. `donkey-kong-ju.toml` names JU's 57, the same spans less what only Original Edition has. The maps' header comments say what loads from where.

## Tapper's map: an arcade board

An arcade game comes in several romsets that share a board and a layout but not always their art: Tapper's `tapper` and `tapperg` run the same code with different sprites. So an arcade map names its assets by the board's graphics region (MAME's `gfx1`, `gfx2`) and the index of each tile or sprite in it, and lists the sets whose chips are laid out that way:

```toml
schema = 1
platform = "arcade"
source = "tapper"
title = "Tapper"
board = "mcr_91490"
sets = ["rbtapper", "tapper", "tappera", "tapperg", "sutapper"]

[[asset]]
name = "tiles"
format = "mcr-tiles"
region = "gfx1"
first = 0
count = 1024
per_row = 32
```

| Format | Region | What |
|---|---|---|
| `mcr-tiles` | `gfx1` | Background tiles, 8 by 8, 4 bits a pixel. The board draws each pixel twice in both directions, as 16 by 16 |
| `mcr-sprites` | `gfx2` | Sprites, 32 by 32, 4 bits a pixel |

`gp assets lift` reads the regions from whichever listed set the zip is, found by its chips' hashes, so the same model loads `tapper`'s art or `tapperg`'s, and writes each asset as an indexed PNG of 4-bit values with gray previews, by default to `$GP_HOME/work/assets/<set>/`. The lifted folder's `source.toml` says which set it was. `gp assets pack DIR --check ZIP` packs the pictures back into the graphics chips and compares them with the set's, byte for byte. The colours aren't in the chips: the board looks them up in its palette RAM, which the game writes as it runs, so a model does the same.

## The lifted directory

| File | Format |
|---|---|
| `assets.toml` | The asset map, copied unchanged. `pack` reads it to know what each file is and where its bytes go, so a lifted directory packs on its own |
| `<name>.png` | Tiles: an indexed PNG |
| `<name>.toml` | Palettes, metatiles, levels, tables, pointer tables, byte rows, blobs and PPU strings: TOML |

Each file is named after its asset. `lift` refuses a directory that already holds a lift (an `assets.toml`) unless `--force`, since lifting again would lose any edits. With `--force` it replaces files of those names and touches nothing else in the directory.

### Tiles

An NES tile is 8 by 8 pixels in 16 bytes: eight bytes of bit plane 0, one per row with the leftmost pixel in bit 7, then eight bytes of bit plane 1. A pixel's color index, 0 to 3, is plane 0's bit plus twice plane 1's. Which colors those are depends on the palette the game draws the tile with, so the PNG stores only the index.

The PNG is indexed (palette) color with 2 bits per pixel. Tiles run left to right, `tiles_per_row` to a row (16 for Moss Hop, so 128 by 256 pixels for 512 tiles). Its palette is four grays from black (index 0) to white (index 3), for viewing only. Pixels past the last tile, when the count doesn't fill the last row, are index 0.

`pack` reads any indexed PNG with the right size, at any bit depth, since image editors often save 8-bit indexed images. It refuses an image of another color type, an animated PNG, a pixel with an index above 3 and a nonzero pixel past the last tile, naming the pixel. The PNG holds the pixels; the tile count and layout are in `assets.toml`. The image's size is checked from its header before its pixels are read.

### Palettes

```toml
schema = 1
asset = "palettes"
palettes = [
  [0x21, 0x1A, 0x17, 0x37], # background 0
  ...
  [0x21, 0x0F, 0x28, 0x30], # sprites 1
  ...
]
```

Each palette is four NES color numbers. When there are eight, the comments name them as the PPU's palette RAM holds them: four for the background, then four for sprites. Any byte value is kept, so a ROM that stores values above $3F still round-trips.

### Metatiles

```toml
schema = 1
asset = "metatiles"

# 3: stone block, level character "B"
[[metatile]]
tiles = [0x05, 0x06, 0x07, 0x08]
solid = 1
```

One `[[metatile]]` entry per metatile, in order: its four tile numbers (top left, top right, bottom left, bottom right, whatever order the ROM stores them in) and a field per property the map names (`solid` for Moss Hop). The comment gives the metatile's number, its name from the map and, when a level uses this metatile set, its level character.

### Levels

```toml
schema = 1
asset = "level"
# 32 metatiles wide and 15 high. The characters, from the map's legend:
#   .  0 sky
#   M  1 moss over soil
#   ...
rows = [
  "................................", # 0
  ...
  "......B..................B...().", # 11
  "MMMMMMMMMMMM..MMMMMMMMMMMMMMMMMM", # 12
  ...
]
```

One string per row, one character per metatile, with the legend from the map: the character at position N in the legend stands for metatile N. The rows read the same as the `level_row` lines in `mosshop.s`. A level byte the legend doesn't cover can't be lifted (exit 2), and a character that isn't in the legend can't be packed.

### Tables

```toml
schema = 1
asset = "slugs"

# 0
[[row]]
start = 128
end = 176
```

One `[[row]]` entry per row, with a field per column. A column is a byte, or a 16-bit number stored as a table of low bytes and a table of high bytes.

### Pointer tables

```toml
schema = 1
asset = "example_pointers"
pointers = [
  "first", # 0: 0x8123
  ...
]
```

The file has one name per pointer: each is an asset, found by where its data starts. `lift` refuses a pointer that holds no such start, or a start two entries share. `pack` writes each pointer from where its target is. A map may also give `window`, the number of bytes the bank maps from `prg_base`; `pack` then refuses a target past the bank's end, whose address would point into whatever the CPU maps there instead. The comments give the addresses the ROM held.

### Byte rows

```toml
schema = 1
asset = "example"
rows = [
  [0x10, 0x11, 0x20, 0x21], # 0: first
  "HELLO", # 1: second, as text
  ...
]
```

One list of `width` numbers per row. With a `charset` in the map, a row whose bytes all have characters is written as text (`"HELLO"`), and either form packs back. The comments give each row's number and its name from the map. A row of more than 16 numbers is written 16 to a line.

### Blobs

```toml
schema = 1
asset = "example_frames"

[pointers]
entity = [
  "0xFFFF", 0, 1, 1, 2, 3, 4, 5, # 0x00
  ...
]

# 0 (entity 0x01), 6 bytes at 0x8700
[[blob]]
head = [0x02, 0x00]
rows = [
  [0x11, 0x00],
  [0x12, 0x00],
]
```

A `blobs` asset is a span of records of any length and the pointer tables that point into it. The span is cut where any pointer points, and each piece is a blob, listed in storage order with the pointers that name it. A blob's bytes are its `head`, then its `rows` of the map's `width`, then its `bytes`; the file uses the map's `head` and `width` where the blob's length fits them and a plain `bytes` list where it doesn't, and any mix packs back. Each pointer in `[pointers]` is a blob's number, or an address in quotes (`"0xFFFF"`) for one that points outside the span, kept as it is.

`pack` lays the blobs out back to back from the span's start and writes every pointer from where its blob ended up, so a blob may grow or shrink. The blobs must fill the span, or with `fill` in the map, at most fill it, the rest taking the fill byte. Blobs that hold CPU addresses of their own, such as songs whose headers point at their patterns, can't move: with `fixed = true` each blob keeps its `address` in the file, and `pack` refuses a blob whose address isn't where the blobs before it end.

### PPU strings

```toml
schema = 1
asset = "example"

[[string]]
name = "first"
updates = [
  { ppu = 0x2043, text = "HELLO" },
  { ppu = 0x23C0, repeat = 63, tile = 0xAA },
  { ppu = 0x3F00, data = "palette_example" },
  { ppu = 0x2100, vertical = true, tiles = [0x24, 0x77] },
]
```

A PPU update is a two-byte PPU address (high byte first), a control byte and its data. Bit 7 of the control byte steps the address down a column instead of along a row (`vertical`), bit 6 writes one byte `repeat` times, and bits 5-0 are the count, so an update holds at most 63 bytes. A string is updates up to the map's `end` byte. Each update has `ppu` and one of `text` (with a charset), `tiles`, `repeat` with `tile`, or `data`, the name of an asset the map embeds there. `pack` refuses a `ppu` whose high byte is the end byte, since the game would read it as the string's end, and with `end_anywhere` the end byte in any byte of an update. Each embedded asset must be the data of exactly one update, at its offset. Donkey Kong's stage screens are PPU strings, with 25m's and 50m's palettes embedded.

Code finds a string at a fixed address, so `pack` refuses strings that don't take exactly the bytes they take in the ROM.

### Sprite sets

A `sprite-set` asset holds sprite frames as layouts of tiles, each tile placed from the entity's position with its own flips, palette and tiles slot, and the sprite palettes each level showed. `gp sprites capture` writes it into a lifted directory from the game's sprite tables and its recordings, so it holds no ROM bytes of its own: the map entry names no span, `gp assets lift` writes nothing for it (and so never replaces a capture), and `gp assets pack` encodes nothing. The file is in [the sprite set format](formats/sprite-set.md).

Every text file starts with `schema` and `asset`, which must match the map. `pack` reads strictly, and stops with an error naming the file and the entry at a missing or unknown key, the wrong number of entries, or a number out of range. Comments and layout don't matter.

## The asset map

An asset map is a TOML file describing one ROM. It's data, so adding a game whose formats are already known means writing a map, with no code change. `gp game new` writes a first one from what the ROM's header says.

```toml
schema = 1
source = "mosshop"
title = "Moss Hop"
sha256 = "44fcedf5050970a0ae6cf10c76ac80d9628f663416c22adb8432a43b0c953ec4"

[[asset]]
name = "palettes"
format = "palette"
label = "palette"
region = "prg"
offset = 0x06F9
count = 8
```

| Key | Meaning |
|---|---|
| `schema` | 1 |
| `source` | The source ID, as the game's `roms.toml` names it, or the game's own name for one that isn't registered, as for Moss Hop. The default output directory is named after it, so it takes the same characters as an asset name |
| `title` | Optional, one line, for the lifted files' comments |
| `sha256` | The ROM this map describes: the SHA-256 of its PRG and CHR data without the header, as `gp roms scan` prints it. `lift` and `pack --base` refuse any other ROM. `pack --check` compares another ROM all the same (an edited one, say), since a comparison can only find real differences, and says it isn't the map's (`rom_is_maps` is false in its JSON) |
| `[[asset]]` | One per asset. Every asset has a `name` (lowercase letters, digits, `-` and `_`; it's the file name), a `format` and the keys below. No two assets may claim the same byte, and no span may be over 16MB, more than any NES ROM holds |

Offsets count from the start of PRG ROM (`region = "prg"`) or CHR ROM (`region = "chr"`), after the iNES header and any trainer. So a map holds for every dump of a revision, whatever header the dump has. Any span may have a `label`, the symbol the game's source or disassembly gives its data. People read it, and a test can check the map against a debug file with it.

| Format | Keys | Bytes |
|---|---|---|
| `chr-2bpp` | `region`, `offset`, `tiles`, `tiles_per_row` (default 16), optionally `charset` | 16 per tile |
| `palette` | `region`, `offset`, `count` | 4 per palette |
| `metatiles` | `region`, `offset`, `count`, `names` (one per metatile), `property`, a list of columns, and optionally `tile_order` | 4 per metatile, and one per metatile in each property's table |
| `level-grid` | `region`, `offset`, `width`, `height`, `legend` (one character per metatile number: printable ASCII, no space, `"` or `\`, none repeated), and optionally `metatiles`, the metatiles asset whose names the file's comments use | One per cell, row by row |
| `table` | `count` (rows) and `column`, a list of columns | One per row in each column's table, two for a split column |
| `pointers` | `region` (`prg`), `offset`, `count`, `cpu_base` (the CPU address of PRG offset `prg_base`: 0x8000 for 32KB of NROM PRG), optionally `prg_base` (default 0; for a switched bank, the PRG offset mapped at `cpu_base`), `hi_offset` (a split table: low bytes at `offset`, high bytes at `hi_offset`; without it, little-endian words) and `window` (the bytes the bank maps from `prg_base`, 0x4000 for a 16KB bank) | Two per pointer |
| `byte-rows` | `region`, `offset`, `count` (rows), `width` (bytes per row), optionally `names` (one per row) and `charset` | `count` times `width` |
| `blobs` | `region` (`prg`), `offset` and `size` (the blobs' span), `cpu_base` and `prg_base` as for `pointers`, `table` (the pointer tables, each a `name`, `offset`, `count`, and `hi_offset` for a split table or `base`, the CPU address one-byte offsets count from, with `label`, `end_label`, `hi_label` and `hi_end_label`), and optionally `head` and `width` (how the file shows each blob), `fixed`, `fill` and `notes` (one line per blob) | `size`, and each table's bytes |
| `ppu-strings` | `region`, `offset`, `size`, `end` (the end byte, default 0x00), `end_anywhere` (true when the game ends a string at the end byte wherever it falls, so `pack` refuses it in any byte), optionally `charset`, `entry` (each with a `name`, optionally `label`, `note` and `records`, the number of updates of a string with no end byte) and `embed` (each an `asset`, its `offset` and its `size`: an asset whose one span is an update's data inside this one) | `size`, less the embedded assets' bytes |
| `sprite-set` | `name`, optionally `written_by` | None |

An arcade map (`platform = "arcade"`) has its own keys and formats ([above](#tappers-map-an-arcade-board)).

A column (in `property` or `column`) has a `name`, a `region` and an `offset`. A 16-bit number split in two tables adds `hi_offset` for the high bytes. `label` and `hi_label` name the two tables. A labelled span may also say where it ends with `end_label` (and `hi_end_label` for a split table's high bytes). `gp` only checks that it's well formed.

A charset (`charset` in `chr-2bpp`, `byte-rows` and `ppu-strings`) is a list of runs, each a first tile number and the characters from it on: `[[0x00, "0123456789"], [0x2B, "!"]]`. Characters are printable ASCII (a space included) other than `"` and `\`, and no character or tile may be given twice.

A metatiles asset may set `tile_order = "columns"` when the ROM stores each metatile's tiles as top left, bottom left, top right, bottom right; the default is `"rows"`. The lifted file lists them by rows either way.

The formats `byte-rows`, `blobs`, `ppu-strings` and `sprite-set` are format modules. Each is one file in `crates/gp-assets/src/` that implements the `Module` trait (`format.rs`) for everything its format does. A new format is a module of its own, registered with a variant of `Asset` and one line in `Asset::module`.

An original's assets (`authored.toml`) are built into a data image in these same formats, with the map that describes it, by `gp template build` ([templates](templates.md#originals)), so every asset tool works on an original the way it works on a lifted game.

## Content

A map describes one revision at fixed offsets. Its content section says what each span holds, so the spans can be found in another dump of the game: another header, a revision, a region, a re-release that keeps the data in another bank. `gp assets hash ROM` writes it from the person's own ROM, after the map's last asset, from the line `# Content, written by gp assets hash ...` to the end of the file; it writes the section again each time and leaves the rest alone. The section holds hashes, never the bytes: a hash can't be turned back into the data. Donkey Kong's two maps have one.

```toml
[[content]]
part = "sprite_tiles"
region = "chr"
offset = 0x0000
len = 4096
blake3 = "..."            # 64 hex digits
chunks = ["...", ...]     # 8 hex digits for each 16 bytes, 32 to a line

[code]                    # with gp assets hash --code
region = "prg"
len = 16384
chunks = ["...", ...]
```

| Key | Meaning |
|---|---|
| `[[content]]` | One per span, as `Asset::spans` names them (`stage_screens (part 1 of 2)`). `part`, `region`, `offset` and `len` must be a span of the map; the parser refuses one that isn't, so a map edited after hashing is refused until it is hashed again |
| `blake3` | The BLAKE3 of the span's bytes |
| `chunks` | The fingerprint of each 16 bytes of the span, the last one of what is left (`gp_assets::content::fingerprint`, a 32-bit mix fixed forever) |
| `[code]` | The fingerprints of the whole PRG, which `gp assets find` compares a ROM's program with (`asset_map.code`) |

game-printer also wrote a `relative` entry for a `pointers` asset with `targets`, hashed after relocation so a table whose lists moved was still the same table. `targets` named an object-list asset, a format only Super Mario Bros. and its sequel's maps had; those formats were cut, so neither the key nor `relative` is here, and no map in gameprinter or the games checkout needs them.

`gp assets hash --check` exits 1 when the section isn't what the ROM gives, as after an edit to an asset's span.

An arcade map has no content section. A romset's chips are listed with their SHA-1s in gp-arcade's set lists (`romset.rs`), so `gp assets hash` on a zip names the set and writes nothing.

## Finding assets in other dumps

`gp assets find GAME --rom ROM` takes the templates whose `template.game` is GAME (or whose id is GAME): the game folders of the games checkout and the tool repo's `templates/`, leaving out authored ones. It searches the ROM for each span of each one's map by its content (`gp_assets::find`):

1. One pass fingerprints the 16 bytes at every offset of the ROM's PRG and CHR. Each that is one of a span's chunks votes for where the span would start. Both regions are searched for every span, so tiles a CHR RAM release keeps in PRG are found. Chunks of one byte value (fill) don't vote, and a chunk that is in more than 8 places in the spans or 256 in the ROM stops voting.
2. Each span's best starts are checked chunk by chunk. So are its map offset and that offset moved by the shifts most unchanged spans moved by. The span is found unchanged where its BLAKE3 agrees, with small changes where at least half of its bytes are in chunks that agree, and missing otherwise. Spans shorter than 16 bytes are found by their one fingerprint at every offset.
3. Where a span was found is said as its region and offset, and the bank and CPU address the ROM's mapper shows it at where they are fixed (`PRG 0x4014 ($C014)`, `CHR 0x2000 (bank 1)`).

An asset is `same` when every span is. It is `changed` if every span is found and some changed, and `missing` if any isn't found. The model is picked by the program (`find::choose`): the template whose ROM it is; else the one with most of its `[code]` chunks found anywhere in the ROM's PRG, leaving out fill, chunks the map's assets cover and chunks the program has twice; a tie goes to more assets found unchanged.

| Match | When | What it means |
|---|---|---|
| `exact` | The ROM's hash is the map's | The template's own ROM |
| `code` | Every chunk of the program is found | The same code, its data or places changed |
| `close` | Half of the chunks or more | The code is changed in places: the game may not match the ROM's behaviour |
| `none` | Fewer | No template of the game is this ROM |

`--lift DIR` lifts what was found with the chosen map: `find::assemble` makes an image in the map's own layout holding each found span's bytes from the ROM, and lifts the assets whose every span was found, whole groups only (a string asset and the assets it embeds, a level and its metatiles). A ROM no game's roms.toml knows gets a draft `[[source]]` entry to check and paste into the game's roms.toml ([sources.md](sources.md)).

With Donkey Kong's one template (donkey-kong-oe), on 2026-11-13, game-printer's gp and this one gave the same JSON for both of the person's Donkey Kong ROMs: Original Edition is `exact`, and Donkey Kong (JU) is `close` with donkey-kong-oe's assets found at their offsets less 0x4000.

### On an arcade romset

A MAME zip is compared chip by chip. For each set the game's arcade maps list (`sets`) that gp-arcade knows, every chip of the set is looked for among the zip's files by its SHA-1, whatever the file is named: `same` when a file has its hash, `changed` when a file has its name and size but not its hash (the chip's own bytes aren't known here, only its hash, so no count of bytes agreeing), `missing` otherwise. The set is the one with all its chips there; else the one with the most of its main CPU program unchanged, then the most chips. `exact` is a whole set, `code` all of its program chips, `close` half or more of the program's bytes. Each asset of the map is `same`, `changed` or `missing` by the chips of its graphics region (`tiles` by `gfx1`, `sprites` by `gfx2`).

```sh
gp assets find tapper --rom ~/.gameprinter/roms/arcade/tapperg.zip
```

`--lift DIR` lifts through the arcade map, as `gp assets lift --map` does, and only from a whole set, since an arcade lift reads every graphics chip of one set. A zip no roms.toml knows gets a draft `[[source]]` entry whose `sha256` is the identity of the zip's files (the SHA-256 of their SHA-1s sorted, gp_arcade::romset, the board's PROM left out). On tapperg's zip with one sprite chip taken out and another changed, it chose tapperg (`code`: all of its program), with `tiles` the same and `sprites` changed.

## Determinism

The lifted files depend only on the ROM and the map. Text files are written by hand rather than by a TOML serializer, with fixed number formats, so their bytes are fixed. The PNG is written by the pure-Rust `png` crate at a fixed compression setting; `gp-assets` pins it at exactly 0.18.1, and `Cargo.lock` pins its compressors, since another version could compress the same pixels to other bytes. A test in `gp-assets` lifts a made-up ROM and compares the files' digest with a committed one, so a platform or dependency that wrote other bytes fails `cargo test`.

## Tests

| Test | What it checks |
|---|---|
| `crates/gp-assets/src/tests.rs` | On a made-up ROM covering every format: the round trip, the lifted files' digest, one pixel changing exactly that pixel's two plane bytes, index 1 and index 2 each setting only their own plane, one level cell changing one byte, a 16-bit table value changing both halves, 8-bit indexed PNGs, and every refusal (bad maps, overlapping spans, sizes that would overflow, unsafe source names, oversized images, values out of range, unknown keys, characters outside the legend) |
| `crates/gp-assets/src/tests.rs`, the content and find tests | On made-up ROMs: the content section written, checked against the spans and written again unchanged, holding no run of the span's bytes; the source found unchanged at its offsets; a made-up re-release found at its new offsets with the changed chunks named; ROMs of other bytes finding nothing |
| `crates/gp-assets/src/module_tests.rs` | The format modules on a made-up ROM: the round trip of PPU strings (text, tiles, repeats, a vertical update, an embedded palette, a string with no end byte) and byte rows (lists and text); an edit changing exactly its bytes; and refusals (strings that change size, an embed out of place or written as tiles, an address that starts with the end byte, the end byte anywhere with `end_anywhere`, unknown keys) |
| `crates/gp-cli/src/assets.rs`'s tests | A write through a symlink refused |
| `crates/gp-cli/src/assets_find.rs`'s tests | On a romset's files: a chip under its name with other bytes is changed, one not there is missing, and the region states follow |
| `crates/gp-cli/src/assets_open.rs`'s tests | An editor named by its variable or on the PATH is found, and nothing is fetched |
| `just check` | The round trip on the Moss Hop ROM the check builds |

## Decisions

| Question | Decision |
|---|---|
| How are assets located in a ROM? | By an asset map kept as data with offsets into PRG and CHR data, rather than read from a debug file or labels at run time. A commercial ROM has no debug file, so only a map works for every game |
| Which ROM does a map describe? | One revision, pinned by its headerless SHA-256. Without the pin, a map applied to another revision would still round-trip (bytes decode and encode back whatever they mean), so the check would pass on wrong data |
| What file formats? | Tiles as indexed PNG with the color index per pixel, the rest as hand-written TOML. The map's copy in the directory, `assets.toml`, holds what a PNG can't (tile count and layout), so there's no separate sidecar |
| Where do lifted files go? | Outside any git work tree, by default `$GP_HOME/work/assets/<source-id>/`, for Moss Hop as for commercial games |
| May `lift` replace a lifted directory? | Only with `--force`. A directory with an `assets.toml` holds an earlier lift whose files may have been edited |
| How are new formats added? | As format modules: one file each behind the `Module` trait, registered in two lines, so formats for other games don't touch the shared code |
| How does an arcade game's map differ? | It names pictures by graphics region and index, not by offsets in one ROM, and lists every set laid out that way, so one model draws with whichever set's art a player has |
