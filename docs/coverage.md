# Coverage

Brought from game-printer's docs/coverage.md (port wave 4). Changes in gameprinter: no `--metrics`; a game's definition is found by its ROM entry's ID in a game's roms.toml (the old sources/ registry) or a committed baseline's `source`, in `$GP_COVERAGE`, `coverage/` at or above the working folder, or this repo's `coverage/`; recordings of an arcade romset are replayed on gp-arcade and measured by both Z80s' code maps (docs/code-reading.md, "On an arcade romset"), a selection of both kinds gets both reports, and `--rom` an arcade romset measures that set's recordings. The inspector's endpoint is `gp serve`'s (crates/gp-cli/src/serve/coverage.rs).

Coverage says what a recording, or the whole corpus, exercised (ROADMAP.md, Milestone 1, step 9). It tells us when the corpus is big enough and which recordings to make next, and later which routines and levels extraction can rely on. `gp coverage` measures it by replaying recordings, so it works on every recording already in the store.

## The four measures

| Measure | What counts | Read from |
|---|---|---|
| Executed ROM bytes | Every byte of every instruction run from PRG ROM, the opcode and its operands, by offset into PRG ROM. Two banks that share a CPU address count apart. For each 1KB page of PRG ROM the report also lists the CPU addresses it ran at, so a bank shows where it was mapped. Instructions run from RAM are counted (`outside_rom`) but have no ROM offset | The instruction about to run |
| RAM bytes written | Internal RAM ($0000-$07FF, mirrors folded in) and PRG RAM ($6000-$7FFF on boards that have it), by offset | The instruction about to run |
| Screens visited | Per game, from RAM at the end of each frame: a screen name and the level it belongs to, both built from RAM values by the game's definition. Objects seen (enemy types) come from the same definition | RAM after each frame |
| Input combinations | Each distinct set of buttons held on controller 1 (and, separately, controller 2), counted on frames the game read the controllers. Named in input log order, such as `right+b+a`, or `none` | The input log |

A RAM write is worked out from the instruction about to run, with the registers and memory it will see: the address a store or read-modify-write reaches (official and unofficial opcodes alike), and the stack bytes `PHA`, `PHP`, `JSR` and `BRK` push. An NMI or IRQ pushes three bytes just above where the stack pointer ends up. Writes to PPU, APU and mapper registers aren't RAM writes. Bytes read as data aren't executed bytes, and OAM DMA reads RAM rather than writing it. The unstable unofficial stores (`SHA`, `SHX`, `SHY` and `TAS`) are counted at the address they reach without a page cross, even when a page cross sends them elsewhere. A test in game-printer (`crates/gp-coverage/tests/pass.rs`, not brought over) runs a program that writes in every addressing mode, calls a subroutine and takes NMIs, and checks that the bytes reported are exactly those it runs and writes, and that no RAM byte changes value without being reported. `BRK` counts two executed bytes, its opcode and the padding byte the CPU fetches after it. PRG RAM writes count even while a board write-protects PRG RAM.

## How it's measured

`gp-coverage` replays a recording with an observer on `Nes::step_with`, the hook instruction traces use (docs/runtime-protocol.md). The hook reads the console before each instruction and changes nothing but the record of the last interrupt, so the replay reaches the same frame hashes as one without it, and coverage needs no change to the runtime. A recording made by this runtime replays in segments that start at its checkpoints, on several threads at once, and each segment must end on the recorded frame hash. A recording made by another `gp-nes` replays from power-on and is checked every checkpoint interval and at its last frame. Coverage of a replay that doesn't match its recording is refused. Recordings made by Mesen are skipped, since coverage replays in `gp-nes`.

The other route was a hook in the `tetanes-core` fork on its read and write paths. Any fork change makes a new runtime identity, which would make every stored recording stale and its checkpoints unusable until docs/open-questions.md's row on observing-only runtime changes is settled. Both were measured on 2026-10-27 on an 18-core Apple Silicon Mac with `crates/gp-coverage/tests/speed.rs`, which only reads the store. The cost of a fork hook is estimated with `tetanes-core`'s own code map, which marks each instruction inside `clock_instr`, where a fork hook would sit:

| Release build, frames per second | `smb-run-right` (1,800 frames) | `smb-warpless-3728M` (67,116 frames) |
|---|---|---|
| Plain replay from power-on | 1,481 | 1,328 |
| Replay with the coverage observer, one thread | 1,349 | 1,242 |
| Replay with `tetanes-core`'s code map (the fork hook estimate) | 1,508 | 1,296 |
| The whole measure, 4 threads | 4,964 | 3,770 |
| The whole measure, 8 threads | 9,026 | 8,855 |

The observer costs about 9% over plain replay on one thread, and segments on threads make measuring several times faster than replaying. In the debug build `just` uses, plain replay runs at 1,266 frames per second, the observer on one thread at 556 and the measure on 8 threads at 3,891. So the observer was chosen, no fork change was made, and coverage doesn't depend on the observing-only row.

## Commands

| Command | What it does |
|---|---|
| `gp coverage <id>...` | Coverage of the given recordings, combined per ROM |
| `gp coverage --store` | Every recording in the store, combined per ROM: the corpus (retired recordings with `--retired`) |
| `gp coverage check ...` | The same, then checks each game's thresholds. Exits 1 if one isn't reached, 2 if no definition has thresholds |
| `--json` | The report as JSON: `groups` (one per ROM, with `summary`, `check` and the full `coverage`) and `recordings` (each with `measured`, `stored`, `skipped`, `diverged` or `error`) |
| `--rom FILE` | The ROM by path, for a game built from source such as Moss Hop. With `--store`, only recordings of that ROM |
| `--definition FILE` | A definition by path, instead of `coverage/<source-id>.toml` |
| `--no-save` | Store nothing; the store isn't opened for writing |
| `--threads N` | Threads per recording (default every core) |

`gp coverage` exits 1 if a recording's replay diverges and 2 if a recording can't be measured or nothing was. A recording whose ROM isn't in `$GP_ROM_DIR` is skipped, unless its coverage is already stored. Without `--definition`, a registered source's definition is found by its ID, and a game built from source (Moss Hop) by the `source` of a committed baseline that pins the same ROM hash, as the inspector finds labels.

## Stored coverage

Each recording's coverage is a store object, its JSON, and one line in `<store>/index/coverage.jsonl` points at it:

```text
{"recording":"<id>","coverage_version":1,"definition":"<blake3 of the definition file>","object":"<hash>","at":"2026-10-27T00:00:00Z"}
```

A report reads the newest line for each recording with this `COVERAGE_VERSION` and this definition's hash, and measures only recordings without one. Changing a definition file (thresholds and comments included, since the key is the whole file's hash), or a change to how coverage is measured (which bumps `COVERAGE_VERSION` in `crates/gp-coverage/src/lib.rs`), measures them again. The objects can always be measured again from their recordings, so a line whose object is missing is passed over rather than reported.

## Definitions and thresholds

The example below is game-printer's Super Mario Bros. definition; gameprinter keeps Moss Hop's, Kirby's Adventure's and Ninja Gaiden's (other games' were cut in the migration).

A game's definition is `coverage/<source-id>.toml`, found by the ROM's registry ID (or given with `--definition`). It's data, so adding a game needs no code. Selectors are `ram[N]` only, in the selector and condition grammar of docs/runtime-protocol.md. This is the Super Mario Bros. file, shortened.

```toml
schema = 1
source = "smb1-ntsc-prg0"
title = "Super Mario Bros."

[screens]
when = "ram[0x0770] == 1 && ram[0x0772] == 3"       # read only while this holds
parts = [
  { name = "world", read = "ram[0x075f]", add = 1 },
  { name = "level", read = "ram[0x075c]", add = 1 },
  { name = "area", read = "ram[0x0760]" },
  { name = "page", read = "ram[0x006d]" },
]
screen = "{world}-{level} area {area} page {page}"  # a screen's name
level = "{world}-{level}"                           # the level it's in

[objects]
when = "ram[0x0770] == 1 && ram[0x0772] == 3"
slots = [{ id = "ram[0x16]", active = "ram[0x0f]" }]  # an ID, read or fixed, counted while active isn't 0
names = { "0x06" = "Goomba" }                         # IDs without a name show as 0x..

[thresholds]
screens = []                    # screen names that must be visited
levels = ["1-1", "1-2"]         # level names that must be visited
objects = ["Goomba"]            # object names that must be seen
inputs = ["right", "right+b+a"] # controller 1 combinations that must be used
```

| Game | File | Screen | Thresholds |
|---|---|---|---|
| Kirby's Adventure (extra) | `coverage/kirby-usa-prg0.toml` | Level, stage and room (`$0558`, `$0559`, `$055E`) outside hubs (`$059D` below 2), from a work-in-progress disassembly's labels; checked on the start of 3226M | None (an extra game; the 39 stages are listed in a comment) |
| Ninja Gaiden (extra) | `coverage/ninja-gaiden-usa.toml` | Stage (`$6D`) and room (`$6E`) while Ryu has health (`$65`), from Data Crystal; a draft, since no run plays the whole game | None (an extra game; 20 stages listed in a comment) |
| Moss Hop | `coverage/mosshop.toml` | The player's screen (`PLAYER_X_HI`) while playing; the two slugs. Given by path, since the built ROM's hash changes with the game | Both screens, the level, the slug, and the five combinations its baselines use |

How much corpus is enough is measured by coverage, not hours (docs/open-questions.md). A game's corpus is enough when `gp coverage check --store` passes its thresholds. They ask for every level visited and every enemy type seen, with the main input combinations used. The Milestone 1 targets are Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 (step 11); Kirby's Adventure and Ninja Gaiden are extra games without thresholds, so `gp coverage check --store` passes when the three targets do (docs/corpus.md).

## In the check suite

`cargo test` (so `just check`) builds Moss Hop, records every baseline in `baselines/mosshop/` and runs `gp coverage check` on them with `coverage/mosshop.toml` (`crates/gp-cli/tests/detectors.rs`). It must pass the definition's thresholds and reach the floors game-printer kept in budgets.toml, set a little under what the baselines reached on 2026-10-27: 1,150 executed bytes, 2,000 RAM bytes written, 2 screens, 1 level, 1 object and 5 input combinations.
