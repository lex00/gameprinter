> A research record brought from game-printer (docs/research/rom-event-model.md) with the commands it describes. Paths and commands are the old repo's where they differ: `gp serve` and `gp mcp` are retired (the watch routes are on the mechanic servers, docs/watch.md), capture runs come back in wave 7 of the port, and `gp capture dense` and `carry` read a game's package for now (docs/capture.md).

# gp-events: one event model around a running ROM

A design of 2026-10-31, on branch `m3/stage0-coverage`. Nothing here is built. It is the architecture for running a ROM, watching it, searching its inputs and keeping what was learned, for extraction (stage 0 to stage 3) and for porting (comparing the original with the rebuilt game). It draws on the stage 0 audit ([stage0-coverage-design.md](stage0-coverage-design.md)) and on an audit of gp's overlapping machinery done the same day, both summarized below with their numbers.

The user, 2026-10-31, in order:

- "audit the capabilities of gp and find out where there is overlap it might need redesign. first we designed a loop that recorded clips, a lot of this is already here i bet"
- "there was a server to render the record loop. it got really complicated. but the idea of the live render is still good. the agent was a bit disconnected from it. for it to be better it feel like we would want to be able to stream all the events out of the rom live for a collector the agent can monitor"
- "streaming could be too slow for this but the full streaming event model around a rom used for extraction and porting could be worth thinking about"
- "we might be starting from scratch kinda with this core idea"
- "think about what if we can do some kind of instrumentation that would make this more efficient, like embed a collector in a rom, or a virtual rom with a shim that is a collector"
- "we shouldn't be tied to anything do whatever is architecturally best for gp-events"

## Summary

| Question | Answer |
|---|---|
| The core idea | A run is its recipe: the ROM, a start and the presses. gp-nes is deterministic, so every event a run can produce is a function of the recipe and of how closely it is watched. Keep cheap summaries of every run, send a watcher only what is new, and make any detail again by replaying a window |
| Why it is cheap | Moss Hop runs 9,664 instructions a frame, but takes a median of 91 distinct jumps and calls a frame, 175 over a 300-frame run, and a new one in only 33 of the 300 frames |
| Where the collector sits | In a virtual cartridge: a shim around the board's mapper inside the emulator. It knows the bank of every fetch, because the mapper decides it, and calls out only at hooked addresses, at no emulated cycles. The ROM looks instrumented to the collector and untouched to the console. A collector patched into the ROM itself is rejected for originals: it changes cycles and bytes |
| What it records | Four levels: L0 footprints (bitmaps), L1 edges (a table of transfers with banks and pointer values), L2 frame events (the only level sent live), L3 detail (never stored; replayed on request) |
| What it replaces | Six instruction observers, two event streams, a code map keyed by CPU address, five press searches with their own seeds, seven start finders, nine recipe formats, and a replay per tool per recording |
| What the agent gets | A subscription to the ROM's novelty and to its own hooks and watches, beside the pipeline events it sees now |
| What porting gets | The rebuilt game emits model events in the same envelope, so the original and the port are compared event by event, frame by frame |
| Crates | `gp-events` (schema, collector, levels, store, queries) and `gp-explore` (starts, recipes, the one search and its scores), with a collector hook in the tetanes-core fork and gp-nes |
| First step | The collector as a host observer with L0 and L1, checked against `gp coverage` on Moss Hop, the practice variants and the 68 test ROMs |

## 1. What exists today

### Measured: what running finds that reading doesn't

From the stage 0 audit, release build of ce7e47d1. "Cheap" is six scripts of 3,600 frames from power-on (idle, Start presses, four seeded random), about 20 s a game.

| ROM set | Static code map | What runs show |
|---|---|---|
| Moss Hop and six practice variants (UxROM, MMC1, MMC3, a bank per mode, bank in RAM, jump engine) | Every routine | Every executed byte is inside the static map: the two agree where the source can check them |
| 68 test ROMs (idle 900 frames each) | Misses code in 50 | 12,968 executed bytes outside the map. In 48, `jmp ($000E)` through a pointer filled at run time (up to 20 targets at one site); in `official_only.nes`, MMC1 in 32 KB mode: 42 bytes mapped, 8,526 run |
| Ninja Gaiden, MMC1 128 KB | 11,255 bytes, 45 of 168 routines read | Cheap runs execute 10,270 bytes, 2,054 outside the map |
| Mega Man 2, MMC1 256 KB | 8,154 bytes, 43 of 127 read | 12,487 bytes, 7,823 outside |
| Kirby's Adventure, MMC3 512 KB | 2,238 bytes, 44 of 45 read | 27,615 bytes in 19 banks, 25,572 outside |
| SMB2, MMC3 128 KB | 14,500 bytes, 38 of 194 read | 10,516 bytes, 5,076 outside |

### Measured: the clip loop on one run

`gp bench clips mosshop --work DIR` built a capture run of Moss Hop on its hand-written model and mapping (13.1 s), then each tool ran on that run with seed 1.

| Tool | Result | Time |
|---|---|---|
| Generated corpus (`gp capture generate`) | 87 clips | Inside the run's build |
| Code matrix (`gp clips matrix --labels rom`) | 16 recipes, 2,125 frames; 899 of 977 fillable cells; 172 of 172 reachable branch directions | 73.7 s for two seeds |
| Grid (`gp capture grid --code`, `--drive`) | 16 clips made; 884 of 1,004 cells | 46.0 s |
| Dense (`gp capture dense real --budget 500 --seed 1`) | 13 clips, 1,444 frames, reach 765 CPU addresses and 170 branch ways; the run's own 19 clips (2,802 frames) reach 762 and 168 | 6.5 s |
| Steer (`gp capture steer real --seed 1`) | 0 clips: no routine is left unrun, and its start rule needs a "ground frame", a model notion | 2.6 s |

Each tool answers "which presses, from which start, reach something new" with its own search, its own seed, its own code map and its own recipe file.

### The overlap

| Job | Copies | Where |
|---|---|---|
| Watch every instruction | 6 | `gp_nes::coverage` (dense, steer, matrix; by CPU address), `gp_coverage::pass` (`gp coverage`; by PRG offset), `gp_coverage::activity` (`gp gaps`; entries and writers), `gp_nes::slice` (`gp element inputs`), `gp_nes::ramrole`, `gp_nes::instructions` (`gp trace instructions`) |
| Event stream | 2 | The recording's event stream (ROM events, collected by a fork hook, stored compressed, read after the fact) and capture events (pipeline stages and findings, NDJSON, live) |
| Code map | 4 | `codemap::Routines` (unbanked; matrix, steer), `RomCode` and `BankedRoutines` (harvest), activity's entries by PRG offset, label files |
| Press search | 5 or more | `dense::search` and `dense::mutate`, `steer::directed`, `steer::search_branch`, the matrix's search, the grid's scout and drive; two `Rng` types (`serve/dense.rs`, `gp-interp` corpus) |
| Starts | 7 | `generated::starts_of`, `gp-interp`'s `corpus/scan.rs`, steer's `start_snaps`, `window_snaps` and `seed_runs`, `dense::start_at`, two `lockstep::start_at`, free play's save points |
| Recipe formats | 9 | `clips.toml`, `aimed.toml`, `corpus.json`, the matrix's `recipes.toml` and `matrix.json`, `dense-pick.json`, `steer.json`, baseline `holds`, input logs (and `.presses` files) |
| Loop runners | 5 | `gp capture start` (stages), the `gp serve` background queue, `gp loop`, `gp bench clips`, `gp bench scratch` |

Each observer replays the recording on its own. On Moss Hop's 300-frame `finish-level`, `gp coverage` took 0.28 s, `gp trace instructions` 1.22 s and `gp replay` 0.30 s, each emulating the same frames again.

The clip tools sit behind a capture run (a model with a player, a state mapping, generated starts) because part of their scoring reads mapped measures. Their code-only half needs no model, so stage 0, which has no model yet, can't use any of them, and none ran on Ninja Gaiden.

The agent sees capture events through `gp mcp` (`capture_events`, `capture_input`), never the ROM's.

## 2. The core: a run is its recipe

| Term | Meaning |
|---|---|
| Recipe | A ROM hash, the power-on settings (region, RAM fill), a start, and presses. Text; holds no frame or ROM byte; shareable for any game |
| Start | Power-on, or another recipe's end (a prefix). A save point, a recording's frame and a clip's start are all prefixes. A snapshot is a cache of a prefix, never the start itself |
| Run | A recipe executed once in gp-nes with a collector attached |
| Detail | How closely a run is watched: a level (below) and the hooks and watches set |

Because gp-nes is deterministic (fixed timestep, seeded state, no wall clock, checked by the conformance suite), the events of a run at any detail are a function of (recipe, detail). Two consequences shape everything else:

- Nothing has to be streamed or stored at full detail. What isn't kept can be made again.
- Every derived thing (a footprint, an edge table, a curve, a coverage hash) is keyed by the recipe's hash and the collector's version, so it is cached once and shared by every tool.

## 3. Rates

| Measure | Value | Command |
|---|---|---|
| Instructions a frame | 9,664 (Moss Hop `finish-level`, 300 frames); about 9,700 on Moss Hop and Ninja Gaiden over 1,800 frames (17.4M and 16.8M) | `gp trace instructions ID --frames 1..`; `gp coverage ID --json` |
| Instructions that leave sequential flow | 4,366 a frame (45%), most in the wait for vblank | The same trace |
| Distinct transfer edges a frame | Median 91, at most 98 | The same trace |
| Distinct edges over the run | 175 | The same trace |
| Frames with a new edge | 33 of 300 | The same trace |
| ROM events | About 7 a frame on Ninja Gaiden (11,717 mapper writes, 1,791 NMIs, 1,800 frames) | `gp events ID --kind bank-switch,nmi,irq --json` |
| Cost of a host observer | About 9% over plain replay, one thread | docs/coverage.md, 2026-10-27 |
| Run speed | 1,481 frames a second plain replay, release, one thread (docs/coverage.md, `smb-run-right`); 900 to 1,200 with the audit's scratch observer (3,600 frames in 3.0 to 4.0 s) | |

Instruction-level output would be about 580,000 records a second at 60 frames a second. Novelty is a few hundred records over a whole run.

## 4. Levels

| Level | Content | Kept | Sent live | Cost |
|---|---|---|---|---|
| L0, footprints | Bitmaps by PRG offset: instruction starts, operand bytes, ROM bytes read as data, branch directions. RAM and PRG RAM bytes written | Per run, merged per ROM | No | A bit set per fetch |
| L1, edges | Transfers as (kind, bank:from, bank:to) with counts and the first frame seen. Kinds: `jsr`, `rts`, `rti`, taken branch, `jmp`, `jmp (ptr)` with the pointer's address and each value seen, `rts` used as a jump, interrupt. Bank states seen per region | Per run, merged per ROM | No | A table lookup per transfer |
| L2, frame events | Per frame, in position order: the ROM events (interrupt, sprite 0, bank mapped, controller read, PPU and APU write), novelty since the run's start (new code ranges, new edges, new bank states, new pointer values), RAM bytes that changed (a 2 KB compare at frame end), watched writes with their writer, hook calls | Per run | Yes, filtered by subscription | Tens of records a frame early in a run, falling toward none |
| L3, detail | Every instruction with registers, every bus access with its value, writer slices back from a byte | Never | On request, for a window | The replay of the window from the nearest checkpoint |

L0 and L1 answer coverage and reach. L2 answers "what happened this frame" for people, agents and the gap detectors. L3 answers "why" for one byte or one routine.

## 5. The collector

### Where it sits

| Place | How | Sees | Cost | Changes the run? | Verdict |
|---|---|---|---|---|---|
| Host observer | A closure on `Nes::step_with` before every instruction (today's six) | Everything | One call per instruction, about 9% | No | The first build, and the fallback |
| Cartridge shim | A wrapper around the board's mapper in the emulator. PRG reads go through it, marked by the CPU as opcode fetch, operand or data (the CPU knows which; the fork passes it on); mapper register writes go through it | (bank, pc) of every fetch exactly, where the bank is decided; data reads from ROM; mapper writes and the bank state after each | A bit set per fetch, inline; no call unless a hook bit is set | No. A fork change that only observes keeps recordings' checkpoints (decided 2026-10-27, docs/open-questions.md; `Lineage` in crates/gp-nes/src/recording.rs). Its state stays out of the save state | The target |
| Collector patched into the ROM | The game writes its own events to an unused range ($5000 to $5FFF) at each site | Only what is patched in | Cycles and bytes in the game | Yes: cycles move lag frames, sprite 0 timing and the RNG, and a banked ROM has no space at most sites. The run is no longer the original's | Rejected for originals |
| Virtual ROM: hooks in the shim | A hook bitmap over PRG offsets. A fetch of an opcode at a hooked offset calls the collector before the instruction runs, at no emulated cycles | What the hook reads: registers, the pointer, bank state, chosen RAM | A bit test per fetch; work only at hooks | As the shim | The target: a collector in the ROM with no byte or cycle changed |

Internal RAM is on the console side of the bus, out of a cartridge's sight, so L0's RAM bitmap and L2's watched writes come from the CPU's write path (a second, equally small hook in the fork), and L2's RAM changes from a compare at frame end.

### What the hooks are for

| Hook set | Placed by | Gives |
|---|---|---|
| Transfers | The collector, at every `jsr`, `rts`, `rti`, `jmp (ptr)` and branch it has seen in L0 (hooks follow coverage) | L1 |
| Entries | The static code map, at every routine start | Entries counted by routine |
| Questions | The agent or a tool: "when bank 4's code first runs", "every write to $F0", "the pointer at 04:8051" | L2 records for exactly that |

Cost follows the questions asked, not the instruction count.

### Games built from source

A real in-ROM collector has one use: an instrumented build of Moss Hop, a variant or homebrew that writes semantic events (an element's state, a spawn) to a debug port gives the grader an oracle. Tools never read it, and graded runs stay on the uninstrumented build, since the instrumented one's cycles differ. Where code is, ld65's debug file already says; the oracle adds what the code means.

## 6. The schema

One envelope for every record:

| Field | Meaning |
|---|---|
| `run` | The recipe's hash |
| `source` | `original` or `model` |
| `frame` | The frame, as recordings count it |
| `pos` | Scanline and dot, for ROM events; the step within the frame for model events |
| `level` | 0 to 3 |
| `kind` | Below |
| `data` | The kind's fields |

| Kind | Level | Fields |
|---|---|---|
| `frame` | L2 | Polled or lag; instruction count |
| `interrupt` | L2 | NMI, IRQ or BRK; the handler's bank:pc |
| `bank` | L2 | Region, bank now mapped, the write that mapped it |
| `controller`, `ppu-write`, `apu-write`, `sprite0` | L2 | As today's recording event stream |
| `edge` | L1 (counted), L2 (when new) | Kind, bank:from, bank:to, pointer address and value |
| `new-code` | L2 | A range of PRG offsets first executed |
| `ram` | L2 | Bytes changed this frame with old and new values |
| `watch` | L2 | Address, value, the writer's bank:pc |
| `hook` | L2 | The hook's id and what it read |
| `model` | L2 | A model event from the rebuilt game: an entity spawned or gone, a transition taken, a counter changed, with the model path |
| `insn`, `access`, `slice` | L3 | Detail, made on request |

The schema is a versioned format with a JSON Schema generated from its Rust types, like the capture events. The recording's event stream becomes the L2 ROM kinds; capture events (the pipeline's own) stay a channel of their own on the same transport.

## 7. The store and queries

| Kept | Key | Notes |
|---|---|---|
| Recipes | Recipe hash | Text, committed or shared when wanted |
| Checkpoints and the frame trace | Recipe hash | The recording format as today, so a window replays fast |
| L0 bitmaps, L1 tables | (recipe hash, collector version); merged per ROM | Small: Moss Hop's L1 is 175 edges |
| L2 records | (recipe hash, collector version, hook set) | Chunked by frame, compressed like the event stream |

| Query | Answer |
|---|---|
| Footprint of a set of runs | L0 union, by bank |
| Which runs reach bank:pc, an edge or a bank state | L1 index, with the first frame |
| What happened in frames A to B of a run | L2 |
| Detail of frames A to B | L3, by replay |
| Who wrote a byte, and from what | L3 slice |
| What the static code map has that no run reaches, and the reverse | L0 and L1 against the harvest's banked code map |

## 8. Exploration: one search

The five searches become one, in `gp-explore`:

| Part | One version |
|---|---|
| Start | A recipe prefix: power-on, a save point, a recording's frame, another run's end. Snapshots are a cache |
| Candidates | Press sequences mutated and crossed from kept ones (today's `dense::mutate`), one seeded RNG |
| Run | Each candidate runs with a collector at L0 and L1 only |
| Score | A plug-in over the run's levels |
| Keep | A greedy cover over scores, priced by frames |
| Out | Recipes, as clips |

| Score | Needs a model? | Replaces |
|---|---|---|
| New code, edges, bank states, pointer values | No | Dense's code value; stage 0's coverage |
| Distance to a target in the banked call graph (a routine, a stop of the unread report, a table entry not taken) | No | Steer's directed search |
| A branch direction | No | `steer::search_branch` |
| A matrix cell (a routine while a byte it reads varies) | No for the cell, yes for naming it in the model | The matrix's search |
| Mapped measure values, grid cells, mutants caught | Yes | Dense's measure value, the grid, the prune |

Code-only scores run with no capture run, which is what stage 0 needs. Model scores switch on when a model exists.

## 9. Recipes are clips

Every stored input becomes a clip in clip set format (docs/formats/clips.md): a start (a recipe prefix) and presses, plus the hashes of what the original gives on it. A clip's `coverage` hash becomes the hash of its L0 and L1, and its `curve` stays the hash of mapped measures.

| Today | As a clip |
|---|---|
| Stage 0's scripts | Clips from power-on |
| Matrix `recipes.toml`, `dense-pick.json`, `steer.json` | Clips with the search's name in `source` |
| `aimed.toml` | Clips with `aims` |
| `corpus.json` | Clips with their context |
| Baseline `holds`, input logs | Clips from power-on |
| Free play save points | Clips whose presses reach the save point |

One file format, one reader, one regenerator (`gp capture regenerate` and `gp corpus verify` already check a clip against its hashes).

## 10. Who uses it

| Consumer | Reads | Instead of |
|---|---|---|
| Stage 0 | L0 and L1 seed the harvest's code map (executed entries as routine starts, observed `jmp (ptr)` and `rts` edges as jumps, bank states as RAM value sets); the static rules read the code for behaviour; the unread report's stops become search targets | A static walk alone |
| Stage 1, clips | The search with model scores | Five searches |
| Stage 2, trace harvest | L3 slices, L2 RAM changes | `gp element inputs`' own replay, `ramrole` |
| Stage 3, refinement and porting | L2 `model` records lined up with the original's L2 | Mapped measures only |
| Gaps | L2 | `gp gaps`' replay with `activity` |
| Coverage | L0 | `gp coverage`'s replay |
| The inspector and the live render | L2 drawn beside the frames: the code a frame ran for the first time | A separate server path |
| The agent | Section 11 | Capture events only |

## 11. Live, and the agent

| Part | Design |
|---|---|
| Transport | The one gp already has: an NDJSON file tail, server-sent events in `gp serve`, and `gp mcp` |
| Subscription | A filter over L2: kinds, banks, address ranges, novelty only. Batch runs (thousands of search candidates) publish nothing live; kept runs publish their L2 |
| Agent tools | `events_subscribe` (a filter), `events_hook` and `events_watch` (add a hook or a watched byte to the next runs), `events_window` (L3 for frames A to B of a run), `events_query` (section 7) |
| Render | The game view draws L2 beside the frames, and the novelty as it arrives |

The agent then watches the ROM as well as the pipeline: "bank 4 ran for the first time at frame 1,204 in run X", and can ask for that window's detail.

## 12. Porting

The interpreter and the generated Bevy game emit `model` records in the same envelope as they run a recipe. Lined up by frame through the state mapping, the original's L2 and the port's L2 give the first frame and the first event where they part, not only the first measure that differs. The same comparison works between two runtimes of the original (gp-nes and the Mesen oracle) for ROM kinds.

## 13. Determinism and versions

| Rule | How it holds |
|---|---|
| Same recipe, same events | The collector only observes; its state is skipped in the save state; a test runs a recipe twice and compares L0, L1 and L2 hashes |
| The shim doesn't change the console | It is an observing-only fork change: recordings keep their checkpoints where each round-trips (2026-10-27 decision) |
| Derived data is keyed | By recipe hash and collector version; a collector change makes derived data stale, never recordings |
| The schema is versioned | A new optional kind or field keeps the version; a changed meaning bumps it, with what readers change |

## 14. Crates

| Crate | Holds | Depends on |
|---|---|---|
| `gp-events` | The schema and its JSON Schema; the `Collector` trait and the levels; the host-observer collector and the shim collector; the store and queries; replay for L3 | gp-nes, gp-evidence |
| `gp-explore` | Recipes and starts (prefixes, snapshot cache), the one search, the score plug-ins (code-only here; model scores take a trait that gp-interp implements) | gp-events, gp-nes |
| tetanes-core fork and gp-nes | The shim's two hooks (fetch with type, CPU write) and the hook bitmap, observing only | |
| gp-cli, gp-mcp, gp serve | Commands, agent tools and the live transport over the two crates | All |

Generators still depend only on the model. The harvest (gp-nes `harvest`) takes L0 and L1 as input and never runs the ROM itself.

## 15. What retires

| Today | Replaced by |
|---|---|
| `gp_nes::coverage`, `gp_coverage::pass`, `gp_coverage::activity`, `gp_nes::slice`, `gp_nes::ramrole`, `gp_nes::instructions` | gp-events L0 to L3 |
| The recording event stream's own hook and format | L2 ROM kinds |
| `codemap::Routines` in the matrix and steer | The harvest's banked code map seeded by L0 and L1 |
| `dense::search`, `steer::directed`, `steer::search_branch`, the matrix's search, the grid's scout | gp-explore's search with scores |
| Seven start finders | Recipe prefixes and a snapshot cache |
| Nine recipe formats | Clips |
| A replay per tool | One run per recipe and detail, cached |
| Five loop runners each ordering stages | One pipeline description that `gp capture`, `gp loop` and the benches run (a later step; the runners' own audit comes first) |

## 16. Steps

Each step is small, keeps the old path working until the new one gives the same output, and is checked on the test ROMs, Moss Hop and the practice variants before the four banked games.

| # | Step | Exit test |
|---|---|---|
| 1 | `gp-events` with the schema and a host-observer collector for L0 and L1 (the audit's scratch observer, /tmp/stage0-coverage-runcov.rs, is most of it); `gp reach ROM --clip FILE` | Its footprints equal `gp coverage`'s on Moss Hop, every practice variant and the 68 test ROMs; two runs give identical hashes; within about 10% of plain stepping |
| 2 | L2 and the store: ROM kinds, novelty, RAM changes, watches; L3 by replay | `gp events` (today's command) gives the same ROM events from L2 on stored recordings; `gp trace instructions` output equals L3's for the same window |
| 3 | Stage 0 seeded by runs: `gp model static --clips FILE` | Executed bytes outside the code map 0 on the 68 test ROMs (12,968 today), every practice variant and the four games' cheap clips; graded rows unchanged, wrong 0; same ROM and clips, byte-identical `static.json` |
| 4 | `gp-explore` with code-only scores; dense and steer moved onto it | Same clips for the same seed on Moss Hop; aimed at an unread stop on a practice variant whose banked routine runs only after an input sequence, it reaches it with no model |
| 5 | The shim in the fork: fetch and write hooks, hook bitmap | Same L0, L1 and L2 as the host observer on every ROM above; stored recordings still restore their checkpoints; faster than the host observer |
| 6 | Live: subscriptions over the transport, the agent tools in `gp mcp`, the render in the game view | An agent subscribed to novelty on a stage 0 run sees every new bank and pointer value, and its `events_window` matches L3 |
| 7 | The other consumers moved: gaps, coverage, element inputs, ramrole, the matrix, the grid, recipes as clips | Each command's output unchanged or the difference explained; the old observers deleted |
| 8 | Porting: `model` records from the interpreter, lined up with the original's L2 | On Moss Hop's baselines, the first parting event equals the first differing measure today, or comes earlier |

## 17. Open decisions

| Question | Default until decided |
|---|---|
| Whether stage 0 becomes "the ROM, run or read" | Decided 2026-10-31: yes |
| Where the collector is built first | Decided 2026-10-31: a host observer, then the shim |
| How much the next goal covers | Decided 2026-10-31: steps 1 to 4 |
| Whether recordings keep their own event stream format beside L2 during the move | Yes until step 7, then L2 only |
| The loop runners (five) | Decided 2026-10-31: audited after step 4 |
