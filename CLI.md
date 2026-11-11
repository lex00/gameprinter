# The gp commands

Generated from gp's own commands by `gp self docs`; `just check` fails if it differs. Edit the commands, not this file.

## The public set

For making and playing games: what a person installing gp uses.

| Command | What it does |
|---|---|
| `gp assets lift` | Decode a ROM's assets into a directory outside any git work tree |
| `gp assets pack` | Pack a lifted directory and compare it with the ROM, byte for byte |
| `gp corpus score` | Score a game's corpus on a model: every clip made again from the ROM, played through the model and checked against its curve. Prints the clips exact and the fidelity and, with --json, the score record |
| `gp game attest` | Attest a published version of a game: derivation, module build and score, as signed statements under attest/<id>/ in the registry. Takes a clean `gp game score` output of the package |
| `gp game build` | Build a game's module from its package folder and the kit, in a sandbox that reads nothing else and has no network. Exits 1 when the hook crate breaks the rules, 2 when the build fails |
| `gp game fetch` | Fetch NAME@VERSION by id into a folder, checking every signature (with --root) and hash, and lift every ROM entry it finds in the ROM folder |
| `gp game hash` | Print a built module's BLAKE3 hash |
| `gp game kit` | Write the module kit: the tool's sources for a game module and the crates they need, ready for a build with no network |
| `gp game new` | Start a new game from a ROM you own that no game names yet: its package folder in the games checkout, with a model skeleton for the genre, the ROM's hashes in roms.toml, pins that hold, and no clips. Writes over nothing |
| `gp game score` | Score a game from its package and the ROM alone, in this $GP_HOME: every clip and curve derived from the corpus plan and the ROM, played through the module, and the score record written to FILE. Exits 1 when the corpus has problems |
| `gp model check` | Check a model, its profiles, its contract and its asset slots |
| `gp model fmt` | Fail if the model or contract isn't in canonical form |
| `gp model gate` | Hold a patch to a game in the games repo to its corpus: PASS or FAIL, and what passes lands |
| `gp package hooks` | Print the hash of a package folder's hook code |
| `gp package scan` | Sort every file and every field of a package folder into four buckets: ours, reference, from the ROM and needs rewrite. Compares with each ROM of its `[[rom.entry]]` found by hash in $GP_ROM_DIR and prints no ROM byte. With no ROM it says that nothing was compared. Exits 1 on any needs-rewrite item or a failed rebuild |
| `gp registry check` | Test a registry from outside, against its root if --root is given |
| `gp registry init` | Make a registry in a folder or s3://BUCKET |
| `gp registry publish` | Publish a folder of files as one exact version |
| `gp registry reindex` | Rewrite the list files from these name@version entries, signed if --key is given |
| `gp registry serve` | Serve a folder registry and a built page on this machine only |
| `gp roms add` | Copy ROM files you own into $GP_ROM_DIR/nes/, identifying each one. A ROM no game names yet is added too, as unknown: `gp game new` starts a game for it |
| `gp roms clean` | Write a cleaned copy of a NES image: header from the game's roms.toml (or the file's, by the header rules), no trainer, no trailing bytes. Never changes the input. Exits 1 when the cleaned data matches no game's roms.toml |
| `gp roms scan` | Identify every NES image in $GP_ROM_DIR, whatever it's named, by the game whose roms.toml holds its hash, and report junk around the game data |
| `gp template build` | Build an authored template's lifted files and hold them to its pin |
| `gp template check` | Check a ROM against the template: accepted hash, map pin, lift pin |
| `gp template served` | Check that a package's open version holds none of the original's data: no line of 24 characters or more of its built files may be held by the original's lifted files, and none of its bytes or lifted files may be the ROM's. With --folder, the same for every file of a folder to be served. Exits 1 on any find, and 2 when a ROM isn't found |

## The lab set

For research, tests and the workflow's own tools: what an agent or a contributor uses. May change without notice.

| Command | What it does |
|---|---|
| `gp baseline check` | Run baselines and compare them with their expected results |
| `gp clip log` | Write a corpus clip's input log from power-on: the presses to its start, then its own presses or presses file. Prints the file and the frame the clip starts at |
| `gp clip score` | Score corpus clips on a model: each clip's lockstep from its start, straight through and with resync, both with the original's random draws when the state mapping sets `score_draws`. Prints the frames together both ways, the frame pairing, the exact stretches, the first lasting split and the splits by measure, and writes each clip's log, both lockstep JSONs and its score into OUT/CLIP/ |
| `gp interp compare` | Compare the model with the original on a baseline's input |
| `gp interp reach` | Check that each level's goal can be reached by the player's own moves (gp_interp::reach, the player alone) and measure the jump's envelope, through the game's module. With `--record`, also search each level with its enemies and rules for a run to the goal and write it as a model baseline, `MODEL/baselines/<level>.toml`. Exits 1 when a goal can't be reached or no run is found |
| `gp interp run` | Run a model through its game's module on an input log and write the state hash at power-on and after every frame, one in hex a line |
| `gp level difficulty` | Search a level for a run to its goal (or to `--to`) with the player's own moves, measure how many frames earlier or later each input can come and still clear, and score the level from those windows, by section. This is a research spike, so the run is the first one found and a window is a lower bound |
| `gp level measure` | Measure a stage of sloped girders and ladders on the model (gp_interp::stage). It gives the stage's girders, ladders and things. It also gives each ladder's climb and the direct route to the top. For the rollers (barrels) it gives their routes, as the girders they roll on, the ladders they take and the frames from the throw to the end, and the jump window over each roller that meets the player on each girder |
| `gp self docs` | Write the command list, CLI.md, from the commands gp has. With --check, fail if the file differs |
| `gp self path` | Print the newest published build: the file $GP_HOME/bin/gp points at, which the tool gate keeps at the newest commit it built gp for. A worker that isn't changing Rust runs this build instead of building gp. Exits 1 when none is published yet |
| `gp self publish` | Publish this binary as the build of the commit checked out here, as the tool gate does after a landing. For a first setup, before any gate has run: `just publish`. Refuses a tree with uncommitted changes |
| `gp tool gate` | Hold a patch to gameprinter to the tool's checks and a Donkey Kong canary, and land it when it passes (docs/gate.md) |
| `gp worker env` | Print a worker's setup. The setup is its private output folder ($GP_HOME/work/workers/NAME, made if missing), the cargo target folder of the worktree it runs in ($GP_HOME/work/target/BRANCH/HASH, HASH from the worktree's path; $GP_TARGET_DIR replaces $GP_HOME/work/target), the build profile (fastdev) and the gate's published gp ($GP_HOME/bin/gp), a nice level and a job limit from the machine's load and core count, and the rules every brief repeats. With `--shell`, the same as lines for `eval "$(gp worker env NAME --shell)"`: exports of GP_WORKER, GP_WORKER_OUT, CARGO_TARGET_DIR, GP_PROFILE, GP_BIN, GP (the gp the games repo's `just check` runs), CARGO_BUILD_JOBS and GP_JOBS, $GP_HOME/bin put first on PATH (so `gp` is the published build) unless `--rust`, a renice of the shell, and the rules as comments |
