# gp roms diff

`gp roms diff A B` compares the PRG and CHR of two NES images, each range of differing bytes with its address and the bytes on each side. Either side may be a raw PRG binary, such as a disassembly build, which is compared as PRG only. Runs of differing bytes closer than `--gap` (8 by default) merge into one range; `--limit` (40) caps the ranges shown per section.

It is step 2 of bringing in a game with several revisions: compare them, and choose one as the target. A disassembly build is checked the same way: it should give no difference against the ROM it was made from.

Ported from game-printer (its main.rs `roms_diff` and romdiff.rs, and the `--cpu` part of docs/code-reading.md).

## What each range is

The first image's game is found by its hash in the games checkout's `roms.toml` files (`--games`, else `$GP_GAMES_DIR`, else a `games` folder at or beside the current folder). Its `package.toml` entry for that ROM names its asset map, and its `labels.toml` names its routines and data tables (docs/formats/labels.md; `--labels FILE` gives another). Each range is then named by the assets it falls in and the routines and data it overlaps, and given a kind: graphics, level data, text, table, code, data or unknown. A CHR range is graphics. The counts by kind, the assets touched and the routines touched end the output.

`--summary` prints the names and counts without any bytes, so the output can be quoted. Without it the output shows ROM bytes: keep it on your machine.

## --cpu

`gp roms diff` compares PRG by file offset. Two versions whose programs sit at different offsets differ almost everywhere that way: Donkey Kong (JU)'s 16 KB program is Original Edition's upper 16 KB, and by offset the two differ in 32632 bytes in 2 ranges. When the PRG sizes differ, the output says so and names `--cpu`. `--cpu LO-HI` reads each image as its CPU sees LO to HI, through its own board (`gp_nes::codemap::Board`), and names ranges by CPU address. On a banked board (UxROM and MMC1 at $8000, MMC3 at $8000 and $A000), `--bank-a N` and `--bank-b N` choose the bank its switched region holds; by default it is what the CPU sees at power-on, and a bank asked of an unbanked board is refused. CHR isn't compared in this mode. Asset and label names come from the first image's PRG offset of each range.

| Compared | Found |
|---|---|
| `gp roms diff JU OE --summary` | 32632 bytes in 2 PRG ranges (the sizes differ), CHR 24576 bytes in 1 range |
| `gp roms diff JU OE --cpu C000-FFFF --summary` | 104 bytes in 35 ranges, all tables, in `play_tables`, `play_tables_c2f0`, `code_c500`, `code_c74e` and `sound_rom` |

## Arcade

Two arcade romsets (MAME zips) are compared chip by chip (port wave 6), from gp-arcade's chip table: each region of the first set (the main program, the sound program, the tiles, the sprites) by its chips, each paired with the second set's chip at the same place, said identical (and "the same chip" when its SHA-1 is) or with its differing ranges, by offset in the region and, for the programs, by CPU address (`main $1234`). `--summary` leaves the bytes out; `--limit` caps the ranges per chip. `--cpu`, `--labels` and the asset map are the NES's and refused for romsets; an NES image against a romset is refused.

```text
gp roms diff tapper.zip tapperg.zip --summary
```

On the Tapper sets (2026-11-12): tapper against tapperg, 10 chips identical (both programs and the tiles, all the same chips) and the 8 sprite chips differ, 622 to 2,822 bytes each, so tapperg is tapper with other sprites. Against tappera (1/12/84), the sound program and all the graphics are the same chips and the four main program chips differ (293 to 7,775 bytes), an older revision of the game code. Against rbtapper and sutapper every chip differs, the sound program included.
