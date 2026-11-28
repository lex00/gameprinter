# Scripted scenes

A scripted scene is a stretch of a game that a script plays rather than the player: an attract demo, an instruction screen that walks a character through what to do, an intermission between rounds. Today a port models one with a hook in its game's `ext` crate, a body of Rust that writes each object's place and picture frame by frame. That hook is debt in the catalog, since its `like` names no component ([mechanics](mechanics.md), "The rule for game-specific code"), and it is the kind of hook code the package scan reads hardest: a routine that is a list of loads, stores and waits, written out as a function, keeps the routine's operands in their order, which is what the same-order signal looks for ([package scan](package-scan.md), "Transcriptions").

This page is the design of the other way, written before any of it is built: a scene is model data, found from the ROM's code by the tools, played by one general component in the model, and checked with the picture-gap detector. No game needs a hook for it. "Build order" lists the rows that build it, each with its measure. Issue 39 asked for it.

The example throughout is a made-up game, Rooftop, whose title screen runs a demo: a cat walks in from the left edge and stops under a chimney, a pigeon drops onto the chimney beside it, both hold still for a second, and the cat walks off to the right. Its addresses are made up too.

## Finding a scene

A scene is found from a clip that shows it and the code that ran in it, with the commands that exist today. Nobody reads the scene off the screen and types it in.

| Part | How it is found | What the model cites |
|---|---|---|
| The clip | A clip of the corpus or a probe's clip that plays the scene from power-on, and the frames A..B it runs in. Rooftop's demo runs from frame 600 to 960 of a clip that waits on the title | The clip and frames, in each step's evidence |
| The routine's entry | `gp code listing ROM --clips SET --only CLIP --frames A..B` lists the routines that ran in the window, with each instruction's count. The same with frames before A lists what ran before. The scene's routine is the one whose entry ran on every frame of the window and on none before it: Rooftop's $C400, with a count of 361 | `scenes.NAME.routine` |
| Its waits | A wait is a byte the routine counts down once a frame, and a branch after it that returns while the byte isn't 0. The listing gives that branch's ways: taken on every frame but the last of each wait, fell once a wait. The load that set the byte before the count started gives the wait's length: an immediate (`LDA #$3C`) or a table's row (`LDA $D210,X`) | A `wait` step, its `pc` the branch, its length the load's source |
| The object slots it writes | The state mapping's `[slots]` gives the address of each slot field (x, y, the picture byte, the flags; [state mapping](formats/state-mapping.md), "Slots"). `gp code listing ROM --clips SET --only CLIP --frames A..B --writes ADDR`, once for each, lists every instruction that wrote it in the window, with its count, its routine and where that routine was called from | A `set` step for each write inside the scene's routine |
| Where each value comes from | The instruction before each write, in the same listing. `LDA #imm` is an immediate, `LDA $D200,X` a row of a table, `LDA $0310` another slot's field, `INC` and `ADC #imm` a change of the field by an amount | The step's source, by address |
| The order | The first frame on which each write and each wait's branch ran, from the listing over the window. Two writes in one frame keep the order of their addresses along the routine's path | The order of `scenes.NAME.steps` |

A write in the window to a byte the mapping names no field for (a sound request, a screen tile) is listed apart as unexplained. It is either a field the mapping is missing, a part of the scene this design doesn't play yet ("Limits"), or the game's other work going on during the scene. `gp scene find` (build row scene-find) does the whole of this table in one run and prints a draft of the steps.

## The steps

A scene is a list of steps. A step is a wait or a write, and each cites the instruction it models by its address (`pc`). Every number a step uses comes from the person's ROM through the lift, by the address it is at, so the model holds no number of the ROM's.

```toml
[scenes.demo]
routine = "$C400"
while = "title.demo"
then = "demo.over"
code = ["code_c400"]

[scenes.demo.actors.cat]
entity = "cat"
slot = 0

[scenes.demo.actors.pigeon]
entity = "pigeon"
slot = 1

[[scenes.demo.steps]]
pc = "$C40C"
set = { actor = "cat", field = "x", from = { imm = "$C40B" } }

[[scenes.demo.steps]]
pc = "$C412"
set = { actor = "cat", field = "y", from = { table = "demo_places.y", row = 0 } }

[[scenes.demo.steps]]
pc = "$C42A"
wait = { until = { actor = "cat", field = "x", at_least = { imm = "$C427" } }, each = [{ actor = "cat", field = "x", add = { imm = "$C421" } }] }

[[scenes.demo.steps]]
pc = "$C440"
set = { actor = "pigeon", field = "x", from = { actor = "cat", field = "x" } }

[[scenes.demo.steps]]
pc = "$C452"
wait = { frames = { imm = "$C44D" } }
```

A step is one of two kinds:

| Kind | Keys | What it does |
|---|---|---|
| `set` | `actor`, `field`, and `from` (the value) or `add` (an amount added to the field) | Writes one field of one actor |
| `wait` | `frames` (a value: so many frames) or `until` (a test on an actor's field or a counter: `at_least`, `below`, `equals`, each a value), and optionally `each`, writes made on every frame of the wait | The scene does nothing more until the wait ends. A routine that moves an object a pixel a frame until it reaches a place is one wait with an `each`, as the code is one loop of an add and a compare |
| | `same_frame` (default false) | The step after the wait runs on the frame the wait ends, for a routine whose branch falls through into the next step instead of returning. The listing shows which: a fall into the next step's code, or into an RTS |

A value is one of these:

| Value | What it is |
|---|---|
| `{ imm = "$C40B" }` | The byte at that address in the ROM's code: the operand of an immediate load or compare. It is read from the scene's `code` slots, `table` slots that lift the code's span as [cycles](cycles.md) ("Costs from the ROM's code") lifts it for a walk |
| `{ table = "SLOT.COLUMN", row = N }` | A row of a column of a `table` slot, as `ext_tables` names a column ([model](model.md), "Entities"). `row_from` in place of `row` takes the row from a value (an actor's field or a counter), for an indexed load |
| `{ actor = "cat", field = "x" }` | Another actor's field as it is when the step runs |
| `{ counter = "round" }` | A counter of the model |
| `{ value = 0 }` | A number the ROM doesn't hold as data, such as the 0 a `STZ` or the 1 an `INC` stands for, with evidence like any fact |

The fields a step writes are an actor's `x`, `y`, `vx`, `vy`, `facing`, `shown` (drawn or not) and `word0` to `word3`, the body's words that a sprite's clips already pick pictures by (`when.word`, [model](model.md), "Presentation"). A scene that writes a picture byte writes the actor's word, and the entity's sprite has a clip for each value.

How the package scan buckets a scene: `routine` and `pc` are reference keys, `imm` is a string of addresses only (reference too), `code` and `table` name asset slots whose recipe lifts them (from the ROM), and the rest is ours. A scene written this way adds no needs-rewrite item. A scene with the ROM's numbers typed in would be needs rewrite, as a hook with them is.

Each step is a fact with evidence: `evidence."scenes.demo.steps[2]"` gives the clip, the frames and the listing's line it came from, `observed` when `gp scene find` drafted it.

## The scene player

One core system, `scenes`, plays every scene of every model. It is listed in `model.systems` where the original runs its scene routine among the frame's other work, so the order is stated as every other system's is.

| Part | What it does |
|---|---|
| Start | A scene starts on the frame its `while` rule state (`MACHINE.STATE`) is entered. Its actors appear as bodies of their entities, taking none of a level's spawn slots, and the scene is at its first step |
| Each frame | From the step it is at, the scene runs steps in order until it meets a wait that hasn't ended, as a routine that steps its script once a frame does. A wait's `each` writes run on each frame it holds |
| End | After the last step the scene fires its `then` event (a model event, `model.events`), so the game's rule moves on as the original's routine does when it sets the next game state. The actors stay while the `while` state holds and go when it is left |
| Freezing | A state's `stops` can name `scenes`, as any system that moves things. A scene runs on frames the world is frozen only when the model says so, with `stops` on the other systems instead |
| The actors' own systems | A body a scene drives runs no other system: the platformer systems leave it where the scene puts it. An actor with `free = true` keeps its components, for a demo in which the scene only sets where a body starts |
| A start in the middle | A clip or a resync that starts in the middle of a scene needs the scene's step and its wait's count. The state mapping gains `[[scenes]]`: the scene's `name`, the address of the routine's step byte with `cases` from its values to step numbers, and the wait's counter byte. A start loads the step and the count, and the actors as bodies from the slots the mapping already reads |
| The state | The canonical state gains each running scene's step and wait count. Its version is a hash of the layout's code, so this changes it with no one counting |
| Its record | `gp_sim::mechanic::COMPONENTS` gains `core.scene`, class `flow`. A hook a scene replaces leaves the catalog's debt, and a game whose only hooks were scenes runs in the page's own build, with no module |

The player is the interpreter's own code, so a scene plays the same in the native interpreter and in the page, and the web parity check covers it like any system.

## Checking a scene

| Check | What it catches |
|---|---|
| `gp model check` | An actor whose entity isn't in the model, a `while` state no machine has, a `then` event the model doesn't declare, a field no actor has, a `table` column the slot doesn't have. With `--assets`, an `imm` address outside every span of the scene's `code` slots |
| The scene's clips in the corpus, scored exact (`gp corpus score`) | Any frame of the scene the model draws differently: the screen measure compares the whole picture |
| `gp picture GAME --run RUN --clip ID --sprites DIR` over each of the scene's clips | Per frame, the original's sprites and the model's bodies: `missing` (a sprite of the original no actor covers) and `extra` (an actor over no sprite), with the runs of frames each holds for. A scene passes when no frame of its window has either. This is what names a step's error by where it shows: an actor that walks one frame late is `extra` on one side of its path and `missing` on the other for a frame at each of its stops |
| `gp scene check` (build row scene-check) | Each step against the original: the frame its `pc` ran on in the original (from the listing) beside the frame the player ran the step on. A wait one frame long or short shows here as the step after it moving by a frame, before any picture differs |
| The package scan in the gate | That the scene added no needs-rewrite item, before and after the patch as for any change ([package scan](package-scan.md), "In the gate") |

## The MCP tool

`scene_find` is read only. It runs `gp scene find --json` through the server's own gp, as `moments` and the others run theirs ([mcp](mcp.md)), and writes nothing. The agent puts the draft into the model by hand or with `model_edit`, and runs `gp model check`.

Its request:

```json
{
  "game": "rooftop",
  "clip": "title.003",
  "frames": "600..961",
  "routine": "$C400",
  "name": "demo"
}
```

`game` and `clip` are needed. Without `frames` the whole clip is read, and without `routine` the tool picks the routine as "Finding a scene" says and gives the others it weighed in `candidates`. `name` names the scene in the draft; without it, `scene`.

Its reply:

```json
{
  "routine": "$C400",
  "frames": "600..961",
  "candidates": [{ "routine": "$C400", "ran": 361, "before": 0 }],
  "actors": [
    { "actor": "cat", "slot": 0, "entity": "cat" },
    { "actor": "pigeon", "slot": 1, "entity": "pigeon" }
  ],
  "steps": [
    { "pc": "$C40C", "frame": 600, "kind": "set", "actor": "cat", "field": "x", "from": { "imm": "$C40B" } },
    { "pc": "$C42A", "frame": 601, "kind": "wait", "until": { "actor": "cat", "field": "x", "at_least": { "imm": "$C427" } }, "ran": 96 }
  ],
  "unexplained": [
    { "pc": "$C460", "writes": "$0480", "frames": "700..701", "why": "the state mapping names no field at $0480" }
  ],
  "toml": "[scenes.demo]\nroutine = \"$C400\"\n...",
  "next": "Put the draft into model.toml, then gp model check and gp picture --sprites over the clip"
}
```

`actors` takes each slot's entity from the state mapping's `[[slots.kind]]` by the object ID the slot held when the scene began; a slot whose ID no kind lists has `entity` empty, which is the person's or the agent's to name. `steps` hold the frame each first ran on and, for a wait, the frames it held. `toml` is the same draft as the model's text. A reply never holds a value of the ROM: each value is the address it is read from.

## What the model gains

Every part is optional and a named feature, so no version bumps ([model](model.md), "Features"). The paths, with the example's names:

| Path | What it holds |
|---|---|
| `scenes` | The model's scenes by name, a new top-level table beside `rules` and `counters` |
| `scenes.demo.routine` | The routine the scene models, by address (a reference key) |
| `scenes.demo.while` | The rule state the scene plays in |
| `scenes.demo.then` | The model event fired after the last step |
| `scenes.demo.code` | The `table` slots that lift the routine's code, which `imm` values are read from |
| `scenes.demo.actors.cat.entity` | The entity an actor is a body of |
| `scenes.demo.actors.cat.slot` | The original's object slot the actor is, for a start in the middle and for `gp scene find` |
| `scenes.demo.actors.cat.free` | The actor keeps its own components |
| `scenes.demo.steps[*].pc` | The instruction each step models, by address |
| `scenes.demo.steps[*].set` | A write: `actor`, `field`, and `from` or `add` |
| `scenes.demo.steps[*].wait` | A wait: `frames` or `until`, with `each` and `same_frame` |
| `scenes.demo.steps[*].wait.each` | The writes made on each frame of a wait |

The system `scenes` joins the core's systems, which `model.systems` lists. The features are `scene` (a scene with `set` and `wait.frames` steps), `scene.wait_until`, `scene.wait_each`, `scene.from_table`, `scene.from_actor` and `scene.free_actor`, each in `crates/gp-model/src/feature_list.rs`. The state mapping gains `[[scenes]]` (`name`, `step` with its `cases`, and `wait`) as feature `mapping.scenes`. The evidence table gains entries keyed by the steps' paths.

## Limits

A scene here moves objects. A scene that draws text or tiles on the background (an instruction screen's words) writes no object slot; its writes are listed as unexplained until a step kind `tile` writes a cell of the level, a later row. A demo that plays the game's own physics from a script of presses is an actor with `free = true` and a step that holds an input action, which is `ExtCx::hold`'s job for a hook today: that step kind, `hold`, is a later row too. A scene that draws random numbers or reads the player's input is not a script, and stays a hook until a component says what it does.

## Build order

Each row waits on the one before it (`--waits-on`), but scene-find and scene-mcp-tool need only scene-format and can go beside scene-player. A row lands with its measure in the same patch.

| Row | What it builds | Measure |
|---|---|---|
| scene-format | `scenes` in `gp_model::model` with its schema, the features, and `gp model check`'s reference checks | `cargo test -p gp-model scene` (a made-up model with a scene parses, formats back the same, and is refused with an actor of no entity, a `while` of no state and a `then` of no event) and `gp self schemas --check schema` |
| scene-player | The `scenes` system in `gp-sim`, the canonical state's scene words, and the record `core.scene` | `cargo test -p gp-sim scenes::tests` (on a made-up game, a scene of a set, a wait of 3 frames, a wait until with `each`, and a copy from another actor: the bodies' x and y frame by frame are the expected list, and `then` fires on the last step's frame) |
| scene-lift-sources | `imm`, `table` and `row_from` read from the lift as the interpreter compiles the model | `cargo test -p gp-interp scene_sources` (a made-up lift's code slot and table: each value read at its address, and an `imm` outside every code span refused by name) |
| scene-mapping | The state mapping's `[[scenes]]`, so a start in the middle of a scene loads its step and wait | `cargo test -p gp-interp mapping::tests::a_start_mid_scene_loads_its_step` (a made-up mapping and RAM: the start is at the step the step byte's case names, with the wait's count) |
| scene-find | `gp scene find ROM --clips SET --only CLIP --frames A..B [--routine ADDR]`, the draft from the code listing | `cargo test -p gp-cli --bin gp scene_find` (a made-up 6502 program that waits 8 frames on a counter and writes a slot's x from an immediate and from a table: the draft has the wait and both sets in order, each with its `pc` and source address) |
| scene-mcp-tool | The MCP tool `scene_find` over `gp scene find --json` | `cargo test -p gp-mcp scene_find` (the tool is listed with its schema and an arm in `call`, and on the same made-up program its reply has the steps and no ROM value) |
| scene-check | `gp scene check`, each step's frame in the original beside the model's | `cargo test -p gp-cli --bin gp scene_check` (a made-up scene with a wait a frame short is named by the `pc` of the step after it; the right one passes) |
| scene-first-port | In the games repo, the first game with a scene hook moves it into `scenes` and drops the hook | `gp corpus score` on that game's scene clips, every clip exact, and `gp model mechanics` on its model listing one hook fewer and no new debt; `gp picture --sprites` over the scene's clips with no frame of `missing` or `extra` |
