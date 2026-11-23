# Scoring corpus format

A game's corpus is the set of clips it is scored against: recipes anyone makes again from their own ROM, with a name, a semver version and a hash, so a score always says which version of which corpus it is on. In gameprinter a game's corpus is its corpus plan, `corpus.toml` at format 1.0.0 in the game's folder, with each clip's presses in `inputs/`. A plan keeps no curves: `gp corpus score` and `gp game score` derive them from the ROM each time.

The code is `crates/gp-cli/src/plan.rs` (reading a plan and its hashes), `corpus_score.rs` (`init`, `bump`, `score`) and `corpus_import.rs`. Ported from game-printer's docs/formats/scoring-corpus.md, which described the older form: a clip set (`clips.toml`, with curves) and a manifest beside it. gameprinter still reads that form for `score`, `init` and `bump`, and nothing in the games checkout uses it.

## The plan

Donkey Kong's, cut short:

```toml
[corpus]
format = "1.0.0"
name = "donkey-kong-oe-25m"
version = "7.0.0"
title = "Donkey Kong: Original Edition, 25m"
recipes = "b3:398917c6..."

[target]
game = "donkey-kong-oe"
rom = "45440d8e..."

[needs]
mapping = "22"

[import]
why = "a clip of the {session} mechanic session"

[[import.exclude]]
what = "walk"
why = "sessions from before the corpus 4.0.0 cut; their clips were never in the reduced set"

[[clip]]
id = "dk.walk.001"
name = "walk 001: walk"
source = "played"
start = "inputs/dk.walk.001.start.presses"
presses = "inputs/dk.walk.001.presses"
frames = 352
why = "a clip of the walk mechanic session, 25m"

[[changelog]]
version = "7.0.0"
date = "2026-11-08"
says = "The corpus plan: curves leave the package and are derived (format 1.0.0); no clip or press changed"
```

| Key | What |
|---|---|
| `corpus.name`, `version`, `title` | Lowercase letters, digits, `-` and `_`; `MAJOR.MINOR.PATCH` by the rules below; a title for people |
| `corpus.recipes` | The recipe-set hash, below |
| `target.game`, `rom`, `patch` | The source ID, the ROM's headerless SHA-256, and for a hack the patch's |
| `needs.mapping` | The state mapping's schema |
| `import` | How `gp corpus import` names and leaves out clips, below. In no hash |
| `clip` | Each clip: `id`; `name`, `source` and `why` (metadata); `start`, a `.presses` file of the presses from power-on, whose frames are where the clip starts; `presses`, a `.presses` file or spans inline; `frames` |
| `changelog` | One entry a version: `version`, `date`, `says`, and the IDs `added`, `removed` and `changed` |

A `.presses` file is `gp-presses 0`, `frames N` and a span a line (`right@18-351`).

An arcade clip's presses carry an `arcade SET COLUMNS` line, which names the romset the clip plays. A corpus can hold clips on more than one of the game's sets. A clip whose set is another `[[rom.entry]]` of the game's `package.toml` (matched by its `id` or its MAME `system`, whatever its `role`) is made and scored on that set's ROM, found by its hash in the ROM folder. It plays with that set's assets, lifted by the game's asset map into `$GP_HOME/work/assets/SOURCE@SET` and lifted again when the map changes. Every other clip plays the scored set, and `[target] rom` stays the scored set's hash. A clip on a set the package doesn't name is an error, and so is a clip whose set's ROM isn't in the ROM folder; the error names the set. `gp corpus score`, `gp clip score`, `gp picture` and `gp model gate` score each clip on its own set, and `gp clip presses` replays a new clip on its own set. The set isn't in the recipe hash: it is the presses' own line, which the start's hash already holds.

## The hashes

A clip's recipe hash is the BLAKE3 of its canonical JSON: its ID, the BLAKE3 of its start's presses file in canonical form, its own spans and its frames. Names, reasons and the source are metadata, in no hash.

The recipe-set hash is `b3:` and the BLAKE3 of `gp-corpus-recipes 1` and a line `ID RECIPE-HASH` per clip, sorted by ID. A plan records only that hash, not each clip's, so to say which clips changed `gp corpus bump` compares with the plan as git has it at HEAD, when HEAD's is the recorded version, and `gp corpus import` with the plan before the import.

## Score records

`gp corpus score GAME --run NAME --write TEMPLATE` writes a score record to `TEMPLATE/evidence/MODEL/CORPUS.json`, named by the model's hash (`model.hash`) and the corpus's recipe-set hash. It is an in-toto Statement (`_type` `https://in-toto.io/Statement/v1`) with `predicateType` `urn:game-printer:score-record:0` and these predicate fields:

| Field | What |
|---|---|
| `record_format` | `0.3.0`; `0.2.0` before `results.picture`, `0.1.0` before `module` |
| `corpus` | `name`, `version` and `hash`, the recipe-set hash |
| `rom` | `sha256` of the ROM or romset the clips were made from, and `patch` for a hack, else null |
| `model` | `hash` (BLAKE3 of its canonical tree), `short` and `name` |
| `mapping` | `hash` (BLAKE3 of mapping.toml) and `schema` |
| `contract` | BLAKE3 of fidelity.toml (of nothing when there is none) |
| `module` | `{"hash": ...}`, the BLAKE3 of the hooks module the clips were played through (named with `--module`, or built for the game), as `gp game hash` prints it; null for a model with gp's own hooks alone. Added in 0.2.0 |
| `runtime`, `gp` | The interpreter and scenario version; gp's version and build |
| `results` | `clips`, `exact`, `errors`, `fidelity_permille`, `bars` (`held`, `met`) or null, and `picture`. `exact` is on state alone: clips whose state curve matches the original's. `picture` (added in 0.3.0) is the screen apart: `frames` compared, `frames_same` and `clips_same`, the clips whose every compared screen frame is the same |
| `clips` | One row a clip: its result (`exact`, on state), curve, pairing and bars, and `screen` when its screen was compared: `sprites` and `background`, each `same` (frames within the screen tolerance) of `total` (frames compared) |

A clip's picture, in `results.picture` and in the publish rule, is read from its `screen`: a frame is the same only when both layers are, so a clip's frames the same are the lower of its two layers' `same` and its frames compared the higher of their `total`. A clip is the same on picture when those are equal and more than none. `gp corpus score` says the two on two lines: "On corpus NAME VERSION (HASH), on state: E of N clips exact, fidelity P%, X couldn't be scored. Model HASH." and "On picture: S of F screen frames the same, C of N clips the same on every screen frame compared."

`gp registry publish` reads it for an arcade game ([packages.md](../packages.md#publishing-an-arcade-game)), taking one only when every clip is exact on state and the same on picture; for a game with hooks the record's `module.hash` must be the module the package ships (`play/module.wasm`), and a 0.1.0 record, which has no `module`, is refused; for a game without hooks a 0.1.0 record is taken.

## Versions

| Change | Step | Why |
|---|---|---|
| A clip added | Minor | A score on the old version is still a score on its clips |
| A clip removed | Major | A score on the old version covers a clip the new one doesn't |
| A clip's recipe changed: its start, presses or frames | Major | The same ID measures something else |
| Another ROM or mapping schema | Major | The clips are for another game, or measured another way |
| A changed recipe-set hash with no recorded version to compare clip by clip | Major | Nothing says it is less |

| Command | What it does |
|---|---|
| `gp corpus bump DIR [--write] [--says S] [--date D]` | The step from the version the plan records to its clips now, and the next version. `--write` sets `version` and `recipes` in `[corpus]` where they stand and adds a changelog entry at the end; every other byte stays |
| `gp corpus init DIR --name N [--version 1.0.0] [--title T] [--patch SHA]` | For the older form only: writes `corpus.toml` beside `DIR/clips.toml`. A game started with `gp game new` has a plan with no clips already, so it needs no `init` |
| `gp corpus import GAME [--template DIR] [--from HOME] [--session NAME] [--remove ID] [--replace ID] [--write]` | The game's mechanic sessions as the plan's clips, below |

## Import

`gp corpus import GAME` turns the clips of the game's mechanic sessions (`$GP_HOME/mechanics/GAME/`) into the plan's clips and `inputs/` files, so a clip joins the corpus without a script. The game's folder is GAME in the games checkout (its folder's name, or a source ID its `roms.toml` lists), or `--template DIR`.

Each clip of each session becomes a `played` clip `PREFIX.SESSION.NNN`: its input from power-on to its start (the stored path's, else its own log's) as `inputs/ID.start.presses`, and its own presses as `inputs/ID.presses`. `PREFIX` is `[import] prefix`, else GAME; its name is the session, its number and the person's note; its `why` is `[import] why` with `{session}` filled in. A clip of an arcade session (a `gp-arcade-input` log) names its set's buttons (`coin`, `start1`, `up+tap`) and both its files carry the log's `arcade SYSTEM COLUMNS` line, so it plays on that set, on the screen page too, whose arcade player refuses NES presses. A clip the plan has already whose start and presses are the same is kept as it is, with its name, so only what the sessions changed moves.

An import removes a clip only when it is told to: `--remove ID`, repeated, or a pattern with `*` (`dk.climb-sweep.*`). A clip already in the plan whose session isn't in the home (a probe's clips, or another person's sessions) is kept as it is, and so is one the rules below now leave out; the output names the latter with the `--remove` that would take it out (`kept_without_session` in the JSON lists both). `--session NAME`, repeated, imports only those sessions' clips, and leaves the plan's clips of other sessions as they are.

An import changes a clip already in the plan only when it is told to: `--replace ID`, repeated, or a pattern with `*`. A clip whose session now gives other presses, start or frames is otherwise held: kept as it is, byte for byte, and listed in `held` with what would change (its frames then and now, its start, whether its presses hold other buttons, and `layout`, the buttons its files would be written in when that changes: an older import wrote an arcade session's clips in NES names with no `arcade` line), so a curated clip is never made shorter by a session of the same ID. A session log's `system` line is the set loaded when the clip was saved (the page's picker's, or an older triage's), which can be another set of the same board than the one the plan scores the clip on, and session.toml's `rom` is only the set the session was first made on. So a `played` clip already in the plan on an arcade set keeps that set when its log names another set with the same button columns: it is compared and written (on `--replace` too) on the plan's set, and `other_set` in the JSON lists each such clip with the plan's set and the log's (`{"id", "plan", "log"}`), with a line of the text saying so. A log on a set with other columns is held, its `layout` naming that set. A clip `--replace` names gets the session's presses, start and frames and keeps its name, why, level and its other lines. The JSON's `replaces` lists what the call changes or removes of the clips already there, each with `says` in words; gp_run reads it as the call's plan and shows it on the person's card (docs/mcp.md, "Writes that change what is already there").

| Left out | How it is known |
|---|---|
| Sealed runs | A session named `sealed*`; a clip whose input is a sealed run's: the same log, path and start as a clip under `$GP_HOME/sealed/GAME/sealed-run/clips/`, the same input from power-on as a `.log` directly under `$GP_HOME/sealed/GAME/` or one of those clips, or one that plays into a sealed run's own presses (docs/sealed.md) |
| Rows out of scope | The quest log row's `scope = "out"` |
| By name | `[import] exclude`: a session (`title`) or a session's clip (`walk.006`), with why |
| What can't be a recipe | A clip with no frames, or one that presses reset or a third or fourth controller |

### Scripted clips

Two people record a row differently, so their sessions' presses differ, and so would the clips each model is scored on. With an `[import.script]` table a session's recording only reaches the clip's start; its own presses come from the recipe:

```toml
[import.script]
seed = 1
frames = 600
buttons = ["left", "right", "up", "down", "tap"]
hold = [2, 30]
gap = [0, 15]
arcade = "tapper CSRLDUT"
```

| Key | What |
|---|---|
| `seed` | The recipe's seed. The session's name (the row) and the clip's number are mixed in through BLAKE3, so a row's clip 1 presses the same for everyone |
| `frames` | The clip's own frames, whatever the person's clip ran |
| `buttons` | The buttons pressed, one at a time, as spans name them; on an arcade set the names its columns give (`tap`, `coin`, `start1`) |
| `hold`, `gap` | A press's frames held and the frames between two presses: least and most, drawn from a splitmix64 stream |
| `arcade` | An arcade set's `SYSTEM COLUMNS`, written as the `arcade` line of both the clip's presses files, its start's and its own; left out on the NES |

Such a clip is `source = "scripted"`, with the person's start presses and the recipe's own presses. The import adds, keeps and holds scripted clips as it does played ones: a clip already in the plan as `played` is held, not rewritten, until `--replace` names it. A button the controls don't have, a hold under one frame or a range whose least is above its most is refused.

Without `--write` nothing is written. With it, the plan is edited in place: a new clip goes after the last clip (so before the changelog), a replaced clip's block takes the session's presses, start and frames with its other lines kept, and a removed clip's block and its `inputs/` files go. Every other byte stays as it was. Then the version is recorded by the rules above, with a changelog line naming the clips. A mapping that maps no field yet (as `gp game new` writes it) is refused, since a clip scored over nothing agrees on every frame. `--from HOME` reads the sessions, paths and sealed runs from another home.

The old import also measured new clips' curves in a capture run (`--run`) and recorded the version only once every clip had one. A plan keeps no curves, so the version is recorded on the same `--write`.

On 2026-11-12, from the user's Donkey Kong sessions (the old home, `~/.game-printer`), with Donkey Kong's `[import]` above and prefix `donkey-kong-oe`: the import kept the plan's 21 session clips, left out the same 8 sessions as the old binary, and added the one clip neither corpus has yet (`two-p-over` 001). With those 21 clips removed and imported again, all 222 `inputs/` files came back byte for byte, with the same 114 clip IDs and recipe-set hash; six clips' names and reasons, edited by hand since, came back as the import writes them.

## Could a person play it

A clip proves the model matches the original for its presses; it says nothing about whether a person could press that way. `gp clip feasible GAME [ID...]` measures each corpus clip's own presses (from its start, not the path to it) against a human floor at the machine's own frame rate (the model's `frame_rate`: 30 for an MCR board, 39375000/655171 for the NES), and prints for each clip:

| | |
|---|---|
| shortest press | The fewest frames a control is held, and in ms. A press held into the clip's first frame or past its last is cut by the clip's edge, its length unknown, so it is left out of this (not of the gaps or the count) |
| shortest gap | The fewest frames let go between two presses of one control |
| a second | The most presses one control starts within a second |
| under the floor | How many presses break any rule of the floor, of all the clip's presses |
| verdict | `a person could`, or `a person could not` with the first press under the floor: its frame, its control and why |

A control is one button or one direction: up, down, left and right are each their own, so a lever pushed up and at once down is no gap. The last line counts the clips a person could play, and how many of each `source`. `gp corpus show` prints the same count with the clips a person could not play, and the scoreboard (`gp loop health`, the loop's page) has it as a line, A PERSON COULD PLAY: N of M of the game's tests. Nothing is changed: it measures and labels.

The floor is one table in `crates/gp-cli/src/feasible.rs` (`FLOOR`):

| Rule | Floor | Where it is from |
|---|---|---|
| A press | at least 50 ms | The owner, 2026-11-22, on Tapper's row clips-a-person-could-play: "if we are going to require it to be held for any length of time, then we have to know that our tests are in fact possible for a human to achieve"; the row's starting numbers |
| Let go between two presses of one control | at least 50 ms | The same |
| Presses a second on one control | at most 8 | The same |

At 30 frames a second a frame is 33 ms, so a press of 1 frame or a gap of 1 frame is under the floor and 2 frames (67 ms) are not; on the NES 3 frames are 49.9 ms, under it, and 4 are 67. With these numbers the press and gap rules already keep one control under 8 a second at either rate (a press and a gap of at least 2 frames each at 30 a second, 4 each on the NES: at most 7.5 a second); the rate rule matters for a floor with a shorter press or gap.

The day the row was raised, Tapper's 76 clips held 1341 tap presses, 1055 of them exactly 8 frames; 1651 direction presses, 1592 of them exactly 1 frame; and 458 gaps between two taps of exactly 2 frames, with 68 of the clips marked `played`: a script's presses, not a person's.

## Reduce

`gp corpus reduce GAME` proposes the fewest clips of a corpus that still run every routine and branch way the whole corpus runs. It reads the clips the way `gp corpus score` does (the corpus plan, else `clips.toml`) and changes nothing: `--write PLAN` writes the kept and dropped clips, each with a reason, to a TOML file outside any git work tree.

Each clip's own frames, from its start to its end, are counted and cut where the stage byte changes. The stage byte is the first RAM byte of the mapping field the model's `[stages] counter` names (`$53` on Donkey Kong), or `--stage-byte`. A clip's features are the routines it ran and the branch ways it took, each tagged with the stage it ran on; system code isn't tagged unless `--stage-system` is given. The cover is greedy: the clip with the most features not held yet per frame goes first, then a reverse pass drops, dearest first, any clip whose features are all held without it. `--twice` holds each tagged feature by two clips where two take it. The first clip each fix commit of the model, mapping or hooks names (from `git log`) stays whatever the cover chooses, unless `--no-guards`, and so does each clip `--keep` names. Clips that are the same run (the same input from power-on and the same own frames) count as one.

`--values` then goes through each sweep family, the clips with the same start whose own presses differ only in timing. It keeps a member when a compare near a boundary (the register one below, on or one above the operand) at an event branch shows a class no kept member of its family shows.

On an NES ROM the windows are counted as `gp code listing` counts them and kept in the events store (`$GP_HOME/work/events`, `--store`; `--no-store` counts them all again). On an arcade romset each run is counted on gp-arcade with its levels watching both Z80s, as `gp reach --window` does. A routine is an entry of the code map, named `main:5FAC` or `sound:0276`, and a branch way is a conditional's edge taken or fallen. The stage byte is a main CPU address. A model with no `[stages] counter` (Tapper's has none yet) is counted as one stage, `all`. Arcade windows aren't kept in the store. With `--values` the runs keep a Z80 compare log as the 6502's are kept: each conditional jump, call or return straight after a `cp` on the same CPU logs the compare's class (A minus the operand: an immediate, a register, or the byte `(hl)` or `(ix+d)` read) each time it goes the other way from its last run, and a sweep family's window runs to one second after the last frame its presses differ, 30 of the board's frames. Tapper's 54 clips with `--values`: 169 value features, one sweep family (tapper.tip.001 and .002), none added, in 70 s.

On 2026-11-12, on 24 Donkey Kong clips from the plan written as a `clips.toml`, the old game-printer binary and this one gave the same plan JSON (22 kept), with and without `--values --twice`. Only the plan's `note` differs, since it names the clips rather than `clips.toml`. On Tapper's 52 clips it kept 22, 19,887 of 39,121 own frames, and named 18 clips any cover needs and 200 untaken branch ways in 75 routines.
