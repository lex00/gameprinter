# Labels format

A labels file gives names to the RAM variables, routines and data tables of one game. `gp labels` and `gp roms diff` read it. A game keeps its labels in `labels.toml` in its own folder: Moss Hop's is committed here (`games/mosshop/labels.toml`), and a game in the games checkout keeps one beside its `roms.toml`. A game is named on the command line by its folder's name or by a source ID its `roms.toml` lists, and a ROM hack (a patched source) is read with its base's labels.

A labels file names a game's code and RAM and quotes none of its bytes. One derived from a disassembly is still derived from the game: keep it out of a repo unless its names are your own.

Ported from game-printer, where the Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 files were written under `$GP_HOME/work` from pinned disassemblies. Those games and their extractors are not in gameprinter.

## File

Schema 0 has the top level keys `schema`, `game` (the source's ID), and optionally `disassembly` (the pin, for people) and `bank_size` (bytes per PRG bank, which `bank` counts).

`[[label]]` is a RAM variable in internal RAM ($0000-$07FF):

| Key | Meaning |
|---|---|
| `name` | The name. Several names may share an address |
| `addr` | CPU address |
| `size` | Bytes: the declared size where the source gives one (`.dsb N`), 1 otherwise |
| `description` | One line |

`[[routine]]` and `[[data]]` are labels in PRG ROM, with the same `name` and `description` and:

| Key | Meaning |
|---|---|
| `bank` | `prg` divided by `bank_size` |
| `addr` | The CPU address the label is run or read at |
| `prg` | Offset in the PRG ROM |
| `size` | Bytes up to the next global label in its bank, or the end of the bank |

A label is a routine when the first statement after it is an instruction, and data otherwise. Local labels (`@name`) are not listed. Cartridge RAM ($6000-$7FFF) is not listed.

## Commands

`gp labels show SOURCE [--kind ram|routine|data]` lists a file. `gp labels lookup SOURCE QUERY` takes an address (`$0400`, `0x0400`, decimal) or a name; an address matches every label covering it, RAM by `addr` and `size`, ROM by CPU `addr` in any bank. Both take `--file` for a file of your own. `gp labels check MAPPING` reads a state mapping's `ram` and `frac` addresses and lists those with no RAM label, exiting 1 if any; `just check` runs it on Moss Hop's. `gp roms diff` names the routines and data tables a changed range falls in from the first image's labels.

## Drafting labels from a run (arcade)

`gp labels propose ROM.zip --input LOG [--mapping mapping.toml] [--out FILE]` drafts a labels file from a run of an arcade romset, from gp-arcade's access log and levels on the main Z80, for a person or agent to rename and keep what holds. `[[label]]` is the work RAM the run wrote, adjacent bytes written by the same routines as one variable, named from the mapping's field that reads it (`entities.player[0].y` gives `player_y`) or `ram_XXXX`, described by its writers, its readers and how many frames it changed on (a byte up by one on most frames is called a counter). `[[routine]]` is each routine of the Z80 code map that ran, `sub_XXXX` or `irq_XXXX`, with how often it was entered, its callers and the variables it wrote. `[[data]]` is the ROM read as data, each reader's range with overlapping ranges joined; a reader of more than 1,024 bytes (a ROM check, a copy) stands alone. The draft reads back as a labels file; it goes to `--out` outside any git work tree, or standard output, never over a game's `labels.toml`.

On Tapper's five-level autoplay with games branch tapper-2's mapping (16 s): 735 RAM labels, 186 routines and 36 tables. Every one of the 25 RAM labels written by hand while modelling Tapper has a draft label at its address, and 12 of its 15 hand-named routines are routines of the draft; the other 3 ($06C2, $2B3B, $5EF3) aren't routine starts of the draft's code map: none is the target of a call, restart, interrupt or `jp (..)` in this play.
