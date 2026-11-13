# ROM dataset key

> In gameprinter (2026-11-12): `gp dataset grade DATASET [KEY]` is `crates/gp-cli/src/rom_grade.rs`. gameprinter has no game keys yet: the coverage rows (routines, RAM and immediates the ROM runs, against the dataset's rows) apply with no key, on an NES ROM and on an arcade romset, where the starts are stage 0's arcade starts and the names are the arcade dataset's (`5FAC`). A key for a romset is refused until a Z80 reading of its constants exists. game-printer's Moss Hop key is kept as the grader's test fixture in `crates/gp-cli/tests/keys/mosshop/key.toml`; the `games/keys/` paths below are game-printer's.

A key is what one game's dataset ([the ROM dataset](rom-dataset.md)) should say, written from the game's real model, its state mapping and its code. `gp dataset grade DATASET KEY` (crates/gp-cli/src/rom_grade.rs) grades a dataset against it and prints the stage 0 scorecard. Keys live in `games/keys/<game>/key.toml`. Format version 0.1.0.

The tool lane never reads a key. It runs `gp dataset grade --counts`, which prints counts only: no address, value, name or path from the key.

## Where a key comes from

| Part | Source |
|---|---|
| `[[ram]]` | Each RAM byte the mapping maps (`[[field]]`, `[slots]`, `[cells]`, `[[ignore]]`), with the element, quantity, part and role the code gives it |
| `[[element]]` | The model's entities, cameras, rules and counters that the mapping places in RAM. An element's RAM is the `[[ram]]` rows that name it |
| `[[constant]]` | The model's behaviour numbers, each with the instruction or table entry that holds it where the code has one |
| `[[machine]]`, `[[transition]]` | The state machines the mapping names, as the byte, its values and each change the code writes |

A hand-written model can be wrong. Where the model and the code disagree, the code wins: the key says what the code does and its header comment says what differs from the model.

## Classes

Every fact has a `class`, which says what settles it.

| Class | What settles it |
|---|---|
| `rom` | The ROM alone, by reading or running its code. The fact cites the instruction or table (`src`, `pc`) |
| `trace` | Values from a clip: the code holds no single number for it |
| `name` | A choice the model made, such as a state's name |

Only `rom` constants count in the scorecard's numbers row. Every class is graded in "wrong over every row stated".

## Addresses

An instruction is named as the dataset names it: `C123` on an unbanked board or in a region nothing switches, `03:8123` in a switched one (bank number, then CPU address, hex). RAM addresses are integers.

## Tables

### `[key]`

| Key | Meaning |
|---|---|
| `format` | This format's version |
| `game` | The game's source ID |
| `rom` | The ROM's headerless SHA-256, as the dataset's `rom` column. The grader refuses a dataset of another ROM |
| `rom_file` | Optional: where the ROM is found, relative to the repo, `$GP_ROM_DIR` or `$GP_HOME` (`gp dataset grade` also searches by hash) |
| `model`, `mapping` | The model and mapping it was written from, if any |
| `sources` | What else it was written from: a source file, a disassembly and its pin |

### `[[element]]`

| Key | Meaning |
|---|---|
| `name` | The key's own name for it. A dataset's element is matched to it by shared RAM, never by name |
| `kind` | The dataset's `elements.kind` vocabulary |
| `slots` | For a group of object slots, how many |
| `model` | The model path it stands for |
| `class` | What settles that these bytes belong together |

### `[[ram]]`

One entry per byte, or per run of bytes with the same facts.

| Key | Meaning |
|---|---|
| `addr` | The RAM address |
| `len` | Optional: this many bytes from `addr`, each with these facts |
| `slots` | Optional: this many bytes from `addr`, byte `i` being slot `i` of the element |
| `slot` | Optional: the slot of a single byte |
| `element` | The key element it belongs to; absent if it belongs to none (then a dataset's element for it isn't graded) |
| `field` | The mapping field it is part of (`entities.player[0].x`, `cells`); the scorecard's mapping row counts these |
| `quantity` | The dataset's `ram.quantity` vocabulary; a list when more than one is right |
| `part` | `whole`, `lo`, `hi` or `fraction`; a list when more than one is right (the whole byte of an 8.8 speed is `["whole", "hi"]`) |
| `role` | The dataset's `ram.role` vocabulary; a list when more than one is right |
| `class`, `src`, `note` | As above |

A fact left out isn't graded.

### `[[constant]]`

| Key | Meaning |
|---|---|
| `path` | The model path of the number (`entities.player.jump.gravity`) |
| `value` | The value as the model states it |
| `number` | What a dataset's `number` should be: the operand in its format. Absent when it equals `value` |
| `raw` | The operand as stored, a byte (or a 16-bit number for a two-byte operand) |
| `format` | The dataset's `constants.format` vocabulary |
| `quantity`, `mechanic`, `parameter` | The dataset's vocabularies; `mechanic` is a part of docs/mechanics.md |
| `element` | The key element |
| `pc` | The instructions whose own operand is the whole value, a list, or a table's first byte with `entry` |
| `entry` | A table's entry, 0 for an instruction's operand |
| `part_pc` | Instructions that hold only part of the value (one byte of a 16-bit number). A row there is graded on what it says the constant is, never on its number |
| `class`, `src`, `note` | As above |

A constant with no `pc` (no single operand holds it) is matched to a dataset row by its element (as matched), `mechanic` and `parameter`.

### `[[machine]]`

| Key | Meaning |
|---|---|
| `byte` | The RAM address the state lives in (slot 0's for a slot array) |
| `bytes` | Optional: every address it lives in, for a slot array |
| `element` | The key element |
| `values` | Every value it takes |
| `initial` | Optional: its value at the model's start |
| `complete` | True when `[[transition]]` lists every change the code writes; then a dataset transition not listed is wrong, else it is extra |
| `class`, `note` | As above |

### `[[transition]]`

| Key | Meaning |
|---|---|
| `byte` | The machine's byte |
| `from` | The value before, or a list of them |
| `to` | The value after |
| `pc` | Optional: the instruction that writes it |
| `class`, `src`, `note` | As above |

A change from a value to itself is no transition and is never graded.

## Grading

`gp dataset grade DATASET [KEY] [--rom ROM] [--counts]` prints one column of the scorecard for one ROM.

### Coverage, for any ROM

The grader runs the ROM itself with stage 0's starts (`gp_explore::presses::start_set(4, 1, 3600)`, through gp-events as `gp reach` runs them), so the denominators never come from the dataset.

| Row | Denominator | Counted |
|---|---|---|
| Routines | Routines a run entered (the target of a `jsr`, a jump through a pointer or a pushed address, or an interrupt) | A `routines` row with an element, or an `open` row whose `what` is `routines:NAME` |
| RAM | RAM and PRG RAM bytes a run wrote whose value changed from one frame to the next (internal RAM read every frame, PRG RAM every 15), since a reset that clears RAM writes every byte | A `ram` row with a role, or an `open` row whose `what` is `ram:$XXXX` |
| Constants | Executed `cmp`, `cpx`, `cpy`, `lda`, `ldx` and `ldy` with an immediate operand | A `constants` row at the instruction, or an `open` row whose `what` is `constants:NAME` |

### Keyed rows

Elements are matched first. Each key element's RAM is compared with each dataset element's (the `ram` rows naming it), and the pairs sharing the most bytes are matched one to one, largest first.

| Row | What is counted |
|---|---|
| Mapping fields | A field is right when every byte of it has a `ram` row in the dataset element matched to its key element; wrong when any byte is in another element; open when a byte has no element but a row or an `open` row; missing otherwise |
| Elements | Key elements matched, of all |
| Numbers | Over `rom` constants: right when a row at a `pc` (and `entry`) has the key's number, wrong when one has another number, open when the row has no number or an `open` row names it, missing otherwise |
| Transitions | Key transitions found in the dataset (its `transitions` rows through `states` to byte and values), and dataset transitions on a keyed machine the key lacks (extra; wrong when the machine is complete) |
| Numbers estimated | Over the dataset's `estimates` rows (measured numbers, not known ones), matched to key constants by element (as matched), `mechanic` and `parameter`: close when within 10% of a key number there, within 2 of a number of frames (a `duration` or `period`, or a row in frames), or within the row's own `spread` if larger; far otherwise. An estimate on nothing the key states is unplaced. An estimate is never wrong and never counts in the next row |
| Wrong over every row stated | Each `ram`, `elements`, `constants`, `states` and `transitions` row that lands on something the key states is graded on each column that both the row and the key state (a null column is never graded). Rows that land on nothing the key states are counted as unplaced. The breakdown counts, per `table.column`, the rows graded on it and the wrong ones, and per table the unplaced rows (counts only, also with `--counts`) |

A dataset row on a slot array's later byte counts as slot 0's machine.

One instruction can hold two model facts (one load gives every enemy its hit points). A row at such a place is graded against the fact it fits best, and a number that is another fact's there doesn't count against this one.

Every `[[ram]]` entry states something the grader can grade (an element, a slot, a quantity, a part or a role); an entry that states none is refused.
