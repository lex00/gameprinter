# Reading a ROM's code

> Brought from game-printer with wave 3b of the port. In gameprinter, `gp code listing`, `gp code tables`, `gp reach`, `gp roms diff --cpu` and `gp template cites` are all here. A game's template is its package folder in the games checkout, so `templates/donkey-kong-ju` below is `games/donkey-kong` now. The section "On Donkey Kong, against the planners' numbers" is game-printer's record of 2026-11-05 and is kept as it was. Wave 6 brings them to an arcade romset's two Z80s: see "On an arcade romset" at the end.


These commands do the reading the Donkey Kong level planners did by hand for 50m, 75m and 100m (workers lv50, lv75, lv100, m50, m75, m100 and ride, 2026-11-04 and 05). Each of their cost logs named a scratch script or a step read off bytes that one of these replaces. They read the ROM and runs of it in gp-nes; none needs labels or source, and none writes code into the repo.

| Command | What it gives | Replaces |
|---|---|---|
| `gp code listing ROM [--clips SET] [--only ID] [--frames A..B]` | Each routine of the code map with how often each instruction ran and how often each branch was taken and fell through | A scratch 6502 disassembler and an aggregator over `gp reach --detail` |
| `gp reach ROM --clip SET --window A..B` | The same counts, by routine: instructions run, branches both ways, one way (which way), not run | The planners' per-routine tables (`routine-table.py`, `cover.py`, `merge.py`) |
| `gp reach ... --diff-clip SET`, `--diff-only ID`, `--diff-window A..B`, `--diff-reach FILE` | What one window or clip set ran that another didn't: routines, instructions and branch ways | `reachdiff.py` and the join of a trace with a reach |
| `gp roms diff A B --cpu C000-FFFF [--bank-a N] [--bank-b N]` | Two images compared as the CPU sees an address range, each through its own board | PRG banks cut out by hand to diff a 16 KB program against a 32 KB one's upper half |
| `gp code tables ROM --index $53` | Every ROM table the code reads with that byte as its index: rows, row size, readers, the row each value reads, where pointers point, an asset map entry for each | Pointer tables decoded by hand from `LDA $53; ASL; TAX; LDA table,X` |
| `gp template cites TEMPLATE...` | Every ROM address a template's model, mapping, probes, delta and hooks cite, with file and line and the routine it is in | `cited.py` and grepping `$XXXX` |

## The code map

Every command names routines by the static harvest's code map read from the ROM alone, as `gp model static --no-run` reads it (docs/plan-extraction-process.md, stage 0): on Donkey Kong (JU) that is 249 routines. Both sides of a comparison use the one map, so routines compare by name. Code a run ran that no routine of the map holds is listed as "outside the code map".

## Windows and clips

A run is a clip of a clip set (docs/formats/clips.md) from power-on. `--only` picks clips by ID, or by a prefix ending in `*` (`--only 'dk.m50-*'`); a pattern that picks nothing is an error. A window numbers frames from power-on as `gp reach --detail` does: `2442..5233` is frames 2442 to 5232, `2442..=5232` the same, `3052..` to each clip's end, and `..` the whole clip. Frames before the window run unwatched.

The counts come from `gp_events::profile`: a host observer on gp-nes that counts each instruction by PRG offset and reads each branch's way from the next instruction (the address an NMI pushed, when one came between). A branch that is the window's last instruction counts as run but neither way. The store keeps each clip's counts by the ROM, the run's input and the window (`$GP_HOME/work/events`, or `--store`), so a window is counted once; `--no-store` counts again.

## gp code listing

```text
gp code listing ROM --clips clips.toml --only held --frames 8..
```

Each routine is a header (name, element, addresses, instructions, how many ran, how often its first instruction ran), then a line an instruction: address, instruction, count, and for a branch `taken N, fell M` and the way in words (`both`, `taken only`, `fell only`). An instruction no counted frame ran shows `-`. Without clips it prints the code alone. With clips it lists the routines that ran; `--all` lists every routine, `--routine ADDR` the ones holding those addresses. The last line counts routines, instructions and branches.

A listing is a disassembly of the ROM, so it is never kept in the repo (the repo never holds a ROM or anything taken from one). It goes to standard output, or with `--write FILE` to a file, which must be outside any git work tree (use `$GP_WORKER_OUT` or `$GP_HOME/work`).

## gp reach by window and the diff

`--window` (or any `--diff-*` option) turns `gp reach` from its levels L0 and L1 into a report by routine: one line a routine that ran, with its instructions run of its size, its branches run both ways, its branches not run, and each one-way branch with the way it went. `--cites TEMPLATE` (repeatable) adds the number of addresses those templates cite in each routine and how many routines hold one. Stage 0's starts run only when the side names no clip set; give `--clip` for clips.

The other side (B) is one of:

| Option | B |
|---|---|
| `--diff-clip SET` | Those sets' clips, whole unless `--diff-window` |
| `--diff-only ID`, `--diff-window A..B` alone | A's sets with this pick and window: two windows of one clip, or two clips of one set |
| `--diff-reach FILE` | A `gp reach --write` output (reach.json): its instruction starts and branch edges, so a corpus reached once is compared without running it again. It has no counts |

For each side the report says how many routines it ran, how many of them the other side also ran with no instruction it didn't, how many have instructions only this side ran (with those instructions as address ranges), how many the other side never ran, and the branch ways this side took that the other didn't in instructions both ran.

## gp roms diff --cpu

`gp roms diff` compares PRG by file offset. Two versions whose programs sit at different offsets (Donkey Kong (JU)'s 16 KB program is Original Edition's upper 16 KB) differ almost everywhere that way: 32632 bytes in 2 ranges. When the PRG sizes differ, the output says so and names `--cpu`. `--cpu LO-HI` reads each image as its CPU sees LO to HI, through its own board (`gp_nes::codemap::Board`), and names ranges by CPU address. On a banked board (UxROM and MMC1 at $8000, MMC3 at $8000 and $A000), `--bank-a N` and `--bank-b N` choose the bank its switched region holds; by default it is what the CPU sees at power-on. CHR isn't compared in this mode. Asset and label names come from the first image's PRG offset of each range.

## gp code tables

```text
gp code tables ROM --index '$53' [--values 1..=4] [--name stage]
```

From every load of the index byte, the reader follows each path through register moves, `INX`/`DEX`, `ASL A`, `ADC`/`SBC` with a known carry, unconditional jumps and calls (3 deep), and records each `table,X` or `table,Y` read in ROM whose index is a line in the byte: `2*$53 - 2`. Reads of one stride whose rows start within a stride of each other are columns of one table. A table starts at the row of the least value whose index is 0 to 255, and ends where the next table found starts; a value whose row lies past it is marked so. A table of 2-byte rows whose two bytes are both read and whose every row holds a ROM address is a pointer table, and each place pointed at is listed with the bytes to the next address listed (another target or table), which bounds its span.

The values default to the smallest to the largest constant the code compares the byte with or stores in it; `--values` gives them. The output ends with an `[[asset]]` entry for each table (`pointers`, `table` or `byte-rows`, with the PRG offset of the first row inside and the count of rows inside), named `--name` and the address, to rename and add to the asset map.

## gp template cites

```text
gp template cites templates/donkey-kong-ju templates/donkey-kong-oe
```

Indexes every address of $8000 or more written `$C46B` or `0xC46B` in each template's model.toml, mapping.toml, probes.toml and delta.toml and in its hooks (the `[ext] crate`'s sources, or for a derived template its base's), each with its files and lines, and places it in a routine of the code map of the ROM (`--rom`, else the one in `$GP_ROM_DIR` the first template accepts). Addresses in no routine are tables and data. A base's hooks given twice (Donkey Kong and Original Edition) count once.

A template is a folder, a template id under `templates/` or a game in the games checkout (`gp template cites donkey-kong`). On an arcade romset (`gp template cites tapper`) the main CPU's program starts at 0000, so every 4-digit address below its end (DFFF on Tapper) counts, and RAM above it doesn't; the addresses are placed in the Z80 code map of "On an arcade romset" below, seeded from an idle run of 1800 frames from power-on, a routine running from its entry to the next and holding an address one of its instructions covers. A routine is named `Z80_` and its address, with why it is one (called, restart, interrupt handler, `jp (..)` target).

## On Donkey Kong, against the planners' numbers

Each row is a number a planner found by hand and what the tool gives, run on 2026-11-05 on m3/stage0-coverage at bd6c466f.

| Planner (worker) | By hand | Tool | Command |
|---|---|---|---|
| lv100: routines TAS 5658S ran on 100m (frames 2442 to 5232) | 141 | 141 (2759 instructions, 31 outside the code map) | `gp code listing JU --clips lv100/clipset/clips.toml --frames 2442..=5232` |
| lv100: of those, against the 25m corpus's reach (179 runs) | 126 shared, 12 with 100m-only paths, 3 new ($E73C, $E770, $EF38), 193 instructions | 126, 12 (128 instructions), 3 new (65 instructions): 193 in all, the same three routines, $EE79 60 and $E538 24 instructions | `gp reach JU --clip lv100/clipset/clips.toml --window 2442..=5232 --diff-reach lv100/reach-25m/reach.json` |
| lv100: of the 141, routines holding an address the JU and OE templates cite (at 52f61f73) | 94 | 98; 94 counting `$XXXX` only, as lv100's script did. The other 4 are cited as `0xXXXX` in the hooks | `... --cites t/donkey-kong-ju --cites t/donkey-kong-oe` |
| lv100: JU against OE's upper bank | 104 bytes in 35 ranges | 104 bytes in 35 ranges, named by the asset map (`play_tables`, `code_c500`) | `gp roms diff JU OE --cpu C000-FFFF --summary` |
| m75: 75m's stage-indexed pointer tables | $C46B, $C473, $C47B, $C483, $C489, $C493, $C49B, $C4A1 | The same, with $C48B where the log has $C489 ($C489 is $C483's row for $53 = 4), and $C4A7; $C49B and $C4A1 indexed by `2*$53 - 4`. 18 tables in all, 9 of pointers | `gp code tables JU --index '$53'` |
| m50 (inference log, 2026-11-05): OE's $C46B row for 50m | $8260 | $8260, and $C473's row is $8276 | `gp code tables OE --index '$53'` |
| lv50: routines the six m50 clips ran on 50m's frames (2940 frames) | 174 | 173, with 1 instruction outside the code map (lv50's map was seeded by the clips' runs) | `gp reach OE --clip lv50/m50clips/clips.toml --window 3052..` |

lv75's 118 routines on 75m (frames 1611 to 2440) isn't reproduced: the tool gives 127 for that window, and lv75's aggregate (agg-ju-75m.json, frames 1560 to 2440) holds 131 routines by its own map, so its count came from a step its log doesn't give.

The code is `crates/gp-cli/src/code.rs`, `reach_window.rs`, `template_cites.rs`, `roms_diff` in `main.rs`, and `crates/gp-events/src/profile.rs`; the tests are `crates/gp-cli/tests/code_reading.rs`, on made-up ROMs.


## On an arcade romset (Z80)

Given a romset zip (`gp code listing tapper.zip --clips games/tapper/corpus.toml`), `gp code listing`, `gp code tables` and `gp reach` read both of the board's Z80s through gp-arcade's disassembler (`crates/gp-arcade/src/dasm.rs`, written from Zilog's manual and the x/y/z opcode decomposition; it is checked against the CPUs themselves on Tapper, every instruction followed by the one its length and flow say) and the levels a watched run keeps (`crates/gp-arcade/src/levels.rs`: instruction starts, bytes executed, bytes written with their writers, ports, and every transfer of control by kind). `crates/gp-cli/src/code_z80.rs` has the rest:

- **The code map** is a recursive descent from each CPU's reset at 0000, and from every instruction a run started, every interrupt handler and every target a `jp (hl)`, `jp (ix)` or `jp (iy)` went to in a run, which reading the code alone can't find. A routine is a call's or restart's target, an interrupt handler, a `jp (..)` target or the reset. The main CPU sees its ROM at 0000-DFFF with no banking (the MCR board has no bank switch), the sound CPU at 0000-3FFF, so an address is a place in one chip, and each routine's header names its chip.
- **`gp code listing`** prints each routine that ran (all with `--all`, or those of `--routine ADDR`) with each instruction's count, each conditional's taken and fell counts, and where each `jp (hl)` went. `--write` refuses a git work tree, as for the NES: the listing is the ROM's code.
- **`gp code tables --index ADDR --input LOG`** runs the log and follows each read of the index byte through the registers for 32 instructions; a ROM read whose address register it reached is a table read. Each table is listed with its range, its entry size (the smallest step between the first addresses two values read) and the addresses each value read, with an asset map entry.
- **`gp reach`** runs the clips from power-on, after stage 0's seeded starts, which on a romset are gp-explore's arcade palette's (`idle`; `start`, a coin at frame 120 and Start at 180, again at 600; `rand1` on, the same and then the stick and tap held 8 to 60 frames at a time; `--no-starts` leaves them out) and reports each chip's bytes executed, the instructions run against the code map, the edges by kind, every `jp (..)` with its targets, the ports and the RAM written. `--detail`, `--frames` and `--watch` are the NES's reports still and are refused for a romset.
- **`gp reach --window` and `--diff-*`** give the report by routine (crates/gp-cli/src/reach_window_z80.rs): one code map per CPU, from both sides' runs, with routines named `main:5FAC` and `sound:0276` and the element column saying why each is one (called, restart, interrupt handler, `jp (..)` target). A conditional's ways are its edges (a jump, call, return or `djnz` loop taken, or a fall). Every window is counted now; there is no store of arcade windows. `--diff-reach` (an arcade `gp reach --write` holds counts by chip, not instruction starts) and `--cites` (the templates' cite index reads addresses from $8000 up) stay the NES's.
- **`gp template cites`** places a template's cited addresses in the main CPU's routines ("gp template cites" above).
- **`gp coverage`** of arcade recordings replays each on gp-arcade and gives, per CPU, the code map's instructions run and its conditional branches run both ways and one way. gp-coverage's definitions (state regions, mechanics) are the NES's and aren't read.

`--clips` takes a game's `corpus.toml` as well as a clip set: its clips are the corpus's recipes.
- **`gp model static`** on a romset reads the main Z80's code alone, with nothing run (`crates/gp-arcade/src/harvest/`, the port of gp-nes's harvest; `crates/gp-cli/src/model_static_z80.rs`). It is an abstract interpretation from the reset and from each interrupt handler, every call followed in its caller's state, and it gives the NES harvest's output (static.json: routines, dispatch, elements, machines, facts, open, constants). What it adds for the Z80:
  - IM 2 through the CTC: `ld i,a` and the vector written to the CTC's channel 0 give the vector table; the word for each channel is a handler, read on the next pass (Tapper: I = 0, vector D0, handlers 0144, 0158, 0584 and 00D8).
  - A place an interrupt handler writes is read as what it holds now, never as what the path last wrote, so a wait for the frame counter (`ld a,($e16c); cp 1; jr c`) ends.
  - 16-bit pairs and their halves: `add a,l; ld l,a; ret nc; inc h` is HL plus A; `ld a,(hl); inc hl; ld h,(hl); ld l,a; jp (hl)` with HL a table plus a doubled byte is a jump table by that byte, bounded by the values the code writes to it.
  - Record tables: a pair a loop steps by a constant (`add ix,de` with DE = 8) is a walk through records from its first value; `(ix+d)` in it is a field of the table, and a byte of a record named by its address is the same field. The table's first record, stride and djnz count are facts; each field's role (state, timer, counter, position) is a mapping row.
  - The random routine: a byte written from the R register and its own last value; a routine that only calls it and compares is a wrapper (random below a bound); every call outside them is listed.
  - The alternate registers (`exx`, `ex af,af'`), `push`/`pop` and `push hl; ret`, `ldir`, `in` (input bytes) and `out` are read as the Z80 does them.

  It doesn't yet run clips (`--clips`), report where it stopped (`--unread`) or read the sound CPU; a jump through a pointer in RAM (Tapper's timed callbacks at E417 + 6k) is an open fact. On Tapper it reads 237 routines in about two minutes on a debug build (five passes).
