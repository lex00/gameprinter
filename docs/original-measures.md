# Measuring the original, and the model beside it

These commands came from game-printer in the second porting wave. They read what the original game does and hold the model to it, and they're how a new game gets from its first clips to a model worth gating. Each starts the original through `OriginalRuntime` (gp-nes for an NES image, gp-arcade for a romset zip), so the ones that only read RAM and the mapping work on either platform. A model with hooks runs through its game's module: give `--module FILE` (`gp game build` makes it), as `gp clip score` takes it.

The old pages these come from are game-printer's docs/interpreter.md ("Moments", "Scenarios: one press at a time", "The picture-gap detector"), docs/benchmarks.md ("Random choices"), docs/formats/state-mapping.md ("Slots") and the probe recipe table at the top of its probe.rs. What changed from them is listed under each command.

The input a command plays is `--input LOG` (an input log, such as `gp clip log` writes) or `--baseline FILE`. `--recording ID` is kept for the old spelling and refused until recordings come back with `gp record`.

## gp mechanic probe

```text
gp mechanic probe GAME [ID...] [--file FILE] [--rom ROM] [--games DIR] [--save] [--import SESSION:CLIP] [--json]
```

Probe recipes, kept in the game's package folder as `probes.toml` so they run again with no agent. A probe starts from a base (power-on, a stored path, or a mechanic clip's inputs from power-on) at a frame, then plays its steps:

| Step | What it does |
|---|---|
| `PADS N` | Holds the buttons for N frames (NES: letters of `RLDUTSBA`, or `.` for none; `P1\|P2` holds both controllers) |
| `walk_to NAME V [max N]` | Holds Right while RAM NAME is below V and Left while above, until it equals V |
| `until COND PADS [max N]` | Holds PADS until the condition holds (checked before each frame) |
| `repeat K: STEP, STEP` | The steps, K times |
| `replay_until COND [max N]` | Plays the base's own inputs on until the condition holds |
| `mark` | Measures start here, not at the probe's start |

A sweep names variables and their values, and `rows` names an asset whose rows (lifted from the ROM by the asset map its ROM entry names) give more: each combination is a cell, with `{VAR}`, `{VAR+N}` and `{VAR-N}` filled in. Measures read RAM at the probe's start and after each frame, by the frame count from power-on:

| Measure | Value |
|---|---|
| `first COND`, `last COND` | The first or last frame the condition holds, or none |
| `any COND`, `count COND` | Whether it holds on any frame; on how many |
| `edges COND`, `gaps COND` | Each frame it starts to hold; the frames between those starts |
| `between COND1 then COND2` | Frames from the first COND1 to the first COND2 at or after it |
| `pairs COND1 then COND2` | For each start of COND1, the frames to the next COND2 |
| `changes NAME`, `intervals NAME` | Each frame RAM NAME changes; the frames between those changes |
| `values NAME at changes OTHER` | NAME on each frame OTHER changes |
| `delta NAME`, `value NAME at F` | Its last value less its first; its value after frame F (`+K` from the probe's start) |
| `min NAME`, `max NAME`, `range NAME` | Its least, its greatest, the difference |

Any measure may end in `while COND`. A condition is `NAME OP V` (OP one of `== != < <= > >=`), or several joined by `and`; names come from the file's `[watch]` table or are written as addresses. Each measure with an `expect` (or `expect_each`) is checked, and the command exits 1 when one fails. A probe that names its session's clips is compared with them input for input from power-on; `--save` writes the cells with no clip yet into the session, as the mechanic page saves a clip. `--import SESSION:CLIP` prints a saved clip as a probe to start from.

Changed: the original runs through `OriginalRuntime`. On an arcade board the addresses are the main CPU's (0 to 0xFFFF, read with `ram[A]`), the buttons are the board's columns `CSRLDUT`, and `P1|P2` is refused. The file is the package folder's `probes.toml`, found in the games checkout (`--games`), and `--rom` defaults to the ROM `roms.toml` names. Sealed runs aren't refused yet, since gameprinter has none.

## gp interp moments

```text
gp interp moments MODEL --rom ROM --input LOG|--baseline FILE [--module WASM] [--kind KIND] [--assets DIR] [--mapping FILE] [--json]
```

The game's own moments in a run: its jumps, landings, deaths, lives lost, counter changes and level changes, each numbered by kind. `gp_interp::moments` reads them from the mapped measures of a resync lockstep with values:

| Kind | Read from |
|---|---|
| Jump | An entity's `on_ground` going from true to false with upward speed (`vy` below 0, or `y` going up without `vy`), to its next landing; its height is how far `y` rose from takeoff to the top |
| Landing | `on_ground` going from false to true |
| Death | An entity's `alive` going false; on the model's side also an event whose kind ends in `.died` |
| Life lost | A `lives` counter going down or a `deaths` counter going up |
| Counter | Any mapped counter changing, except a clock (a counter the model changes `every` so many frames) |
| Level | The level the mapping reads changing |

Each moment names the mechanic that owns its kind for its part, by the catalog. The model is then run on each moment from the original's state half a second (30 frames) before it, on the same presses, and its own moment of the same kind is looked for there: "jump 1 at frames 61-100 (1.0 s): rises 47.5 px; the model too, on the same frame, 47.5 px", or "the model doesn't".

`--json` gives `moments` (each with `kind`, `n`, `part`, `mechanic`, `from`, `to`, seconds, `peak`, `height`, `before` and `after` for a counter or a level, `focus`, and `model`, the model's moment with its `offset` in frames and its `height`) and `kinds`, how many of each the original has and the model does too.

## gp interp translate

```text
gp interp translate MODEL --rom ROM (--input LOG|--baseline FILE --frame N | --snapshot FILE) [--out STATE] [--module WASM] [--json]
```

The original's state as a model state, through the state mapping read in reverse: at frame N of an input (the first frame the model stands for at or after it, as a lockstep from there starts), or in a snapshot of the original. It prints a note for every part, loaded, approximated or missing. With `--out` it writes the canonical model state, which any runtime of the model loads with the runtime protocol's load-state.

Changed: `--snapshot` is a snapshot of whichever emulator the ROM's platform runs, not only gp-nes's.

## gp picture

```text
gp picture GAME --run RUN [--module WASM] [--model DIR] [--assets DIR] [--rom ROM] [--clip ID] [--top N] [--shots 12,40-45 --shots-dir DIR] [--json]
```

The picture-gap detector. Every clip of the game's corpus is made again into the `gp corpus score` run folder RUN (`$GP_HOME/capture/RUN`) and played again in lockstep, and on each compared frame the screen measure's boxes are put down to a cause by what is on screen there: the model's bodies and their looks, the original's sprite table, the level's cells and the HUD. The causes are a fallback box (no sprite), an animation frame or flip, a palette or color step, the HUD, background tiles, draw order and priority, scroll offset, drawn in one game only, flicker, placed elsewhere, and other. Each group gives its clips, its frames, the frames it alone explains, its pixels and an example clip and frame. With `--clip` and `--shots`, the frames named are written as PNGs (the original, the model and the pixels that differ, side by side at 2x) into a folder outside any git work tree. It never writes the model or the corpus.

Changed: the old command read a capture run of `gp serve`; this one takes the game's package folder and a corpus score run, with the flags `gp corpus score` takes. Its fidelity line is the corpus score's (the mean of frames together) with the screen counts, since the old fidelity parts (behaviour, coverage, picture) aren't in gameprinter. An arcade original has no sprite table the tools read yet, so its boxes are put down to causes with no sprites.

## gp interp scenarios

```text
gp interp scenarios MODEL --rom ROM --recording LOG|BASELINE [--module WASM] [--at FRAME] [--tier N] [--bar exact|99%] [--played FILE]... [--assets DIR] [--json]
```

The scenario ladder. A scenario is a few frames of presses run on both games from the same start, the original's state at one frame: the original replays the input to that frame, the model loads the original's state there translated (as `gp interp translate` does), and both take the scenario's presses. `gp_interp::scenario::ladder` makes the ladder from the model's input actions and the player's mechanics, in tiers:

| Tier | Scenarios |
|---|---|
| 1 | Nothing pressed for 60 frames; then each button that isn't a direction: tapped for 1 frame, held 5, 15 and 30 frames, tapped twice |
| 2 | A second jump in the air (named "no double jump" when no mechanic of the player jumps in the air), a jump held through the landing, a jump again on landing, a jump pressed just before landing when the jump has `buffer_frames`, the run button with the jump, any other button in the air, two buttons together |
| 3 | Left and right tapped, held 15 and 60 frames, each with the run button; right then left; right with a jump at frames 0, 10 and 30; a running jump; a turn in the air; right in the air after a standing jump; up and down only when a mechanic reads them |

Without `--at`, the start is a quiet frame found in the input: the player in the level, on the ground and still, nothing alive within 80 px. Each scenario is exact or names its first split in words. `--bar exact` (the default) passes a scenario with no split on any mapped measure; `--bar 99%` passes it with that share of every measure's frames within tolerance. Each scenario is shown as passed, open, failing or locked, with the next one to work on and a score history across model changes. It exits 1 while a scenario doesn't pass.

Changed: `--recording` takes an input log or a baseline file, since there is no recordings store yet. `--refine` and `--auto` refuse in words until the refinement loop (`gp model refine`, ported apart) lands. The ladder's run metrics are gone with the old run recorder.

## gp solve

```text
gp solve GAME --run RUN [--module WASM] [--model DIR] [--assets DIR] [--rom ROM] [--clip ID] [--check] [--threads N] [--missing] [--json]
```

Each clip of the game's corpus that isn't exact, its detector class and its proposals: the exact model numbers (arithmetic over the measured values, timers, parameters) that would close the gap. The corpus is made again and scored into the `gp corpus score` run folder RUN, reusing its cache. With `--check`, each clip's best proposal is scored on that clip and on every other clip: better on the clip, and no other clip worse. `--missing` also runs the body and state detectors (a body one game has alone, a value the original takes that the model never does, a split on a state's entry). It never writes the model.

Changed: the old command read a `gp serve` capture run and its queue of scores; this one takes the game's package folder and a corpus score run, with the flags `gp corpus score` takes, and solves every clip that isn't exact. `--threads` defaults to `--jobs`.

## gp interp slots

```text
gp interp slots MODEL --rom ROM --input LOG|--baseline FILE [--module WASM] [--frames N] [--trace ENTITY] [--assets DIR] [--mapping FILE] [--json]
```

The model's enemies (their components and `ext` hooks) against the original's enemy slots. The original runs on the input and, for each slot life the state mapping's `[slots]` names, one body of that entity is stepped beside it with the player held where the original's is. Each compared value gets a line: the frames it agreed within a pixel, its largest error, and both sides' speed and period. `--trace ENTITY` prints every compared value of that entity's lives to stderr. It exits 1 if the model has no hooks for a mapped entity.

Changed: a body is stepped outside a whole frame, so for a model whose hooks are in its module the module steps it, through the `step-hooks` call (`wit/game/game.wit`). `ext_hooks` names the module's hash.

## gp interp odds

```text
gp interp odds MODEL --clips CLIPS_TOML|GAME... [--only ID,ID] [--rom ROM] [--module WASM] [--assets DIR] [--json]
```

The random-choice behaviour check. The model plays each clip with its own generator, and at each draw at a `[[draws]]` site the frame is run again from the state before it with the draw set to each of the 256 bytes. The bytes are grouped by the state they lead to: each group is a branch the rule takes there, and its share of the 256 is the rule's odds for it. Decisions with the same groups are pooled, and the model's count of each branch is tested against the odds with a chi-square test, failing below p 0.001; a pool with an expected count under 5 is reported and not tested. With `--rom`, the original's draws over each clip are put in the same groups and tested the same way, and the rebuild, given them in a resync lockstep, has to take each one. It exits 1 when a test fails. It is slow: allow about half an hour a clip.

Changed: `--clips` also takes a game's package folder (its corpus plan). The command's own `--jobs` is gone; gp's global `--jobs` sets the clips run at once. A model whose hooks are in its module reads its draws through the `set-draw-script` and `draw-calls` calls, and a frame with a forced draw is given a call's deadline rather than a frame's, since a rare byte can take a branch that runs long.
