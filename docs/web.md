# The page and its checks

The public page plays a visitor's own Donkey Kong ROM. It is built from the game's package in the games repo beside this one (`../games/donkey-kong`) and holds no game data: the player and the ROM lift are the page's, the game's files and module come from a registry, and the ROM is lifted in the visitor's browser. This file covers the page's checks, the release check, the deploy and the web build's parity with the native interpreter. Modules in the page are in [modules.md](modules.md), registries in [registry-backends.md](registry-backends.md).

## The public page

```sh
just page                    # build into target/page
just web-page                # build into $GP_HOME/work/page
just web-page check          # build, then the headless check and the lift's tests
just web-page accept-pictures  # the picture sections alone, new hashes into pictures.txt
```

`scripts/web/build-page.sh GAME_DIR OUT` writes the page into OUT and the game's play files (the lift's folder and the game's module, published as the package's `play/`) into `OUT.play`. Its last step is `gp template served` on both folders against each ROM the game names. It exits 1 when a file holds the ROM's bytes, lifted files or pictures made from them, and 2 when it can't build or a ROM isn't in `$GP_ROM_DIR` to compare with.

### Checks

| Check | What it does | Needs |
|---|---|---|
| `scripts/web/public/check-page.mjs` | The page in Chrome Headless Shell under its own `_headers`, in sections, each in its own browser context and server: with no ROM nothing plays and the page says so; a wrong ROM is refused naming the right one; each ROM the page knows plays through every stage in the real game's order, in the theme and in the original's look, with its saves, deaths and unload; the visitor's input on Chrome and Safari rules; the game's module in a Worker and its limits; the install from the registry, and each way a changed source or a changed local copy is refused. Every request each frame makes must be a GET with no body to the page's own server, its source or a `blob:` URL, and no WebSocket may open | A built page; the page's source (the registry it names, on the network, or `--registry-folder`) |
| `check-page.mjs`'s picture sections | The game canvas at fixed moments against the hashes in `scripts/web/public/pictures.txt`; `--accept-pictures` records new ones | A ROM |
| `scripts/web/public/lift.test.mjs` | The lift under Node (`node --test`): a ROM of no Donkey Kong refused naming the ones the page knows, a file that isn't a ROM refused, the JU ROM lifted onto Original Edition's model without 50m and the stage-clear scene, Original Edition with every asset found, a JU dump with its sprite tiles gone refused, a made-up 1 MB ROM searched and refused in under a second, and no theme in the templates | `GP_WEB_LIFT_OUT`, the lift's module and folder together; the ROM tests skip without the ROMs |

Which sections of check-page.mjs need a ROM: `no-rom`, `wrong-rom`, `registry-link`, `registry-links`, `registry-goes-bad`, `registry-local-change` and the `registry-bad-*` sections don't. The `rom-*`, `pictures-rom-*`, `visitor-*`, `later-visit`, `kept-look`, `module`, `registry-rom-*` and `registry-theme-*` sections play a ROM and are left out, with a `skip` line, when it isn't in `$GP_ROM_DIR`. None of them runs in `just check`, which holds only what this repo builds on Moss Hop: every section needs the page, which is built from the Donkey Kong package in the games repo, and installs the game from the page's registry. They run in `just web-page check` and in the release check.

check-page.mjs takes playwright-core from `scripts/web/node_modules` (`npm ci --prefix scripts/web`, which the recipes run when it's missing), as check-mechanic.mjs does.

## Deploying the public page

The host is Cloudflare Workers static assets: a Worker with no code that serves the page's folder, named in `scripts/web/deploy/wrangler.toml`. Nothing deploys on a push; the deploy runs from this machine, after the release check.

```sh
just web-page-release        # build the page at this tip and run every check, no upload
just web-page-release --full # the same with nothing reused
just web-page-deploy list    # the full release check, then what would upload; stops there
just web-page-deploy         # the full release check, what would upload, a prompt, then wrangler
```

Wrangler is `4.157.0` from npm and needs `npx wrangler@4.157.0 login` once, which opens a browser and keeps the token outside the repo.

`scripts/web/release-check.sh` runs every step, prints each step's time, and ends with PASS or FAIL. It builds into `$GP_HOME/work/page-release/<commit>-games-<games commit>`, each with `-dirty-<digest>` when that repo has uncommitted changes, so a new folder for each state of the two repos.

| Step | Fails when |
|---|---|
| `build-page.sh`, ending in `gp template served` | A file holds the ROM's bytes, lifted files, pictures made from them or a game image; or the ROMs aren't in `$GP_ROM_DIR` to compare with |
| `check-page.mjs` under the page's `_headers` | Any check of the page fails, a policy refusal in any frame among them |
| `lift.test.mjs` on the page's lift files | Any of the lift's tests fails |
| `gp package scan` on the game's package | It has a needs-rewrite item, or it wasn't compared with the ROMs |
| The size and files | Over 20000 files or 25 MiB in one file (the Workers static assets limits). It prints every file with its SHA-256 and writes them to `<page>.manifest.txt` |

It builds the page with `GP_PAGE_CONTENT=lex00/turnips`, the deployed page's theme, unless `GP_PAGE_CONTENT` is set (empty for none); check-page.mjs's theme checks fail on a page with no theme. On a pass it records the page's digest (`scripts/web/page-digest.py`: one SHA-256 over every file's SHA-256 and path) in `$GP_HOME/work/page-release/passed/DIGEST`. It also fails if the page's digest after the checks isn't the one it had after the build.

`scripts/web/deploy.sh [PAGE_DIR] [--list]` uploads a folder only when its digest has a pass recorded, so what goes up is what passed, file for file: a file changed, added or removed after the check, or a page built from other sources, has another digest and is refused. With no PAGE_DIR it runs the full release check first and takes the page it built, which is what `just web-page-deploy` does. It shows the worker, the folder, the file count and size, the pass it goes up under and the command, asks for the worker's name to be typed, takes the digest again, and runs `npx wrangler@4.157.0 deploy --assets PAGE_DIR`. `--list` runs wrangler's `--dry-run`, which uploads nothing. `GP_WRANGLER` replaces the wrangler command, so the script can be tried up to the upload against a stand-in. Cloudflare keeps earlier versions: `npx wrangler rollback` from `scripts/web/deploy` returns to the last one.

On 2026-11-11, at this branch's tip with uncommitted changes and games at 07d52be, with both ROMs in `$GP_ROM_DIR`, the release check passed in 448 s: the build in 32 s with the modules kept (471 s with nothing kept), 32 page sections in 400 s (one at a time: the machine's load was near 80), the lift's 7 tests in 8 s, the package scan with 0 needs-rewrite items, and a page of 22 files in 7.19 MiB. The run before it, built with no theme, failed 17 checks in `rom-*-play` and `kept-look`. The 1 MB search test took 741 to 1184 ms under that load against its limit of 1000 ms, so it can fail on a loaded machine.

### Reuse

The release check reuses what didn't change: the optimized modules and gp-web-prep's output (build-preview.sh and build-lift.sh with `GP_WEB_REUSE=1`), and a package scan with nothing to rewrite. Each is kept in `$GP_HOME/work/page-release/reuse/` (`$GP_PAGE_REUSE`) by `scripts/web/reuse.py` with the files it was made from or read and their SHA-256s, and is used only when every one of them is unchanged. A reused result is printed with "kept". `--full` makes and checks everything again and keeps the results. The page check runs every section each time: the game comes from a registry, which a kept result's inputs can't name. Deleting the store is safe at any time.

## Parity

```sh
just web-parity
```

`scripts/web/parity.sh OUT MODEL ASSETS CLIPS` checks a preview build (`scripts/web/build-preview.sh OUT MODEL ASSETS`) against the interpreter built for this machine. CLIPS is a TOML list of baselines under `scripts/web/parity/`. For each, gp-web-prep plays the baseline's input natively and writes the frames and the state hash at power-on and after every frame; `scripts/web/parity.mjs` then plays the same frames in the build's `gp_web.wasm` under Node and requires every hash and the spec digest to be the native run's. `scripts/web/parity/mosshop.toml` holds Moss Hop's five baselines, so it needs no ROM but Moss Hop and runs in `just check`. On 2026-11-11 every frame of the five (1,200 frames) agreed. With the module built it takes about 30 s; the first build of gp_web.wasm and gp-web-prep in a fresh target folder took 3 min 40 s.

The native side was `gp interp run` in the old tool. Now `gp interp run` runs a model through its game's module and has no `--baseline`, `--frames`, `--level` or `--json`, and Moss Hop has no module, so the native run is gp-web-prep's. A clip of button spans, a store recording or another level, which the old lists had, is refused until gp can play those natively again. gp-web-prep doesn't write the hooks' label, so the native label is taken as empty, which is right for a model with no hooks in crates/gp-exts, such as Moss Hop.
