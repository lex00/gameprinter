> A research record brought from game-printer (docs/research/code-steered-clips.md) with the commands it describes. Paths and commands are the old repo's where they differ: `gp serve` and `gp mcp` are retired (the watch routes are on the mechanic servers, docs/watch.md), capture runs come back in wave 7 of the port, and `gp capture dense` and `carry` read a game's package for now (docs/capture.md).

# Code-steered clips: choosing frames by the original's code

A research report of 2026-10-30, on branch `research/code-steered`. The user asked, 2026-10-30: "how much role is the original rom code itself driving which frames get captured?" The answer then was that it mostly measures coverage, and steers only through the grid's scout and the dense pick's third weight. Then: "put a research agent on refining that. compare a few potential ones first to the ones we already have". Later the same day the user sharpened it: "earlier you were asking for help on what a fireball was doing. i feel like the code should have driven that clip and answered itself, and thats how it should work for any element in the game. work from code through to the asset and generate clips to validate behavior."

Then: "the clips we have so far have helped us refine but probably need to re-evaluate what the clip matrix is to generate when its truly driven from the code base. our original clip generation was not based on this right?" It wasn't: the generated clips are level starts times a fixed action menu, the grid is model mechanics times places, the dense set is weighted by code coverage, and aimed clips are a few by hand. And: "try to build determinism now into the algo to generate the clip recipes". The main result is that tool, `gp clips matrix` (section "The code-driven clip matrix"); the method comparison it grew from follows it.

It reads against [dense clips](dense-clips.md) (the coverage map, the weighted pick), [the clip set report](clip-set.md) (mutants, the kept set), [graphs](graphs.md) (the element-to-code walk), docs/show.md ("The grid", "The scout"), docs/progress.md (stage 6) and docs/corpus.md (the TAS movies). The prototype is `gp capture steer` (`crates/gp-cli/src/serve/steer.rs`), with a data-access hook in `gp-nes` (`coverage::access`, `NesRuntime::step_accesses`). It writes only into a scratch run.

## Summary

| Question | Answer |
|---|---|
| What the code-driven matrix is | Rows are the routines and branch directions the recordings take; columns are the RAM bytes each routine reads that take two or more values; a cell is filled by a clip that takes the row while the routine reads two or more values of the column. For Super Mario Bros. it is 20,893 cells: 4,306 rows over 811 routines |
| How much today's clips fill | The run's 99 clips (without the 2 save points) fill 7,998 cells (38%). 58 of them fill no cell that another doesn't: 47 generated clips and 11 of the grid's. They are the candidates to drop |
| What the tool adds | 137 recipes (12,785 frames), recording windows from starts the rebuilt game can begin at, fill 5,434 more: 13,432 of 20,893 (64%). 7,461 cells stay gaps, each with its reason: 6,634 because the one window per row doesn't vary that input, 497 with no frame on the ground before the row, 194 for an object a start can't carry, 136 not reached by the directed search |
| Deterministic | `gp clips matrix` uses no clock, no hash-map order and no model; ties go by a hash of the seed and the window. On Moss Hop two fresh homes give byte-identical `matrix.json` and `recipes.toml`, `--jobs 1` and `--jobs 8` agree, and another seed gives the same matrix and fill (`crates/gp-cli/tests/clips_matrix.rs`) |
| Recommendation | Replace level starts times the action menu with this matrix as the clip generator's definition, and make the grid this matrix (rows by routine and branch, columns by input), as section "The matrix as the generator" says |
| Methods compared | Five new ones (routine gaps, branch gaps, cited routines, directed search, and per element, code first) against four that exist (the generated set, the grid, the dense pick, the prune kept set), on the same run, model and 46 mutants, at 15 and 40 clips |
| Best single method at 15 clips | Branch gaps: 15 clips of 1,189 frames run 6,902 of the playthrough's 8,727 addresses and 1,692 of its 2,290 branch directions, more than any other set of 15, and show 25 failing measures no current clip shows. The dense pick catches more mutants (32 against 22) and holds more model parts (142 against 128) |
| Best mix | The dense pick's weighted cover with the per-element clips among its candidates: 15 clips, 1,357 frames, 168 model parts (the most of any 15), 35 of 46 mutants, 27 new failing measures (the most) |
| Per element, code first | On the lava ball (podoboo) it finds the random launch with no one asking. Run against the model before the fix (the `leap.draw` mutant), the detector says: "the original's MovePodoboo reads PseudoRandomBitReg+1, PseudoRandomBitReg+2, PseudoRandomBitReg+3, PseudoRandomBitReg+4, PseudoRandomBitReg+5 (counters.random) for the podoboo on 21 frames; the model's podoboo reads no random counter". It needs one traced pass of the recordings, and the first of its 8 podoboo clips runs that code and catches the mutant |
| Generality | On 6 elements: the detector names the podoboo's fix and no false fix elsewhere; it flags the hammer bro, whose random reads live in a hand-written hook the model doesn't declare. Its clips catch 3 mutants no clip of the run catches. It doesn't see fixes that aren't a missing input (the fire bar's leftover angle, the cannon bill's leave distance) |
| Code steering that doesn't work yet | Directed search to routines nothing runs: 1 of 12 reached (PlayerLoseLife). The rest are title, demo, pause, game over, two-player and continue code, which an in-level search can't reach |
| Recommendation | Make per-element, code first, a pick kind of the grid (`element` rows aimed by their routines), with its detector sentence as the brief an agent gets. Keep the dense pick's weighted cover as the way clips are chosen, with per-element and branch-gap windows as candidates. Drop directed search for now |

## The code-driven clip matrix

`gp clips matrix RUN [--recording H] [--seed S] [--write DIR] [--labels FILE] [--search-rows 8] [--search-budget 200] [--min-frames 60] [--max-frames 180]` (`crates/gp-cli/src/clips.rs`, the work in `serve/steer.rs`). RUN names the template's model and mapping, the ROM and the recordings; it is only read. With `--write`, `matrix.json` and `recipes.toml` go to that folder, never into a run.

| Part | How it is made |
|---|---|
| Rows | One traced pass of each recording from power-on (`NesRuntime::step_accesses`). Every routine (a label of the game's labels file) that runs is a row ("DecTimersLoop runs"), and so is each direction of each of its branches the recordings take ("the branch in MovePodoboo at XXXX, taken"). Element tags are the per-element method's slot attribution (section "Per element, code first"); the basic-elements lane's checklist can replace them when it lands |
| Columns | For each routine, each RAM byte (not the stack page) its instructions read that takes two or more values over the recordings: its inputs, named by the labels file (PseudoRandomBitReg+1, EnemyIntervalTimer, Player_State) |
| Cells | A row times its routine's columns |
| Filled | A clip fills a cell when it takes the row and the routine reads two or more values of the column within the clip |
| Existing clips | The run's queue pool, in ID order, run on the original from each start. A cell they fill gets no recipe |
| New recipes | For each row with an empty cell, the window of the recording from the last start the rebuilt game can begin at, 8 to 154 frames before the row first runs (`gp_interp::starts::cant_start_here`, with the carry rules), 60 to 180 frames. Each window's cells are read by running it on the original. Greedy cover of the empty cells, most new cells per frame first, compared in whole numbers, equal ones by a SplitMix64 hash of the seed and the window |
| Search | For the rows no window reaches, the most empty cells first (ties by the seed's hash), a seeded directed search from the run's starts (section "The five new ones") |
| Gaps | Every cell still empty, with its reason: no usable start (and the carry reason), the directed search didn't take it, or the input doesn't vary in the clip that takes the row |
| Recipe | `id`, `cell` (routine, branch and input in words), the number of new cells, frames, presses as spans, the hash of the cells it fills (`cells_hash`, the check a regenerated clip must pass), and its start (recording by hash, frame, model frame, level). The file's `[matrix]` table holds the generator and its version (`0.1.0`), the seed, and BLAKE3 hashes of the ROM, labels, mapping, model and each recording's input log |

### Super Mario Bros.

On a scratch copy of `smb-side-by-side-2` with the branch's model (core 0.4.22, mapping 19), the three TAS movies, seed 1, 8 jobs, a release build:

| Measure | Value |
|---|---|
| Matrix | 20,893 cells: 4,306 rows over 811 routines with an input (1,577 routines run) |
| Filled by the run's 99 clips | 7,998 (38%) |
| Clips that fill no cell of their own | 58 of 99 (47 generated, 11 grid) |
| New recipes | 137 windows, 12,785 frames; the directed search took none of its 8 rows |
| Filled after | 13,432 (64%); 5,434 new |
| Gaps | 6,634 the input doesn't vary in the row's window; 497 no frame on the ground in play before the row; 194 an object a start can't carry (108 Bowser's flame, 28 the cheep cheep frenzy, 21 Bowser, 12 the blooper, 9 the vine, the rest the hammer bro, the jumpspring and three unmapped objects); 136 not reached by the search |
| Time | 540 s: the traced pass 134 s, the starts scan and windows 327 s, the search 79 s |

The routines with the most cells left are the collision code (BlockBufferCollision 1,299 cells, 399 filled; FirstBoxGreater; CollisionCoreLoop; HandlePECollisions; ECLoop), whose inputs are the block buffer and every object's place: one window per row can't vary them all. That is the next step for the tool: a second window per row aimed at the inputs its first left constant.

Elements, by their routines' cells, before and after:

| Routine | Rows | Inputs | Cells | Filled before | Filled after |
|---|---|---|---|---|---|
| MovePodoboo | 3 | 10 (EnemyIntervalTimer+0 to +4, PseudoRandomBitReg+1 to +5) | 30 | 12 | 15 |
| HammerBroJumpCode | 9 | 12 | 108 | 3 | 18 |
| ProcFirebar | 4 | 2 | 8 | 4 | 8 |
| MoveLakitu | 3 | 2 | 6 | 0 | 3 |
| BulletBillHandler | 8 | 4 | 32 | 29 | 31 |
| PlayerPhysicsSub | 7 | 2 | 14 | 2 | 14 |

### Moss Hop

1,077 cells (167 rows over 15 routines). With 10 of the run's clips in the pool, they fill 452; 7 recipes fill 492 more; 133 gaps (78 the input doesn't vary, 50 no usable start, 5 not reached by the search, with `--search-rows 1 --search-budget 16`). The test runs it from two fresh homes, at 1 and 8 jobs and with seeds 1 and 2.

### The matrix as the generator

| Today | The matrix |
|---|---|
| The clip corpus: each level's starts times a press menu (walk 3 s, run, jump tap, hold) | Each row and input of the original's code: a clip exists because a branch and an input need it |
| The grid: rows are the model's mechanics, options and entities; columns are levels and places; the next clip goes where the model is most in doubt | Rows are routines and their branches (grouped by element when the checklist tags them); columns are inputs. A cell's state is the same (empty, differs, matches); its doubt is the model's and mutants' disagreement on the clips that fill it. The next clip goes to an empty cell in a row whose filled cells differ, then the biggest empty rows |
| A clip's name says its start and its press pattern | A clip's name says the routine, the branch and the input it was made for |
| Dropping clips: the prune's cover of model parts | A clip that fills no cell of its own is a drop candidate; the prune's mutant guard still keeps one per fix |
| The scout every 4th clip | The matrix's own order; the scout's code features become the rows |

To make it the default: the queue takes `recipes.toml` as it takes the dense set (a recipe is a recording frame and presses, as a dense recipe is; `gp capture regenerate` checks `cells_hash` as it checks a dense clip's `coverage`), the grid's rows and columns are read from `matrix.json`, a second window per row for inputs left constant, the element tags from the basic-elements checklist, and a bank-aware key before a game that switches banks.

## The run

`smb-side-by-side-2`, Super Mario Bros. (PRG 0), model `b0c148f16006` (main at `aa540854`), cloned on 2026-10-30 into a scratch `GP_HOME` (APFS clones of the store, results and play folders; the real store read only), `GP_ROM_DIR=/Users/alex/Documents/roms/nes`, a release build of this branch. The live server on 7700 was not used.

| What | Value |
|---|---|
| The run's clips the queue scores (grid mode) | 101: 25 the grid made, 74 tool clips scored before the grid, 2 save points |
| Mutants | 46: the template's 36 (`templates/smb1-ntsc-prg0/clips.toml`) and 10 new ones for the fixes merged on 2026-10-30 after them, in [code-steered-mutants.toml](code-steered-mutants.toml) (the fire bar's two slots and leftover angle, the lava ball's random launch, walkers turning at walls, hard-mode spawns, hit in turn, the cannon bill's leave distance, and others) |
| Caught by some clip | 40 of 46. The run's own clips catch 37; the new clips add `enter_keeps_fraction`, `run.face_on_ground` and `springboard.boost` |
| The playthrough | 2676M, "warpless, walkathon" (docs/corpus.md), on its 70,215 frames in play: 8,727 CPU addresses and 2,290 branch directions, the same 8,727 `gp progress` counts for stage 6 |
| The run's 101 clips | 6,771 of the 8,727 addresses (78%) and 1,651 of the 2,290 branch directions |
| Labels | doppelganger's SMBDIS.ASM, pinned, as `$GP_HOME/work/smbdis/labels.toml`: 1,643 code labels. Only their names and addresses are read |

Every candidate clip of every method was written into one scratch run, `steer` (made by `gp capture dense` and added to by `gp capture steer`), and scored once with `gp capture prune --mutants` on the model and each mutant. The run's own clips were scored with `gp capture prune smb-side-by-side-2 --mutants --guard`, whose score cache the scratch run reused, so every clip has one score on each of the 47 models.

## The methods

### The four that exist

| Method | How it picks | Where the code comes in |
|---|---|---|
| The generated set | Level starts and contexts times press patterns (the clip corpus); in grid mode only those scored before the grid began, in the corpus's order | Not at all |
| The grid | The cell with the most doubt (model and mutants disagreeing with the original); every 4th clip a scout | The scout takes the clip, of 48 open cells, that reaches the most code features no clip has reached. 6 of the grid's 25 clips were scouts |
| The dense pick | Weighted greedy cover: each mutant caught 1,000, each model part 30, each code feature 1, priced by frames, over the run's clips and dense candidates (a coverage-guided search and TAS windows) | The candidates are chosen for new code; the pick weighs code last |
| The prune kept set | Greedy cover of model parts, then every mutant some clip catches (`--guard`) | Not at all |

### The five new ones

All five are `gp capture steer RUN --into SCRATCH --method M`. The recording methods share one pass of each of the run's three recordings on the original: each frame's code map, and at every 6th frame with the player on the ground in play, whether the rebuilt game can start there (`gp_interp::starts::cant_start_here`, with the carry rules).

| Method | Target | Clip |
|---|---|---|
| `routine` | A code label the recordings run and none of the run's clips runs: 418 | From the latest usable start 8 to 160 frames before the label first runs, the movie's own presses, 60 to 180 frames, named "covers <Routine>". Greedy cover of the targets, priced by frames |
| `branch` | A branch direction the recordings take and no clip takes: 942 | The same, named "covers the branch in <Routine> at <address>, taken" |
| `cited` | A routine the model's evidence cites (the label above a cited SMBDIS.ASM line), not run by any clip: 43 of 240 cited | The same, weighted 1, plus 2 when the fact's entity has an open question, plus 2 when it has a failing clip |
| `directed` | A routine neither the recordings nor the clips run, nearest first in the call, jump and branch graph from code run in play: 12 of 57 | A coverage-guided search (dense's mutations) from the 3 starts whose seed runs come nearest, rewarded by the least graph distance any routine it ran has to the target, as AFLGo (Bohme, Pham, Nguyen and Roychoudhury, CCS 2017) rewards distance to a target; 600 sequences per target, up to 300 frames |
| `element` | Each of the model's entities named with `--element` | Below |

The graph for `directed` is decoded from the ROM in memory: each label's code is swept from its start to its first return or jump, with edges for calls, jumps, branches, fall-through into the next label, and inline jump tables (two or more words after a call that each start a label). It is never written.

### Per element, code first

The user's example: the lava ball's second leap was too high. An agent found, after the user reported it, that MovePodoboo reads the game's random register (PseudoRandomBitReg) for each launch and the model's leap read none (attempt 117, `b9939bf4`). The method below finds that from the code.

| Step | Here |
|---|---|
| 1. Element to code | One traced pass of each recording. Before each instruction, `gp_nes::coverage::access` works out the data address it reads or writes from its addressing mode and the registers. A routine is the element's when an instruction in it reads or writes a field of a slot that holds the element (the mapping's slot fields, the ID, flag and state bytes) with X naming that slot. Every kind the mapping's slots name is traced, so a routine that touches more than a quarter of the kinds seen (the enemy loop, the bounds checks, collision) is marked shared |
| 2. Code to inputs | Every address those routines read with X on the element's slot, sorted into its own fields, other objects' fields, other RAM and ROM tables, named by the labels file and by the mapping field that holds it (`counters.random`, `entities.mario[0].x`, the frame counter) |
| 3. Inputs to clips | Windows of the recordings from usable starts with the element live, chosen by greedy cover of the instructions and branch directions of the element's routines, priced by frames, 8 per element. Each is named by the first branch direction it adds in one of the element's own routines |
| 4. Clips to an answer | For each mapped counter that the element's own (not shared) routines read, if the model's entity names that counter nowhere in its table, a sentence: "the original's <Routine> reads <Label> (<field>) for the <element> on N frames; the model's <element> reads no <counter> counter". Run on each mutant model too, it says which fixes the sentence would have found |

The graph report's walk (docs/research/graphs.md, "From an enemy's assets to its code and clips") goes the same way statically, from the disassembly's text: routines that name the enemy's constant, and routines that write RAM mapped to its fields. The trace gives the same links from the ROM alone, with no disassembly text, and gives the inputs a routine reads for one slot, which the static walk can't separate (EnemyGfxHandler draws every enemy). `docs/research/notes/graphs/build_graph.py` needs pyarrow, which this machine doesn't have, so its Parquet wasn't rebuilt; nothing here depends on it. There is no `ramrole.rs` in the repo yet; the trace stands in for it.

## The method comparison

Each row's clips were scored on the model and the 46 mutants. ROM code is counted within what the playthrough runs. "New failing measures" counts the distinct (body, measure) pairs that some clip of the set gets wrong first and none of the run's 101 clips does: the run's clips show 14 such pairs, so a set of the run's own clips shows no new one by definition. "Lost to carry" is the method's targets that had a recording frame before them but every such frame was refused by `cant_start_here`. Scoring time is one score on the model from an empty cache, 8 jobs, this machine (18 cores).

### At 15 clips

| Method | Clips | Frames | ROM addresses (of 8,727) | Branch directions (of 2,290) | Model parts (of 236) | Mutants (of 46) | New failing measures | Lost to carry | Score time |
|---|---|---|---|---|---|---|---|---|---|
| Generated, first 15 | 15 | 847 | 3,254 | 710 | 24 | 9 | 0 | 24 of 114 starts | 2.3 s |
| Grid, first 15 | 15 | 3,300 | 5,266 | 1,295 | 108 | 22 | 0 | (cells with no start are skipped) | 4.0 s |
| Dense pick | 15 | 1,023 | 6,580 | 1,617 | 142 | 32 | 22 | not counted | 2.3 s |
| Routine gaps | 15 | 1,206 | 6,639 | 1,621 | 117 | 20 | 22 | 13 of 418 | 3.2 s |
| Branch gaps | 15 | 1,189 | 6,902 | 1,692 | 128 | 22 | 25 | 47 of 942 | 2.5 s |
| Cited routines | 14 | 1,549 | 6,252 | 1,549 | 104 | 18 | 23 | 2 of 43 | |
| Directed search | 1 | 278 | 2,719 | 604 | 27 | 6 | 2 | | |
| Per element (6 elements in turn) | 15 | 1,295 | 5,990 | 1,457 | 124 | 21 | 23 | | 3.5 s |
| Dense pick over dense and per-element candidates | 15 | 1,357 | 6,765 | 1,661 | 168 | 35 | 27 | | 3.5 s |
| Dense pick over dense and branch-gap candidates | 15 | 1,114 | 6,671 | 1,642 | 152 | 33 | 24 | | |
| Dense pick over dense and every new candidate | 15 | 1,277 | 6,535 | 1,601 | 159 | 36 | 22 | | |

### At 40 clips

| Method | Clips | Frames | ROM addresses | Branch directions | Model parts | Mutants | New failing measures | Score time |
|---|---|---|---|---|---|---|---|---|
| Generated, first 40 | 40 | 3,250 | 5,637 | 1,363 | 86 | 26 | 0 | |
| Grid, all it made | 25 | 5,260 | 5,793 | 1,417 | 123 | 22 | 0 | |
| Prune kept set | 63 | 7,531 (15,706 with the save points' lead-ins) | 6,561 | 1,580 | 154 | 37 | 0 | 6.8 s (61 clips, without the 2 save points) |
| Every clip of the run | 101 | 11,762 | 6,771 | 1,651 | 154 | 37 | 0 | |
| Dense pick | 40 | 4,212 | 8,106 | 2,018 | 213 | 38 | 42 | 7.1 s |
| Routine gaps | 40 | 3,475 | 7,747 | 1,922 | 173 | 26 | 41 | |
| Branch gaps | 40 | 3,289 | 7,965 | 2,019 | 184 | 28 | 40 | 4.7 s |
| Per element | 40 | 5,010 | 6,937 | 1,736 | 165 | 31 | 43 | 7.1 s |
| Dense pick over dense and per-element candidates | 40 | 4,061 | 8,087 | 2,014 | 219 | 39 | 43 | 6.3 s |
| Dense pick over dense and routine-gap candidates | 40 | 3,820 | 8,017 | 1,986 | 214 | 39 | 45 | |
| Dense pick over dense and every new candidate | 40 | 3,688 | 8,070 | 2,029 | 219 | 40 | 42 | |

What the tables say:

- Every method that cuts clips from the recordings where the code is (dense windows, routine, branch, cited, element) finds 20 to 27 failing measures in 15 clips that the run's 101 clips don't show. The run's own sets, chosen by the model's doubt or by press patterns, find none new: their failures are the 14 the run already knows.
- Aimed at code alone (routine, branch), a set runs the most code but catches fewer mutants than the dense pick (22 against 32 at 15). As the dense report found, a mutant is often one comparison on one frame; code reached is not the same as the model checked there.
- Per element, the set runs less code (each element's clips repeat the enemy loop) but catches mutants of its elements, and as candidates in the weighted pick it gives the most model parts and new failing measures at 15.
- Cited routines are mostly covered already: 197 of the 240 routines the model cites run in some clip. The 43 that don't fit in 14 clips.
- Carry still blocks: 2,116 of the 6,271 recording frames checked (34%) can't be started from. The largest causes are the flagpole's flag (object 0x30, 319 frames), the flying cheep cheep frenzy (312), the power-up (250), the green koopa's timer (247), Bowser's flame (212), the hammer bro's word (143) and the jumpspring (125). For the routine and branch targets the loss is small (13 and 47) because most code that matters runs again later from a usable frame.

Time to make the candidates (release build, 8 jobs): `gp capture dense` 405 s; `steer` routine, branch and cited 725 s including directed's first pass (one scan of the three movies is about 80 to 160 s); per element, 6 elements, 739 s (the traced pass 94 s, the rest the 400 windows per element run on the original).

## Per element, on six elements

| Element | Its own routines (not shared) | What they read besides their own slot | Detector on the model as it stands | Mutants it names | Clips | Mutants its clips catch of the element's own |
|---|---|---|---|---|---|---|
| Podoboo (lava ball) | InitPodoboo, MovePodoboo, CheckForPodoboo, PositionEnemyObj, CheckRightExtBounds | PseudoRandomBitReg+1 to +5 (MovePodoboo), Up_Dir, VerticalFlipFlag | Nothing | `leap.draw`: "the original's MovePodoboo reads PseudoRandomBitReg+1, ... +5 (counters.random) for the podoboo on 21 frames; the model's podoboo reads no random counter" | 8, 988 frames | `leap.draw`, by the first clip (World 3-4, 60 frames) and the fourth |
| Bullet bill cannon | BulletBillHandler, SetupBB, KillELoop, ChkInj, and others | TimerControl, the player's y | Nothing (its model names `random`) | None | 8, 934 frames | Not `cannon.despawn_ahead`; one generated clip catches it |
| Hammer bro | ProcHammerBro, HammerBroJumpCode, SetHJ, HJump, SetHPos, SetHSpd, SetMOfs, ChkJH, and the landing and collision checks | PseudoRandomBitReg+1 to +6 (HJump, HammerBroJumpCode, SetHJ), the level's block buffer | "the original's HJump and HammerBroJumpCode and SetHJ read PseudoRandomBitReg+1 ... (counters.random); the model's hammer_bro reads no random counter" | None | 8, 1,008 frames | No hammer bro mutant in the set |
| Spiny | FallE, LandEnemyProperly, ProcEnemyDirection, SetNotW, and the landing and side checks | The level's block buffer | Nothing | None | 7, 901 frames | Not `walk.turn_after_move`; 1 element clip (a hammer bro window in World 8-3) and 15 others catch it |
| Lakitu | MoveLakitu, ChkLS, SetLSpd, ChkLakDif, ChkSpinyO, CheckToMirrorLakitu | TitleScreenModeValue, sprite RAM | Nothing | None | 6, 686 frames | None in the set |
| Fire bar | ProcFirebar, SusFbar, SetupGFB, FirebarSpin | FirebarSpinDirection, FirebarSpinSpeed, RetainerObject+0 to +3 | Nothing | None | 5, 673 frames | `firebar.extra_slots` (second clip, World 5-4); not `orbit.angle_in_slot` |

The podoboo answers the user's question: yes, it would have found the random launch, from one traced pass and with no clip failing first, and the first of its clips is enough to check it (it catches `leap.draw`). The hammer bro's sentence is true of the code and wrong as a finding: its jumps do read the random bits (attempt 149, `bb0b740a`), through the hammer bro's hand-written hook, whose `ext_params` give the byte (`draw_at`) but not the counter's name. The detector can only see inputs the model declares. The fire bar's and the cannon bill's fixes aren't a missing input (a leftover angle in the slot, a leave distance), so a detector of inputs doesn't name them; its clips catch the fire bar's two-slot fix all the same.

Names in the table are labels of the pinned SMBDIS.ASM; zero-page temporaries are sometimes named by a constant of the same value (the labels file lists constants), which the report shows as is.

### Example clips, per element

| Clip | Targets | Exposed |
|---|---|---|
| World 3-4, from frame 25347 of 4313M, 60 frames: "covers the podoboo: the branch in PositionEnemyObj at C1BE, taken" | The podoboo's spawn and MovePodoboo's random read | Catches `leap.draw` (and 5 Mario mutants); exact on the model as it stands |
| World 5-4, 160 frames: "covers the podoboo: the branch in CheckRightExtBounds at C1EF, not taken" | The podoboo kept past the right edge | Catches `leap.draw`; fails on the model (new failing measures) |
| World 5-4, from frame 52527 of 2676M, 180 frames: "covers the firebar: the branch in SusFbar at CD5F, taken" | The fire bar's spin | Catches `firebar.extra_slots` and `leap.draw` |
| World 8-3, 153 frames: "covers the hammer bro: the branch in HammerBroJumpCode at CA22, taken" | The hammer bro's jump choice, which reads the random bits | Fails on the model; catches `jump.head.hold`, `levels.despawn_after_move` |
| World 8-2, 180 frames: "covers the lakitu: the branch in ChkLakDif at CF90, taken" | Lakitu's hard-mode check | Catches `enter_keeps_fraction`, which no clip of the run catches |

### Example clips, the other new methods

| Method | Clip | Exposed |
|---|---|---|
| Routine | World 6-4, from frame 50752, 116 frames: "covers MoveDefeatedBloober" | 7 failing measures no current clip shows (Bowser's x, the fireball's life, the time, the hammer's x and more), first wrong at frame 13 |
| Routine | World 3-1, from frame 22714, 60 frames: "covers Hole_Water" | 4 new: the koopa's x, Mario's speed up and down, the piranha plant's y, the score |
| Routine | World 6-3, from frame 61949, 60 frames: "covers MoveDropPlatform" | 4 new: the drop lift's y, the balance lift's life, the camera |
| Branch | World 1-3, from frame 40275, 69 frames: "covers the branch in ChkOtherEnemies at D78B, taken" | 5 new: the bullet bill's life, the fireball's y, the coins, the goomba's x |
| Branch | World 4-2, from frame 37460, 60 frames: "covers the branch in FMiscLoop at BB8E, not taken" | 4 new: the buzzy beetle's and the koopa's x, the coins, Mario's speed up and down |
| Branch | World 8-3, from frame 13439, 124 frames: "covers the branch in WriteBlockMetatile at 8A7B, taken" | 4 new: the hammer's y, the hammer bro's life, Mario's y, the score |
| Cited | World 1-2, from frame 4265, 154 frames: "covers PlatLiftUp" | 5 new: the large lift's life, the red koopa's life, the goomba's y |
| Cited | World 1-3, from frame 6401, 137 frames: "covers InitRedPTroopa" | 4 new, from frame 1: the red paratroopa's life, the vertical lift's y, Mario's y, the time |
| Cited | World 1-4, from frame 9346, 172 frames: "covers MoveEnemySlowVert" | 3 new: Bowser's y, Bowser's flame's life, Mario's y |
| Directed | World 1-1 start, 278 searched frames: "covers PlayerLoseLife" | 2 new: Mario's state and the goomba's life after the death, from frame 257 |

The directed search's other 11 targets (TimeUpOn, SpawnFromMouth, SelectBLogic, ResetTitle, GoContinue, DemoEngine, EndChkBButton, GameOverMode, FlagBalls_Residual, VineEntr, DisJoyp) were not reached in 600 sequences each. All but two are title, demo, game-over, continue or pause code; the search doesn't press Start, and the rebuild doesn't model those modes. TimeUpOn needs the timer near 0 and SpawnFromMouth a start near Bowser, which carry refuses (Bowser's word isn't read).

## Recommendation

The code-driven matrix as the clip generator's definition (section "The matrix as the generator"), in place of level starts times the action menu. Within it, the method comparison says how to fill cells and what to tell the agent. Per element, code first, gives the rows their element tags and the brief: its detector sentence. The dense pick's weighted cover stays the way a few clips are chosen when the budget is small:

| Part | How it fits |
|---|---|
| Rows | The grid already has a row per entity kind. An `element` pick takes the row's entity, its routines and inputs from the traced pass (kept per run, like `generated/dense/code.json`), and its next clip is the uncovered branch direction of one of the element's own routines, at a usable start where the element is live |
| Order | An element's first clip goes before the doubt order when the detector has a sentence for it; then its uncovered branches, the most features per frame first |
| The brief | The detector's sentence goes to the clip agent with the clip, in place of a question to the user: "the original's MovePodoboo reads PseudoRandomBitReg+1 (counters.random) for the podoboo; the model's podoboo reads no random counter" |
| The scout | Becomes the branch-gap pick: the clip, from a recording window, that takes the most branch directions no clip has taken. It ran more code per clip than any other method here |
| The dense set | Its candidates gain the per-element and branch-gap windows; its weights stay (mutant, part, code). That mix had the most model parts and new failing measures at 15 clips |
| Dropped | Directed search to code nothing runs (1 of 12), and cited routines as a method of their own (83% are covered already; the rest fit in the element rows) |

## What it would take to make it the default

| Step | Why | Cost |
|---|---|---|
| Declare a hook's inputs | The hammer bro's false sentence: a hand-written hook reads the random counter without the model saying so. An `ext_params` key naming each counter a hook reads (`reads = ["random"]`) lets the detector see it. A core schema change, so a version bump | Small; one key and a check in `gp model check` |
| Detect more than missing counters | The fire bar's leftover angle and the cannon bill's leave distance aren't missing inputs. Next: own-slot fields the original reads at spawn that the model sets fresh (the angle), and ROM tables a routine reads that the model has no fact for | A detector per kind of gap, as docs/gaps.md does |
| Keep the trace per run | One traced pass of the recordings is 94 s for 39 kinds; it changes only with the ROM and the mapping, as the code map does | Store it beside `generated/dense/code.json` |
| A bank-aware key | Routines by CPU address are right for mapper 0. Mega Man 2 and SMB2 switch banks | The event stream's bank switches, as the dense report says |
| Carry the refused objects | 34% of recording frames can't be started from; the flag, the frenzy cheep cheep, the power-up and Bowser's flame are most of it | The carry work (docs/interpreter.md) |
| A routine heuristic | "Shared" is more than a quarter of the kinds; PositionEnemyObj and CheckRightExtBounds count as the podoboo's own, since few kinds go through them | Use the graph's call edges when its Parquet is built here |
| Rebuild the grid's pick | The grid code (`serve/grid.rs`) picks by cell doubt; an element pick and a branch pick are two more kinds beside the scout | Moderate: the pick, its state in `grid/state.json`, the page's row line |

## The command

`gp clips matrix` is above. The comparison's prototype is:

```text
gp capture steer RUN [--into SCRATCH] [--method routine,branch,cited,directed,element]
                     [--write 40] [--seed 1] [--budget 600] [--targets 12] [--labels FILE]
                     [--element podoboo,...] [--per-element 8] [--mutants FILE]
                     [--min-frames 60] [--max-frames 180]
```

It reads RUN, writes its clips into SCRATCH (default `RUN-steer`; a run this command or `gp capture dense` made), and never into RUN. Clips are `dense.steer.<method>.<level>.<frame>.<hash>`, a dense clip's kind of recipe (a recording's frame and presses), so the queue and `gp capture prune` take them as dense clips. A run of some methods keeps the clips of the others. The report is `generated/steer.json`, and every clip's code `generated/steer/code.json`. Its Moss Hop test is `crates/gp-cli/tests/steer.rs`; the detector, the graph and the cover have unit tests in the module.

Nothing in this report or the repo holds ROM bytes or disassembly text: routine and RAM label names only, as docs/corpus.md and the model's evidence already use them.
