# A game's module

A game with hooks ships one Wasm module: the interpreter, the presentation and the game's hook crate, compiled together. Games with no hooks have none and run in the page's own build.

## Building one

```sh
gp game kit -o KIT
gp game build GAME_DIR --kit KIT -o module.wasm
```

The kit holds the tool's source for those crates and the crates they need. `gp game build` copies the kit and the package into `$GP_HOME/work/module-build/NAME` and runs cargo in a sandbox: it reads that folder, the toolchain and the system's own tools, writes only that folder, and has no network. On macOS that's `sandbox-exec`, on Linux `bwrap`. With neither, it won't build.

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

## Not done yet

- The official client's scan of a module on fetch waits for 3.3, when `gp` runs modules.
- The native limits (epoch deadline and `StoreLimits`) come with the wasmtime host in 3.3.
- The kit is made from a checkout. A signed kit inside each tool release is T3.
