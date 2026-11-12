> A research record brought from game-printer (docs/research/dense-clips.md) with the commands it describes. Paths and commands are the old repo's where they differ: `gp serve` and `gp mcp` are retired (the watch routes are on the mechanic servers, docs/watch.md), capture runs come back in wave 7 of the port, and `gp capture dense` and `carry` read a game's package for now (docs/capture.md).

# Dense clips: fewer clips that each exercise more of the game

A research report of 2026-10-30, on branch `research/dense-clips`. The user, 2026-10-30: "i want data sets that are more dense and offer a better outcome without nearly so many clips to study".

It reads against [the clip set report](clip-set.md) (the kept set and the mutants), [the clip corpus report](clip-corpus.md), [the deterministic inference note](deterministic-inference.md), [the corpus](../corpus.md) and [recording](../recording.md). The prototype is `gp capture dense` (`crates/gp-cli/src/serve/dense.rs`), with a coverage map in `gp-nes` (`crates/gp-nes/src/coverage.rs`). It writes only into a scratch run.

## Summary

| Question | Answer |
|---|---|
| Can 10 to 15 clips match the kept set? | Yes. 15 clips of 1,028 frames catch all 23 mutants that any clip catches and hold 139 model parts, where the 91 clips `gp capture prune --guard` keeps today take 15,867 frames, catch the same 23 and hold 127. The 15 also run more of the original's code (7,331 ROM addresses against 6,597) |
| What makes a clip dense | It runs code of the original that the other chosen clips don't: CPU addresses, the direction each branch goes, and the values the mapped measures and enemy slots take. 12 clips of 60 frames chosen on that alone run more ROM code than all 169 scored clips of the run |
| Where dense clips come from | Two sources, both from the original alone: a seeded coverage-guided search of presses at the run's starts (as AFL and libFuzzer search inputs), and windows of the three TAS movies the run already has, with the movie's own presses. Together they beat either one |
| Is code coverage enough? | No. Chosen on code alone, 12 clips catch 17 of the 23 mutants and hold 122 parts. The recommended set weighs each mutant caught first, then each model part, then code, with each clip priced by its frames |
| Method chosen | Search and TAS windows for candidates, then the weighted cover over the candidates and the run's own clips (`gp capture dense --pick`). 10 of the 15 are dense clips (5 searched, 5 TAS windows); 5 are clips the run already had |
| Time per full score | The 15 score in 2 s at 8 jobs and 6 s at 1 job, from an empty cache. The kept 91 take 15 s and 43 s, all 169 take 13 s and 96 s (section 4) |
| Reproducible | A dense clip is a recording by hash and frame plus press spans (`generated/dense-clips.toml`). The search reads its random numbers on one thread and its batches' results in order, so the same seed, ROM and run give the same clips at any `--jobs` (checked at 8 and 2 jobs) |

## The run

`smb-side-by-side-2`, Super Mario Bros. (PRG 0), cloned on 2026-10-30 into a scratch `GP_HOME` with the store, results and play folders (APFS clones of the real files, the real store read only), `GP_ROM_DIR=/Users/alex/Documents/roms/nes`, a release build of this branch and `--jobs 8`. Nothing in the user's run or store was written.

The run had moved on since the clip set report: the model is `eb710182bc15`, the queue scores 169 clips (167 generated, 2 of the user's save points) and 264 wait. `gp capture prune --guard` with [clip-set-mutants.toml](clip-set-mutants.toml) on main keeps 91 of them (the cover, plus the clips their `requires` need). That is the kept set compared here, in place of the 67 the brief expected. 23 of the 30 mutants make some clip worse.

## 1. The density signal: code coverage of the original

### What the emulator reports

`gp_nes::Nes::step_with` already calls a closure before every instruction with the bus, which is how the instruction tracer (`gp trace`) reads the CPU. The coverage map uses the same hook, so no change to `tetanes-core` was needed and the runtime identity is unchanged. `NesRuntime::step_covered` runs one frame that way, and a test checks that a covered frame ends in the same state as an uncovered one.

A map is a bitset of 262,144 features:

| Range | Feature |
|---|---|
| `0x0_0000-0x0_FFFF` | An instruction at this CPU address ran |
| `0x1_0000-0x1_FFFF` | The conditional branch at this address was taken |
| `0x2_0000-0x2_FFFF` | It fell through |
| `0x3_0000-0x3_FFFF` | A measure value: each mapped field's value in a bucket (small values exact, larger ones by power of two, with the sign, as AFL buckets hit counts), and each enemy slot's (kind, state byte) pair, hashed |

A branch's direction is read from the next instruction's address: the branch's target means taken, the next instruction means fell through, anything else (an interrupt in between) leaves it unjudged. Addresses are the CPU's. Super Mario Bros. is mapper 0 and has no banks; for a board that switches banks, the key should add the bank, which the event stream's bank switch events (docs/recording.md) give.

AFL counts edges, `shared_mem[cur_location ^ prev_location]++`, in buckets of "1, 2, 3, 4-7, 8-15, 16-31, 32-127, 128+" (AFL's technical details, `docs/technical_details.txt` in google/AFL). Branch directions are the 6502's edges that matter: every other edge is a fall-through or a fixed jump, so addresses and branch directions together hold what AFL's tuples hold, without hit counts. Hit counts were left out because a clip's length changes them without the clip doing anything new.

### What it measures on the run

Each of the run's 169 scored clips, run on the original from its start (`generated/dense/coverage.json`):

| Clips | Frames | ROM addresses run | Branch directions | Measure values |
|---|---|---|---|---|
| All 169 | 14,631 | 6,737 | 1,701 | 275 |
| 12 dense clips chosen on code | 782 | 7,410 | 1,763 | 257 |

The 169 clips repeat each other's code: most start standing at a level's start and press one or two buttons, so the same routines run. A single 60-frame dense clip, World 3-1 with B and Right held and A pressed twice, runs 4,090 ROM addresses, 61% of what the 169 run together.

Code coverage is deterministic and needs no model or language model. It says what of the game a clip reaches; it doesn't say whether the model gets it wrong there, which the mutants say.

## 2. Where dense clips come from

### Coverage-guided search

The search follows AFL's loop: run an input, keep it if its trace has something new, and mutate kept inputs. libFuzzer's documentation says the same: if a mutation "triggers execution of a previously-uncovered path in the code under test", "that mutation is saved to the corpus for future variations". FuzzBench (google.github.io/fuzzbench; Metzman et al., ESEC/FSE 2021) compares fuzzers by the code coverage they reach on real programs, the measure used here.

| Step | Here |
|---|---|
| Starts | The run's 39 starts the rebuilt game can begin at, from their snapshots: level starts and the corpus's contexts (by a pit, a wall, a block) in 29 levels |
| Seeds | At each start: nothing for 60 frames, Right for 180, Right and B for 180, Right and B with A held 20 of every 40 frames, Left for 60 |
| Mutations | 1 to 4 of: a button toggled over a span of up to 40 frames, a chord held over a span, idle frames put in or taken out, the length changed, a splice with another kept sequence from the same start. Only A, B and the directions; no opposite directions together |
| Parents | Half the time any kept sequence, half the time one of the last 16 kept (AFL also favours new queue entries) |
| Keep | A sequence is kept when its map has a feature no kept one has. It is then cut to 12 frames past the last frame that found something new, and to 60 to 180 frames |
| Budget | 4,000 sequences per seed, in batches of 64 scored on the helper threads; the results are read in the batch's order |
| Random numbers | SplitMix64 (Steele, Lea and Flood, OOPSLA 2014), seeded by `--seed` |

Seed 1 kept 177 of 4,000; seed 2 kept 168. The search took 190 s and 250 s at 8 jobs.

Mid-level starts are the open problem. A start the rebuilt game can begin at must have no enemy in a reused slot, since the mapping doesn't carry enemies into the model yet (`gp_interp::starts`, "enemies aren't carried over yet"). That rules out most frames in a level, which is why 75 of the run's 114 starts can't be used. A searched clip reaches mid-level from a level start within its 180 frames instead.

### TAS windows

The run's recordings are three TAS movies (docs/corpus.md): 4313M "all items" (71,437 frames), 2676M "warpless, walkathon" (91,303) and 1715M "warps" (17,867). One pass over each movie's input log records each frame's map, and at every frame the rebuilt game can begin at (on the ground two polled frames running, playing, no enemy in a slot, at least 30 frames after the last such frame; `gp_interp::starts::cant_start_here`), it opens windows of 60, 120 and 180 frames with the movie's own presses. The model's frame there is counted as the corpus scan counts it (polled frames since the mapping's start). The three movies give 1,227 windows in about 175 s. The movies are read from the run's copies of their input logs; nothing is written to the store.

The same rule about enemies limits the windows: they are where a level starts, or where the TAS has cleared the screen. Within those, a TAS is dense: 12 windows chosen on code run 7,278 ROM addresses in 780 frames.

### Against each other

Seed 1, greedy on code, a clip's price its frames (`--keep 12`, the command's report):

| Candidates | Clips | Frames | ROM addresses | Branch directions | Measure values |
|---|---|---|---|---|---|
| Search | 12 | 930 | 6,863 | 1,646 | 252 |
| TAS windows | 12 | 780 | 7,278 | 1,712 | 232 |
| Both | 12 | 782 | 7,410 | 1,763 | 257 |
| Search, 30 clips | 30 | 3,097 | 7,748 | 1,981 | 317 |
| TAS windows, 30 clips | 30 | 2,280 | 8,295 | 2,033 | 300 |

Seed 2's search gives 7,059 ROM addresses for 12 clips and 7,589 for both, so the result doesn't rest on one seed. The windows reach more code than the search (the movie plays tricks a random search rarely finds), and the search reaches measure values the movies don't (Mario standing, turning, walking back). Both together beat either.

## 3. Discriminating clips: mutants and model parts

The dense clips were written into a scratch run beside the run's own and scored with `gp capture prune --mutants --guard` on the same model and mutants. Of 70 dense clips (seed 1), 60 fail on the model, against 43 of the run's 169: they reach more of what the model gets wrong. Together the dense clips catch 21 of the 23 mutants. They miss `path.wait_near_wrap` (only two World 8-1 walks catch it) and `run.skid_turn_with` (four single-button clips), and catch `run.face_on_ground` in 21 clips where the run's catch it in 1.

Model parts are prune's `part`, `mechanic`, `body` and `event` features: a measure the original moves with its owner, a mechanic, a body, a body's event. Gaps, levels and the kind of start are left out, since they say where a clip is rather than what of the model it runs. The 239 scored clips hold 182 parts between them; the run's 169 hold 127.

Each set below is measured the same way: ROM addresses from the coverage maps, parts and mutants from the two prunes. "Every clip" means the run's 169 and seed 1's 70 dense clips together.

| Set | Clips | Frames | ROM code covered | Model parts covered | Mutants caught |
|---|---|---|---|---|---|
| Every scored clip of the run | 169 | 22,806 | 6,737 | 127 of 182 | 23 of 23 |
| Kept set (`gp capture prune --guard`, today) | 91 | 15,867 | 6,597 | 127 of 182 | 23 of 23 |
| Search, greedy on code | 12 | 930 | 6,863 | 113 of 182 | 16 of 23 |
| TAS windows, greedy on code | 12 | 780 | 7,278 | 109 of 182 | 16 of 23 |
| Search and TAS, greedy on code | 12 | 782 | 7,410 | 122 of 182 | 17 of 23 |
| Search and TAS, greedy on code | 15 | 1,142 | 7,911 | 133 of 182 | 19 of 23 |
| Every clip, greedy on code | 15 | 1,003 | 7,722 | 130 of 182 | 17 of 23 |
| Every clip, greedy on parts and mutants, equal weight | 15 | 1,071 | 6,951 | 154 of 182 | 20 of 23 |
| Dense only, weighted (mutant 1000, part 30, code 1) | 15 | 1,202 | 7,707 | 146 of 182 | 21 of 23 |
| Every clip, weighted, 12 clips | 12 | 805 | 6,753 | 118 of 182 | 23 of 23 |
| Every clip, weighted, 15 clips (the recommendation) | 15 | 1,028 | 7,331 | 139 of 182 | 23 of 23 |

The frames of the run's sets include the two save points' lead-ins from power-on (8,554 frames), which the dense clips don't need, since each starts from a snapshot.

Code alone finds where the game does many things, but a mutant is often one comparison in one routine on one frame, so code coverage misses about a quarter of them, as statement coverage misses faults that mutants find (Just et al., FSE 2014, cited in the clip set report). Parts and mutants with equal weight let a clip that holds many parts crowd out the one clip that catches a rare mutant. Weighting each mutant far above a part, and a part far above an address, makes the greedy catch every mutant first, then add parts, then code; with prices by frames it stays short.

### The 15

`gp capture dense dense-s1 --pick prune-s1.json --keep 15`, in the order taken:

| Clip | Frames | Kind |
|---|---|---|
| World 8-2 start, searched presses | 60 | dense, search |
| World 6-2, jump held 15 | 55 | the run's |
| World 3-1 start, searched presses (B and Right, two jumps) | 60 | dense, search |
| World 4-1, B tapped | 41 | the run's |
| World 7-3 (the model's `w2_3`, whose area it reuses), 4313M from frame 58,631 | 60 | dense, TAS |
| World 2-1, 4313M from frame 10,013 | 60 | dense, TAS |
| World 7-1 by the pit, searched presses | 60 | dense, search |
| World 4-4, 4313M from frame 35,014 | 60 | dense, TAS |
| World 8-1, walk then jump | 160 | the run's (the one that catches `path.wait_near_wrap`) |
| World 3-3 start, searched presses | 69 | dense, search |
| World 7-3, 4313M from frame 57,993 | 60 | dense, TAS |
| World 2-4, nothing pressed | 60 | the run's |
| World 8-4, 4313M from frame 69,314 | 60 | dense, TAS |
| World 5-2 start, searched presses | 122 | dense, search |
| World 4-2, jump tapped | 41 | the run's |

All five windows chosen are from 4313M, the "all items" run, which collects every power-up and so passes through more of the game's states than the others.

Seed 2, scored and picked the same way, gives 15 clips of 1,078 frames, 7,358 ROM addresses, 139 of its 186 parts and 23 of 23 mutants (10 of them dense clips); 12 clips catch 22 of 23. So the result holds on another seed.

Leave one out, as the clip set report checks its guards: choosing without one mutant's weight and seeing whether the set still catches it, 20 of 23 are caught. The three missed (`path.wait_near_wrap`, `run.skid_turn_with`, `springboard.throw_late`) are each caught by 2 to 4 clips only, as `jump.land_depth` and `path.wait_near_wrap` were in the clip set report. Each fix still has to be a guard of its own.

## 4. Time

Each set scored once on the model from an empty cache (`gp capture prune` without mutants on a run holding only that set, its `score_seconds`), release build, this machine (18 cores):

| Set | Clips | 8 jobs | 1 job |
|---|---|---|---|
| Every scored clip of the run | 169 | 13.0 s, 14.3 s | 96.3 s |
| Kept set | 91 | 14.6 s (25.9 s on a first, busier run) | 42.9 s |
| Search and TAS, greedy on code | 12 | 2.6 s | 8.0 s |
| The recommendation | 15 | 1.9 s, 2.4 s | 5.7 s |

At 8 jobs the kept set is held up by its two save points, which replay 3,591 and 4,963 frames from power-on on one thread each (the clip set report measured 11.2 s for them alone). The 15 have none, so a full score is 6 to 7 times faster at 8 jobs and 7.5 times faster at 1 job. A refine round and every model change score the set again, so the saving repeats.

## 5. Making the clips again

A dense clip's recipe is written to the scratch run's `generated/dense-clips.toml` in the clip set format of the clip set report, section 4:

```toml
[clipset]
format = "0.1.0"
rom = "1d383cd6..."                 # BLAKE3 of the ROM file
generator = "gp capture dense"

[search]
run = "smb-side-by-side-2"
seed = 1
budget = 4000
min_frames = 60
max_frames = 180

[[clips]]
id = "dense.search.w3_1.start.cd04c55a"
name = "World 3-1: searched presses, 60 frames, from frame 18387"
method = "search"
presses = ["b+right@0-10", "a+b+right@11-30", "b+right@31-50", "a+b+right@51-59"]
frames = 60
coverage = "57f46e28..."            # hash of the original's code map over the clip
rom_code = 4090

[clips.start]
recording = "c8f4e908ba10"          # 4313M, imported with gp movie import
frame = 18387
model_frame = 18180
level = "w3_1"
```

No snapshot is committed or needed: the start is the recording's input played to the frame, and the snapshot written into the scratch run (`generated/dense/<id>.snap`) is only a cache of that. `coverage` is the check a regenerated clip must pass, as `curve` is for a kept clip; it depends only on the ROM and the presses. A TAS window's presses are the movie's (TASVideos' CC BY 2.0, docs/open-questions.md), so they can be committed, though the movie's URL and hash are enough.

The search is deterministic: SplitMix64 from the seed, mutations drawn on the calling thread, each batch scored on the helper threads and read back in the batch's order. Run twice with seed 5 and a budget of 600, at `--jobs 8` and `--jobs 2`, it wrote the same `dense-clips.toml`. A unit test holds that the same seed gives the same mutants.

## The command

```text
gp capture dense RUN [--into SCRATCH] [--budget 4000] [--seed 1] [--keep 15] [--write 30]
                     [--min-frames 60] [--max-frames 180] [--no-search] [--no-tas]
gp capture dense SCRATCH --pick PRUNE.json [--keep 15]
```

The first form reads RUN, searches, mines the recordings, and writes each method's first `--write` clips into SCRATCH (default `RUN-dense-SEED`): a copy of RUN's model, queue and clips without its work record's files, lockstep or stage outputs. It refuses to write into RUN, or into a run it didn't make. The second form reads a `gp capture prune --json --mutants` of SCRATCH and writes the weighted choice to `generated/dense-pick.json`. A full pass on this run took 450 s for the search and windows and about 10 minutes for the prune with 31 mutants.

## What it would take to make it the default

| Step | Why |
|---|---|
| Carry enemies into a start | The largest limit: no start with an enemy loaded can be used, so mid-level starts and most of a TAS can't. With slots translated into the model, every frame of the movies is a candidate |
| `gp capture regenerate` for `dense-clips.toml` | The clip set report's design; a dense clip is the same kind of recipe, a recording frame plus presses, checked by `coverage` instead of `curve` |
| The queue scores the dense set first | It is 15 clips; the queue today waits on `requires`, which a dense clip doesn't have |
| Re-pick when the model changes | The pick's weights use mutants and parts, which are scored on the model; the code map is not, so the candidates stay, only the choice is made again |
| A mutant per new fix | As the clip set report says: a fix the set doesn't catch is a guard it lacks |
| Bank-aware coverage | Before Mega Man 2 or Super Mario Bros. 2: key addresses by the PRG bank mapped there |
| Faster candidates | Each candidate restores a snapshot and reads every mapped measure each frame; the search runs about 20 candidates a second at 8 jobs |
