# Benchmarks

`gp bench` times the whole pipeline on a baseline: recording, replay, seeks and trace reads, and how much the recording stores. Its numbers go to this run's record as metrics ([run manifests](formats/run-manifest.md)).

## gp bench

```sh
gp bench baselines/gp-testrom                          # the built-in test ROM, no ROM file needed
gp bench --rom ~/roms/nes/my-game.nes my-baselines/    # a baseline folder of your own
gp bench my-baselines/tapper                           # an arcade baseline: the romset is found in $GP_ROM_DIR
```

For each baseline, `gp bench` records the scripted run into a temporary store (never `$GP_STORE`). The recording keeps its frame trace and event stream, as `gp record` does by default. The replay that follows starts at power-on and checks every frame against the recording. Then come up to `--seeks` seeks (8 by default) to the last frame before a checkpoint, plus one to the recording's last frame: a restore and 59 frames of replay, the most a seek costs. Each one is checked against the recorded state, and the same frame is read from the trace.

| Metric | Unit | What |
|---|---|---|
| `<prefix>.record.fps` | frames/s | Recording, with trace and events |
| `<prefix>.replay.fps` | frames/s | Replaying from power-on, with every check |
| `<prefix>.seek.max_ms`, `<prefix>.seek.mean_ms` | ms | Opening the recording at a frame: restore and replay |
| `<prefix>.trace.read.max_ms` | ms | Reading one frame's state from the trace, checked against its hash |
| `<prefix>.record.bytes` | bytes | Every object of the recording |
| `<prefix>.store.bytes_per_minute` | bytes | The same per recorded minute, at 60.0988 frames a second on the NES and 30 on an arcade board |

`<prefix>` is `bench.<source>.<baseline>`, where `<source>` is the baseline's `source` and `<baseline>` its file name, as in `bench.gp-testrom.buttons.replay.fps`. It exits 1 if a replay or seek doesn't reach the recorded state, and 2 if every baseline was skipped for want of a ROM.

A baseline is the `gp baseline check` format: `source`, `sha256`, `frames` and `holds`. When its `sha256` is an arcade romset's identity (the romset given with `--rom`, or one in `$GP_ROM_DIR`), its holds name the board's buttons (`coin`, `start1`, `right`, `left`, `down`, `up`, `tap`) and it is recorded, replayed and sought on gp-arcade. Its `frame_hashes` and `lag_frames` are read but not checked here; `gp baseline check` checks them on the NES.

## From game-printer

`gp bench` came from game-printer on 2026-11-12. On `baselines/gp-testrom` and on a Donkey Kong baseline made from corpus clip `dk.walk.001` (its presses from power-on, 4,933 frames), the old binary and this one recorded the same bytes (2,280,094 and 3,037,625) and both replayed and sought cleanly. On a Tapper baseline made from `tapper.score.001` (719 frames), a debug build recorded at about 870 frames a second and replayed at about 1,200, with a slowest seek of 44 ms and 3.9 MB stored, 9.8 MB per recorded minute.

What it left behind:

- `--video` is refused: it wrote each recording as a video with `gp video`, which gameprinter hasn't got yet.
- game-printer's budgets (`budgets.toml`, held by `just check` on the test ROM and Moss Hop) and its criterion micro-benchmarks didn't come with it.

## gp bench clips, scratch and report

These grade the ROM-only tools on a game whose answer key is known. game-printer ran them on Moss Hop, its own homebrew, through its capture-run loop and against a ground truth written from Moss Hop's source. gameprinter runs them on a game package over its corpus, with the package's hand-written model and state mapping as the answer key. The bench files are `bench/donkey-kong.toml` and `bench/tapper.toml` in this repo; their paths are under the games checkout (`$GP_GAMES_DIR`, else `../games` beside the repo, or `--root`). Each bench runs its tools in a home of its own (`$GP_HOME/work/bench/GAME`, or `--work`) with a copy of the ROM, and writes `report.json` there. Only crates/gp-cli/src/bench_grade.rs reads the answer key.

`gp bench clips GAME` runs these stages:

| game-printer's stage | Here |
|---|---|
| The run: `gp capture start`, `grid --mode batch`, `generate --start`, `background` | `gp corpus score` on a copy of the package: every clip made again from the ROM and scored on the hand-written model |
| The code matrix: `gp clips matrix --labels rom`, once per seed | `gp corpus reduce` (the untaken branch ways, the clips any cover needs) and `gp capture dense --seed S` for each of the bench file's seeds |
| The grid: `gp capture grid --code` and `--drive N` | The dense clips of each seed |
| A wrong model | The corpus scored on the model with the bench file's `[wrong_model]` change, `--reuse` of the real score |
| Mutants: `gp capture prune --mutants` | `gp model mutants`' mutants, each scored on the corpus with `gp corpus score --model M --reuse` (a clip the real model played exactly stops at its first split). `--mutants N` scores only the first N |

A mutant is caught when a clip scores worse on it than on the real model: exact before and not now, a higher score, fewer frames together per thousand, or a clip that can't be made. The measures table keeps game-printer's rows. Elements found, unreachable directions told apart and inputs found need a ground truth written from the game's source, which Donkey Kong and Tapper don't have, so those rows say so.

`gp bench scratch GAME` starts from a model holding only its header, in a sealed folder holding the ROM, the asset map and the package's corpus (template.toml, corpus.toml, inputs/) but no model, mapping or hooks, under `sandbox-exec` on macOS:

| Stage | game-printer | Here |
|---|---|---|
| 0. Static harvest | The bench file's `static_command` | The same (Donkey Kong's and Tapper's: `gp model static {rom} --model {model}`); `gp model derive` and source or labels files refused |
| 1. Clip generation | The capture run and `gp clips matrix` | `gp capture dense` on the stage's package |
| 2. Trace harvest | `gp model propose` on the run's recordings | `gp model propose` on recordings of the first `recordings` corpus clips (`gp clip log`, then `gp record`) |
| 3. Refinement | `gp solve --check` and `gp model refine --lockstep` on each baseline | Rounds of `gp solve --check` and `gp model refine --lockstep` on the corpus |

Exact play is the corpus scored on the stage's model by the grader (frames together and clips exact, through the real package so its hooks build), in place of the baselines through `gp interp compare`. The system order and tile kinds are graded by behaviour on the same scores: two models play the same when every clip's model curve is the same. `--self-test` and `--scramble` are game-printer's checks of the grader and the seal.

`gp bench report GAME` prints the last reports' tables and exits 2 when there is neither.
