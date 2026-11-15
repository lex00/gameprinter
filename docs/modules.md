# A game's module

A game with hooks ships one Wasm module: the interpreter, the presentation and the game's hook crate, compiled together. Games with no hooks have none and run in the page's own build.

## Building one

```sh
gp game kit -o KIT
gp game build GAME_DIR --kit KIT -o module.wasm
```

The kit holds the tool's source for those crates and the crates they need. `gp game build` copies the kit and the package into `$GP_HOME/work/module-build/NAME` and runs cargo in a sandbox: it reads that folder, the toolchain and the system's own tools, writes only that folder, and has no network. On macOS that's `sandbox-exec`, on Linux `bwrap`. With neither, it won't build.

`gp game test GAME_DIR --kit KIT` runs the hook crate's own tests the same way: the kit and the package copied into `$GP_HOME/work/module-build/NAME-test`, cargo in the sandbox, offline, with the package's `Cargo.lock`, built for this machine instead of the browser. Nothing is written in the package folder, so no `ext/Cargo.lock` or `ext/target` is left behind as a `cargo test` in `ext/` would. It exits 1 when a test fails, with cargo's last lines. The game gate runs the same thing for a patch that changes hook code. On Tapper's package (games branch tapper-2): "ext: 3 tests pass" in under 5 seconds.

The package ships a `Cargo.lock` at its top, made once with `--update-lock`. A hook crate may depend only on gp-sim and gp-cycles and may have no build script, proc macro, or target-specific dependency.

The same kit, package and lock give the same bytes on any machine. `games`' CI builds the module on this Mac and on a Linux machine and fails when the hashes differ. To get there, a small wrapper in front of rustc names each crate by its name and version and remaps build paths, because cargo otherwise mixes the host and the path into each crate's identity.

`gp package scan DIR --wasm FILE` reads every data segment and custom section of a built module, and any `.wasm` in the folder, against the package's ROMs. Thirty-two matching bytes fail it.

## In the page

A game's `play/module.wasm` is installed and checked like its other files, and `page/module-host.js` runs it in a Worker from `page/module-worker.js`. A game with hook code and no module is refused at install. The page's own `gp_web.wasm` has no game's hooks and refuses a model that names any.

The page's code calls the game synchronously, so it waits on a shared buffer. Shared buffers need the two `Cross-Origin` headers in `public/_headers`, and without them the loader refuses to start a module.

Limits, checked by the `module` section of `check-page.mjs` with small test modules:

| Limit | Value | What happens |
|---|---|---|
| One frame | Ten frame-times (167 ms at 60 fps) | The Worker is ended and the page says so |
| Any other call | 30 s (`gp_run_frames`, 600 s) | The same |
| Memory | The module's declared maximum, at most 256 MB | A module with no maximum, or more, is refused. One that grows to its maximum fails there |
| Imports | None | A module with any is refused |

## Frame timing

Donkey Kong's module in the page, about 600 frames of play in headless Chrome on an Apple M5 Pro:

| | Mean | Worst |
|---|---|---|
| Time in the module | 0.11 to 0.13 ms | 2.2 to 2.8 ms |
| The page's whole call | 0.13 to 0.16 ms | 2.2 to 2.8 ms |

A frame is 16.7 ms. Phones haven't been measured (P4).

## The searches

`gp interp reach`, `gp level measure` and `gp level difficulty` run a game's hooks through its module with the world held on the host: `load-state`, `step` and `canonical-state` each frame, and each spec variant loaded as edited model text. The measure's flat floor is the one spec change model text can't say, so it is `run.flat-level`, a call that puts one session's level on the flat floor after `load`. See [docs/inventory.md](inventory.md).

## Reading a frame's values

A lockstep (`gp clip score`, `gp corpus score`, the mechanic page's review) reads the model's mapped values and its tracked bodies on every compared frame. `gp` calls a module through one export, `gp_rt`, with an op number and bytes (`gp_runtime::wire`). Before game-printer:game 0.2.0 each mapped path was a `query` of its own: Donkey Kong's mapping is about 160 of them a frame, and the body tracker read `entities` whole, every leaf with its path as text (about 56 KB a frame on Tapper). The first `query` of a frame also built the frame's whole view, every leaf of the state as text and a TOML tree, which took about 1 ms a frame on Tapper and 0.35 ms on Donkey Kong.

`run.query-values` in `wit/game/game.wit` (op 29) takes a list of selectors and answers, for each, its values without their paths or its error, in one call. The module reads a leaf, or one field of a kind's bodies (`entities.KIND[*].x`), from the world itself, with the same code that builds the view (`gp_interp::state::Leaves`), and builds the view only for any other path. A lockstep now makes two or three of these calls a frame: the mapping's fields, the bodies' fields, and with `--values` the level.

A module answers `info` (op 2) with the runtime protocol's package and, from 0.2.0 on, `game-printer:game@0.2.0` after it. `gp` reads that when it opens a module, and with a module that names no 0.2.0, one built before, it reads each path with `query` as before, with the same results. `gp --json clip score` and `gp --json corpus score` say which in `model_reads`: `query-values`, `query` (a module built before) or `native` (no module).

The page doesn't read values this way: it plays a module through the preview's exports, so the browser players are unchanged.

## Sound requests

An arcade game's model asks its sound board for sounds as the original's main CPU does: a byte written to one of the board's latches. A hook keeps the requests it made on a frame and gives them back through `Ext::sound_requests` (each a latch, 0 to 3, and a byte). `run.set-sound-events` in `wit/game/game.wit` (op 30, game-printer:game 0.3.0) makes `step` return them as `sound-request` events with `latch` and `value`, which `gp interp compare` holds to the original's `latch-write` events (gp-arcade's `latch_events`) frame by frame. The page reads them through the preview's export `gp_sound_requests` and feeds them to `gp_arcade_play.wasm`'s sound board ([docs/web.md](web.md)). A module built before 0.3.0 refuses op 30 and has no `gp_sound_requests`: it loads and plays as before, silent.

## Not done yet

- The official client's scan of a module on fetch waits for 3.3, when `gp` runs modules.
- The kit is made from a checkout. A signed kit inside each tool release is T3.
