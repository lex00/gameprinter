# ROM dataset

What stage 0 extracts from one ROM, as tables a data scientist loads with pandas, DuckDB or Hugging Face `datasets`. `gp dataset rom ROM --out DIR` writes it from the ROM alone: the ROM is run in gp-nes under the recipes the tool makes and its code is read. The same ROM gives the same bytes. Format version 0.1.0; the row schemas are `schema/rom-dataset.json`, made from `TABLES` in `crates/gp-cli/src/rom_dataset.rs`.

## Layout

| File | What it is |
|---|---|
| `data/TABLE.parquet` | One Parquet file a table, every table always present (with no rows if the tool found nothing), one row group (none when it has no rows), uncompressed, rows in key order |
| `README.md` | The dataset card: the Hub's YAML (one config a table), every column with its type, unit and meaning, and example queries |
| `croissant.json` | Croissant 1.0 metadata: a file and a record set a table |
| `dataset.json` | The format, the ROM's hash, and each file's SHA-256 and rows |

The schema is the same for every game. A column only one game needs doesn't go in. The datasets of many ROMs union into one table each and join on `rom`, so a query over `datasets/*/data/constants.parquet` reads the whole library.

## What every row carries

| Column | Meaning |
|---|---|
| `rom` | The ROM's headerless SHA-256 (PRG then CHR) |
| `src` | Where the row came from: `read:BANK:ADDR` for an instruction the reader read (no bank on an unbanked board), `run:RECIPE@FRAME` for a run, `asset-map:ENTRY`, or a JSON list of these |
| `rule` | The rule of the tool that made it |
| `tool` | The tool and its version |

Names the tool makes (`player`, `slots_20`, `state_05`, `s0`) are its own, from the ROM's addresses and values. Naming things as a game's players would is a later choice, by a person or by priors, and is never a column here.

## The sharing line

No table holds bytes of the ROM: no code text, tile data, level data or tables copied out. A number the code compares or loads is a row of `constants`, one value at a time. `gp dataset rom` checks every file it writes for a run of 16 bytes of the ROM (the pre-commit hook's window) and fails on one. Runs that hold little of a ROM are skipped: fewer than four distinct bytes, a short repeat, an even step, or ten or more zero bytes (two small numbers in 64-bit columns look like that). Datasets of commercial games stay under `$GP_HOME/work/` and are never committed or uploaded.

## Known and to refine

A row in every table but `estimates` is known: stated only when the rules settle it exactly, and a wrong one is worse than none. `estimates` holds what the runs measure close but not exactly (an enemy's walking speed, the player's gravity from whole-pixel positions, a timer's length): each row gives its value, how far its samples spread, how many there are, the measurer and what would make it exact. Later stages refine them; a grader scores them close or far, never wrong.

## Unknowns are rows

A wrong row is worse than an open one. Where the tool can't settle a value (a RAM byte's role, a constant's meaning, a bank it can't place), the column is null and the `open` table has a row naming what decides it: the routine, the instruction, the bytes, and what would settle it (`settle_by`: `run`, `trace`, `bank` or `name`).

## Shared vocabularies

| Column | Values |
|---|---|
| `ram.role` | state, counter, timer, pointer, input, sound, scratch, level |
| `ram.quantity` | x, y, speed_x, speed_y, state, timer, facing, kind, flag, count, pointer, buttons, cell, other |
| `ram.part` | whole, lo, hi, fraction |
| `elements.kind` | player, slots, camera, level, hud, game, sound, input, other |
| `constants.quantity` | gravity, jump_speed, max_speed, accel, decel, threshold, period, duration, count, offset, size, limit, kind, mask, other |
| `constants.format` | u8, s8, u16, s16, or fixed point as INT.FRAC bits (`8.8`, `4.4`, `0.8`) |
| `constants.mechanic`, `mechanics.mechanic` | The mechanic parts of docs/mechanics.md (`platformer.jump`) |
| `open.settle_by` | run, trace, bank, name |
| `runs.kind` | idle, start, random, clip, explore |

## Tables

Each table's columns, in the order the Parquet files hold them. A column is null only where its meaning says so; booleans never are.

### `rom`

One row: the ROM and the extraction that made the dataset. Key: rom.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `mapper` | integer |  | The iNES mapper number |
| `board` | text |  | The board as the reader models it (nrom, uxrom, mmc1, mmc3, or another name for one it reads only in part) |
| `prg_bytes` | integer | bytes | PRG ROM size |
| `chr_bytes` | integer | bytes | CHR ROM size (0 for CHR RAM) |
| `asset_map` | text |  | BLAKE3 of the asset map read beside the ROM; null when the ROM alone was read |
| `runs` | integer | count | Runs of the ROM the extraction made (the runs table) |
| `frames` | integer | frames | Frames run, over all runs |
| `format` | text |  | This format's version |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `routines`

One row a routine of the ROM's code: where it is, what calls it, the element it belongs to. Key: rom, routine.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `routine` | text |  | Its entry as `BANK:ADDR` (hex; the bank left out on an unbanked board): the name every other table uses |
| `bank` | integer |  | The PRG bank it is in, null on an unbanked board |
| `addr` | integer |  | Its entry's CPU address |
| `size` | integer | bytes | Bytes of code from its entry to its last instruction |
| `element` | text |  | The element it belongs to (the elements table), null if none |
| `read` | boolean |  | The reader read its instructions |
| `ran` | boolean |  | A run entered it |
| `callers` | text |  | The routines that call or jump to it, a JSON list |
| `callees` | text |  | The routines it calls or jumps to, a JSON list |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `branches`

One row a conditional branch: the bytes its test reads, the value compared, and which ways runs took it. Key: rom, pc.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `pc` | text |  | The branch instruction, `BANK:ADDR` |
| `routine` | text |  | The routine it is in |
| `op` | text |  | Its mnemonic (beq, bne, bcc, ...) |
| `tests` | text |  | The RAM places its flags come from, a JSON list (`$05`, `$20,x`) |
| `compare` | integer |  | The constant the test compares with, null if it compares with none |
| `taken` | integer | count | Times runs branched here; null if no run reached it |
| `not_taken` | integer | count | Times runs fell through here; null if no run reached it |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `ram`

One row a RAM byte the game writes: its role, the element and slot it belongs to, the quantity it holds, and the routines that read and write it. Key: rom, addr.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `addr` | integer |  | The CPU address ($0000 to $07FF, or $6000 to $7FFF for PRG RAM) |
| `role` | text |  | state, counter, timer, pointer, input, sound, scratch or level; null if not known (then an open row says why) |
| `element` | text |  | The element it belongs to, null if none |
| `slot` | integer |  | For an element with slots: the slot it is in, from 0 |
| `quantity` | text |  | What it holds, in a vocabulary shared by every game: x, y, speed_x, speed_y, state, timer, facing, kind, flag, count, pointer, buttons, cell, other; null if not known |
| `part` | text |  | Which byte of a wider value: whole, lo, hi or fraction; null if not known |
| `written` | boolean |  | A run wrote it |
| `readers` | text |  | Routines that read it, a JSON list |
| `writers` | text |  | Routines that write it, a JSON list |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `elements`

One row a thing in the game: the player, a group of object slots, the camera, the level, the HUD, the game's own state. Key: rom, element.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `element` | text |  | Its name, made by the tool from the ROM (`player`, `slots_20`): the name every other table uses |
| `kind` | text |  | player, slots, camera, level, hud, game, sound, input or other |
| `slots` | integer | count | For a group of object slots, how many |
| `routines` | integer | count | Routines that belong to it |
| `ram` | integer | count | RAM bytes that belong to it |
| `tile_kinds` | text |  | The level tile kinds it acts on, a JSON list, null if none |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `states`

One row a state of a state machine: the byte it lives in and the value that means it. Key: rom, machine, value.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `machine` | text |  | The machine's name, made by the tool (`state_05`) |
| `byte` | integer |  | The RAM address that holds the state |
| `element` | text |  | The element the machine belongs to |
| `value` | integer |  | The byte's value in this state |
| `state` | text |  | The state's name, made by the tool (`s0`) |
| `initial` | boolean |  | The state the machine starts in |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `transitions`

One row a change of state: from, to, the instruction that writes it and what must hold. Key: rom, machine, from_state, to_state, pc.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `machine` | text |  | The machine (the states table) |
| `from_state` | text |  | The state before |
| `to_state` | text |  | The state after |
| `pc` | text |  | The instruction that writes the new state, `BANK:ADDR` |
| `routine` | text |  | The routine it is in |
| `guards` | text |  | What must hold on the path to it, a JSON list of conditions (`$12 >= #$C0`) |
| `after` | integer | frames | When the change waits on a timer: the frames it waits |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `constants`

One row a value the code uses, at a compare, a load or a ROM table entry: its raw value, what it is, its fixed-point format and its value as a number. Key: rom, pc, entry.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `pc` | text |  | The instruction, or the table's first byte, `BANK:ADDR` |
| `entry` | integer |  | The entry of a table, 0 for an instruction's own operand |
| `routine` | text |  | The routine that uses it |
| `op` | text |  | The use: cmp, cpx, cpy, lda, ldx, ldy, adc, sbc, and, ora, or table |
| `raw` | integer |  | The value as stored: a byte, or two bytes as one 16-bit number |
| `format` | text |  | How the code reads it: u8, s8, u16, s16, or a fixed-point form as INT.FRAC bits (`8.8`, `4.4`, `0.8`); null if not known |
| `number` | float |  | The value as a number in its format (-3.5); null if the format is not known |
| `quantity` | text |  | What it is, in a vocabulary shared by every game: gravity, jump_speed, max_speed, accel, decel, threshold, period, duration, count, offset, size, limit, kind, mask, other; null if not known (then an open row says why) |
| `element` | text |  | The element it belongs to, null if none |
| `mechanic` | text |  | The mechanic part it is a parameter of (docs/mechanics.md: platformer.jump), null if none |
| `parameter` | text |  | The parameter of that part (gravity), null if none |
| `compared` | text |  | For a compare: the RAM place compared with it (`$12`), null otherwise |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `mechanics`

One row a mechanic part an element uses, with one of its options (docs/mechanics.md). Key: rom, element, mechanic, option.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `element` | text |  | The element |
| `mechanic` | text |  | The mechanic part (platformer.jump) |
| `option` | text |  | The option, or `*` for the part itself |
| `value` | text |  | The option's value as JSON, null for the part itself |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `facts`

One row a fact of the game's model the ROM settles, by model path in the tool's own names. Key: rom, path.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `path` | text |  | The model path (entities.slots_20.walk.speed) |
| `value` | text |  | The value as JSON |
| `number` | float |  | The value as a number, null if it isn't one |
| `element` | text |  | The element it is about, null if none |
| `settled_by` | text |  | read (the code), run (a run), or asset-map |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `open`

One row a question the ROM leaves: what is not known, the routine and bytes that decide it, and what would settle it. Key: rom, what, pc.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `what` | text |  | What is not known: a table and key (`ram:$0507`, `constants:C4CE`) or a model path |
| `element` | text |  | The element it is about, null if none |
| `routine` | text |  | The routine that decides it, null if none is known |
| `pc` | text |  | The instruction that decides it, `BANK:ADDR`; empty if none is known |
| `bytes` | text |  | The RAM places it depends on, a JSON list |
| `why` | text |  | Why the tool can't settle it |
| `settle_by` | text |  | What would settle it: run (a run that reaches the instruction), trace (values from a clip), bank (a bank the reader can't place), name (a choice no code makes) |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `links`

One row a routine's part in an element: the routine that draws it, or one that updates it, found by following the element's sprites back to the code that wrote them. Key: rom, element, routine, link.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `element` | text |  | The element |
| `routine` | text |  | The routine, `BANK:ADDR` |
| `link` | text |  | draws (it writes the element's sprites from its state) or updates (it writes the element's position) |
| `pc` | text |  | For draws: the instruction that writes the sprite's x, `BANK:ADDR` |
| `bytes` | text |  | The RAM bytes involved, a JSON list: for draws those it reads, for updates those it writes |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `estimates`

One row a value measured from the runs, close but not exact: a fact to refine, with how it was measured, how much the samples spread and what would make it exact. Key: rom, element, mechanic, parameter.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `element` | text |  | The element it is about |
| `mechanic` | text |  | The mechanic part (docs/mechanics.md: platformer.jump), or a pattern for an element no part fits |
| `parameter` | text |  | The parameter of that part (gravity) |
| `quantity` | text |  | What it is, in the constants table's vocabulary; null if not one of them |
| `value` | float |  | The estimate: the median of the samples |
| `spread` | float |  | Half the distance between the samples' lowest and highest values; 0 when they agree |
| `samples` | integer | count | Measurements the estimate is made from |
| `unit` | text |  | pixels, pixels a frame, pixels a frame a frame, frames, or count |
| `method` | text |  | The measurer: feel (gp-extract's player and enemy feel), pattern (gp-track's behaviour classes), timer (a byte's count steps) |
| `refine` | text |  | What would make it exact: the code that writes the value, or clips that isolate it |
| `src` | text |  | Where the row came from: `read:BANK:ADDR` (an instruction the reader read; the bank is left out on an unbanked board), `run:RECIPE@FRAME` (a run's recipe hash and frame), `asset-map:ENTRY`, or a JSON list of these |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |

### `runs`

One row a run of the ROM the extraction made: its recipe and what it reached. Key: rom, run.

| Column | Type | Unit | Meaning |
|---|---|---|---|
| `rom` | text |  | The ROM's headerless SHA-256 (PRG then CHR), the same in every table: the column datasets of many ROMs are joined on |
| `run` | text |  | The recipe's hash: the ROM, the start and the presses |
| `kind` | text |  | idle, start, random, clip or explore |
| `seed` | integer |  | The seed of a random or explored run |
| `frames` | integer | frames | Frames run |
| `routines` | integer | count | Routines it entered |
| `prg_bytes` | integer | bytes | PRG bytes it executed |
| `ram_written` | integer | bytes | RAM bytes it wrote |
| `rule` | text |  | The rule of the tool that made the row |
| `tool` | text |  | The tool and its version (`gp 0.1.0`) |
