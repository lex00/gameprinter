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

## The hashes

A clip's recipe hash is the BLAKE3 of its canonical JSON: its ID, the BLAKE3 of its start's presses file in canonical form, its own spans and its frames. Names, reasons and the source are metadata, in no hash.

The recipe-set hash is `b3:` and the BLAKE3 of `gp-corpus-recipes 1` and a line `ID RECIPE-HASH` per clip, sorted by ID. A plan records only that hash, not each clip's, so to say which clips changed `gp corpus bump` compares with the plan as git has it at HEAD, when HEAD's is the recorded version, and `gp corpus import` with the plan before the import.

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
| `gp corpus import GAME [--template DIR] [--from HOME] [--session NAME] [--remove ID] [--write]` | The game's mechanic sessions as the plan's clips, below |

## Import

`gp corpus import GAME` turns the clips of the game's mechanic sessions (`$GP_HOME/mechanics/GAME/`) into the plan's clips and `inputs/` files, so a clip joins the corpus without a script. The game's folder is GAME in the games checkout (its folder's name, or a source ID its `roms.toml` lists), or `--template DIR`.

Each clip of each session becomes a `played` clip `PREFIX.SESSION.NNN`: its input from power-on to its start (the stored path's, else its own log's) as `inputs/ID.start.presses`, and its own presses as `inputs/ID.presses`. `PREFIX` is `[import] prefix`, else GAME; its name is the session, its number and the person's note; its `why` is `[import] why` with `{session}` filled in. A clip the plan has already whose start and presses are the same is kept as it is, with its name, so only what the sessions changed moves.

An import removes a clip only when it is told to: `--remove ID`, repeated, or a pattern with `*` (`dk.climb-sweep.*`). A clip already in the plan whose session isn't in the home (a probe's clips, or another person's sessions) is kept as it is, and so is one the rules below now leave out; the output names the latter with the `--remove` that would take it out (`kept_without_session` in the JSON lists both). `--session NAME`, repeated, imports only those sessions' clips, and leaves the plan's clips of other sessions as they are.

| Left out | How it is known |
|---|---|
| Sealed runs | A session named `sealed*`; a clip whose input is a sealed run's: the same log, path and start as a clip under `$GP_HOME/sealed/GAME/sealed-run/clips/`, the same input from power-on as a `.log` directly under `$GP_HOME/sealed/GAME/` or one of those clips, or one that plays into a sealed run's own presses (docs/sealed.md) |
| Rows out of scope | The quest log row's `scope = "out"` |
| By name | `[import] exclude`: a session (`title`) or a session's clip (`walk.006`), with why |
| What can't be a recipe | A clip with no frames, or one that presses reset or a third or fourth controller |

Without `--write` nothing is written. With it, the plan is edited in place: a new clip goes after the last clip (so before the changelog), a changed clip's block is written again, and a removed clip's block and its `inputs/` files go. Every other byte stays as it was. Then the version is recorded by the rules above, with a changelog line naming the clips. A mapping that maps no field yet (as `gp game new` writes it) is refused, since a clip scored over nothing agrees on every frame. `--from HOME` reads the sessions, paths and sealed runs from another home.

The old import also measured new clips' curves in a capture run (`--run`) and recorded the version only once every clip had one. A plan keeps no curves, so the version is recorded on the same `--write`.

On 2026-11-12, from the user's Donkey Kong sessions (the old home, `~/.game-printer`), with Donkey Kong's `[import]` above and prefix `donkey-kong-oe`: the import kept the plan's 21 session clips, left out the same 8 sessions as the old binary, and added the one clip neither corpus has yet (`two-p-over` 001). With those 21 clips removed and imported again, all 222 `inputs/` files came back byte for byte, with the same 114 clip IDs and recipe-set hash; six clips' names and reasons, edited by hand since, came back as the import writes them.

## Reduce

`gp corpus reduce GAME` proposes the fewest clips of a corpus that still run every routine and branch way the whole corpus runs. It reads the clips the way `gp corpus score` does (the corpus plan, else `clips.toml`) and changes nothing: `--write PLAN` writes the kept and dropped clips, each with a reason, to a TOML file outside any git work tree.

Each clip's own frames, from its start to its end, are counted and cut where the stage byte changes. The stage byte is the first RAM byte of the mapping field the model's `[stages] counter` names (`$53` on Donkey Kong), or `--stage-byte`. A clip's features are the routines it ran and the branch ways it took, each tagged with the stage it ran on; system code isn't tagged unless `--stage-system` is given. The cover is greedy: the clip with the most features not held yet per frame goes first, then a reverse pass drops, dearest first, any clip whose features are all held without it. `--twice` holds each tagged feature by two clips where two take it. The first clip each fix commit of the model, mapping or hooks names (from `git log`) stays whatever the cover chooses, unless `--no-guards`, and so does each clip `--keep` names. Clips that are the same run (the same input from power-on and the same own frames) count as one.

`--values` then goes through each sweep family, the clips with the same start whose own presses differ only in timing. It keeps a member when a compare near a boundary (the register one below, on or one above the operand) at an event branch shows a class no kept member of its family shows.

On an NES ROM the windows are counted as `gp code listing` counts them and kept in the events store (`$GP_HOME/work/events`, `--store`; `--no-store` counts them all again). On an arcade romset each run is counted on gp-arcade with its levels watching both Z80s, as `gp reach --window` does. A routine is an entry of the code map, named `main:5FAC` or `sound:0276`, and a branch way is a conditional's edge taken or fallen. The stage byte is a main CPU address. A model with no `[stages] counter` (Tapper's has none yet) is counted as one stage, `all`. Arcade windows aren't kept in the store, and `--values` is refused on a romset, since the Z80 runs log no compares yet.

On 2026-11-12, on 24 Donkey Kong clips from the plan written as a `clips.toml`, the old game-printer binary and this one gave the same plan JSON (22 kept), with and without `--values --twice`. Only the plan's `note` differs, since it names the clips rather than `clips.toml`. On Tapper's 52 clips it kept 22, 19,887 of 39,121 own frames, and named 18 clips any cover needs and 200 untaken branch ways in 75 routines.
