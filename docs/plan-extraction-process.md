# Plan: the ROM's code drives the clips and the coverage

> game-printer's plan, kept as it was written (2026-11-05). Its tools come to gameprinter in waves 3b to 5 of the port: `gp model static`, `gp reach`, `gp code`, `gp element inputs` and `gp interp draws --find` with wave 3b. Paths name game-printer's repo (`templates/GAME` is a game's package folder in the games checkout now).


Status: in force from 2026-10-31. Every lane brief names the step of this plan it serves and that step's exit test.

## Why

Super Mario Bros. reached 128 of 131 clips exact, but the clips that got it there didn't come from the code. They came from level starts times a fixed set of actions, the model's own mechanics, and recordings. An LLM then read the disassembly to explain each difference, at about $4.60 per clip fixed (docs/research/determinism-audit.md). Clips chosen that way missed basic elements: a death clip ended before the life did, and no kept clip ever changed the coin counter. The gains per hour were falling.

## The goal

The ROM's code decides what must be checked and generates the clips that check it, with coverage measured from the code. It has to work from the ROM alone, because most games have no source and SMB's isn't available to us.

Moss Hop is the testbed because it does have its source. The tools never read that source. It's used only to grade them: whether they found every element and branch, whether they wrongly marked a reachable branch as unreachable, and whether the clips would catch a wrong model. SMB is left alone for the whole plan.

## The target: 1 minute per second of gameplay

Set by the user on 2026-10-31: for every second of gameplay, take at most one minute to derive the model from it, with accuracy.

| Term | Definition |
|---|---|
| Gameplay | The recordings the run learns from, in seconds at the NES's 60.0988 frames a second. For Moss Hop that is its 8 played baselines in `baselines/mosshop/`: 2,280 frames, 37.9 s |
| Time | Wall clock of the whole from-scratch run on this machine with 8 workers, from the ROM and asset map to the finished model: the four stages and every build the run needs. No LLM and no person; a step that needs one fails the target |
| Accuracy | The finished model plays every frame of those recordings with every mapped value equal to the original's, and every fact it states agrees with the real model and the source. Seconds that don't play exactly don't count |
| The measure | Minutes of run time per second of gameplay played exactly. Target: 1 or less. For Moss Hop that is the whole game rebuilt in at most 38 minutes |

`gp bench scratch mosshop` reports this number first, with the time split by stage, so each round shows which stage to make faster or more accurate.

### Where this goes

The 1-minute target is the first attempt. The user, 2026-10-31: "eventually we should be able to port an nes rom within a matter of hours, with a highly stable output". A full NES game has far more gameplay than Moss Hop. At 30 minutes of distinct play, 1 minute per second is 30 hours, so "hours" means a few seconds of run time per second of gameplay. The ratio comes down by deterministic methods, not by more machines or more LLM:
- reading facts from the code instead of fitting them to clips;
- clips chosen so each one checks something no other clip does;
- scoring that reuses work (the original's side cached, only affected clips re-scored, a clip stopped at its first wrong frame);
- refinement that solves for a value instead of searching for it.

"Highly stable" means the same ROM gives the same model on every run, on every machine. Each round records the ratio, and a round that makes the output less stable doesn't merge.

## From scratch, four stages

The benchmark is a run from scratch: Moss Hop's ROM, its asset map and nothing else. The tools build the model with no LLM and no person. The run is graded against the real model and the source, then repeated after every improvement. Moss Hop itself is frozen: no new game features, only what a measure needs. The work is the technique, in four stages in this order, each improving on its own:

| Stage | What it does | Measured by |
|---|---|---|
| 0. Static harvest | Build every part of the model the ROM settles, run or read (decided 2026-10-31, docs/research/rom-event-model.md): the code is found by running the ROM in gp-nes from power-on under generated and searched press recipes, and read by the static rules for behaviour; the same ROM and recipes give the same result. From it: assets, palettes and level data from the asset map; elements from dispatch tables and each one's routines; constants, tables, thresholds and periods (frame-counter masks); state machine skeletons from each routine's branches; mapping rows from the bytes each routine reads and writes. Every fact it can't settle is listed as open, with the routine and bytes that decide it | Share of the final model built statically; facts right, wrong and missing against the real model; open facts left |
| 1. Clip generation | Use every source of data (stage 0's open facts, the code matrix's branches and inputs, the ROM's tables, recordings already made) to generate the smallest set of clip recipes that settles every open fact and covers every branch and input the code shows. Smallest means fewest clips and fewest frames. A recipe that settles nothing another doesn't is dropped | Recipes and frames needed; open facts each recipe settles; branches, inputs and coverage filled; mutants caught; stable targets |
| 2. Trace harvest | Facts read from the original's traces of those clips: the values a routine actually writes, timers that reload, which inputs mattered (`gp element inputs`, the solvers' fits) | Open facts closed; facts right and wrong |
| 3. Refinement | The model is scored on the clips and corrected (fit, solvers, detectors) until clips are exact. Clips that check nothing new are dropped; a fact no clip could disprove gets one | Clips exact; rounds; facts the loop fixed; steps that needed a person or an LLM |

Each stage should leave less for the next. The more stage 0 settles, the fewer clips stage 1 needs and the less stage 3 has to search.

### Baseline from scratch, 2026-10-31

`gp bench scratch mosshop` (docs/benchmarks.md) after the audit's fixes. The start holds only the header the schema needs (schema, name, title, source, view and frame rate) and `template.toml`: no asset slots, entities, states, transitions, rules, counters, evidence or state mapping. Two runs give the same report and the same hash for each stage (result `73c264b0...`). It takes 7 seconds, because every stage after the start stops at its first tool.

| Measure | Value |
|---|---|
| Minutes per exact second | None: no second plays exactly |
| Exact seconds | 0 of 37.9 s (0 of 2,280 frames) |
| Facts right by tools | 0 of 284 (the leaves of the real model and its state mapping, documentation and evidence left out); 5 were in the start (its header: schema, name, source, view, frame rate) |
| Stage 0: tools | None reads only the ROM. `gp model derive --from code` is a source reader, excluded; it derived 0 of 184 facts from the earlier start, which had the model's shape |
| Stage 0: elements found | 0 of 8 (entity kinds, tile kinds and counters), none named |
| Stage 0: built statically | 0 of 284 (0%) |
| Stage 0: open facts | None listed by a tool; all 279 the start lacks are open |
| Stage 1: clip generation | `gp capture start` and `generate` fail ("no such input"), and `gp clips matrix` needs a state mapping: 0 recipes, 0 frames. No tool ties a recipe's trace to a model fact, so no recipe is credited with one. On the hand-written model and mapping, `gp bench clips` gives the measures below |
| Stage 2: trace harvest | `gp model propose` runs and writes nothing: it measures only mechanics the model already names. `gp element inputs` needs a scored clip and a state mapping |
| Stage 3: refinement | `gp solve` and `gp model refine --lockstep` both need a state mapping; 1 round, nothing changed, 0 of 8 baselines exact |
| Steps a person or an LLM did | The model's shape, the state mapping and the baselines' presses: without them nothing after stage 0 runs |

Before the audit the start kept the real model's shape and its mapping, with only the numbers removed. That run took 16.2 minutes (15.0 of them clip generation): `gp model propose` set 1 of 36 numbers right, the solvers kept nothing, the fit left 8 questions, and 0 of 2,280 frames played exactly.

## The measures

`gp bench clips mosshop` (`just bench-clips-mosshop`, docs/benchmarks.md) measures every row of this table, on the hand-written model and state mapping (not from scratch); two runs give the same report (result `3001e64c...`), in about 15 minutes. Every report leads with this table, graded against Moss Hop's source (games/mosshop/ground-truth.toml, made by scripts/mosshop/ground_truth.py). The baseline is round 1's measurement on main (docs/research/mosshop-testbed.md); Phase 1 fills in the blanks.

| Measure | What it grades | Round 1 | After m3/mosshop-round2 |
|---|---|---|---|
| Elements found from the ROM | Entity kinds, objects and player mechanics the code shows, against the source's list | None named: no tool grouped routines into elements | 9 of 9 parts a player sees have a code row, 8 with a clip (death's `kill_player` row has none); the code part names 24 groups |
| Branch directions found | Reachable directions found from the ROM, and how many found directions are real | 206 of 206, all real | Confirmed: 206 of 206 reachable rows (172 of 172 directions), 180 of 180 directions found are real; 202 rows with a clip in the grid after 16 driven clips |
| Unreachable told apart | Directions the level's data can never take (8 by the source), marked unreachable, not "hard" | 0 of 8 | Confirmed: 8 of 8 on seed 1, none wrong; 8 of 8 and one wrong on seed 2 |
| Inputs found | Bytes the routines read that matter, against the source | 79 of 84 | Confirmed: 84 of 84 in the matrix and in the grid's code part |
| Coverage filled | Fillable cells (a direction while an input varies) with a clip | 803 of 904 (89%) | Confirmed: 899 of 977 (92.0%) in the matrix; the grid's code part 884 of 1,004 cells after 16 driven clips |
| Clips needed | For that coverage | 12 | 18: 16 recipes (2,125 frames) and 2 of the run's clips |
| Wrong models caught | Mutants of the real model that some clip fails, per kind of mutant | Not measured (the bench came after round 2) | 45 of 71 by the run's 87 clips: input 5 of 7, constant 27 of 41, transition 8 of 9, systems' order 1 of 8, period 4 of 6; a dropped mapping field isn't a mutant the existing code can make. The 26 uncaught are listed with their routine and branch in the report |
| Stable targets | The same targets across seeds and across wrong models | the matrix yes; the product grid no (84 model cells, none for the ? block or coins) | Confirmed: matrix cells Jaccard 1.0 across seeds 1 and 2 and on a wrong model (gravity 0.1875); the grid's code cells Jaccard 1.0 on the wrong model |
| Hand or LLM steps | Steps of the run that needed a person or an LLM | 0 of the run's steps; 3 inputs by hand: the state mapping, the model, the baselines' presses | The same: 0 of the bench's 8 steps; the same 3 inputs by hand |

## Wrong models as test cases

A clip set is good if it fails every wrong model. Mutating the real Moss Hop model gives hundreds of wrong models with known faults, and the mutation code already exists (the clip set's pruning uses it). Each mutant kind is a way extraction can go wrong:
- drop an input from a rule;
- change a constant;
- delete a transition;
- swap two systems' order;
- drop a mapping field;
- change a period.

The measure is how many of each kind some clip catches, and which code branch the uncaught ones live in. An uncaught mutant points at a hole in coverage.

`gp model mutants DIR` writes the mutants for any model, and `gp capture prune RUN --mutants FILE` scores them and prints the measure: by kind, the uncaught with the model path each changes, each clip's catches and the ones only it makes, and the smallest set of clips by greedy cover that catches every mutant some clip catches (docs/benchmarks.md, "The mutant measure"). On a clip the model plays exactly, a mutant's run stops at its first split, with the original's side from the clip's tape, so the emulator doesn't run.

On Donkey Kong JU's corpus this catches the same mutants on the same clips as scoring each clip in full, about 9 times faster (docs/benchmarks.md has the check).

Donkey Kong JU, measured on 2026-11-05 with `gp model mutants templates/donkey-kong-ju` and `gp capture prune --mutants` on the corpus's 179 clips (116,443 frames as the run's clips give them), every clip played exactly by the model:

| Kind | Made | Caught | Couldn't be made |
|---|---|---|---|
| input | 116 | 45 | 0 |
| constant | 699 | 361 | 4 |
| transition | 81 | 54 | 0 |
| order | 8 | 3 | 0 |
| period | 37 | 18 | 0 |
| All | 941 | 481 | 4 |

The 4 that couldn't be made move a ladder or surface table index past its table (constant-609, 611, 638, 645). Of the 456 uncaught, the largest groups are input mutants in `rules.game.states` (36), the `ext_params` of the hammer (21), idle (19) and barrel (14) hooks, tile kinds (13) and the level's counters (11); the list is in prune's JSON under `measure.uncaught`. Every clip catches at least one mutant; 24 clips catch one no other clip does, 70 such catches in all (clear-25m 001 has 17). A greedy cover of 35 clips and 31,598 frames catches all 481. Scoring the 941 mutants took 11,145 s with 8 jobs on the gate's published build (gp-3b6e33717909); the comparison, not the interpreter, takes most of it (a profile of one mutant: the model's mapped values 57%, the original's 31%, the interpreter's step 9%).

## Ideas: logic instead of search

Round 1 fills cells with recording windows and a seeded random search aimed at a branch. These ideas replace search with reading the ROM's code. Each works from the ROM alone.

| Idea | The logic | Improves |
|---|---|---|
| G1. Elements from dispatch tables | Games pick an element's routine by an ID byte through a jump table. Find the indexed jumps in the code and list one element per table entry. That gives the element checklist without labels or source | Elements found |
| G2. Starts built from a branch's test | For a direction no recording takes, read the branch's test (a byte compared with a constant). Slice back to what sets that byte. If it's the controller, pick the presses. If it's state the mapping carries, build a start with that value, then check that both games accept it | Directions filled, clips needed |
| G3. Unreachable by data | Follow where the tested byte's values come from. If they only come from level tables and constants in the ROM, and none of those values passes the test, the direction is unreachable, not hard | Unreachable told apart |
| G4. Inputs from what branches test | A byte matters if some branch tests it or it indexes a table, even when every recording holds one value of it. Key input columns on the instruction's operand, including indexed ones | Inputs found |
| G5. Boundary values | Each compare in the code (`CMP #n`) names a boundary. Aim clips at n-1, n and n+1 for the byte it tests. Most timing and threshold faults sit at those values | Wrong models caught |
| G6. Paths, not frames | Describe a clip by the branch outcomes it takes in each routine. A clip that adds no new path is redundant. Keep the smallest set that covers every path and catches every mutant | Clips needed |
| G7. Grid from the code | The product grid's targets come from G1 to G5, not from the model, so they can't move when the model is wrong | Stable targets |

## Phases

| Phase | Work | Exit test | Limit |
|---|---|---|---|
| 0. Main green | Done 2026-10-31: m3/mosshop-round2 merged after a passing merge-check, 73 budgets, 0 over | `just check` passes on main | done |
| 1. Measure | `gp bench clips mosshop` for clip generation, then `gp bench scratch mosshop` for all four stages from scratch. No new ideas built | Runs twice with the same report; baseline table filled in here | 1 lane |
| 2. Build the ideas | One idea per round, in the order the baseline shows the biggest gap; rerun the bench after each | Each round raises its measure and makes no other worse. A round that gains nothing ends the phase | 2 lanes; check in with the user after 4 rounds |
| 3. Next | Decided with the user | | |

### Phase 2, stage 0: the static harvest

`gp model static ROM --assets MAP` (lane m3/static-harvest) builds what Moss Hop's code settles without running it, from the ROM and the asset map only. The reader is `gp_nes::harvest`: an abstract interpretation of every routine from the vectors, each call followed in its caller's state, on the shared code map `gp_nes::codemap` that `gp clips matrix --labels rom` also uses. Rules for patterns NES games share (a frame counter, a controller read, a state byte, slot arrays, tile probes, 16-bit speeds) turn its events into elements, dispatch, state machines, constants, mapping rows and model facts. Each fact cites its instructions, and each open fact names the routine and bytes that decide it. `--grade games/mosshop/static-key.toml` grades it (`crates/gp-cli/src/static_grade.rs`, the grader the bench's stage 0 calls); the key names the harvest's names (`slots_20`, `state_05`, `s0` to `s3`) in the real model's terms and is read by the grader only.

Each round graded against Moss Hop's source, 2026-10-31. Numbers are the real model's 36 behaviour numbers, as `gp bench scratch` counts them. Shape facts are its other leaves the harvest states (65 in the model). Wall time is the harvest alone, debug build; the whole command takes 0.14 s.

| Round | Change | Elements found | Routines placed | Numbers right, wrong, missing | Shape right, wrong | Transitions | Open facts | Share built statically | Wall time |
|---|---|---|---|---|---|---|---|---|---|
| 1 | The reader and the first rules | 11 of 11 (13 elements) | 32 of 37 | 21, 0, 15 | 6, 1 | not graded | 3 | 58% of numbers | 0.12 s |
| 2 | A written place holds its own symbol, so probe offsets survive the frame's moves; argument bytes found by value, not by scratch | 10 of 11 (12 elements) | 30 of 37 | 29, 0, 7 | 9, 0 | not graded | 2 | 81% | 0.12 s |
| 3 | High bytes told from counters by the wrap test; timers by decrement; a digit split isn't a counter's compare | not recorded | 32 of 37 | 32, 0, 4 | 11, 0 | not graded | 2 | 89% | 0.12 s |
| 4 | Two contents of one place meet as its symbol; an add commutes, so the sprite row keeps the y | 11 of 11 | 32 of 37 | 36, 0, 0 | 11, 0 | not graded | 1 | 100% | 0.11 s |
| 5 | Routines placed by the calls the reader followed; a tile kind per element; uncited thresholds and stored constants become open facts | 11 of 11 (11 elements) | 36 of 37 | 36, 0, 0 | 11, 0 | not graded | 17 | 100% | 0.12 s |
| 6 | The level's slots and properties, roles, frozen states, machine starts; power-on and level-start writes aren't transitions | 11 of 11 | 36 of 37 | 36, 0, 0 | 23, 0 | 6 of 6, 0 extra | 17 | 100% | 0.12 s |
| 7 | The level's row pointers; what a head hit or a pickup leaves; air control; the death state's sides | 11 of 11 | 37 of 37 | 36, 0, 0 | 29, 0 | 6 of 6, 0 extra | 17 | 100% of numbers, 65 of 101 leaves | 0.10 s |
| 8 | Generality: 73 non-commercial ROMs and a second test program (below); four rules that fit only Moss Hop fixed | 11 of 11 | 37 of 37 | 36, 0, 0 | 29, 0 | 6 of 6, 0 extra | 17 | 100%, 65 of 101 | 0.10 s |
| 9 | The open facts graded; open thresholds name every byte their routine reads, loop counts and caller registers said as such; a frozen slot's touch does nothing; spawns name their entity | 11 of 11 | 37 of 37 | 36, 0, 0 | 32, 0 | 6 of 6, 0 extra | 16, all 16 naming their routine and bytes right | 100%, 68 of 101 | 0.10 s |

Round 8 ran the harvest on every non-commercial ROM there is: our four `gp_nes::testrom` programs, our MMC1 and MMC3 programs, and the 66 free test ROMs `gp testroms` downloads (blargg's and others', `$GP_HOME/work/test-roms`). All 73 finish, the slowest in 0.15 s, and none gets an entity, a hitbox or a camera it doesn't have. They showed four rules that fit only Moss Hop, each fixed:

| Found on | What was wrong | Fix |
|---|---|---|
| `gp_nes::testrom`, which reads both controllers | `input.pad` named controller 2's byte | Controller 1 is the byte shifted in from $4016's bits, not $4017's |
| The same | The reader remembered a value written to $4016 and read it back | Registers ($2000 to $401F) are never remembered |
| nestest | A jump table was listed once per activation, and a jump table's entries were never read | One per site; each entry is read as a tail call from the jump |
| The second test program | A byte stepped both ways was called a counter; a slot's state written beside the game's was taken for a level start; the game's state needed a compare chain that calls routines | Counters count one way; a level start writes the state the reset code writes; the frame loop's most-tested state byte stands in |
| Audit variant B, Moss Hop with its state compare chain replaced by `asl a; tax; lda tbl,x; ... jmp (ptr)` | The reset had just written the state byte, so the index was a known constant and the table read became one byte; the jump went nowhere, the frame loop wasn't seen as a loop, and 19 of 37 routines were found | A value read from a ROM table at an index keeps the table and the index byte (with how many times it was shifted left) even when the index is a constant. An indirect jump through a table is a branch to each entry with the guard "index byte == value", so a frame loop that dispatches this way reads as one with a compare chain: same elements, machine and numbers as variant A. Each entry starts a routine; one that never returns and that the frame loop goes on into is read as the loop's own code, with its guards |

The second test program (`test-roms/static/static.s`, tested in `crates/gp-cli/tests/model_static.rs`) is laid out unlike Moss Hop: UxROM (mapper 2) with 64 KB and every routine in the fixed bank, RAM at other addresses, the controller shifted in with ROR (A in bit 0), the mode tested by CMP and BNE, and each object's routine picked by an RTS jump table on its kind. The harvest gives each of its 17 stated facts (frame counter $40, the pad bytes, x and y, the jump of -3.5, gravity 0.125, the fall limit 3, four slots, the crawler kind's period 4 and step 1, the stun's 45 frames, the lost mode's 75, both machines' starts), and one element per table entry (`slots_400_k0`, `slots_400_k1`); every call it makes is into the fixed bank or a bank it selects, so no call is left unread.

Round 9 graded the open facts against the ground truth: each names a routine whose own instructions read or write every byte it names (16 of 16). The 33 shape leaves still missing are names the model chose:

| Kind | Leaves | Why the code can't settle them |
|---|---|---|
| Event names (`on`, `event`, `pit_event`) | 13 | The code shows the branch and the write; the name is the model's |
| Transition targets (`to`) | 10 | Graded as transitions instead (6 of 6); a `to` leaf only repeats the name |
| `do` lists and `stops` | 6 | The effects are in the transitions' guards and the frozen states; the vocabulary is the model's |
| Tile kind and action names | 4 | The asset map names the metatiles "? block" and "coin"; the model calls them `coin_block` and `coin` |

Round 9 is where it stops: what's left is the model's vocabulary, and a round that only renamed would gain nothing measured.

What stays open (16 facts after round 9, each with its routine and byte): the player's facing and standing flags at the level's start and on a turn, the slots' patrol direction, a loop's count of 16 cells, the right edge of the world (`cmp #$F0` with the high byte, 496 pixels, which the model doesn't state), the redraw queue's length (8), the camera's clamp at the level's end, the digit split's 10, and the level-start resets. A wrong fact is worse than an open one, so a rule that can't decide writes an open fact (the death state's `tile_collision` stays open if any probe path doesn't say the game's state).

The harvest reads the ROM, not the source: a copy of the ROM with the jump speed and gravity patched reads -6 and 0.1875. Running it twice gives byte-identical output (`crates/gp-cli/tests/model_static.rs`). Boards: mapper 0 in full; UxROM (2), MMC1 (1, PRG mode 3) and MMC3 (4, mode 0) by the banks the code selects, each address in a switched region read with the bank a path has mapped there (`03:8123`), and a call into a bank no path selects left as an open fact; other banked boards have their first and last 16 KB read and an open fact for the rest. Moss Hop has no jump table, so G1's table rule is tested on made-up ROMs (`gp_nes::harvest` tests): a three-entry table gives three elements, a frame loop that jumps through a table of words by a shifted state byte gives that byte's dispatch and machine, and an `RTS` table indexed by the byte unshifted gives its machine.

The bench's stage 0 can call `crate::model_static::harvest_rom` and `crate::static_grade::grade`, or `gp model static ROM --assets MAP --model DIR --names KEY` for a model.toml in the real model's names.

## Redundant work and what runs in parallel

Found on 2026-10-31.

| Redundant now | Fix |
|---|---|
| Two ROM code walkers: `Routines::from_rom` (serve/steer.rs) for the code matrix, and the one stage 0 would need | One shared code map in gp-nes (routines, call graph, branch sites with tests and operands, indexed reads), cached by ROM hash. The static-harvest lane moves it |
| The original is replayed separately with a tracer by the matrix (steer.rs), dense clips, `gp progress --measure`, `gp element inputs` and the RAM-role classifier | One trace per recording, cached by ROM and recording hash, that all of them read. Next lane after the code map |
| Two graders against Moss Hop's ground truth (the bench and the static harvest) | One grading module both call |
| The bench reruns 20 minutes to prove it is deterministic, every round | Hash each stage's output; the full second run only before a merge |

| Can run in parallel | Why it's independent |
|---|---|
| Stage 0 (static harvest) and stage 1's code matrix | The matrix needs only the code map and the recordings, not stage 0's facts. Stage 0's open facts only reorder and trim stage 1's recipes |
| Static harvest per element | Each element's routines are analysed on their own |
| Recording clips, scoring clips, the mutant runs | Each clip and each mutant is independent; they already use the worker pool |
| The bench and the static-harvest lane | Different files. They meet only at the grader and the stage 0 hook |

The stages themselves stay in order for data (0, then 1, then 2, then 3), but within each stage the work splits by element, by clip or by mutant.

## Audit of stage 0, 2026-10-31

An adversarial audit (read-only, by a fresh agent) found stage 0 not met. The bench's stage 0 numbers are not valid until these are fixed:
- The tool it measured, `gp model derive --from code`, reads the assembly source, not the ROM. It derives 0 of 184 facts from the bench's start model.
- The grader scored the start model instead of the tool's output.
- 64% of the "built statically" denominator was copied in: entities, states, transitions and the hand-written mapping.
- The open facts and their deciding bytes came from the answer key.
- Placeholder values of 0 or 1 counted as right.
- The code matrix defaults to the game's labels file.

What passed:
- The new ROM reader (m3/static-harvest) is deterministic, takes 0.1 s, and shares one ROM walker with the code matrix.
- It produces no facts yet.

The fixes went to both lanes. Stage 0's honest baseline is 0% built from the ROM.

## Second audit, 2026-10-31: ROM to a deterministic dataset

A second adversarial audit (read-only, fresh agent) found the workflow short of its purpose. Verdicts:

| Goal | Verdict |
|---|---|
| Deterministic data | Partly: stage 0 and the export's tables repeat; clip generation changes with the thread count, and the export with the build |
| Repeatable by someone else | Not met: the asset map, templates, hand-played baselines, unpinned ca65 and env vars are hidden inputs |
| ROM-only | Not met: the asset map was written from the source, and stage 0 was run with an answer key (`--names games/mosshop/static-key.toml`) |
| Generalizes beyond Moss Hop | Not met: moving RAM or OAM gives 32 of 36 numbers; an ordinary `jmp (ptr)` state table gives 0 of 36 |
| Guards enforced | Not met: 1 of 11 existed, and the merge gate could be bypassed six ways |
| Ready for Hugging Face | Partly: the format is close; the export is of the hand-written template, not the pipeline, without per-row provenance |
| Speed target | Not met: the from-scratch run stops after stage 0 and plays 0 frames exactly |

Stage 0's honest numbers, without the key: 27 of 284 facts; from the ROM alone without the asset map, 24 of 36 numbers and 9 of 11 elements. The round table above that reached 36 of 36 used the key; it is kept as a record and is not a result.

The merge gate is fixed on m3/main-guard: commits are read at the start, the record holds the merged tree, the base must be main's tip or differ from it only in Markdown, and a reference-transaction hook refuses fast-forwards, squashes, cherry-picks and code commits straight on main (11 cases tested in a scratch clone). The rest went to the static-harvest and bench-guards lanes.

## Guards that keep the work honest

Added after the stage 0 audit. Each guard is a check that fails `just merge-check`, so the rules hold without anyone remembering them.

| Guard | Check | Stops |
|---|---|---|
| Sealed inputs | The bench runs each stage tool in an empty folder holding only the ROM and the asset map, under a sandbox that denies reads elsewhere | Tools reading source, labels, templates or the answer key |
| Scramble test | Each stage runs again with source, labels and answer key scrambled or removed; its output must be byte-identical | Indirect leaks |
| Safe defaults | Labels or source only with `--with-source`; the bench refuses that flag | Leaky defaults |
| Null tool | A tool that outputs nothing scores 0 on every measure | Credit for copied, start or placeholder facts |
| Oracle tool | A tool that outputs the real model scores 100% | A grader that undercounts |
| Wrong tool | The real model with values changed: every change counts as wrong | Loose tolerances, name-only matches |
| Empty start | The start model has no entities, states, transitions, mapping or numbers | Shape copied in |
| One grader | Only the grading module reads the answer key; a test checks no other module does | Graders doing the stage's work |
| Ratchet | The bench's measures are budgets; a merge that lowers one or raises wrong facts fails | Regressions |
| Phase-exit audit | An adversarial audit by a fresh agent before a phase is declared done, findings recorded here | Drift the checks don't cover |
| Tool-made reports | Status reports quote `gp bench report`'s table | Numbers nobody measured |

On m3/bench-guards (docs/benchmarks.md, "The guards"), every guard but the phase-exit audit is built. Sealed inputs use `sandbox-exec` on macOS and the sealed folder alone elsewhere. The scramble test plants reversed copies of the source, the labels and the answer key in the sealed folder; all four stages' outputs stay byte-identical. The grader's self-test passes: the null tool scores 0 of 284 facts, the oracle 284 of 284 with 2,280 of 2,280 frames exact, and the wrong tool's 48 changes are all graded wrong. `just check` runs the scratch bench and holds its measures to budgets.toml. The clip measures are held to budgets-bench.toml by `just bench-clips-mosshop` in `just check-cycle`, since that bench takes 15 minutes. Two source readers keep reading labels by default and are noted rather than gated: `gp element inputs` names what it slices by them, and `gp model derive` reads the source by design.

## How work runs

- Each lane brief names its phase, its idea and its exit test, or it doesn't start.
- The tools read only the ROM. The source is used only by the grader.
- Merge only through `just merge-check`, and only when no measure gets worse.
- Research never waits on a merge-check. A lane runs it in the background and keeps researching. While iterating it runs only the crates it touches and the bench, and it runs the full check once, before merging (the user, 2026-10-31: the other work was blocking the extraction research).
- A lane never grades its own tool. Graders, answer keys and benchmarks are another lane's work, fixed before tool work starts; a tool lane can't change them.
- Held-out tests decide progress. Moss Hop is the practice set. The audit's variant ROMs (and any new benchmark ROM) are graded only by the coordinator or an audit, never by the tool lane while it iterates. A result counts when it holds on the held-out set.
- No number reaches the user unless the coordinator ran the check or a scripted guard on main produced it. An agent's report alone is a claim, not a result.
- Reports show the measures table, then decisions.
- If two rounds in a row gain nothing, stop and re-plan with the user.

## Lanes

| Lane | Phase | Status |
|---|---|---|
| m3/mosshop-round2: product grid from the code matrix, Moss Hop budgets | 0, and G3, G4 and G7 in part | Merged |
| m3/mosshop-bench | 1 | Re-scoped to `gp bench clips mosshop` and the measures above |
| m3/static-harvest: `gp model static`, stage 0 (G1 and the static parts of G3 to G5) | 2, stage 0 | Seven rounds, stopped at 36 of 36 numbers, none wrong (the table above); not merged |

## Decided

- The ROM's code drives clips and coverage. Moss Hop's source only grades.
- SMB is left alone: no runs, fixes or benchmarks on it (the user, 2026-10-31).
- Focus now (the user, 2026-10-31): refine and optimize the static harvest (stage 0), in rounds graded against the source. Wrong facts count worse than open ones.
- The goal is the technique, not Moss Hop. Moss Hop is frozen. Clip generation, fact harvesting and clip refinement all keep improving, measured from scratch (the user, 2026-10-31: "refining the technique of starting mosshop from scratch with better strategies").
