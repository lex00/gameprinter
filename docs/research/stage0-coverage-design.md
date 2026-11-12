# Stage 0 from the running ROM: an audit and a design

> game-printer's research record, kept as it was written. In gameprinter the run observer is gp-events (`gp reach`, wave 3a and 3b), the static harvest is `gp model static` (wave 3b), `gp element inputs` takes a game's package folder for `--run`, and capture runs come back with wave 7.


A snapshot of 2026-10-31, on branch `m3/stage0-coverage` (from `m3/static-ram`, commit ce7e47d1). It audits the extraction tooling and proposes a stage 0 that finds the code by running the ROM in gp-nes, driven by button presses as clip generation drives it, and reads that code for behaviour with the static rules. Nothing here is built yet; the design waits for the user's agreement.

The user, 2026-10-31: "identifying the code is more about coverage than reverse engineering it, we are just trying to identify behaviors"; "this process was working with a ROM in our emulator and live state, there's no reason it can't be"; "the clip generation has a way to drive the rom with button presses and those get recorded as a clip recipe; we should be using the button driving to help with extraction too". During the audit: "stay focused on the live extraction loop that loads the rom into our emulator, going beyond static inferrence"; "we have test roms to validate the machinery with before moving on to big famous roms"; "our emulator can record lots of metrics, and you should fill in gaps where it cant for this process".

## Summary

| Question | Answer |
|---|---|
| What the static rules reach | Everything on Moss Hop, its variants and the practice variants: every byte a run executes is in the code map. On the four banked games, little: the code map is 2,238 to 14,500 bytes, and 38 to 45 routines are read |
| What cheap runs reach | Six scripts of 3,600 frames each (idle, Start presses, four seeded random scripts), about 20 s a game, execute 10,270 to 27,615 PRG bytes. On Mega Man 2, Kirby's Adventure and SMB2 that is more than the whole static code map; 2,054 to 25,572 executed bytes are code the map doesn't have |
| Where the static walk misses code a run shows | Pointers filled at run time (`jmp ($000E)` in 48 of the 68 test ROMs, up to 20 targets at one site), board modes the static board doesn't model (MMC1 in 32 KB mode: 42 bytes mapped, 8,526 run), and banks chosen by RAM. Runs need none of these rules to find the code |
| What the emulator records today | Executed PRG bytes and instruction starts by bank (`gp coverage`), routines entered by JSR and interrupt (`gp_coverage::activity`), state per frame and per instruction (`gp trace`), raw mapper writes and interrupts (`gp events`). Missing for this process: indirect and `rts` jump targets with the pointer values, the bank mapped at each entry, and a command that gives them from a script without a capture run |
| What the steering tools need first | A bank-aware key. The coverage map the search, the dense pick and the code matrix use (`gp_nes::coverage`) is keyed by CPU address, so two banks' code at one address counts once. They also need a capture run (a model with a player, a mapping, generated starts), which none of the four games has |
| Is it reproducible | Yes. The same ROM and scripts give the same run output (two runs, the same hash), and the same ROM gives a byte-identical `static.json` |
| The design | Run recipes (presses from power-on) -> traces -> the code map seeded with the executed (bank, pc), entries, jump targets, banks and pointer values -> the static rules read that code -> the unread report's stops become steering targets for the next runs. Six steps, each with an exit test, the first on the test ROMs and Moss Hop |

## How the numbers were made

Every number is from a command run in this session on this machine (Apple Silicon, release build of ce7e47d1, `cargo build --release -p gp-cli`). Two tools were used.

| Tool | What it is |
|---|---|
| `target/release/gp` | The repo's CLI at ce7e47d1 |
| `runcov` | A scratch measuring program written for this audit and not committed (its source is kept at /tmp/stage0-coverage-runcov.rs): put at `crates/gp-coverage/examples/runcov.rs` and built with `cargo build --release -p gp-coverage --example runcov` and moved out of the tree afterwards. It powers on gp-nes with `Nes::power_on`, plays an input script through `Nes::step_with` and, before each instruction, notes its PRG offset (through `gp_coverage::pass::Layout::rom_offset`, so the bank is exact) and where control entered: after a `jsr`, an interrupt, a `jmp (ptr)` (with the pointer's address and value) and an `rts` to an address not just after a `jsr`. It then compares the union with the `static.json` and `unread.json` that `gp model static ROM --unread --write DIR` wrote. Step 1 below makes it a command |

`runcov ROM DIR OUT SCRIPT...` takes scripts as `idle:N` (N frames, no input), `spans:N:SPANS` (the spans `gp input-log generate --hold` takes) and `rand:SEED:N` (Start at frames 60 and 200, then holds of 8 to 60 frames of a random button set chosen by a seeded xorshift, a fifth of them nothing). Each script runs from power-on with zeroed RAM.

The scripts named below:

| Name | Scripts |
|---|---|
| idle | `idle:3600` |
| start | `spans:3600:start@120-124,start@300-304,start@480-484,start@660-664` |
| cheap | idle, start, `rand:1:3600`, `rand:2:3600`, `rand:3:3600`, `rand:4:3600`: 21,600 frames |
| Moss Hop's baselines | The holds of the eight files in `baselines/mosshop/`, each as `spans:FRAMES:HOLDS` |

"Entries" below are the targets of `jsr`, interrupts, `jmp (ptr)` and `rts` used as a jump. The places an `rti` returns to are left out, since an interrupt returns into the middle of any code.

## 1. Inventory: what each tool gives

Measured on Moss Hop (`target/mosshop/mosshop.nes`, built by `just mosshop`) and on Ninja Gaiden (MMC1, 128 KB). Recordings went to a scratch store (`GP_STORE` in the session's scratch folder), never the real one, from `gp input-log generate --frames 1800 --hold start@120-124 --hold start@300-304`.

| Tool | Which code runs, by bank | RAM and registers | Inputs that drive it | Cost, Moss Hop | Cost, Ninja Gaiden |
|---|---|---|---|---|---|
| `gp record --rom R --input LOG` | None directly; it stores what the others replay | State every frame (the trace), checkpoints every 60 frames, the event stream | An input log: a recipe's presses from power-on | 1,800 frames in 1.61 s, 927,930 bytes in the store | 1,800 frames in 1.78 s, 1,860,522 bytes |
| `gp trace frame ID F` | None | CPU, RAM, PPU, OAM at one frame, from the trace | A recording | 0.03 s | 0.03 s |
| `gp trace instructions ID --frames 900..960` | CPU address of each instruction; no bank | A, X, Y, P, SP and cycle per instruction, the operand's value | A recording | 581,469 lines in 0.63 s | 569,342 lines in 0.66 s |
| `gp events ID --kind bank-switch,nmi,irq` | Raw mapper register writes (address, value, scanline, dot), not the bank they map | None | A recording | 1,795 NMIs | 11,717 mapper writes and 1,791 NMIs over 1,800 frames |
| `gp coverage ID --no-save --json` | Executed PRG bytes and instruction starts by PRG offset, so by bank; the CPU addresses each 1 KB page ran at | RAM and PRG RAM bytes written | Recordings, or the whole store | 1,293 bytes, 652 starts, 0.13 s | 6,527 bytes, 3,230 starts, 0.16 s |
| `gp_coverage::activity` (inside `gp gaps`) | Routines entered (`jsr` targets, NMI and IRQ handlers) by PRG offset, per frame | Each RAM write with the instruction and routine that made it; A at each call | A recording | Not a command of its own | Not a command of its own |
| `gp element inputs --run --clip` | The routine that wrote a byte, and the bytes and branches its value depends on | The slice's bytes, by label | A scored clip in a capture run with a state mapping | Needs a run, a clip and a mapping | Can't run: no template, no mapping |
| `gp clips matrix RUN --labels rom` | Routines and branch directions the run's recordings take, by CPU address (`codemap::Routines`, unbanked) | Columns: RAM bytes each routine reads that take two or more values | A capture run's generated starts, then a seeded search of presses | In `gp bench clips mosshop --work DIR --mutants 1` (its own home, the hand-written model and mapping): 172 of 172 reachable branch directions, 84 of 84 inputs, 899 of 977 cells, 16 recipes of 2,125 frames; 69.5 s for the matrix's two seeds, 209 s for the whole bench | Can't run: no capture run (no model with a player, no mapping) |
| `gp capture steer RUN` | Routines and branches no clip runs; a directed search rewarded by distance in the call graph, by CPU address | Measures through the run's mapping | A capture run; press sequences mutated by `dense::mutate` | On the bench's run (`GP_HOME=DIR/home gp capture steer real --method routine,branch,directed --targets 4 --budget 100`): 37 routines, 21 branch targets, 0 routines for the directed search (every routine already runs), 0 clips, 2.6 s | Can't run, as above |
| `gp capture dense RUN` | Coverage-guided search of presses, valued by CPU addresses and branch directions (`gp_nes::coverage`) | Mapped measures | A capture run | Needs a capture run | Can't run, as above |
| Clip recipes (`docs/formats/clips.md`) | A dense clip carries the hash of its code map | None | A start (a recording's frame, or presses from power-on) and presses | Text only | Text only |
| `gp model static R --unread --write DIR` | The static code map and the routines no path read | Constants, places read and written, guards | None | 0.04 s, 37 of 37 routines read, 1,785 bytes | 0.07 s, 45 of 168 read, 11,255 of 131,072 bytes |
| `runcov` (scratch) | Executed (bank, pc), entries by kind with their sites, pointer values at each `jmp (ptr)` | Pointer bytes at indirect jumps | Scripts from power-on, no recording | Eight baselines, 2,280 frames, 1.9 s | cheap, 21,600 frames, 20.3 s |

The steering tools key their coverage map by CPU address. `crates/gp-nes/src/coverage.rs` says so: "Addresses are the CPU's, so on a board that switches banks two banks' code at one address count once." `codemap::Routines`, which the matrix and steer share, is the unbanked code map; the harvest has its own banked one (`BankedRoutines`).

## 2. Validation where the source is known

Before any commercial game, the machinery was checked where the answer is known.

### Moss Hop and the practice variants

`gp model static ROM --unread --write DIR`, then `runcov ROM DIR OUT` with Moss Hop's eight baselines. Variants built with `scripts/mosshop/practice_variant.py NAME --cfg ... --prg-base 0xC000 --patch ...` from `scripts/mosshop/variants/` (the ground truth step failed for three of them, which doesn't touch the ROM).

| ROM | Code map bytes | Executed bytes | Executed, not in the code map | Entries the code map lacks | Code map routines run | Banks run |
|---|---|---|---|---|---|---|
| Moss Hop | 1,785 | 1,772 | 0 | 0 | 37 of 37 | 1 |
| UxROM, slugs in bank 1 (`uxrom-slugs-in-bank1.patch`) | 18,174 | 1,777 | 0 | 0 | 37 of 40 | 2 |
| MMC1, slugs banked | 18,220 | 1,823 | 0 | 0 | 38 of 41 | 2 |
| MMC3, slugs banked | 18,179 | 1,782 | 0 | 0 | 37 of 40 | 2 |
| A bank per mode (`ram-bank-per-mode.patch`, `uxrom-2banks.cfg`) | 18,182 | 1,785 | 0 | 0 | 39 of 40 | 3 |
| The bank number in RAM on UxROM (`ram-uxrom-bank.patch`) | 18,181 | 1,784 | 0 | 0 | 37 of 40 | 2 |
| Jump engine (`jump-engine.patch`) | 1,796 | 1,783 | 0 | 0 | 42 of 42 | 1 |

On all seven, the runs and the static map agree on the code: nothing runs that the map lacks. The banked variants' code maps are about 18,000 bytes because three routines the reader didn't read run on over the fill bytes of empty banks; the runs show they are not code. The jump engine's four `jmp (ptr)` targets are routines the map has.

Five more scripts on Moss Hop (`idle:1800`, `rand:1:1800` to `rand:4:1800`) executed 1,707 bytes, all in the code map, at about 1.5 s per 1,800 frames.

### The 68 test ROMs

`gp model static ROM --unread --write DIR` and `runcov ROM DIR OUT idle:900` on each `.nes` under `$GP_HOME/work/test-roms` (they drive themselves). 49.1 s in all, the slowest 0.82 s.

| Measure | Value |
|---|---|
| ROMs where a run executes code the static map doesn't have | 50 of 68 |
| Those bytes, in all | 12,968 of 92,571 executed |
| Entries the static map lacks | 465, in 48 ROMs |
| The cause | In 48 of the 50, `jmp ($000E)` through a pointer the test framework fills at run time (`4-irq_and_dma.nes`: one site, 20 targets). The other two (`mmc1.nes`, 7 bytes; `thorough_test.nes`, 2 bytes) have only `jsr` entries; their cause wasn't examined |
| `official_only.nes` (MMC1, 256 KB) | The static map has 42 bytes, the run executes 8,526. The ROM switches 32 KB at once (MMC1 PRG mode 0 or 1); the static board knows only mode 3 |

The test ROMs are where the run-driven code map first proves itself: their code is known (blargg's source is public), they need no input, and the static walk misses code in most of them.

## 3. The four banked games

Measured, never fitted to. `gp model static ROM --unread --write DIR`, then `runcov ROM DIR OUT` with the scripts named. The code map columns are counted by PRG offset; an unbanked name and a banked name of the same bytes count once, which is why SMB2's map is 13,934 bytes here and 14,500 in `--unread`'s line.

| Game | Static: routines read, code map bytes | idle: executed bytes (not in map) | start: executed (not in map) | cheap: executed (not in map) | cheap: banks run | cheap: time |
|---|---|---|---|---|---|---|
| Ninja Gaiden (MMC1, 128 KB) | 45 of 168; 11,255 | 4,483 (1,322) | 6,454 (941) | 10,270 (2,054) | 5 of 8 | 20.3 s |
| Mega Man 2 (MMC1, 256 KB) | 43 of 127; 8,154 | 4,058 (1,976) | 4,241 (1,901) | 12,487 (7,823) | 4 of 16 | 18.7 s |
| Kirby's Adventure (MMC3, 512 KB) | 44 of 45; 2,238 | 20,640 (18,651) | 15,691 (13,686) | 27,615 (25,572) | 19 of 64 | 23.8 s |
| SMB2 (MMC3, 128 KB) | 38 of 194; 13,934 | 1,877 (3) | 2,216 (150) | 10,516 (5,076) | 8 of 16 | 20.5 s |

Against the code map's routines and the unread report, cheap runs:

| Game | Code map routines that ran | Entries run that the map lacks | Unread stops | Stops whose site ran | Stops whose target was entered (some bank) |
|---|---|---|---|---|---|
| Ninja Gaiden | 139 of 168 | 47 | 20 | 14 | 17 |
| Mega Man 2 | 80 of 127 | 110 | 8 | 5 | 6 |
| Kirby's Adventure | 42 of 45 | 631 | 1 | 1 | 1 |
| SMB2 | 104 of 186 | 101 | 48 | 25 | 3 |

SMB2's stops are mostly "only unread callers", which name no target, so few targets can be matched.

Entries by kind over the cheap scripts: Ninja Gaiden 118 `jsr` targets, 43 `jmp (ptr)` targets, 1 interrupt handler; Mega Man 2 133, 32, 1 and 1 `rts` jump; Kirby's Adventure 333, 290, 1 and 54 `rts` jumps; SMB2 173, 12, 1.

For scale, what longer play reaches (`gp coverage ID --no-save --json` on recordings already in the real store):

| Recording | Frames | Executed bytes | Banks |
|---|---|---|---|
| `35a900f9`, Mega Man 2, TASVideos submission 10604S | 122,940 | 38,428 | 6 |
| `c31a0471`, SMB2, TASVideos 6366M | 66,221 | 26,444 | 8 |
| `b691f1ba`, Ninja Gaiden, `baselines/ninja-gaiden-usa/walk-right-1-1.toml` | 2,400 | 9,397 | 5 |
| `33cceeec`, Kirby's Adventure, `baselines/kirby-usa-prg0/enter-1-1.toml` | 2,400 | 23,069 | 20 |

A scripted walk into the first stage of Ninja Gaiden reaches about as much code as six cheap scripts; a full game run reaches two and a half to three times what cheap scripts do (Mega Man 2 38,428 against 12,487; SMB2 26,444 against 10,516). Steering is what closes that gap without a TAS.

## 4. Which static reach rules the runs make unneeded

The reach rules are the ones that decide where code is, as opposed to what it does. Behaviour rules (elements, constants, guards, machines) are untouched by this design.

| Rule (commit) | With runs | Why |
|---|---|---|
| Walk from the vectors (`codemap`) | Stays | Fills in branches no run took, and is the cross-check: on every ROM with source, executed code is inside it |
| Banked boards: bank registers followed per path (805f2747) | Not needed for code a run executed; stays for code reached only by the walk from an executed site | A run knows the bank of every instruction |
| Value sets for a bank a RAM byte chooses (ce7e47d1) | Not needed where a run took the call; the run's banks seed it | The bank byte's values are observed |
| Frames and interrupts start from what the reset code left (90b62cbc) | Stays for reading behaviour; not needed for reach | Observed pointer values replace the inferred ones |
| Pointer hints: a jump through a RAM pointer goes to every constant pair written to it (805f2747) | Not needed where a run took the jump; stays as a cross-check | The pointer's value is read at the jump |
| Jump tables read at an index (b2282732), bounded tables (82513a09) | Stays | A run takes some entries; the table gives the rest, which become steering targets |
| Jump engines (c2142aaf) | Stays for the table's other entries | A run sees the entries it takes as `rts` jumps or `jmp (ptr)` |
| MMC1 PRG modes 0 and 1, MMC3 mode 1 | Never needs writing for reach | The emulator maps them; the test ROM `official_only.nes` shows the cost of not having them statically |

The rules that still matter for code no run reaches are the walk, the tables and the jump engines: they list what exists but hasn't run, which is exactly the steering's work list.

## 5. The design

### The pipeline

| Step | What | Where it lives today | Missing |
|---|---|---|---|
| 1. Run recipes | Scripts of presses from power-on, written as recipes: `idle:N`, Start presses, seeded random holds, and later steered sequences. A recipe is what a clip set holds (`presses_from_power_on`, docs/formats/clips.md), so it is shareable and reproducible from the user's ROM | `gp input-log generate`; `dense::mutate` and `Rng` in `crates/gp-cli/src/serve/dense.rs`; the clip set format | A recipe file for stage 0 (`runs.toml`) and a seeded generator outside the capture run |
| 2. Traces | Each recipe run in gp-nes with an observer: executed (bank, pc), entries by kind with their sites, the bank mapped at each entry, pointer values at `jmp (ptr)`, `rts` jump targets | `gp_coverage::pass` (executed by PRG offset), `gp_coverage::activity` (`jsr` and interrupt entries) | Indirect and `rts` jump targets, pointer values and the bank at each entry: `runcov`'s observer, made a module of `gp-coverage` |
| 3. Seeded code map | The harvest's code map starts from the executed entries (as routine starts) and observed jump edges (site -> target), as well as the vectors. Its bank values for RAM bytes start from what the runs saw | `harvest::passes` already takes `extra` starts and `jumps` edges, and `Reader::frame_values` holds value sets | Passing the run's output in. Nothing else |
| 4. Read for behaviour | The reader reads each executed routine from an entry state with the banks the run had there. The rules run on what it read | `Reader::call(start, entry, caller)` is public; `rules::run` reads `Reader::acts` | Calling it for entries no path from the vectors read |
| 5. Steering | The unread report's stops (sites not reached, tables' untaken entries, routines with only unread callers) become targets. A search of presses from the runs' snapshots, rewarded by distance in the banked call graph, writes new recipes; back to step 2 | `gp capture steer`'s `directed`, `codemap::distances`, `dense::mutate` | A bank-aware coverage key, a search that runs without a capture run or a mapping, and the banked call graph from the harvest's `BankedRoutines` |
| 6. Report | `gp model static --unread` gives, per stop, whether a run reached its site and target | `harvest::unread` | The run's reach beside each stop |

### What the emulator records, and the gaps filled for this process

gp-nes already observes every instruction through `Nes::step_with` without changing the console's state, and several observers exist. The process needs one more, built on the same hook:

| Fact | Recorded today by | Gap |
|---|---|---|
| Instruction starts and executed bytes, by bank | `gp_coverage::pass::Observer` | None |
| `jsr` targets and interrupt handlers, by bank, with callers and A | `gp_coverage::activity` | None |
| `jmp (ptr)` targets with the pointer's address and value | Nothing | New |
| `rts` used as a jump (jump engines, pushed addresses) | Nothing | New |
| The bank in each switched region when an entry runs | `gp events` has the raw writes; the bank is only implicit in the PRG offset | New: the mapped bank per region at each entry, from the board's state |
| RAM values at entry for the bytes a reader would infer (bank bytes, pointers) | The trace has every frame's RAM, not per entry | New, small: the values of the pointer and bank bytes at each entry |
| Branch directions by bank | `gp_nes::coverage` by CPU address | Key by PRG offset |

### Recipes and data

A run recipe is text: the ROM's hash, the script, the seed. Like a clip set, it holds no frame and no ROM byte, so recipes for a commercial game can be committed or shared, and anyone with the ROM makes the same runs. Traces, recordings and the run output are materialized data, made locally under `$GP_HOME` and never committed, as docs/research/datasets.md's "layers" table says. Assets stay separate: they come from the asset map through `gp assets lift`, and stage 0 reads only the map's offsets and properties, as now.

### How it is measured

| Set | Measure | Expected |
|---|---|---|
| Moss Hop, variants A and B, every practice variant | The graded rows of ROADMAP.md, "Where it stands" | Byte-identical `static.json` where the runs add nothing new (the practice set's code is all in the map today), or better; wrong stays 0 |
| The 68 test ROMs | Executed bytes outside the code map | 12,968 today, 0 after step 3; facts byte-identical on the 18 ROMs whose runs add nothing |
| The four banked games | Code map bytes, routines read, unread stops, before and after each step, with the cheap scripts and then with steering | Code map covers every executed byte; routines read rise |
| Every ROM | Time per step | Cheap runs about 20 s a game today |

### Determinism

| What | Checked |
|---|---|
| A run is reproducible from the ROM and its script | `runcov` twice on Ninja Gaiden with `idle:3600 rand:1:3600`: the same output hash (`68a5906f7fc166d1`, times left out) |
| The same ROM gives the same `static.json` | `gp model static` twice on Ninja Gaiden: byte-identical (`0c5ef02f489109f7`) |
| The runs depend only on the ROM, the script, and the power-on settings (zeroed RAM, NTSC) | gp-nes's fixed timestep and the recording format's `power_on` fields. Some games read uninitialized RAM (Mega Man 2 seeds its RNG from it, docs/corpus.md), so the power-on fill is part of the recipe |
| The same ROM and recipe file give the same `static.json` | To be tested by step 2's exit test |

## 6. What changes in the plan and the roadmap

Proposed, not decided until the user agrees. It is listed in ROADMAP.md's open questions.

| Doc | Today | Proposed |
|---|---|---|
| docs/plan-extraction-process.md, stage 0 | "Build every part of the model the ROM's code settles without running it" | "Build every part of the model the ROM settles, run or read: the code is found by running the ROM from power-on under generated and steered press scripts, and read by the static rules for behaviour. Every run is a recipe; the ROM and the recipe file give the same result" |
| ROADMAP.md, phase 1's opening | "reads a game's model from the ROM's code without running it" | "reads a game's model from the ROM, run or read" |
| ROADMAP.md, "How this phase works" | "The tool reads only the ROM and the asset map" | Adds "and runs the ROM in gp-nes under recipes it generates" |
| ROADMAP.md, goals | Goals 3 to 7 | The steps below come first; goal 3's held-out idioms go on after step 4 |

The rules that stay are the same: no source, labels or keys reach the tools; held-out variants are graded by someone else; wrong stays 0; the four games are measured, never fitted to.

## 7. The steps

Each is small and has its exit test. The test ROMs and Moss Hop come first at every step; the four games are measured after.

| # | Step | Exit test |
|---|---|---|
| 1 | The run observer. `runcov`'s observer as a module of `gp-coverage` (`gp_coverage::reach`): executed (bank, pc), entries by kind with sites, the bank per switched region at each entry, pointer values at `jmp (ptr)`, `rts` jump targets. A command, `gp run reach ROM --script ... --json`, that runs scripts from power-on with no store and no capture run | On Moss Hop, every practice variant and the 68 test ROMs, its executed starts equal `gp coverage`'s on the same input. Run twice, identical JSON. A unit test on a hand-made program with each kind of entry. The numbers of sections 2 and 3 come out the same |
| 2 | Run recipes. A `runs.toml` beside the asset map (or generated) listing scripts as recipes; `gp model static --runs FILE` runs them before the harvest. The default set is the cheap one | The same ROM and `runs.toml` give a byte-identical `static.json` twice. With no runs, output is byte-identical to today on every ROM |
| 3 | The code map seeded by runs. Executed entries go in as `extra` starts, observed `jmp (ptr)` and `rts` edges as `jumps`, observed bank bytes into `frame_values` | Executed bytes outside the code map: 0 on the 68 test ROMs (12,968 today), on every practice variant and on the four games' cheap runs. Moss Hop and every variant's graded rows unchanged, wrong 0 |
| 4 | Read what ran. The reader starts at each run entry no path read, with the run's banks, and the rules run on it | Graded rows unchanged or better, wrong 0. On the four games, routines read before and after, reported with the command |
| 5 | Banked steering. A coverage key by PRG offset; the directed search of `steer.rs` moved to a form that runs from the runs' own snapshots with no capture run or mapping, aimed at the unread stops by distance in the banked call graph; its finds written as recipes | A practice variant whose banked routine runs only on an input sequence a cheap script doesn't make (new, by the tool lane): the search reaches it and stage 0 reads it. On the four games, stops whose site ran and routines read, before and after |
| 6 | The plan's definition and the roadmap, once the user agrees | docs/plan-extraction-process.md and ROADMAP.md say "the ROM, run or read"; the decided list says when |

## 8. Carried over from the session before

| Item | Result |
|---|---|
| Full `just check` on commit ce7e47d1 (which holds 90b62cbc) | Not passed. The first run failed while compiling (`No space left on device`, no test failed); the second was stopped when the user deleted the rule that no commit or merge goes in without it (2026-10-31, "delete the rule") |
| m3/static-ram merged into m3/static-harvest | Done 2026-10-31, a fast-forward to ce7e47d1 |
