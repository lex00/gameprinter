# Architecture

This document describes how gameprinter, Game Printer's tool repo, is put together: its crates and what each depends on, the path from a ROM to a scored model, and where state lives on a machine. It came from game-printer's ARCHITECTURE.md, which described a target design; this one describes the repo as it is, on 2026-11-12. The Bevy generator, `gp loop` and other games' code were dropped in the move, and the games themselves now live in a separate checkout.

Scope is NES games and one arcade board, Midway's MCR (Tapper). A game is a model plus hook code, scored against the original by replaying clips on both.

## Overview

```
  your ROM ($GP_ROM_DIR)                        people and agents
       |                                               |
  gp-nes / gp-arcade  ---> recordings, traces    gp serve (the page)
  (the original)            events, clips        gp mcp, gp host
       |                         |                     |
       |                  readers and detectors        |
       |                  (events, track, gaps,        |
       |                   coverage, explore)          |
       |                         |                     |
       |                  GAME MODEL (package) <-------+
       |                  model.toml, mapping.toml,
       |                  fidelity.toml, ext/ hooks
       |                         |
       |                  gp-interp over gp-sim,
       |                  or the game's module under gp-module
       |                         |
       +------- runtime protocol (wit/runtime, gp-runtime) ------+
                                 |
                       lockstep: corpus clips on both,
                       compared through the state mapping
                                 |
                          score record, gate
```

Every tool reaches a running game through one runtime protocol, so the original, the interpreter and a game's Wasm module can be swapped and run side by side. The model is the source of truth: the rebuild is the model run by the interpreter, with a game's hook crate for what model text can't say.

## Fixed interfaces

| Interface | Where | Version |
|---|---|---|
| The runtime protocol: sessions, stepping, state, queries, `load-state` | `wit/runtime/runtime.wit`, mirrored by hand in `crates/gp-runtime` with a test that keeps the two in step | 0.4.0 |
| A game module's calls beyond the protocol: `info`, `describe`, the draws, APU events and reads a hook needs | `wit/game/game.wit`; why each exists is in [docs/inventory.md](docs/inventory.md) | 0.1.0 |
| The simulation API a hook crate sees (`Ext`, `ExtCx`, `World`, `Spec`) | `crates/gp-sim`; a package declares it as `sim-api` | 1 |
| The package format: blobs by BLAKE3 hash and a manifest | `crates/gp-package`, header `gp-package 1` | 1 |

A module declares the `runtime-wit` and `sim-api` it was built for, and the host refuses one it doesn't implement.

## Stack

| Concern | Choice |
|---|---|
| Language | Rust 1.99.1, pinned in `rust-toolchain.toml` with targets for both Mac architectures, `wasm32-wasip1`, `wasm32-wasip2` and `wasm32-unknown-unknown`. One Cargo workspace, `members = ["crates/*"]`, edition 2024, `unsafe_code` forbidden |
| NES emulator | A fork of `tetanes-core`, vendored in `third_party/` with its patch and the upstream hash, as a path dependency outside the workspace |
| Arcade emulator | Our own, in `gp-arcade`: MAME's Z80 and MCR board code turned into Rust, held to MAME 0.276 frame by frame. MAME itself runs only as an external oracle in a Docker image (`docker/mame`, [docs/oracle-mame.md](docs/oracle-mame.md)) |
| Game modules | wasmtime 48.0.7 in `gp-module` |
| Browser builds | `wasm32-unknown-unknown` cdylibs with a small hand-written ABI, no bindings generator |
| The page | Plain HTML, CSS and JavaScript in `mechanic/` and `scripts/web/`; the editor in `editor/` is TypeScript, React and Vite |
| Agent tools | An MCP server in `gp-mcp` |
| Checks | `just check` on Moss Hop, which needs no ROM; the gates in [docs/gate.md](docs/gate.md) |
| Licence | Apache-2.0 for the workspace |

## The crates

There are 38, all under `crates/`. Arrows below mean "depends on" and list only gameprinter's own crates.

### Shared base

| Crate | What it is | Depends on |
|---|---|---|
| `gp-evidence` | Where evidence lives (`Paths`: `$GP_HOME`, `$GP_STORE`, `$GP_ROM_DIR`, all refused inside a git work tree), the content-addressed store, input logs, recording manifests, frame traces and event streams, the sealed-run guard | none |
| `gp-runtime` | The runtime protocol as a Rust trait | none |
| `gp-package` | Packages and the local store's keys (`blobs/`, `packages/`, signatures, theme indexes); depends on blake3 alone so the browser reads packages with the code that writes them | none |
| `gp-capture` | The capture event stream: event and input types, a run folder's reading and appending, the writer lock | none |
| `gp-imagefmt` | Image and sprite sheet formats with a JSON sidecar, palette files for editors | none |

### NES side

| Crate | What it is | Depends on |
|---|---|---|
| `gp-nes` | The NES runtime on `tetanes-core`: power-on, one frame per input frame ending at vertical blank, recording and replay, traces, the protocol | gp-evidence, gp-runtime |
| `gp-play` | The original played live in the page: `gp-nes` a frame at a time, its input log and frame hashes, RGBA out, built for the browser | gp-evidence, gp-nes |
| `gp-events` | What a running ROM does, kept as levels: footprints, edges, frame events, code activity ([docs/research/rom-event-model.md](docs/research/rom-event-model.md)) | gp-evidence, gp-nes |
| `gp-explore` | Searching for presses that run new code, on one search loop; also the arcade board's chord palette | gp-events, gp-evidence, gp-nes, gp-runtime |
| `gp-coverage` | What recordings exercised: ROM bytes, RAM written, screens and objects, button combinations | gp-events, gp-evidence, gp-nes, gp-runtime |
| `gp-cycles` | A cost-only 6502 for the cycles of one routine, from a slice of the person's ROM under `$GP_HOME/work` ([docs/cycles.md](docs/cycles.md)) | none |
| `gp-track` | Objects found from traces without a mapping: sprite tracks bound to RAM bytes, proposed slots | none |
| `gp-gaps` | What a recording shows that the model doesn't name ([docs/gaps.md](docs/gaps.md)) | gp-track |
| `gp-assets` | Asset lifting: tiles, palettes, metatiles, levels and tables from a ROM by an asset map, packed back with a round-trip check | gp-evidence |
| `gp-sound` | Sound as model data: the APU register log, an APU model, capture into songs and instruments, a player and a synthesizer ([docs/sound.md](docs/sound.md)) | none |
| `gp-sound-bank` | Songs rendered through SF2 banks, the rest on the APU | gp-sound |

The readers on this side read the NES's 6502 code, sprite table, nametables or APU. A command built on them refuses an arcade romset in words rather than guessing (the list is under "The arcade side").

### Arcade side

| Crate | What it is | Depends on |
|---|---|---|
| `gp-arcade` | Arcade boards held to MAME frame by frame, first Midway MCR with the 91490 CPU board (MAME's `rbtapper`): the Z80s, the board, an access log with the PC of each read and write, a Z80 disassembler | gp-runtime |
| `gp-arcade-play` | Tapper played live in the page: the same plain functions as gp-play, plus the screen, frame rate, buttons and every frame's picture hash | gp-arcade, gp-evidence |

A package says which side it is on with `platform` in each `[[rom.entry]]`: `"nes"`, or `"arcade"` for a MAME romset (a zip). Anything that starts the original goes through that platform. Tapper is the one arcade game.

On an arcade romset, `gp reach`, `gp code listing` and `gp code tables`, `gp coverage`, `gp element inputs`, `gp interp draws --find`, `gp gaps`, `gp sound capture` (an AY log and its notes), `gp movie import` (MAME `.inp`) and `gp labels propose` run on gp-arcade instead. These refuse one, saying why: `gp model blind`, `gp model static`, `gp cycles`, `gp explore`'s search, `gp watch start`'s stage 0, `gp dataset rom`, `gp farm` and `gp farm-topdown`, `gp mechanic harvest`, `leads` and `sweep`, `gp sprites capture`, `gp roms diff`, `gp roms clean` and `gp validate`. The page's ON THE MODEL and PLAY need a model, which Tapper doesn't have yet.

### The model and the interpreter

| Crate | What it is | Depends on |
|---|---|---|
| `gp-model` | The game model as TOML: the core schema, the platformer and top-down extensions, fixed-point numbers, model paths, validation, classes of path | none |
| `gp-contract` | `fidelity.toml`: which level each part of a model is held to, and rule resolution | gp-model, gp-runtime |
| `gp-sim` | The deterministic simulation every model runtime shares: `Spec`, `World` with its canonical bytes, systems, fixed-point math, the seeded RNG, the `Ext` hook trait | none |
| `gp-present` | What a runtime draws each frame, compiled once from the model's presentation; theme art rules (`gp_present::art`) | gp-package, gp-sim |
| `gp-interp` | The model interpreter: a model and its lifted assets compiled to a `gp_sim::Spec` and run as a `gp_runtime::Runtime`; state mappings, translation, the lockstep and comparison with the original | gp-assets, gp-evidence, gp-model, gp-present, gp-runtime, gp-sim |
| `gp-theme` | Themes by role: formats, stacking, readability and Style checks, a compositor, a placeholder painter | gp-sound, gp-model |
| `gp-look` | What every player draws and plays from a model and its assets: pictures, looks, the sound bundle. A look drawn at build time and one drawn in the browser are the same bytes | gp-assets, gp-interp, gp-model, gp-present, gp-sim, gp-sound, gp-sound-bank, gp-theme |
| `gp-sprites` | Full sprite capture: every frame a game's sprite tables define and its recordings show, by entity and model state | gp-assets, gp-interp, gp-model, gp-runtime |
| `gp-validate` | The contract compiled into checks and run on recordings of the original through the state mapping ([docs/validation.md](docs/validation.md)) | gp-contract, gp-evidence, gp-model, gp-interp, gp-runtime, gp-sim |
| `gp-derive` | Deriving hand-written facts from what their evidence cites: a 6502 source reader, constant propagation, encodings ([docs/derivation.md](docs/derivation.md)) | gp-model |
| `gp-extract` | Facts from a recording's tracks: feel measured, proposed facts, the questions file, facts applied with evidence | gp-model, gp-track |
| `gp-exts` | The hook crates the tool's own builds run. It holds only the demo hook | gp-sim, gp-ext-game |
| `gp-ext-game` | A placeholder that the page's build fills with one game's hook crate, in a copy of the repo. This repo holds no game's hooks | gp-sim, gp-cycles |

### Modules under wasmtime

A game with hooks ships one Wasm module: gp-sim, the interpreter core, gp-present and its hook crate, compiled together ([docs/modules.md](docs/modules.md)). A game with no hooks has no module and runs in the built-in interpreter.

| Crate | What it is | Depends on |
|---|---|---|
| `gp-module` | A game's module as a `gp_runtime::Runtime`: wasmtime with NaN canonicalization on, relaxed SIMD off, no imports, a deadline on every call and a memory cap. Forks of one runtime share one instance | gp-runtime |

`gp game kit` writes the tool's sources a module needs; `gp game build` and `gp game test` build and test a game's hook crate from that kit in a sandbox with no network (`sandbox-exec` on macOS, `bwrap` on Linux), under `$GP_HOME/work/module-build/`. A hook crate may depend only on gp-sim and gp-cycles. The same kit, package and lock give the same bytes on any machine, and `gp package scan --wasm` holds a built module to the package's ROMs.

### Packages and registries

| Crate | What it is | Depends on |
|---|---|---|
| `gp-package` | (above) The format and the local store | none |
| `gp-web-registry` | What the page asks of a registry, answered by the code gp uses: it says which key it needs next, the page fetches it and asks again, so no check is written twice | gp-package |

A registry is static files: a folder, a git repo (`git:PATH`), an S3 bucket, or an `https://` address read for checking ([docs/registry-backends.md](docs/registry-backends.md)). `gp registry init`, `publish`, `reindex`, `check` and `serve` work it. Publishing compares every file with each ROM the package names, and 32 matching bytes stop it. Signing is off unless a source opts in with a root key in `scripts/web/sources.toml`; hashes are always checked. The public registry is `lex00/gp-registry`, unsigned, read at its raw GitHub address.

### The page

| Crate | What it is | Depends on |
|---|---|---|
| `gp-web` | The interpreter as a browser preview, `gp_web.wasm`; it has no game's hooks and refuses a model that names any | gp-exts, gp-interp, gp-model, gp-package, gp-present, gp-runtime, gp-sim, gp-sound |
| `gp-web-lift` | What the preview loads, written from a model and its assets, and a visitor's own ROM lifted into it, natively or on `wasm32-wasip1` over the page's in-memory files | gp-assets, gp-evidence, gp-look, gp-present, gp-interp, gp-model, gp-sound, gp-theme |

There are two pages.

| Page | Built from | Serves |
|---|---|---|
| The public page | `scripts/web/build-page.sh` with a game's package from the games checkout, deployed to Cloudflare Workers by `scripts/web/deploy.sh` after the release check ([docs/web.md](docs/web.md)) | Donkey Kong played from a ROM the visitor loads, its files and module from the registry, the clay theme `lex00/turnips` beside it |
| `gp serve` | `mechanic/` and `editor/`, served at `http://127.0.0.1:7710/` ([docs/mechanic-page.md](docs/mechanic-page.md)) | A game's tabs CLIPS, REFINE, PLAY and THEMES, with the agent's log beside them |

The page holds no game data. A game's module runs in a Worker (`page/module-host.js`), under the limits in [docs/modules.md](docs/modules.md).

### Agents and the local host

| Crate | What it is | Depends on |
|---|---|---|
| `gp-mcp` | The agent tools over MCP on stdio: fork a template, read its surface, edit model and contract by path, preview, validate ([docs/mcp.md](docs/mcp.md)) | gp-assets, gp-capture, gp-contract, gp-events, gp-evidence, gp-look, gp-imagefmt, gp-interp, gp-model, gp-present, gp-runtime, gp-sim, gp-sound, gp-sound-bank, gp-theme |
| `gp-host` | The local host: interpreter sessions per working copy, scenario runs, background builds, playable links on localhost ([docs/host.md](docs/host.md)) | gp-evidence, gp-interp, gp-mcp, gp-model, gp-runtime |

### The command

`gp-cli` builds `gp`, a thin client over the library crates: each command is one call into a library. It links every crate above but the page's own (gp-web, gp-web-lift, gp-web-registry), most of them directly. [CLI.md](CLI.md) lists every command, in a public set and a lab set, and `gp self docs --check CLI.md` fails `just check` when it drifts.

### How the layers stack

```
gp-cli
 |-- gp-mcp, gp-host .................. agents and the local host
 |-- gp-validate, gp-extract, gp-derive, gp-sprites, gp-gaps
 |-- gp-look -- gp-theme, gp-sound-bank
 |-- gp-interp -- gp-present -- gp-sim
 |      |          `-- gp-package
 |      `-- gp-assets, gp-model, gp-runtime, gp-evidence
 |-- gp-module (wasmtime) -- gp-runtime
 |-- gp-explore, gp-coverage -- gp-events -- gp-nes -- tetanes-core
 |-- gp-arcade-play -- gp-arcade -- gp-runtime
 `-- gp-play -- gp-nes

page builds: gp-web, gp-web-lift, gp-web-registry, gp-play, gp-arcade-play
```

The emulator crates don't depend on the model, and the model crates (gp-model, gp-sim, gp-interp and those over them) don't depend on an emulator. They meet in gp-cli, through the runtime protocol and the state mapping.

## From ROM to scored model

1. The person copies a ROM they own into `$GP_ROM_DIR` with `gp roms add`: an NES image to `nes/`, a MAME romset to `arcade/`. `gp roms scan` identifies each file by the hashes in the games' `roms.toml` and `package.toml` `[[rom.entry]]` rows, whatever it is named. A ROM no game names yet is kept as unknown.
2. `gp game new` starts a package folder in the games checkout for an unknown ROM: a model skeleton for the genre, the ROM's hashes, pins that hold, no clips.
3. `gp assets lift` decodes the ROM's assets by the package's asset map into files under `$GP_HOME/work/assets/SOURCE`. `gp assets pack` is the round-trip check. The lifted files never enter a repo.
4. Readers measure the original: `gp record` keeps runs in the store; `gp watch`, `gp reach`, `gp code`, `gp events`, `gp track`, `gp gaps`, `gp coverage`, `gp element inputs` and `gp model static` read what its code does. `gp model blind` drafts a model and a state mapping from the ROM alone, on the NES.
5. Clips: in `gp serve`'s CLIPS tab the person plays the original, records a mechanic's clips and hands them to the agent. Sessions are kept in `$GP_HOME/mechanics/GAME`. `gp corpus import` turns them into the corpus: `corpus.toml` and `inputs/`, presses only.
6. The model is written and fitted: by hand and by agent in `model.toml`, `mapping.toml`, `fidelity.toml` and the `ext/` hook crate, with `gp model refine --lockstep`, `gp rule`, `gp solve` and `gp model questions` proposing numbers with evidence.
7. Scoring: `gp corpus score` or `gp game score` makes each clip again from the ROM on gp-nes or gp-arcade, plays it through the model (gp-interp, or the module under gp-module), and compares the two each frame through the state mapping. The result is clips exact and a fidelity figure, with a score record.
8. Landing: `gp model gate` takes a patch to a game, scores the corpus before and after, and lands it only if no clip got worse ([docs/gate.md](docs/gate.md)).
9. Publishing: `gp registry publish` puts the package, and its module, in a registry. The public page installs it from there and lifts the visitor's ROM in their browser.

Sealed runs, kept under `$GP_HOME/sealed/GAME/`, measure the model on play no worker has seen; every reader goes through one guard that refuses them, and only `gp sealed score` scores one ([docs/sealed.md](docs/sealed.md)).

## Where state lives

ROMs and the store are refused inside any git work tree, so nothing made from a game is committed by accident. The commit hook (`.githooks/`, `scripts/romguard/`) also refuses ROM bytes and every hash in the games' ROM registry.

### `$GP_HOME`

`$GP_HOME` defaults to `~/.gameprinter`, the new tool's home until cutover (the old tool's is `~/.game-printer`).

| Folder | Holds |
|---|---|
| `bin/` | Published builds `gp-<commit>` and the link `gp` to the newest; `bin/locked/cargo`, the locked cargo `just setup` installs |
| `roms/` | `$GP_ROM_DIR` by default: `nes/` and `arcade/` |
| `store/` | `$GP_STORE` by default: the content-addressed store of recordings and other objects (`objects/`, `index/recordings.jsonl`, `lock`, `tmp/`) |
| `work/` | Everything made from a ROM or a build: lifted assets, module builds, the page's build and release check, oracle captures (`work/oracle/`), cycle slices, worker folders (`work/workers/NAME`) and per-worktree cargo targets (`work/target/BRANCH/HASH`) |
| `mechanics/` | Mechanic sessions by game: clips, conversation, watch files |
| `capture/` | Capture runs: `events.ndjson`, `inputs.ndjson`, `writer.lock` |
| `watch/` | `gp watch` sessions and their event streams |
| `sealed/` | Sealed runs, behind the guard |
| `gate/` | The gates' home (`$GP_GATE_HOME`): worktrees, the queue of waiting patches, scores, the tool canary's homes |
| `keys/` | Signing keys, such as the staging key |
| `themes/`, `sound-banks/`, `config/`, `reports/`, `play/`, `author/` | Theme stacks, SF2 banks, image config, gap reports, free-play sessions and the authoring log |

### The games checkout

Games live in their own repo, `lex00/games`, not here. gp finds the checkout through `--games`, then `$GP_GAMES_DIR`, then a `games` folder at or beside the current folder or one above it; normally that is `games/` beside the tool repo. Each game is a package folder: `games/donkey-kong` (its old id was donkey-kong-oe) and `games/tapper`.

| In a package | What it holds |
|---|---|
| `package.toml` | Name, version, tier, the tool versions it needs (`runtime-wit`, `sim-api`, `gp`), its play files and `[[rom.entry]]` rows: hash, platform, board, asset map and lift pins, and a role (`scored`, or loaded only to play) |
| `roms.toml` | The ROMs' hashes, as gp looks them up |
| `model.toml`, `mapping.toml`, `fidelity.toml`, `profiles.toml` | How the game behaves, where the original keeps each value, what has to match |
| `corpus.toml`, `inputs/` | The clips, as presses |
| `assets/` | Asset maps: addresses and formats, never the art |
| `ext/`, `Cargo.lock` | The hook crate and its lock |

Moss Hop, the test game the tool owns, stays in this repo: its source in `games/mosshop`, its model in `templates/mosshop`, its asset map in `assets/mosshop.toml` and its runs in `baselines/mosshop`. `just check` uses only Moss Hop.

## Checks and gates

`just check` builds gp, assembles Moss Hop with cc65, and runs formatting, clippy, the workspace tests, the CLI.md check, the baselines, the asset round trip, the model checks, the interpreter on Moss Hop, the browser players against their native replays (`play-web`, and `play-web-arcade` when an rbtapper romset is on the machine), the editor's tests, the mechanic page, web parity, the template check, the hook's self-test and the ROM guard's tests.

Nothing lands on `main` except through a gate. `gp tool gate` builds the patched tool, runs `just check` and the whole test suite, and plays Donkey Kong's corpus through the old and new gp as a canary; it then publishes the new build to `$GP_HOME/bin`. `gp model gate` does the same for a game, on its corpus. Workers each have a worktree, an output folder and a target folder of their own ([docs/workers.md](docs/workers.md)).
