# The gp commands

Generated from gp's own commands by `gp self docs`; `just check` fails if it differs. Edit the commands, not this file.

## The public set

For making and playing games: what a person installing gp uses.

| Command | What it does |
|---|---|
| `gp assets lift` | Decode a ROM's assets into a directory outside any git work tree |
| `gp assets pack` | Pack a lifted directory and compare it with the ROM, byte for byte |
| `gp corpus bump` | The next version from the clips' changes since the version corpus.toml records. A clip added is minor; a clip removed, a recipe changed, or another ROM or mapping is major; metadata only is a patch. A plan's clips are compared with the plan git has at HEAD. `--write` records it with a changelog entry |
| `gp corpus import` | Turn GAME's mechanic sessions ($GP_HOME/mechanics/GAME) into the clips of its corpus plan: every clip of every session, as its presses from power-on to its start and its own, in inputs/. Sealed sessions stay out, and so do clips whose inputs are a sealed run's ($GP_HOME/sealed/GAME). Rows out of scope in the quest log stay out too, as does what the plan's [import] excludes. Prints what changed and the version step; `--write` writes the clips and records the version |
| `gp corpus init` | Write corpus.toml beside DIR/clips.toml, so an older clip set becomes a scoring corpus with a name, a version (1.0.0 unless given), the ROM and mapping from the set and the content hash of its recipes. A game started by `gp game new` has a corpus plan already |
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
| `gp model mechanics` | A model's mechanic catalog: every mechanic it uses, from the record the simulation's components and the model's hooks declare (system, parameters with types, units and ranges, reads and writes, events, assumptions, invariants, measures, the primitives it's built from, and for a hook the general mechanic it's like), the class of every model path (mechanic, parameter, content or presentation), the hooks that are still game-specific code, and the problems found by holding the model to the records (docs/mechanics.md). Exits 1 when the model and a record disagree |
| `gp package hooks` | Print the hash of a package folder's hook code |
| `gp package scan` | Sort every file and every field of a package folder into four buckets: ours, reference, from the ROM and needs rewrite. Compares with each ROM of its `[[rom.entry]]` found by hash in $GP_ROM_DIR and prints no ROM byte. With no ROM it says that nothing was compared. Exits 1 on any needs-rewrite item or a failed rebuild |
| `gp registry check` | Test a registry from outside, against its root if --root is given |
| `gp registry init` | Make a registry in a folder or s3://BUCKET |
| `gp registry publish` | Publish a folder of files as one exact version |
| `gp registry reindex` | Rewrite the list files from these name@version entries, signed if --key is given |
| `gp registry serve` | Serve a folder registry and a built page on this machine only |
| `gp roms add` | Copy ROM files you own into $GP_ROM_DIR/nes/, or a romset (a MAME zip) into $GP_ROM_DIR/arcade/, identifying each one. A ROM no game names yet is added too, as unknown: `gp game new` starts a game for it |
| `gp roms clean` | Write a cleaned copy of a NES image: header from the game's roms.toml (or the file's, by the header rules), no trainer, no trailing bytes. Never changes the input. Exits 1 when the cleaned data matches no game's roms.toml |
| `gp roms diff` | Compare the PRG and CHR data of two NES images. Either side may be a raw PRG binary, such as a disassembly build |
| `gp roms scan` | Identify every NES image and arcade romset (a MAME zip) in $GP_ROM_DIR, whatever it's named, by the game whose roms.toml holds its hash, and report junk around an NES image's game data |
| `gp template build` | Build an authored template's lifted files and hold them to its pin |
| `gp template check` | Check a ROM against the template: accepted hash, map pin, lift pin |
| `gp template served` | Check that a package's open version holds none of the original's data: no line of 24 characters or more of its built files may be held by the original's lifted files, and none of its bytes or lifted files may be the ROM's. With --folder, the same for every file of a folder to be served. Exits 1 on any find, and 2 when a ROM isn't found |

## The lab set

For research, tests and the workflow's own tools: what an agent or a contributor uses. May change without notice.

| Command | What it does |
|---|---|
| `gp baseline check` | Run baselines and compare them with their expected results |
| `gp capture carry` | What the game's clip starts can't carry into the rebuilt game (gp_interp::carry), for each start and, with `--every`, for frames sampled through the starts' presses. It lists every loaded object a start there can't carry and why, grouped by cause, with the starts each gap alone holds back |
| `gp capture dense` | Dense clips (docs/research/dense-clips.md) are a few short clips that each run much more of the original's code. Candidates from a coverage-guided search of presses at the game's clip starts, valued by the code, branches and measure values they reach on the original, chosen by greedy set cover. They are written as a clip set into a scratch folder (`--into`); the game read from is unchanged. The same seed, ROM and game give the same clips |
| `gp clip log` | Write a corpus clip's input log from power-on: the presses to its start, then its own presses or presses file. Prints the file and the frame the clip starts at |
| `gp clip score` | Score corpus clips on a model: each clip's lockstep from its start, straight through and with resync, both with the original's random draws when the state mapping sets `score_draws`. Prints the frames together both ways, the frame pairing, the exact stretches, the first lasting split and the splits by measure, and writes each clip's log, both lockstep JSONs and its score into OUT/CLIP/ |
| `gp events` | List a recording's events (interrupts, sprite 0 hits, bank switches, controller reads, register writes). Read from its event stream, or by replay when it has none |
| `gp explore` | Search for presses that run code no run has run yet, with no model (gp-explore, docs/research/rom-event-model.md). It starts at the end of each of stage 0's own starts and `--start`'s clips, aims at the routines the code map has that no start clip ran (or those named, or the targets of the places the reader stopped), ranks each candidate by the fewest calls from a routine it ran to a target, and writes what it keeps as clips from power-on. Its output depends only on the ROM, the start and the seed |
| `gp interp compare` | Compare the model with the original on a baseline's input |
| `gp interp reach` | Check that each level's goal can be reached by the player's own moves (gp_interp::reach, the player alone) and measure the jump's envelope, through the game's module. With `--record`, also search each level with its enemies and rules for a run to the goal and write it as a model baseline, `MODEL/baselines/<level>.toml`. Exits 1 when a goal can't be reached or no run is found |
| `gp interp run` | Run a model through its game's module on an input log and write the state hash at power-on and after every frame, one in hex a line |
| `gp labels check` | Check that every RAM address a state mapping reads has a label, and list the ones that don't. Exits 1 if any is missing |
| `gp labels lookup` | Find labels by RAM or CPU address (`0x0400`, `$0400`) or by name. Exits 1 if nothing matches |
| `gp labels show` | List a game's labels, which cover RAM variables and routines and data tables |
| `gp level difficulty` | Search a level for a run to its goal (or to `--to`) with the player's own moves, measure how many frames earlier or later each input can come and still clear, and score the level from those windows, by section. This is a research spike, so the run is the first one found and a window is a lower bound |
| `gp level measure` | Measure a stage of sloped girders and ladders on the model (gp_interp::stage). It gives the stage's girders, ladders and things. It also gives each ladder's climb and the direct route to the top. For the rollers (barrels) it gives their routes, as the girders they roll on, the ladders they take and the frames from the throw to the end, and the jump window over each roller that meets the player on each girder |
| `gp mechanic busy` | Say what you're doing while it's your turn ("measuring the jump"): the person's page shows it until your next turn |
| `gp mechanic demo` | Make a clip of the mechanic with a decision model at the controller: from a clip's start in the session, every few frames the model gets the player's state and the hint and picks the buttons to hold, until the mechanic shows (for a jump, `--times` jumps up and back down) or the frames run out. The page shows it as it plays. The clip goes into the session, made by the model, with each decision's probabilities beside it. NES games only: the player's place is read from the game's state mapping |
| `gp mechanic log` | Add a line to the log the person watches beside the game: what you're doing now. The newest line is your status |
| `gp mechanic pin` | Pin a picture or a clip (png, jpg, gif, webp, mp4, webm) for the person to see beside the game: a sheet of frames, the original against the model. It's copied into the session. The newest few show; `--keep` keeps one at the top. `--drop N` takes pin N off instead |
| `gp mechanic problem` | Add a row to the problem table the person watches: where, what is wrong, the change that fixes it and its state (open, working, fixed, wontfix). With `--n`, changes the fields given of row N |
| `gp mechanic queue` | The game's quest log, shown beside the game: each mechanic with its status (done, asking you, agent working, in line, next) and one plain line. With `--status`, adds the mechanic or changes it; without, prints the quest log |
| `gp mechanic say` | Say something to the person in a session: an answer, what is now known, a question or the clip you want next. It shows over their game, which pauses until they answer. With `--choice` they pick one with the controller. A clip replays under the words when `--show` is given |
| `gp mechanic show` | Show what a session holds, for the agent: the mechanic and its clips with their notes, the conversation and whose turn it is |
| `gp mechanic start` | Open a mechanic's session and serve its page on 127.0.0.1: the game to play, record clips of the mechanic in and hand them to your agent. Makes the session if it is new. Runs until stopped |
| `gp mechanic wait` | Wait until it is the agent's turn (the person handed off a clip or answered), then print the session as `show` does. Run it in the background after each turn you give the person. Without a mechanic, waits on the one the person has open, and also returns when they pick another from the quest log |
| `gp mechanic watch` | The watch panel on a page of its own, on 127.0.0.1: what your agent is doing (its log, pins and problem table) and a box for your notes, for work with no game to play in the mechanic page, such as a new platform's emulator. Makes the session if it is new. Needs no ROM. Runs until stopped |
| `gp movie diagnose` | Find where a recorded movie stops fitting the game: input changes the game never read (made on lag frames and gone by the next read) and changes inside lag runs. A TAS wastes no input, so the first one marks where the replay left the author's path (docs/movies.md) |
| `gp movie hashes` | Print the hashes movies name a ROM by (FCEUX's MD5, BizHawk's SHA-1), for a game's roms.toml, and check them against the registry |
| `gp movie import` | Turn an FCEUX `.fm2` or BizHawk `.bk2` movie into an input log. The movie must name the target ROM by hash; one made on another revision is refused with exit 2 and the revision it names. It imports buttons only, and `gp record` (or `--record`) derives the lag frames |
| `gp query` | Read raw console state such as `ram[0x0770]` or `cpu.pc` at a frame of a recording. It reads the frame from the recording's trace if it has one. For `bus[...]` or a recording without a trace it restores the nearest checkpoint and replays, and exits 1 if the state it reaches isn't the one recorded |
| `gp record` | Run a ROM from power-on through an input log and store a recording (docs/recording.md) |
| `gp recordings list` | Every recording in the store, one line per name, oldest first. Reads the recordings log and the manifests it names |
| `gp recordings show` | Show a recording's names and manifest summary, and the store objects it uses with their sizes |
| `gp replay` | Replay a recording and check every frame against it |
| `gp run-until` | Run a recording until a condition over raw state holds, such as `ram[0x0770] == 1`, and print the frame where it does. A recording with a trace needs no replay up to its last frame. It exits 1 if the condition never holds, or if the state it stops at isn't the one recorded |
| `gp sealed leaks` | List every stored recording, mechanic-session clip and corpus clip of a game that plays one of its sealed runs, the ones every other tool refuses. Only the coordinator runs this command. It reads $GP_HOME/sealed itself, but prints only each leak's ID and the name of the sealed run it matches, never frames or presses |
| `gp sealed score` | Score a sealed run on a model the way `gp clip score` scores a clip (straight through and with resync, with the original's random draws when the mapping sets `score_draws`). It is for the coordinator alone; a worker never runs it, since the run's frames and scores would tell it what the sealed run holds. Everything it writes stays under $GP_HOME/sealed/GAME/, the scores in scores/RUN/ and a line (the model's hash, both percentages and the longest stretch) added to ledger.txt |
| `gp self docs` | Write the command list, CLI.md, from the commands gp has. With --check, fail if the file differs |
| `gp self path` | Print the newest published build: the file $GP_HOME/bin/gp points at, which the tool gate keeps at the newest commit it built gp for. A worker that isn't changing Rust runs this build instead of building gp. Exits 1 when none is published yet |
| `gp self publish` | Publish this binary as the build of the commit checked out here, as the tool gate does after a landing. For a first setup, before any gate has run: `just publish`. Refuses a tree with uncommitted changes |
| `gp tool gate` | Hold a patch to gameprinter to the tool's checks and a Donkey Kong canary, and land it when it passes (docs/gate.md) |
| `gp trace diff` | Compare two recordings frame by frame and report the first frame whose state differs, with the fields that differ. Exits 1 if they differ |
| `gp trace fields` | Print the field table a recording's states follow: each field's name, kind and width, in order |
| `gp trace frame` | Print the state at one frame, by field. Read from the recording's trace, or by restoring a checkpoint and replaying when it has none |
| `gp trace instructions` | Print one line per CPU instruction over a window of frames, in the layout of nestest.log, by restoring the nearest checkpoint and replaying up to the window |
| `gp trace verify` | Check a recording's trace against its frame hashes. Exits 1 if a frame doesn't match |
| `gp watch detail` | L3 for frames A..B (or A..=B) of a recipe the session ran, as `gp reach --detail` gives it |
| `gp watch events` | Print a session's events after `--after` (a `seq`), one JSON line each. With `--follow`, keep printing new ones as the writer adds them, until it is done |
| `gp watch explore` | Search again from the session's start clips (as `gp explore`, aimed at the stops left by default), writing into the session |
| `gp watch list` | List the sessions, with their ROM and how many events each has |
| `gp watch point` | What is under a box of the screen (`--rect X,Y,W,H` in picture pixels) at a frame of a recipe: the sprites there and the routine that last wrote each one's bytes in the sprite table |
| `gp watch rerun` | Run the session's recipes again watching addresses (`--watch $F0`) and hooking instructions (`--hook 01:8000`, `--hook sub_C1FB`), writing each write and call as `watch` and `hook` events (at most 200 for each in each recipe; the rest are counted) |
| `gp watch start` | Open a session and run stage 0 in it: read the ROM alone, run the start clips (stage 0's own starts, and `--start`'s), writing each frame's novelty (code, banks, edges, pointer values, stops passed) as it comes, read again with the runs, then search as `gp explore` does and write each clip it keeps with its novelty. The events go to $GP_HOME/watch/NAME/events.ndjson |
| `gp worker env` | Print a worker's setup. The setup is its private output folder ($GP_HOME/work/workers/NAME, made if missing), the cargo target folder of the worktree it runs in ($GP_HOME/work/target/BRANCH/HASH, HASH from the worktree's path; $GP_TARGET_DIR replaces $GP_HOME/work/target), the build profile (fastdev) and the gate's published gp ($GP_HOME/bin/gp), a nice level and a job limit from the machine's load and core count, and the rules every brief repeats. With `--shell`, the same as lines for `eval "$(gp worker env NAME --shell)"`: exports of GP_WORKER, GP_WORKER_OUT, CARGO_TARGET_DIR, GP_PROFILE, GP_BIN, GP (the gp the games repo's `just check` runs), CARGO_BUILD_JOBS and GP_JOBS, $GP_HOME/bin put first on PATH (so `gp` is the published build) unless `--rust`, a renice of the shell, and the rules as comments |
