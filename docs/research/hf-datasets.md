# A game's data model as a Hugging Face dataset

> In gameprinter (2026-11-12): `gp dataset export --hf DIR --template GAME [--run R [--measures]]` and `gp dataset catalogue --out DIR GAME...` take game packages (games/donkey-kong, games/tapper). A package's clips are its corpus plan's, and their `.presses` files from `inputs/` are copied beside the tables. The ROM check finds the ROM the package accepts in `$GP_ROM_DIR`, an NES image or an arcade romset. On Donkey Kong the facts, mechanics and mapping tables are byte for byte game-printer's; Tapper's export holds 52 facts, 11 mapping fields and 52 clips. The code is `crates/gp-cli/src/dataset_hf.rs`. The rest of this page is game-printer's research report of 2026-10-30, kept as history.

A research report of 2026-10-30, on branch `research/hf-draft`. The user, 2026-10-30: "continue to think about how we are extracting a giant data model from each rom and what it might look like on hugging face".

It builds on [the data sets report](datasets.md), which drew the sharing line, made `gp dataset export` for a capture run's clips, and recommended a card, Croissant and score records. That export is about one run. This one is about the template: the model's facts and their evidence ([the model](../model.md#evidence)), the mechanic parts it uses ([mechanics](../mechanics.md)), the [state mapping](../formats/state-mapping.md), the [clip set](../formats/clips.md) and the [score records](../formats/scoring-corpus.md#score-records). Together they are what Game Printer has learned about a game, and none of it is the game.

Nothing was published. The drafts are under `~/.game-printer/work/hf-draft/`, with no login, no upload and no network writes.

## Summary

| Question | Answer |
|---|---|
| What is the data model of a ROM | Five tables a template already implies: every model fact with its evidence, the mechanic parts and their options, the RAM fields and their roles, the clip recipes, and the score records. Per-frame measures are a sixth, from a run, and held back for commercial games |
| What it looks like on the Hub | One dataset repo per game: `data/<config>.parquet`, a `README.md` card whose YAML has one config a table, `croissant.json`, `gp-dataset.json` with the ROM hash, model hash and gp version, the `.presses` files the clips start from, and `LICENSE.md` for a set that stays local |
| Built | `gp dataset export --hf DIR --template T [--run R [--measures]]` and `gp dataset catalogue --out DIR TEMPLATE...`, in `crates/gp-cli/src/dataset_hf.rs`, reusing the Parquet writer and Croissant code of `gp dataset export` |
| Sizes | Super Mario Bros.: 4,987 facts, 305 mechanic uses, 116 mapping fields, 87 clips, 87 scores, 620 KB in all. Moss Hop: 95, 11, 15, 0, 0. C4 Remastered: 4,987, 305, 116, 4, 0 |
| The cross-game set | Five models (SMB, C4, Moss Hop, Mega Man 2, SMB2): 11,378 facts, 784 mechanic uses, 46 mechanic parts in the taxonomy. Six parts are in all five games |
| Legal | No table holds ROM bytes, level data, graphics or code text. Evidence is cited by file, line, label and routine name; the quote a ref checks against is never written. The export checks its own files for 16-byte runs of the ROM and fails on one; on SMB1 and C4 it found none, and the pre-commit hook's text check (`rom_text.py`) found none either |
| Still the user's call | Whether any commercial game's set is published at all, under what license, and whether `measures` ever are. C4's author has given no permission (`sources/nes.toml`) |

## 1. Per-game layout

```text
smb1-ntsc-prg0/                 # one Hub dataset repo per template
  README.md                     # card: Hub YAML, what it needs, use, configs, files, license
  LICENSE.md                    # "not decided" for a commercial game; absent for an open one (CC0)
  croissant.json                # Croissant 1.0: a FileObject and a RecordSet a config, with keys
  gp-dataset.json               # layout version, ROM SHA-256 (and patch), model, mapping, corpus, gp, files, ROM scan
  data/facts.parquet
  data/mechanics.parquet
  data/mapping.parquet
  data/clips.parquet
  data/scores.parquet
  data/measures.parquet         # only with --run; for a commercial game only with --measures
  clips/*.presses               # the player's own input to a save point, as committed in the template
```

The card's front matter is the Hub's own keys, so the viewer shows one tab a table:

```yaml
pretty_name: "Game Printer: Super Mario Bros. (NTSC, PRG0)"
license: other
license_name: game-printer-recipes
license_link: LICENSE.md
language:
- en
task_categories:
- other
- reinforcement-learning
tags:
- game-printer
- game-mechanics
- nes
- reverse-engineering
- tabular
size_categories:
- 1K<n<10K
configs:
- config_name: facts
  data_files:
  - split: train
    path: "data/facts.parquet"
  default: true
- config_name: mechanics
  ...
```

Moss Hop's says `license: cc0-1.0`, the data license datasets.md section 11 recommends. `time-series-forecasting` is added to the task categories when `measures` is in. Each Parquet file carries `gp.dataset.hf_format`, `gp.game`, `gp.model`, `gp.version`, `gp.table`, `gp.key` and `gp.units` in its metadata, as the run export's do. Files are uncompressed with one row group, so the same template gives the same bytes; the test checks Moss Hop's twice.

### The configs

| Config | Key | One row a | Columns |
|---|---|---|---|
| `facts` | `game`, `path` | model fact (`gp_model::facts`: every leaf but the header, the evidence table and descriptions) | `value` (exact decimal text, a list as JSON), `value_f64`, `value_type`, `unit` (from the mechanic's record), `class` (mechanic, parameter, content, presentation), `mechanic`, `kind` (stated, observed, inferred), `confidence`, `evidence_entry`, `refs`, `ref_at`, `ref_in`, `label`, `routine`, `symbol`, `refs_json`, `core`, `extensions`, `model_hash` |
| `mechanics` | `game`, `mechanic`, `used_by` | mechanic part at one model path (`gp_interp::mechanics::catalog`) | `version`, `implementation`, `carrier`, `system`, `class`, `like`, `hook`, `options` (JSON of the keys set there), `option_names`, `params_declared`, `params_set`, `about` |
| `mapping` | `game`, `scope`, `path` | state mapping field, including each object slot's fields | `schema`, `role`, `owner`, `address`, `ram`, `bytes`, `retro_type`, `signed`, `bits`, `frac`, `scale`, `offset`, `flag`, `invert`, `digits`, `whole`, `values`, `cases`, `presentation`, `shared`, `note` |
| `clips` | `game`, `clip_id` | clip of `clips.toml` | `name`, `source`, `frames`, `presses`, `start_recording`, `recording_name`, `recording_notes`, `start_presses`, `start_frame`, `model_frame`, `context`, `level`, `curve`, `recipe` (the corpus's recipe hash), `holds`, `guards`, `why` |
| `scores` | `game`, `model_hash`, `corpus_hash`, `clip_id` | clip in a score record under `evidence/` | `corpus`, `corpus_version`, `recipe`, `curve`, `model_curve`, `exact`, `permille`, `score`, `error`, `mapping_schema`, `contract`, `runtime`, `gp_version`, `gp_build`, `record` |
| `measures` | `clip_id`, `frame`, `path` | clip, frame and measure of a run | As in datasets.md section 7 |

Evidence has three name columns because the disassemblies don't mark routines. `label` is the nearest label at or above the ref's line, often a branch target such as SMBDIS.ASM's `ChkOnScr`. `routine` is the nearest one that code reaches with `jsr`, `jmp` or a `.dw`/`.addr` table, which is the routine the line is in (`PlayerBGCollision`). `symbol` is the constant or label the line itself defines (`Goomba`, `JUMP_SPEED`). They are read from the pinned download under `$GP_HOME/work/` (`smbdis`, `mm2dis`, `smb2dis`) or the repo file, and are null when it isn't there; the rest of the row doesn't depend on it.

The mapping's `role` is read from the path (position, speed, state, flag, counter, timer, camera, input, presentation, value). `retro_type` is the bytes as Gym Retro's `data.json` writes a type (`|u1`, `<u2`, `>n3`) when they are contiguous, so the table reads like a Data Crystal RAM map or a Gym Retro integration with Game Printer's model paths on top.

`mechanics.options` gives scalars and short lists as they are, and a table or a long list by its size (`"bands": "[list of 4]"`, `"head": "{table of 5}"`), so no level or table data rides along with an option.

## 2. Row counts and examples

Built 2026-10-30 from the templates at commit `72b722ca` plus this branch, with `GP_ROM_DIR=/Users/alex/Documents/roms/nes`, and loaded with pyarrow 23.0.1; PyYAML 6.0.3 parsed every card's front matter and the configs matched the files. The `datasets` library and DuckDB aren't installed here, so they weren't tried.

| Game | facts | mechanics | mapping | clips | scores | Size |
|---|---|---|---|---|---|---|
| `mosshop` | 95 | 11 | 15 | 0 | 0 | 100 KB |
| `smb1-ntsc-prg0` | 4,987 | 305 | 116 | 87 | 87 | 620 KB |
| `c4-remastered-1` | 4,987 | 305 | 116 | 4 | 0 | 452 KB |
| `catalogue` (5 games) | 11,378 | 784 | | | | 612 KB, and 46 taxonomy rows |

| Game | Model hash | Core, platformer | Mapping | Corpus | Facts stated, inferred, observed |
|---|---|---|---|---|---|
| Moss Hop | `67ab83c1d4c8` | 0.4.0, 0.4.0 | 1 | none | 94, 1, 0 |
| Super Mario Bros. | `f7554134925e` | 0.4.17, 0.4.24 | 15 | `smb1-ntsc-prg0` 2.0.0 | 3,650, 961, 376 |
| C4 Remastered | `7d931363f786` | 0.4.17, 0.4.24 | 15 | none | 3,650, 961, 376 |

C4's model is derived from Super Mario Bros.' (`gp template derive`), so it has the same paths with the hack's values where `delta.toml` changes them. Of Super Mario Bros.' facts, 3,587 have a `routine`. SMB1's 87 score rows are from the one record, on model `83dc2714e720`: 63 exact, 990 permille on average. The model has moved on since (its hash is now `f7554134925e`), which is how a score record should read: a score of that model, not of this one.

Three `facts` rows (null columns left out):

```json
{"game": "smb1-ntsc-prg0", "path": "entities.mario.jump.gravity", "value": "0.5625", "unit": "px/frame^2", "class": "parameter", "mechanic": "platformer.jump", "kind": "stated", "confidence": 100, "ref_at": "SMBDIS.ASM:11923", "ref_in": "smbdis@7e8e9777", "label": "ChkOnScr", "routine": "PlayerBGCollision"}
{"game": "smb1-ntsc-prg0", "path": "entities.goomba.walk.speed", "value": "0.5", "unit": "px/frame", "class": "parameter", "mechanic": "platformer.walk", "kind": "stated", "confidence": 90, "ref_at": "SMBDIS.ASM:600", "ref_in": "smbdis@7e8e9777", "symbol": "Goomba"}
{"game": "mosshop", "path": "entities.player.jump.speed", "value": "-5", "unit": "px/frame", "class": "parameter", "mechanic": "platformer.jump", "kind": "stated", "confidence": 100, "ref_at": "games/mosshop/mosshop.s:249", "label": "move_player", "routine": "move_player"}
```

A fact's evidence is its most specific entry, so `entities.mario.jump.gravity` carries the refs of `entities.mario.jump`; the first is the collision routine, and `refs_json` lists them all.

A `mechanics` row and a `mapping` row of Moss Hop's:

```json
{"game": "mosshop", "mechanic": "platformer.contact", "version": "0.4.24", "implementation": "gp_sim::systems::contact", "used_by": "entities.slug.contact", "options": "{\"bounce\":-3,\"otherwise\":\"kill-player\",\"stomp_depth\":7}", "params_declared": 11, "params_set": 3}
{"game": "mosshop", "path": "entities.player[0].x", "role": "position", "owner": "platformer.run", "address": 16, "ram": "[\"0x0010\",\"0x0011\"]", "retro_type": "<u2", "note": "PLAYER_X, PLAYER_X_HI"}
```

Super Mario Bros.' timer is `counters.time`, role `counter`, at `0x07F8` to `0x07FA`, `retro_type` `>n3`, note `GameTimerDisplay`. Its `entities.mario[0].x` has no `retro_type`: the bytes are `0x0086` and `0x006D`, not contiguous, with the fraction at `0x0400`.

A `clips` row and a `scores` row:

```json
{"game": "c4-remastered-1", "clip_id": "c4.w1_1.walk", "name": "Level 1: walk right for 3 seconds", "source": "aimed", "frames": 183, "presses": "[\"right@3-182\"]", "start_presses": "clips/w1_1.start.presses", "start_frame": 217, "context": "level_start", "level": "w1_1", "curve": "4e35139901c5086f257a6e305787d1d2d81fa67b67d76da6c73de97f6e4e8622", "recipe": "89f54b5986df2bdd40472a0dbd66e1bb1b4abe47a9374bb2a1bf8676300ad421"}
{"game": "smb1-ntsc-prg0", "model_hash": "83dc2714e7204acc6abd540de8d4523b0794d89de651e154961e3c7b0d4c93a3", "corpus": "smb1-ntsc-prg0", "corpus_version": "2.0.0", "clip_id": "gen.w1_1.start.t1.nothing", "exact": true, "permille": 1000, "score": 0, "mapping_schema": 14, "runtime": "gp-interp 0.4.0"}
```

## 3. The cross-game set

`gp dataset catalogue --out DIR templates/smb1-ntsc-prg0 templates/c4-remastered-1 templates/mosshop templates/mm2-usa templates/smb2-usa-prg0` writes `facts` and `mechanics` with the same columns (each already has `game`), and `taxonomy`: one row a mechanic part, with the games that use it, how many model paths carry it, every option set at any use, the parameters its record declares, and how many facts belong to it and how many are stated. Mega Man 2 and SMB2 have models but no template manifest; they count as commercial.

| Mechanic | Games | Uses | Facts (stated) | Options seen |
|---|---|---|---|---|
| `core.camera` | all 5 | 5 | 41 (41) | 9 |
| `core.counters` | all 5 | 30 | 146 (146) | 6 |
| `core.flow` | all 5 | 5 | 108 (20) | 2 |
| `core.tile_collision` | all 5 | 25 | 182 (158) | 0 |
| `platformer.contact` | all 5 | 84 | 710 (589) | 11 |
| `platformer.pit` | all 5 | 133 | 133 (119) | 0 |
| `platformer.jump` | SMB, C4, Moss Hop, MM2 | 4 | 118 (118) | 18 |
| `platformer.run` | SMB, C4, Moss Hop, MM2 | 4 | 60 (60) | 26 |
| `platformer.walk` | SMB, C4, MM2, SMB2 | 17 | 22 (21) | 3 |
| `core.spawn_by_view` | SMB, C4, MM2 | 126 | 1,180 (1,150) | 24 |
| `platformer.climb` | MM2 | 1 | 7 (7) | 7 |
| `platformer.carry` | SMB2 | 1 | 6 (6) | 6 |
| `mm2.hang_and_swoop` | MM2 (hook) | 1 | 13 (13) | 0 |
| `smb1.platform` | SMB, C4 (hook) | 18 | 80 (80) | 0 |

The full table has 46 rows: 34 components and core systems, and 12 hooks (eight `smb1.*`, two `smb2.*`, two `mm2.*`), each a mechanic one game needs that no general part covers yet. SMB2's player is a hook (`smb2.player`), so it has no `platformer.jump` row.

The same question asked across games is one query. Jump gravity is 0.5625 px/frame^2 in Super Mario Bros. (`PlayerBGCollision`, `smbdis`), 0.25 in Moss Hop (`move_player`) and 0.25 in Mega Man 2 (`calc_entity_velocity`, `mm2dis`); the jump speed is -5 in Super Mario Bros., C4 and Moss Hop and -4.87109375 in Mega Man 2.

What it is good for:

| Use | How |
|---|---|
| A catalogue of 2D game mechanics with code evidence | Each mechanic part, its options as each game sets them, and the routine in each game's disassembly that the value comes from. Data Crystal's RAM maps and Gym Retro's `data.json` give addresses; these tables add the mechanic, its parameters and their sources |
| Design | "Which games use a jump whose gravity depends on the hold, and what are the numbers?" is a filter on `facts` where `mechanic = 'platformer.jump'`. A designer reads real values, sourced, instead of guessing |
| The schema's own work | `taxonomy` shows which parts are used once, which hooks are debt (the catalog's `debt`, `crates/gp-interp/src/mechanics.rs`: every hook until a general part covers it), and which options no game sets. It measures how general the platformer extension is |
| ML | Facts as labeled tabular data (predict `kind`, or a parameter from the others), the mapping as a RAM map for environment builders, mechanics as a typed program per game. With a run, `measures` are time series of named state for world models, made locally |
| Rebuild projects | The mapping and the clips' curve hashes are what a decompilation or a remaster needs to test against (datasets.md, section 1) |

## 4. What may be shared

The rules are AGENTS.md's "Never commit" list and datasets.md section 3. Table by table:

| Table | Holds | From the game? | Verdict |
|---|---|---|---|
| `facts` | Model values (our numbers, as in the committed model), paths, classes, evidence kinds, file and line refs, label, routine and constant names | Names only, as a RAM map or a disassembly's label list cites them. No quote: SMB1's quotes include whole lines of code (`lda #$fe`) and table rows, so none is written | Shareable within the line, like Data Crystal's maps (GFDL 1.2) |
| `mechanics` | Our part names and options; tables summarized by size | No | Shareable |
| `mapping` | RAM addresses, transforms, names from the disassembly | Addresses and names, as Gym Retro's `data.json` | Shareable within the line |
| `clips` | Recipes, hashes, TAS movie names and hashes | No; TAS movies are credited by name and hash, not copied | Shareable, with CC BY credit for movies |
| `scores` | Hashes and results | No | Shareable |
| `measures` | Per-frame values read from the original's RAM | Memory contents under M1.1 | Local for a commercial game until decided; written only with `--measures` |
| `clips/*.presses` | The player's own input | No | Shareable, as the template already commits them |

What the export does to keep to that line:

| Check | Where |
|---|---|
| No evidence quote, no evidence note, no model description is written | `fill_facts`; the Moss Hop test asserts that the quote `JUMP_SPEED   = $FB00` isn't in the facts file |
| Options summarize tables and long lists | `options_at` |
| Every written file is searched for any 16-byte window of the ROM's PRG and CHR (windows too regular to say where they came from skipped, as `rom_text.py` does), and the export fails on a match | `rom_runs`, run twice: after the tables, then after the card and metadata. The card and `gp-dataset.json` record the verdict |
| The pre-commit hook's text check, for ROM bytes written as numbers | `crates/gp-cli/tests/dataset_hf.rs` runs `scripts/romguard/rom_text.py files` on every file of the SMB1 export |

On SMB1 and C4 the export found no run, and `rom_text.py files` over every file of all four drafts exited 0 with the registered ROMs in `$GP_ROM_DIR`. As a check of the check, a file of 20 bytes of the PRG0 dump written as hex numbers, in the scratch directory, was refused by `rom_text.py` (it named `c4-remastered-1` at offset `0x1000`, the region the two share).

C4 Remastered is a hack. Its card names the patch by hash and URL, damirium as its author, and `permission = "none"` from `sources/nes.toml`, and says the set stays on this machine until it is granted.

## 5. What a person does with it

With the dataset and their own ROM:

```sh
gp roms add /path/to/your.nes                     # found by its hash; never copied anywhere else
gp template extract templates/smb1-ntsc-prg0      # assets lifted from your ROM, checked by hash
gp gen bevy $GP_HOME/work/templates/smb1-ntsc-prg0 --out /tmp/smb1-print
gp corpus verify templates/smb1-ntsc-prg0 --run R # every clip made again; each curve must match `clips.curve`
gp dataset export --hf DIR --template templates/smb1-ntsc-prg0 --run R --measures   # the measures, locally
```

`gp-dataset.json` says which ROM (headerless SHA-256, and the patch for a hack), which model hash, mapping schema, corpus version and gp version the set is for. A clip whose curve matches is the same clip; the `scores` rows say how a given model did on it.

Without a ROM, the tables answer questions directly:

```python
import duckdb
duckdb.sql("""
  SELECT game, path, value, unit, routine
  FROM 'catalogue/data/facts.parquet'
  WHERE mechanic = 'platformer.jump' AND path LIKE '%.gravity'
""").show()
duckdb.sql("SELECT mechanic, game_count, options_seen FROM 'catalogue/data/taxonomy.parquet' ORDER BY game_count DESC").show()
duckdb.sql("SELECT role, count(*) FROM 'smb1-ntsc-prg0/data/mapping.parquet' GROUP BY role").show()
```

On the Hub the same queries would read `hf://datasets/<repo>/data/facts.parquet` (DuckDB's httpfs, datasets.md section 2), and `load_dataset("<repo>", "facts")` would pick the config by name.

## 6. Versioning

| Thing | Version | Where it shows |
|---|---|---|
| The layout | `HF_FORMAT` 0.1.0: configs, columns, files. A column added is minor; one removed or changed is major | `gp-dataset.json`, every Parquet file's `gp.dataset.hf_format`, Croissant's `version` |
| The model | Its canonical hash, and the core and extension versions | `facts.model_hash`, `core`, `extensions`; `gp-dataset.json` |
| The mapping | Its `schema` | `mapping.schema`; `scores.mapping_schema` for the one each score was measured with |
| The corpus | Name, semver version and content hash (`corpus.toml`) | `gp-dataset.json`; `scores.corpus_*`; `clips.recipe` per clip |
| A score | Model hash by corpus hash, as `evidence/<model>/<corpus>.json` | One `scores` row a clip of each record |
| gp | Version and build | `gp-dataset.json`, `scores.gp_version` and `gp_build` |

A dataset version is a model hash plus a corpus version. A Hub repo would tag each release with both (`model-f7554134925e+corpus-2.0.0`), and the card's "What it needs" table is the pin. Old scores stay in `scores` keyed by their own model hash, so the history of a clip across models is a query, not a separate table. The `scores` table of datasets.md section 7 ("not built yet") is this one.

## 7. The code

| Piece | Where |
|---|---|
| `gp dataset export --hf DIR --template T [--run R] [--measures]` | `crates/gp-cli/src/dataset_hf.rs`, `export` |
| `gp dataset catalogue --out DIR TEMPLATE...` | the same file, `catalogue` |
| Parquet, Croissant, the column types | `crates/gp-cli/src/dataset_export.rs`, now shared: `Table`, `write_parquet`, `croissant_for`, an optional integer column, and `scored` split out of the run export so `--run` reuses it |
| Tests | `crates/gp-cli/tests/dataset_hf.rs` (Moss Hop export, card, Croissant, no quote, same bytes twice; SMB1's export and `rom_text.py` find no ROM run when the dump is in `$GP_ROM_DIR`) and two unit tests (labels and constants, the ROM scan on a made-up ROM) |

No dependency was added. `--run` was not tried on the drafts: `crates/gp-cli/tests/dataset.rs`, the run export's own test, fails on `main` as well as here (its Moss Hop run now gets 0 clips from `gp capture generate`), so a Moss Hop `measures` config would have been empty.

## Open questions

- Publish at all? Default: no. Every set stays local until the user decides, with legal advice for the commercial games. Moss Hop's is CC0 and could go first, as the example repo.
- One repo a game, or one repo with a config a game? Default: one a game, plus the catalogue as its own repo, so a game's license and permission (C4's is `none`) don't hold back the others.
- Are evidence `note`s shareable? They are people's words and sometimes name code. Default: left out, as now; a reviewed note could be added later.
- Do label and routine names from a disassembly belong in a public table? Data Crystal and Gym Retro publish RAM names; label names are the disassembler's own words. Default: yes within the line, decided with the measures question.
- Should `routine` come from the code instead of the text? It is the nearest label that `jsr`, `jmp` or a pointer table names, which is right for SMBDIS.ASM's routines and wrong for code that falls through from one routine into the next. Default: keep the heuristic, and use `gp_nes::coverage` (dense-clips.md) to name the routine that runs the line when a recording is there.
- Hooks have no options in `mechanics` (their parameters are under `ext`). Default: read a hook's declared parameters from its record, as the catalog does for components.
- A `taxonomy` key across versions: a part's options change with the extension's version. Default: one row a part for now, with the versions in `mechanics.version`.
