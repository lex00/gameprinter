# Validation

`gp validate` holds a model to its fidelity contract against recordings of the original game. It compiles the contract's rules into checks, runs each recording's input on the original (read through a state mapping) and on a subject runtime, and compares. The code is `crates/gp-validate`, with the command in `crates/gp-cli/src/validate.rs` (ported from game-printer in wave 5; what changed is at the end).

```sh
scripts/mosshop/build.sh /tmp/mh
gp assets lift /tmp/mh/mosshop.nes --map assets/mosshop.toml -o /tmp/mh/lifted
gp validate templates/mosshop --against baselines/mosshop --rom /tmp/mh/mosshop.nes --assets /tmp/mh/lifted
```

With no `--runtime`, the subject is the model, run through its game's module (`--module`, needed for a model with hooks) or the built-in interpreter, with the lifted assets from `$GP_HOME/work/assets/<model.source>` or `--assets`. A game's package validates the same way:

```sh
gp validate games/donkey-kong --against RECORDING --module dk.wasm
```

## What is compared

Every check compares two pairs:

| Against | Measured | Compared with |
|---|---|---|
| The model | The original's run | What the model's own numbers give: a jump's height and airtime from its speed, gravity and fall limit, a patrol's speed from its step and period, the transitions and timings of its rules, the events its counters count, its levels and their spawns |
| The original | The subject's run of the same input | The original's run, measured the same way |

The subject is any runtime that answers state paths through the runtime protocol's `query`:

| `--runtime` | Subject |
|---|---|
| `interp` (the default) | The model through its game's module (`--module`), or the built-in interpreter for a model with no hooks. A model naming hooks is refused without its module |
| `nes` | The original itself, read through the state mapping a frame at a time as the model runs. It tests the engine: every comparison against the original must pass |
| `none` | No subject: only the original against the model's numbers |

The two runs are lined up and read by gp-interp's comparison (`gp_interp::compare::compare`, the code `gp interp compare` uses), so the checks measure both in model frames. Under the mapping's `polled` alignment the model runs one frame for each frame the original reads its controller, and the original's frames before its first read and its lag frames aren't measured.

The frames compared are the ones a clip's score compares (`gp clip score`, the lockstep in `gp_interp::lockstep`), so the two never disagree about what is measured and when:

| What | How |
|---|---|
| The start | The subject takes the original's state at the first compared frame, translated through the mapping, as a clip's score does, and with the mapping's `score_draws` the original's random draws from there on. `--power-on` runs the subject from its own power-on state instead, which tests the model's start values (`gp model refine` runs it so) |
| Unmeasured frames | A path a `[[field.case]]` leaves `unmeasured` on a frame reads as nothing there, on both sides, so no check measures it |
| The scope | The run stops at the first frame out of the mapping's `[scope]` |

On Donkey Kong's four walk recordings these took the failed checks from 131 to 73 (2026-11-03): 4 were unmeasured frames, 54 the start and the draws.

`--jobs N` (or `GP_JOBS`) evaluates the recordings on up to N threads; the results are the same for any number.

## Checks

The compiler finds every part of the model a check can measure and asks the contract which level governs each of its quantities, using the quantity's name as the aspect. A rule with `aspect = "height"` on a jump governs its height alone.

| Part | Kind | Quantities | Exact | Metric | Structural |
|---|---|---|---|---|---|
| `entities.<e>.jump` | jump | `height`, `airtime` | Equal | Within the tolerance of that name | Not checked yet |
| `entities.<e>.run` | run | `speed`, walking and running | Equal | Within `speed` | Not checked yet |
| `entities.<e>.patrol` | patrol | `speed` | Equal | Within `speed` | Not checked yet |
| `entities.<e>.walk` | walk | `speed` | Equal | Within `speed` | Not checked yet |
| `rules.<m>` | machine | `state` | Against the model: every change of state is one of its transitions, a timed transition (`after`) takes exactly its frames, and nothing moves in a `freeze` state. Against the original: the state is equal every frame | Not checked yet | Not checked yet |
| `entities.<e>.machine` | states | `state` | Against the model: each instance's state (`entities.<e>[i].state` in the state mapping) changes only along one of the machine's transitions, a timed one on its `after` frame, a transition that does `remove` removes the instance on its `after` frame, and a change to the initial state comes when the instance appears or a rule's transition restarts the level or the game. Against the original: equal every frame. What a state `set`s for the sprite isn't counted as measured | As Exact | Not checked yet |
| `counters.<c>` | counter | `value` | Against the model: it changes by each `change`'s `by` on the frames its event happens, within `min` and `max`, and on no other frame. Against the original: equal every frame | Not checked yet | Not checked yet |
| `entities.<e>`, for an entity in the state mapping's `[slots]` | slots | `speed`, `period` | Equal | Within the tolerance of that name | Not checked yet |
| `cameras.<c>` | camera | `lead` | Equal | Within `lead` | Not checked yet |
| `levels.<l>` | level | `graph` | As Structural | Not checked yet | The level graph: the levels a run visits, the ways between them (another level, or an end state such as `rules.game=won`), and the entities in each with the most alive at once. Against the model, what the run shows must be in the model, with no more of an entity alive at once than the model spawns there. A run needn't show all of the model, as it needn't take every exit: a scrolling level despawns some enemies before others appear, and a run can pass an enemy before the original loads it (c8f4e908 never shows w3_2's piranha plant, which 782c557d does), so fewer, or none, is noted rather than failed; against the original, the two graphs are equal |
| `levels.<l>` with a goal | reach | `reachable` | As Structural | Not checked | Whether the goal can be reached from the spawn by the player's own moves (`gp_interp::reach`, [mechanics](mechanics.md#reachability)), on the model with its hooks, once per validation rather than per recording. The search's moves walk and jump, so for a player that climbs a miss is unexercised, not a fail |

A metric quantity with no tolerance in the contract must be equal. Free parts aren't measured. Experiential and Style parse and resolve, and Milestone 2 doesn't measure them.

How each profile is measured:

| Profile | Measured as |
|---|---|
| Jump | From the frame the entity leaves the ground moving up to the frame it's back on the ground at the same height, with no state machine changing on the way. Falls, bounces and landings on other ground aren't jumps. Height is the start minus the peak; airtime is the frames in between. Held against the original only where the state mapping gives the original the entity's `on_ground`; without it the original's jumps can't be found and the check is unexercised |
| Walking and running speed | The most common speed the entity settles at: on frames where, over that frame and the 15 before (longer than Super Mario Bros.' 10-frame running timer, which keeps running speed after B is let go), the player controlled the game (observe.toml's `control`), one direction (and the run action, or not) was held, it was on the ground (where `on_ground` is mapped), no state machine changed, none froze the game, and the speed didn't change. Speeding up, slowing down and speed kept in the air are left out. The speed is the entity's mapped `vx` where there is one (Super Mario Bros.' `Player_X_Speed`), and otherwise the distance moved that frame |
| Patrol and walk speed | The most common speed each living instance moves at over 4 steady frames: the distance moved in them, divided by 4. Over a few frames half a pixel a frame (a pixel every other frame) shows whole, and the most common speed is the walking one even when the game holds its enemies still for a moment (Super Mario Bros. does while Mario grows or dies), kicks them, or reuses a slot. Windows without movement are left out |
| Camera lead | On each steady frame the camera moved right, the followed entity's `x` less the camera's in whole pixels: where the camera keeps it while scrolling. The most common is compared with `lead`; the report gives the range too, since a game that scrolls from a window (Super Mario Bros. starts at 80 pixels and keeps up at 112) shows several |
| Event frames | A transition that fires `on` an event shows it on the frames the state makes that change, a rule's or an entity's own machine's. `enemy.stomped` is an enemy's `alive` going from 1 to 0. A head hit's or a pickup's event (platformer `jump.head`'s `hits`, `jump.pickup`) with a kind the cell `becomes` shows as one more of the original's level cells holding that kind's byte (the state mapping's `[cells]` and its `becomes`, read as `cells.becomes.<kind>`): Moss Hop's `block.coin` is one more used block, `coin.touched` one more sky cell. An event none of these shows leaves its counter's check unexercised |

A timed counter change (`every`) is held to its amount, and not yet to its timing.

A counter an `ext` hook may write is held to the model's own changes only where they explain a change; the other changes are the hook's, counted in the outcome's note, and the counter's check against the original holds them frame for frame. A hook may write a counter when its description writes `counters` or `counters.<c>`, and a hook that describes nothing may write any. A counter with `wrap` goes round past its `max` to its `min` (or 0), as the interpreter steps it.

A timed transition (`after`) that waits for something more too (a `word`, a button, a height, an event or an alignment; an `if` picks between transitions and keeps the exact frame) fires on the first frame from its `after` on that both hold, so it may take longer than `after`; one with nothing else to wait for takes exactly `after`.

A state that freezes the game takes effect from the frame after it's entered, in the interpreter as in Moss Hop: a frame is frozen when the state machine was in such a state at the frame before. So the freeze check looks at frames whose state and previous state are both frozen, and movement is measured only on frames that start and end in one state that doesn't freeze.

## Slot checks

Super Mario Bros. keeps its enemies in five object slots it reuses, and a model's Mario soon leaves a recording's path, so a side-by-side run never reaches most of the enemies a recording meets. An entity the state mapping's `[slots]` names is measured slot life by slot life instead, with gp-interp's per-slot comparison (the state mapping's Slots (game-printer's docs/formats/state-mapping.md)): each life gets one body of its entity, stepped beside the slot with the player held where the original's is. The check is held against the original (the body is the model), and it replaces the entity's walk check.

| Quantity | Measured as |
|---|---|
| `speed` | On each field the entity's components move (`x` for `walk`, `fly` and `patrol`, `y` for `hop`, the path's axis for `path`) or, for an entity with an `ext` hook, every field the slots map: the speed over 4 steps of a life in a row, in whole pixels on both sides over the same steps: the most common, as a walk's speed is measured, or for an entity moved by a hook rather than a walk, whose speed can change every step (a lift slowing to turn), and for a hop's `y`, the mean of every window. A walker with a hook (Super Mario Bros.' spiny, whose hook is its egg) is a walker. Whole pixels, because a slot's position can hold no fraction (Super Mario Bros.' `y` doesn't), and a lift at 0.9375 a frame then moves 4 pixels in most windows. Windows without movement and jumps of more than 128 pixels are left out, so a stomped, kicked or held enemy doesn't count against its speed. Each field's two speeds must be within the tolerance; a field only one side moves fails |
| `period` | For an entity that turns back by itself (a `path`, or a hook that doesn't walk): the steps between every second turn back, over all the entity's lives, on `x` and `y` where the original turns on them. A hook's own words are its state, not a rhythm the player sees, so their turns aren't held. A recording where the original never turns back (a firebar spins one way) holds no period. A walker turns where it meets another enemy, which a body alone doesn't, so a walker's turns aren't held |

A recording with no life of the entity leaves its checks unexercised, and so does one where the hook of an entity a hook moves draws a random byte: the body beside the slot draws its own bytes, not the original's, so its choices (Donkey Kong's barrels at a ladder) aren't the slot's; the clip scores, which give the rebuild the original's draws, hold it frame by frame. A rule with a `when` can't govern a slot check yet: the check fails and says so. With `--runtime bevy` the slot checks are the interpreter's, on the same terms as every other measurement ([below](#a-generated-build-as-the-subject)). `--runtime nes` and `none` don't run them.

## Conditions

A rule's `when` is a condition in the run-until grammar of the runtime protocol (game-printer's docs/runtime-protocol.md) over state paths, such as `world == 1 && counters.lives > 0`. It's evaluated on every frame of the original's run, and while it holds its rule replaces the level that would otherwise govern. Each measurement takes the level of the frame it was made on; for a jump that's its takeoff. A path reads its whole part rounded down, so `entities.player[0].x > 100` works. Comparing a name, such as the state of a machine, fails the check. Two conditional rules with different levels that hold on the same frame fail the check at that frame, naming both rules. `gp model check` reports a `when` that doesn't parse.

## Paths into runtime state

The checks read runtime state by the paths below. A subject runtime answers them through `query`, and the original's state mapping says which RAM holds each.

| Path | Value |
|---|---|
| `entities.<e>[<i>].x`, `.y` | Position of instance `i`: the left edge in the level and the top edge, in pixels with fractions |
| `entities.<e>[<i>].vy` | Vertical speed, down positive |
| `entities.<e>[<i>].on_ground` | 1 while standing on something |
| `entities.<e>[<i>].alive` | 1 while alive. Without it, every mapped instance counts as present |
| `rules.<m>.state` | The machine's state, by name |
| `counters.<c>` | The counter's value |
| `level` | The current level, by name. A model with one level, whose mapping has no `level`, is in that level throughout |

The model's graph has an edge from each level to its `next` and to each of its exits' levels. A level check whose level no run enters is unexercised, and each level's check is about that level and its ways out alone. A way between levels counts across frames that are in no level, as while the next one loads. Against the original, the two runs' graphs are compared while they're on the same path, up to the first frame both are in a level and not the same one, and only the levels entered and the ways out: a model whose Mario misses a jump a recording makes can't be held to the levels the recording goes on to, and those are held to the model's own graph instead.

A path the original's mapping doesn't have leaves the checks that need it unexercised, and says so.

## The state mapping

The original is read through the model's state mapping, `mapping.toml` in the model directory (its format (game-printer's docs/formats/state-mapping.md)), or another file given with `--mapping`. It maps RAM bytes to the state paths above, with simple transforms, and says how the original's frames line up with the model's. The interpreter's comparison reads it too, so the two never disagree about what a path means. The result's key includes the mapping's text.

## What the mapping can't say

The state mapping maps fixed model paths. Two things the checks need don't fit it: the current level, when the original names it by several bytes together, and entities kept in slots whose kind changes, as Super Mario Bros. keeps its enemies (a slot holds a goomba one moment and a koopa the next). `observe.toml` in the model directory says where the original keeps these. The file for that game has no `[[slots]]`: `gp validate` builds that table from the object IDs and the `x` field of the state mapping's `[slots]`. `gp validate` runs the original a second time and takes these at the frames the comparison used. The subject runs a second time too, for its `level` and `entities`. Its text is part of the results' key.

```toml
schema = 0
model = "smb1"

[level]
ram = [0x075F, 0x075C, 0x00E7, 0x00E8]   # WorldNumber, LevelNumber, AreaData
names = [
  ["0x00 0x00 0x90 0xa6", "w1_1"],
  ["0x00 * 0x7b 0xad", "w1_underground_3"],
]

[[slots]]
count = 5
kind = 0x0016     # Enemy_ID
active = 0x000F   # Enemy_Flag
x = [0x0087, 0x006E]   # Enemy_X_Position, Enemy_PageLoc

[slots.names]
"0x06" = "goomba"
"0x00" = "koopa_green"
```

| Key | Meaning |
|---|---|
| `control` | Optional: a condition over the original's state under which the player controls the game. Outside it the game moves the player itself, as Super Mario Bros. does in its pipe intros and its walk to the castle (`ram[0x000e] == 8`, PlayerCtrlRoutine), and walking and running speeds aren't measured there |
| `level.ram` | The bytes that name the level together |
| `level.when` | Optional: a condition over the original's state (the run-until grammar) outside which there's no level. Super Mario Bros.' bytes still name the last level between levels, so its map names one only while Mario is under control |
| `level.names` | Pairs of the bytes' values, in `ram`'s order, and a level's name. `*` matches any value, and the first pair that matches wins. Values no pair matches are no level, as while an area loads |
| `slots.count` | How many slots; slot `i` reads every address plus `i` |
| `slots.kind` | The byte saying which entity a slot holds |
| `slots.active` | Nonzero while the slot is in use |
| `slots.x` | The position's whole part, low byte first |
| `slots.names` | Kind values to entity names. A slot holding a kind not named is left out |

A slot's entity reads as `entities.<name>[<slot>].x` and `.alive`, so instance numbers are slot numbers on the original's side and spawn order on the subject's. The checks that read them count and measure instances one by one (the most alive at once, mean speeds), so either numbering works. A subject's entity counts as alive from the frame it spawns.

This is a format of the validation engine's own, schema 0, until the state mapping can say these things (open questions (game-printer's docs/open-questions.md)).

## Results

| Status | Meaning |
|---|---|
| pass | Every measurement the recordings gave is within the contract |
| fail | One wasn't; the report gives the recording, the frame, what was expected and what was measured |
| none | Unexercised: no recording gave the check anything to measure |

A check fails if any recording fails it. The report also counts facts by level and lists the facts no compiled check measures, such as hitboxes and the camera, so a contract's gaps show.

Each recording's result is a store object, its JSON, with one line in `<store>/index/validation.jsonl` pointing at it by a key. The key hashes everything the result depends on. Those are the validation version, the model's and contract's canonical text and the state mapping. The recording (a store recording's ID, or a baseline file's hash) and the runtimes that ran it are in it too. Running the same validation again reads the results back instead of running the recordings. `--no-save` neither reads nor writes them, and with only baseline files needs no store at all. `gp store gc` keeps every object the index names.

It exits 0 when no check failed and 1 when one did. It exits 2 when nothing could run. That happens when no recording's ROM is found, when the contract has problems, and without a state mapping.

The JSON (`--json`) has the totals (`passed`, `failed`, `unexercised`), every check's summary, the facts no check measures by level, each recording's outcomes, and how many recordings `--store` left out for playing a sealed run.

## Moss Hop

On 2026-10-28 game-printer's `templates/mosshop` passed all 20 checks against its five baselines. The interpreter passed them as the subject, and so did the original itself. Half compare with the model's numbers and half with the original:

| Check | Measured |
|---|---|
| Jump | 47.5 pixels high and 39 frames in the air, within 5% and 2 frames |
| Run | 1 pixel a frame walking (from `walk-and-turn.toml`) and 2 running |
| Patrol | Half a pixel a frame, within 3% |
| Camera | The player 120 pixels into the view on every frame it scrolls, the model's `lead` |
| `rules.game` | The dying state lasts 90 frames and freezes the game |
| Counters | Deaths and stomps change on their events, and coins on a used block and a coin taken, seen in the level cells |
| `levels.main` | One level, its goal leading to `rules.game=won`, a player and two slugs |

Broken copies of the model fail the check that covers what was broken (`crates/gp-cli/tests/validate.rs`). Lower gravity fails the jump and a faster slug the patrol, for example. The 56 facts no check measures are 25 metric ones (hitboxes, sizes, contact, the camera's size and clamping, and input) and the 31 style facts of the art.

## Donkey Kong

On 2026-11-12 `games/donkey-kong`, with its module, held to its contract on the corpus clip dk.clear-25m.001 (3674 frames, recorded with `gp record`): 162 checks pass, none fails and 287 are unexercised, over 449 checks and 3671 model frames, the same as game-printer's validate on its template. The barrel's slot checks are unexercised: its hook draws random bytes, and here also because its hooks run in the module (below).

## What changed from game-printer

- The model runs as gameprinter's tools run it: through its game's module (`--module`) or the built-in interpreter. `--allow-missing-ext` is gone, since gp runs a model's hooks only in its module.
- `--runtime bevy`, `--project` and `--target-dir` are gone with the Bevy generator, and so is the conformance check.
- `--metrics` is gone with the run recorder; the JSON has no `run_manifest`.
- The slot checks step the bodies by the hooks gp has itself; a model whose hooks are only in its module leaves its slot checks unexercised and says why, until they step through the module (wave 2's `compare_slots_with`).
- Recordings and baselines name an NES ROM. An arcade romset given with `--rom` is refused in words: an arcade recording's source comes with wave 6.
