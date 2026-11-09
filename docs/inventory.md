# Step 3.1 inventory: what crosses between the tools and a game's hooks

This is step 3.1 of the migration (`migration.md`, line 73): every use by `gp` of the interpreter and the simulation with a model that may have hooks, sorted by what the Wasm game module can carry, with the protocol addition and a speed figure. Scope is Donkey Kong. Line numbers are from `game-printer` at its tip (the old tool, read only), where the tools still are. Its `gp-sim`, its `gp-ext-dk` and the model are byte-identical to `gameprinter`'s and `games`'s (checked with `diff`). `gp-interp` differs in a few files: the differences I read are removals of other games' code (`compile.rs`, `lifted.rs`, `mm2.rs`, `smb.rs`) and a reformatting of `bodies.rs`; I didn't read every line of them.

The module, as the design says, is `gp-sim`, the interpreter core, `gp-present` and Donkey Kong's hook crate (`games/donkey-kong/ext`) compiled to Wasm and run by wasmtime in `gp`. The boundary is the Rust trait `Ext` in `crates/gp-sim/src/ext.rs`, reached today through the `Exts` map of `gp_exts::registry` passed to `Interp::with_exts` or to `gp_sim::step_with_ext`. Donkey Kong has 17 hooks; `sound::SoundDriver` is the only one that overrides `apu`/`apu_peek` (`ext/src/sound.rs:854`), and no hook implements `describe`.

## The boundary, and what does not cross it

`Ext::step`, `appear` and `frame_start` run inside a simulation step. Everything a hook asks for (`ExtCx::draw`, `draw_soft`, `param`, `table`, the requests) stays on the module side, so none of it is a boundary crossing. What a tool sees is one of:

| How a tool reaches the model | Example | What it means for the module |
|---|---|---|
| `Interp` through `gp_runtime::Runtime` | `step`, `canonical_state`, `load_state`, `query`, `state_hash` | Existing protocol calls |
| `Interp::world()` and `.spec()` directly | `show.rs:167`, `point.rs:496` | `World` is the `canonical-state` bytes, decoded on the host by `World::from_canonical`; `Spec` is compiled on the host from the same model and assets, with no hook code in it |
| `Interp` construction options | `with_exts`, `with_draws`, `with_apu_events` | Addition (`set-draws`, `set-apu-events`); hooks themselves are in the module |
| `Ext` methods besides `step` | `describe`, `apu`, `apu_peek` | `describe` is an addition; the APU calls already reach the host as `apu-write` events when the runtime is asked for them |
| `gp_sim::step_with_ext` on a host-held world | `reach.rs`, `stage.rs`, `difficulty.rs` | `load-state`, `step`, `canonical-state` in a loop; see the searches below |
| `gp_sim::reads::recording` | `scenario.rs:117` | Addition: the reads are thread-local to the module |

`Spec` stays on the host because compiling it runs no hook. Every site that reads `.spec()` (about 25 files) needs only the compile in `gp-interp`, which `gp` keeps. The risk is a module built against an older interpreter that compiles the same bytes differently, so the host checks the digest of the module's compile against its own (`run.spec-digest`).

`gp_sim::World::hash` is BLAKE3 over `canonical()` (`world.rs:890`), so the 32-byte `FrameHash` that recordings store is computable on the host from `canonical-state`. The protocol's `state-hash` is the first 8 bytes of it and is what the lockstep reads every frame.

## Sorted uses

### Carried by the runtime protocol as it stands

| Use | Where (old tool) | Calls it makes |
|---|---|---|
| Model recording and replay | `gp-interp/src/recording.rs:65,178,391`; `baseline.rs:164`; `gp-wasm-check/src/main.rs:110` | `load`, `step`, `canonical-state` |
| `gp interp run` | `gp-cli/src/interp.rs:806` | `step`, `canonical-state` (`--apu-log` is in the next table) |
| Lockstep and compare, generic over `Runtime` | `gp-interp/src/lockstep.rs`, `compare.rs`, `scenario.rs:1408` | `step`, `frame`, `state-hash`, `canonical-state`, `load-state` (resync), `query` once per mapped field per frame (160 fields for Donkey Kong OE), `set` on `draws.base` and `draws.lag` |
| `gp clip score`, `gp corpus score`, the gate | `gp-cli/src/clip_score.rs`, `lockstep.rs:143`, `scenario.rs:217,596`, `gate.rs` (runs `gp corpus score` as a subprocess) | The lockstep's calls |
| Starts and checkpoints | `gp-cli/src/scenario.rs:437`, `gp-interp/src/starts.rs`, `carry.rs` | `load-state`, `canonical-state`, `fork` (`moments.rs:101` clones the interpreter) |
| Other lockstep drivers | `moments.rs`, `refine_lockstep.rs:198`, `progress_measure.rs`, `odds.rs`, `mechanic/live.rs:321`, `mechanic/rebuild.rs:279`, `bench_scratch.rs:215,274`, `serve/lockstep.rs`, `serve/remastered.rs:644`, `validate.rs:405` | The lockstep's calls |
| World readers, decoding `canonical-state` on the host | `show.rs:167,224`, `point.rs:496,547`, `generate.rs:711-775` (already decodes at 772), `validate.rs:915`, `picture.rs:240`, `theme.rs:1464-1522`, `serve/themes.rs:248`, `serve/variants.rs:117`, `serve/corpus.rs:1384`, `mechanic/rebuild.rs:57`, the screen measure `gp-cli/src/screen.rs` (`model_frame`, `Probe`) | `canonical-state` per frame (about 6 KB for Donkey Kong OE, measured below) |
| `gp model trace` | `model_trace.rs:55` | `query` on the view of a state |

The screen measure and every presentation reader run `gp-present` on the host, over a decoded world. No hook runs in `gp-present`, so nothing has to cross; the module's own `gp-present` is for the page. Whether a host's `gp-present` and a module's agree is the open question below.

### Needs an addition (`wit/game/game.wit`)

| What | Where (old tool) | The call |
|---|---|---|
| `describe`: a hook's mechanic records | `gp-interp/src/mechanics.rs:260` (`catalog`, read by scoring at `gp-cli/src/lockstep.rs:202`, `scenario.rs:471,538`), `gp-cli/src/mechanics.rs:25`, `catalog.rs:139,750`, `validate.rs:649`, `gaps.rs:520`, `chunks.rs:111`, `scorecard.rs:502`, `agent_brief.rs:364`, `refine_lockstep.rs:966`, `author.rs:584`, `serve/fidelity.rs:177`, `serve/dataset_hf.rs:397` | `module.describe` |
| The hooks a model has; a hook named but absent | `Interp::ext_hooks` at `interp.rs:764`, `validate.rs:259` | `module.info.hooks` compared with the model on the host |
| The random draws script and how the run took it | `with_draws` at `gp-cli/src/lockstep.rs:148`, `scenario.rs:745,901,934,1017`, `refine_lockstep.rs:213`, `validate.rs:408,415`, `mechanic/rebuild.rs:248`; `Interp::draws` at `gp-cli/src/lockstep.rs:225` | `run.set-draws`, `run.report-draws` |
| A hook's sound-driver writes (`Ext::apu`, `apu_peek`, `hook_apu`) | `gp-interp/src/runtime.rs:205` (lockstep's sound), `interp.rs:822` (`--apu-log`), `generate.rs:717`, `theme_original.rs:916`, `gp-web/src/lib.rs:338` | `run.set-apu-events`: the writes come back as `apu-write` events, which `step` already returns |
| What a run read of its spec and hooks | `scenario.rs:117,906` (`gp_sim::reads::recording`), `gp-interp/src/reads.rs`, `affected.rs`, the gate's `--reuse` | `run.start-reads`, `run.take-reads` |
| Which module ran, and whether it read the model the same | `gp_exts::registry` label at `gp-cli/src/interp.rs:749,1430`, `runtime_ref_with_hooks` at `gp-interp/src/recording.rs:97,103,245,469`, `validate.rs:282`, `generate.rs:333` | `module.info` (`runtime-wit`, `sim-api`) and `run.spec-digest`; the recording's label becomes the module's hash, a format change and not a call |

`describe` returns an empty list for every Donkey Kong hook today, so `gp model mechanics` already reports them as opaque; the call is on the scoring path only because `mechanics::catalog` is.

### Carried by existing calls, decided in 3.3 step 7

The searches run hooks through `gp_sim::step_with_ext` on a world the host holds: `gp-interp/src/reach.rs:224,418,506` (`gp interp reach` at `gp-cli/src/interp_reach.rs:48,216`, `gp level` at `level.rs:129,153`, `author.rs:432,437`, `gp-chunks/src/demand.rs`), `gp-interp/src/difficulty.rs:452,494,537` (`author.rs:286`), `gp-interp/src/stage.rs:497-1163` (`stage_measure.rs:401`, `level.rs`). They need `load-state`, `step` and `canonical-state` for each searched frame: two state copies per frame, and no new call. No addition is written for them. What they do need decided is that three of them edit the spec before stepping: `reach::alone` removes spawns and changes a machine's initial state (`reach.rs:250`), `playthrough` and `difficulty` set `start_level` (`reach.rs:542`, `difficulty.rs:336`), and `stage.rs:830` makes a copy of the spec with every other kind's `contact` removed. `Spec` is not serializable and the module compiles its own from `load`'s bytes, so a variant is an edit of the model text passed to `load` (`crates/gp-interp/src/variant.rs`). Three of the four kinds of edit are model text. The jump measure's flat floor is not: it changes the compiled level's cells and keeps its ground boxes and tile surfaces, which no model text says, so it is the one protocol addition, `run.flat-level`, below.

### Stays native because no hook runs

| Use | Where |
|---|---|
| Model text tools: `gp model check`, `gp model fmt`, the mechanic index over components, schema tools | `gp-model`, `gp-cli/src/model.rs`, `schemas.rs` |
| Spec reads, compiled on the host with no hook | every `.spec()` site; `gp_interp::compile`, `translate`, `mapping` |
| The original: gp-nes, the oracle, the evidence store, blind and farm searches, localize | `gp-nes`, `gp-oracle`, `gp-evidence`, `blind.rs:429`, `farm.rs:596`, `localize.rs:1321`, `model_blind.rs:78`, `sweep.rs:757`, `tape.rs:424` (all on `self.nes` or a `NesRuntime`) |
| Coverage and `gp code`: the design asked; `code.rs` and `coverage.rs` have no use of `gp-interp` or `gp-sim`, so they read only the original | `gp-cli/src/code.rs`, `coverage.rs`, `gp-coverage` |
| `gp-sprites`: reads `gp_interp::mapping` only | `gp-sprites/src/observe.rs` |
| Hook source tools: read the crate's text, hash its code | `gp-cli/src/hook_closure.rs`, `affected.rs:47`, `serve/impact.rs:509`; the build script of `gp-exts` goes with it |

### Not moved

`gp-mcp` and `gp-host` build an `Interp` with no hooks (`gp-mcp/src/tools.rs:964`, `gp-host/src/session.rs:42`), so with Donkey Kong they run components alone today; they move, if at all, after the gate. `gp-web` is replaced by the page loading the module in 3.2. `gp-gen-bevy` links hooks into a generated Bevy build and is not brought in. Other games' tools and code (`gp-interp`'s `mm2.rs` and `smb.rs`, the Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 entries in `gp_exts::ENTRIES`, `serve/remastered.rs`'s use of them) are left behind.

## Steps 7 and 8 of 3.3: the searches through the module

`gp interp reach` (with `--record`, which is playthrough), `gp level measure` and `gp level difficulty` run Donkey Kong through its module. `gp_interp::variant` holds each spec edit and the same edit written on the model. A module session loads the edited text and the host checks the module's spec digest against the digest of the host's edited spec, so a text edit that compiles differently stops with an error before a frame runs.

| Edit | As model text | Where it runs |
|---|---|---|
| `StartAt(level)` | `model.start_level` | `playthrough` and `gp level difficulty` |
| `Alone(level)` | `start_level`, the level's spawns cut to the first of the player's, each game machine's `initial` moved off a state that holds the player | `gp interp reach`'s search |
| `AloneFlat(level)` | `Alone`'s text; the floor comes after `load`, through `run.flat-level` (`wit/game/game.wit`, wire op `FLAT_LEVEL`) | The jump measure in `gp interp reach` |
| `Contactless(player)` | Each other entity's `contact` removed and written whole into each of its machine states' `set`, because the spec edit only clears a kind's own `contact` and a state's kind keeps what it compiled to | The rollers and jump windows of `gp level measure` |

The searches hold a `World` on the host and step it through `load-state`, `step`, `canonical-state`, skipping `load-state` when the world is the state the module just returned (`ModuleStepper` in `crates/gp-cli/src/hosted.rs`). `reach.rs`, `difficulty.rs` and `stage.rs` take a `Sim` that opens a `Variant`; `NativeSim` runs a model with no hooks in the process, and `gp_cli::hosted::Source` opens module sessions.

Parity was checked with the old tool's JSON field by field on `templates/donkey-kong-oe` against `games/donkey-kong` and the same lifted assets: `interp reach` on all four levels and each alone, `--record` on all four, `level measure` on all four, `--clears` on 25m, and `level difficulty` on all four. On the real levels the difficulty search stops at the title's frozen first frames, so it also ran on a copy of each tool's model that starts the game in `playing`, to 40 places on two levels. `--record` found no run on the real model, so its writing of a baseline was checked on a copy with a higher jump where the goal is reached: the two tools wrote byte-identical `baselines/main.toml`. Broken cases, each failing the comparison: the jump speed changed in the model (reach, `--record`, measure, difficulty) and the odds of a barrel taking a ladder changed in a hook crate built into a module (measure).

Speed, old release binary against this tool's `fastdev` build and the Wasm module, wall clock, one run each: `reach` over all four levels 0.37 s against 2.78 s, `measure` on `main` 7.4 s against 24.9 s, on `m100` 2.9 s against 9.7 s, `--clears 1` on `main` 1.9 s against 6.3 s. The deep difficulty search of 40 places, 1.5 s against 3.5 s.

### Step 8: every place in `gameprinter` that builds an `Interp`

`gp-interp/src/tests.rs` is left out: tests run Moss Hop and made-up models.

| Where | Hooks? | Result |
|---|---|---|
| `gp-cli/src/hosted.rs` `Source::new` | Compiles the spec and checks, running nothing | Refuses a named hook that neither the module nor the tool's demo crate has |
| `gp-cli/src/hosted.rs` `Source::open`, the native branch | Reached only with no module, after `Source::new` refused a missing hook | Hookless, or the `hooks_demo` model the tool links |
| `gp-cli/src/run.rs` `compare` (`gp interp compare`) | Refuses named hooks it doesn't link (Donkey Kong: "the model names hooks this gp has no crate for") | Hookless. `gp interp compare` has no `--module` path; Moss Hop baselines run here |
| `gp-web/src/rt.rs` `Sessions::load` | The module's own instance, hooks linked into it | Through the module, by construction |
| `gp-web/src/lib.rs` `Preview::load` | The page's own `gp_web.wasm` has no game's hooks and refuses a model naming any; a game's module has them built in | Hookless, or through the module the page's build names |
| `gp-interp/src/recording.rs` (3 sites) | Called from nothing but tests | Not reached by any command |
| `gp-interp/src/baseline.rs` | `baseline::run` (native, hooks by `exts`) is removed. `run_on` runs the module's `Hosted` through `Runs` | `gp interp reach --record` goes through the module |

The old `gp serve` and route modules aren't in this repo yet (side track T1); they are not in this list.

## The protocol addition

`wit/game/game.wit`, package `game-printer:game@0.1.0`, with the world `game-module`. It includes `game-printer:runtime/full-runtime@0.4.0` and exports two more interfaces, `module` (`info`, `describe`) and `run` (`spec-digest`, `set-draws`, `report-draws`, `set-apu-events`, `start-reads`, `take-reads`). `wit/runtime/runtime.wit` is unchanged at 0.4.0 and `wit/game/deps/runtime` is a symlink to it, so there is one copy.

It is a world beside the runtime protocol and not `0.5.0` of it because nothing else implements these calls. gp-nes has no hooks, the hookless interpreter has no use for `describe`, and bumping `game-printer:runtime` would make `gp-nes` and the Go and Godot hosts change for calls they never carry. `gp-runtime`'s test that keeps the Rust trait and `runtime.wit` in step is untouched.

Parse results, with the `wit-parser` crate at 0.262.0 (the version `gp-runtime`'s test uses), through a small binary in scratch that calls `Resolve::push_path`:

```
$ witcheck wit/game
parsed game-printer:game@0.1.0; packages ["game-printer:runtime@0.4.0", "game-printer:game@0.1.0"]
world game-module: exports ["game-printer:game/module@0.1.0", "game-printer:runtime/types@0.4.0", "game-printer:runtime/session@0.4.0", "game-printer:game/run@0.1.0", "game-printer:runtime/present@0.4.0"]
interface module: 7 types, 2 functions
interface run: 5 types, 6 functions

$ witcheck witbroken/game   # a copy with `-> reads` changed to `-> no-such-type`
REFUSED: ... failed to parse package: ... name `no-such-type` does not exist
exit=1
```

`wasm-tools` is not installed on this machine, so the `wit-parser` crate is the only parser used.

## The order the tools move in for 3.3

The step's done-when is `gp model gate` scoring Donkey Kong through its module with the same results as this repo's gate. Everything on the gate's path goes first, in the order that lets each step be checked against the last.

| Step | Move | What it needs | Size, from the code |
|---|---|---|---|
| 1 | The host: a `gp_runtime::Runtime` over wasmtime, and the module built from the kit | The protocol as it stands | One new type; `Interp` is constructed at 39 sites in 25 files of `gp-cli`, of which two (`interp::load_interp`, `scenario::Bench::interp`) cover the gate's path |
| 2 | Recording replay and `gp interp run` through the module: recordings check the module against native by hash | `canonical-state`, `step` | `gp-interp/src/recording.rs` (3 constructions), `baseline.rs`, `interp.rs`; `gp-wasm-check` moves to loading the module |
| 3 | `gp model mechanics` and the `describe` path | `module.describe`, `module.info` | `catalog` is called from `lockstep.rs:202` and `scenario.rs:471,538`, so it precedes scoring. Donkey Kong's list is empty |
| 4 | `gp interp compare`, then `gp clip score` | `set-draws`, `report-draws`, `set-apu-events`, the lockstep's calls | `lockstep.rs`, `clip_score.rs`, `scenario.rs` (draws at 4 sites) |
| 5 | The screen measure and world readers over decoded canonical state | `canonical-state` per frame, host `gp-present` | `screen.rs`, then `show.rs`, `point.rs`, `validate.rs`, `generate.rs` |
| 6 | `gp corpus score` and `gp model gate`: the parity check | `start-reads`, `take-reads`, `spec-digest` | `scenario.rs:117,906`, `affected.rs`, `gate.rs` |
| 7 | The searches | The spec-variant decision; `load-state` and `step` | `reach.rs`, `stage.rs`, `difficulty.rs` and their callers; `gp level measure` and `gp interp reach` are the sensitive case |
| 8 | Whatever else in `gp serve` and the rest of `gp-cli` builds an `Interp` | The earlier steps | The remaining construction sites |

I have not put days on this. The count of construction sites and the hook calls above are the grounded part; how long the wasmtime host and the fixed ABI take is not something this reading can say.

## Speed

The figure measures the design's module pieces, Donkey Kong's hook crate (`gp-ext-dk`) with the new repo's `gp-interp` and `gp-sim`, compiled to Wasm and run under wasmtime against the same program run natively. It is one Rust program, a scratch crate named `inventory-speed` (not in this repo), built twice from the same sources: `aarch64-apple-darwin` and `wasm32-wasip1`, release profile, the toolchain the repo pins.

What it runs is not a split module. The whole scoring program goes under wasmtime, the NES emulator and the screen measure included, because a split needs the module host and the fixed ABI, which are step 3.2 and 3.3 work. The `model` and `nes` rows measure the two halves apart. In the design the NES stays native, so the whole-program ratio overstates what the module will cost. The cost of the boundary itself is a gap, below.

Clip: `dk.walk.001` of the corpus (352 frames from frame 4581, 4933 frames from power-on). The harness plays it as `gp clip score` does, with `GP_START_CACHE=off` so nothing is reused: the draws recorded from the original, then a straight-through lockstep and a resync lockstep, each with the screen measure and sound. `lockstep` is one resync pass with its draws. `model` runs only the interpreter with Donkey Kong's hooks over the same input, doing one `canonical_state` and `state_hash` per frame. `nes` runs only the emulator. `clear25m` is `model` on `dk.clear-25m.001`'s 3674 frames from power-on, the second hook workload.

Machine: Apple M5 Pro, 18 cores, 64 GB, Mac17,8, macOS 26.7.1. wasmtime 48.0.7, rustc 1.99.1. The machine was not idle: its one-minute load was 2.3 to 18.3 during the runs, from other work I didn't stop. Runs alternated between native and Wasm in each of 6 rounds.

| Measure (seconds of work) | Native median (min to max) | wasmtime median (min to max) | Wasm / native |
|---|---|---|---|
| Corpus score, one clip (`score`) | 11.57 (11.42 to 12.29) | 18.66 (18.34 to 19.20) | 1.61 |
| `lockstep` | 7.54 (7.39 to 7.73) | 11.92 (11.80 to 12.54) | 1.58 |
| `model`, walk clip | 0.177 (0.176 to 0.180) | 0.307 (0.305 to 0.309) | 1.73 |
| `model`, `clear25m` | 0.154 (0.153 to 0.157) | 0.276 (0.274 to 0.278) | 1.79 |
| `nes` | 3.42 (3.37 to 3.48) | 5.27 (5.25 to 5.30) | 1.54 |

6 runs each. Seconds are the harness's own time around the work; wall time for a wasmtime run was about 0.1 s more (0.40 s against 0.31 s of work on the model row), the module loaded from wasmtime's cache. `wasmtime compile` of the 6.9 MB module took 0.24 to 0.26 s (3 runs; 13.9 MB `.cwasm`). That module is the whole harness with the emulator in it; a game module's size is not measured.

The model's work is 36 µs a frame natively and 62 µs under wasmtime on the walk clip, each frame including one `canonical_state` and `state_hash`. Against a 16.7 ms frame that is a small part of a frame. The worst single frame was not measured.

Native and Wasm give the same results: every run's state hashes matched across both builds, for both locksteps (`b1aa3e47f13e09fd`), the model (`a1a2ff0d4568acb2`, `d5d3edaae74a6e6c`) and the emulator (`344da496d47dfd65`), and `free_together` and `resync_together` were 353 of 353 frames each.

The reference binary for the native side is `game-printer/target/release/gp`:

| Reference | Command | Wall median (min to max), 6 runs |
|---|---|---|
| `gp clip score` | `gp clip score donkey-kong-oe dk.walk.001 --out OUT` | 11.87 (11.64 to 11.97) |
| `gp interp compare` | `gp interp compare games/donkey-kong --rom ROM --input walk.log --from 4581 --mode resync` | 14.96 (14.79 to 15.01) |

The first is a fair comparison with the `score` row's native 11.57: same work plus the old binary's report and file writes (11.60 wall). The second is not a comparison with `lockstep`: `gp interp compare` first runs a whole-run frame comparison (`compare::compare`, 4931 frames, which is why it exits 1: Donkey Kong's title differs by design), which the harness does not. Compare the `lockstep` row only with the harness's native figure.

Commands, with `S` the scratch directory `/private/tmp/claude-501/-Users-alex-Documents-checkouts-lex00/a3f3a388-49a2-451b-8c32-81443efd41f0/scratchpad/inventory`:

```
cargo build --release                               # in $S/speed, native
cargo build --release --target wasm32-wasip1
$S/target/release/inventory-speed score MODEL ASSETS ROM $S/out/walk.log 4581
wasmtime run -C cranelift-enable_nan_canonicalization=true -W relaxed-simd=n \
  -W timeout=1800s -W max-memory-size=268435456 \
  --dir $ROMDIR::/roms --dir MODEL::/model --dir ASSETS::/assets --dir $S/out::/out \
  $S/target/wasm32-wasip1/release/inventory-speed.wasm score /model /assets /roms/ROMFILE /out/walk.log 4581
```

`MODEL` is `games/donkey-kong`. `ASSETS` is lifted once by the reference binary into a scratch `GP_HOME` (`gp assets lift` with `GP_ROM_DIR=/Users/alex/Documents/roms`), and the clip's log comes from `gp clip log donkey-kong-oe dk.walk.001 --out`. The script `$S/run_speed.py` runs the rounds; the numbers are in `$S/results.jsonl`.

The wasmtime flags follow the design: NaN canonicalization on, relaxed SIMD off. `-W timeout` turns on epoch interruption as a per-frame deadline would, and `max-memory-size` caps linear memory at the design's 256 MB. The whole program stayed inside it; the design's limits of 256 MB and ten frame-times were not otherwise exercised.

### What was not measured

- The boundary. The whole program runs inside one Wasm instance, so no call crosses from host to guest. A split module pays `query` per mapped field per frame (160 for Donkey Kong OE, or a `sample` batch) and a `canonical-state` per frame (5.7 KB to 6.2 KB here) over linear memory. Nothing here measures that, and I have not put a number on it.
- `gp corpus score` itself. It needs a capture run under `GP_HOME` (`--run`), and it goes through the tape cache and reads recording. `gp clip score`, which runs the same clip's lockstep and is what the harness mirrors, was measured instead.
- A whole-corpus score, the gate, and the reach and playthrough searches. Only one clip and the two model-only runs were timed. Those remain the sensitive cases the design names.
- Worst-case frame time and memory high-water mark: the runs show a mean per frame and that a 256 MB cap held, nothing more.
- Linux. The design's sandbox runs both, and only macOS was measured.

## Open questions for the user

1. The screen measure draws on the host with the host's `gp-present`, over a decoded world. A module carries its own `gp-present` for the page. A module built against an older tool release can therefore draw differently from the host that scores it. Should scoring draw through the module's `present` calls instead, at the cost of a frame buffer per frame across the boundary, or should the host's `gp-present` stay authoritative for scoring?
2. Answered in 3.3 steps 7 and 8: the edits are model text loaded per variant, as above. One isn't expressible and got a call (`run.flat-level`, the jump measure's floor).
3. A recording's runtime label becomes the module's hash. Old model recordings' labels name the hook crate's hash and will replay as stale. Is that acceptable at cutover, as with any behaviour change?
