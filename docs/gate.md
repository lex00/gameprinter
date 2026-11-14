# The gates

Nothing lands on `main` in `gameprinter` or `games` except through a gate. A gate takes a patch, tests it in a worktree in the gate's own home (`$GP_GATE_HOME`, default `~/.gameprinter/gate`), and moves the branch only when it passes. Run the gate from your worktree and pass `--onto main`: without it the gate lands on the branch checked out where it runs, which in a worktree is your task branch. It tests the tip of that branch, not your tree, and a failed gate leaves the branch and your tree alone.

Every commit a gate makes passes the ROM-blocking hook. A patch with a ROM's bytes, a game image, or anything else the hook refuses stops the gate with nothing committed. A patch can't change the hook: the tool gate refuses a patch to `.githooks/` or `scripts/romguard/`, and only the person edits those.

One exception: `site/` in `games`, the list of games, holds no game code, and the game gate only takes a game's folder. It lands directly, through the ROM-blocking hook.

## The game gate: a patch to a game

```text
gp model gate donkey-kong --patch "$GP_WORKER_OUT/fix.patch" --onto main -m "Donkey Kong: ..."
```

`gp model gate` scores the game's corpus on the tip of `games`, applies the patch, builds the game's module if its hooks changed, scans the package for ROM data, checks the model, and scores the corpus again. If no clip got worse, the commit lands on the branch. A patch that removes a clip from the corpus has to name it with `--removes`, or the gate refuses it. `--dry-run` says PASS or FAIL and lands nothing.

In order, a try of a patch (or of a batch's patches, below) on the tip:

| Step | What |
|---|---|
| Patch | Applied as it is (`git apply --index`). A patch made on an older tip that no longer applies is merged onto the tip three ways (`git apply --3way`) and the merge is gated; a merge that conflicts is a FAIL naming the files. Three kinds of file have a rule for a conflict: CLI.md, which gp writes of itself, takes the tip's side and the tool gate writes it again with the merged build; an append-only file ("Append-only files" below) keeps both sides' added rows; a game's model.toml or fidelity.toml is merged through its canonical text (`gp model fmt`'s), so a patch made on a hand-laid layout lands as the facts it changes. `--no-rebase` turns the merge off: the patch must apply as it is. A patch may change only the game's folder |
| Hook | The ROM-blocking hook over what is staged, before each commit, so a ROM's bytes never become a commit |
| Removed clips | A clip the patch removes from corpus.toml fails unless `--removes` names it |
| Module | The game's module built for its hook code, once per hook code and kit |
| Package scan | A needs-rewrite item the tip doesn't have is a FAIL |
| Model | `gp model check` against the lifted assets (an arcade map's slots are held to the asset being there), and `gp model fmt --check`: model.toml and fidelity.toml stay in canonical form (the old guard.py's `models`) |
| Loads | No ROM the package loads only to play (a `[[rom.entry]]` whose role isn't `scored`, or a template.toml `[[loads]]`) is named as template.toml's `[source]` or corpus.toml's `[target]` (the old `ju_rom.py`): no tool may score or gate on one |
| Themes | Each theme's files against their pins |
| Hook tests | When the patch changes the hook crate (or the package's Cargo.lock), the crate's own tests, built from the kit in the build sandbox for this machine (`cargo test -p gp-ext-game`, about 12 s cold) |
| Corpus | Every clip scored and compared with the tip's: a clip at 100% stays there, no bar a clip met is lost, no clip that scored stops scoring. A patch to the model's `present` sections and themes alone is held to the picture too |
| Land | Committed in the gate's worktree and fast-forwarded onto the branch, else the branch's ref is moved if it is still at the tip scored |

### A new game

A patch may add a game that isn't on the tip yet: GAME names its folder (`gp model gate tapper --patch ...` from a checkout of `games`, though the folder isn't there), and the patch creates it, with its package.toml, and changes nothing outside it. The ROM is the one the patch's package.toml names. The tip has no score for the game, so the gate holds the patch to the rest: model check valid, the package scan clean (any needs-rewrite item fails), the hook crate's tests pass, and the corpus scores, every clip making a score, whatever it is. That score is printed as the game's first baseline and kept in the gate's home (`scores/state-<commit>-<run>.json`, and `first_baseline` in `--json`); once it lands, the next gate scores the tip with the game on it and compares as usual. A patch to a game the tip doesn't have that doesn't create its package.toml fails, saying so.

### Batches

Gates on one home run one at a time. A gate that waits leaves a ticket in the home's `queue/` with a copy of its patch. The gate whose turn comes takes up to seven waiting patches on the same repo, game, branch and run into its batch, in the order they asked: each patch is tried alone on the tip first (it applies, the hook passes), then the Markdown-only ones (the free lane: no score reads them, so the hook is their check) are committed, then the rest are gated together on top of them. A batch that fails is split in halves: the first half is settled, then the second on what the first left passing, and a first half that all passed means the failure is in the second, so it isn't tried whole again. A patch that fails on its own (doesn't apply, the hook, an unnamed removed clip) drops out without a split. What passed lands at once, and each waiting gate prints its own patch's outcome, with the batch's lines. A gate with `--dry-run` or `--no-rebase` batches with no other. A gate whose batch-taker dies before it answers gates its patch itself.

A batch commits each patch in the gate's worktree before the package scan and the score, so a FAIL can leave unlanded commits there; nothing lands on the branch unless the whole state passes.

When the branch's tip moves while a gate runs, the gate runs again on the new tip, up to five times in all, merging the patches onto it as above.

### Guards the old gate had that the game gate doesn't

The old `scripts/check/guard.py` ran these on the old one-repo gate. Each is accounted for:

| Guard | Here |
|---|---|
| fmt, clippy | The tool gate's `just check`; a game patch changes no Rust in gameprinter, and its hook crate is built and tested by the steps above |
| models | The game gate's Model step |
| pictures | The game gate's picture rule |
| loads | The game gate's Loads step |
| data | The tool gate runs every test; for a game, its hook crate's tests (Hook tests) |
| prose | The tool gate's `just check` (`just site-prose`, the site's prose lint `site/scripts/lint-prose.mjs`) |
| fonts | Not in a gate: `gp template font` is here, but no gate runs it |
| originals | The tool gate's `just check` (`just originals`: Badge In, Signal Failure, Swing State) |
| budgets | Not in a gate: gp-metrics and `gp metrics check` are here, but no gate runs them |

The check levels (`just check-change`, `just check-batch`, scripts/check/level.py) and the flaky quarantine (flaky.txt, nightly.txt) are not brought back: they chose which tests a gate try ran to save time in the old gate's batches of tool patches. The tool gate runs all of `just check` and the whole test suite for every patch, and a quarantine list would let a patch skip tests, which the tool gate exists to stop.

Run it from a checkout of `games` (your worktree is fine). A patch may change only that game's folder.

### A working copy with no git

A person's own folder (from `gp game fetch`) has no git. The gate keeps a private git of it in the gate's home, with no remote and the ROM-blocking hook installed, one per folder:

```text
gp model gate path/to/my-game -m "Faster walk"
```

The first run takes the folder as it stands as the baseline commit, after the hook. Each later run gates your edits since the last landed commit and, if they pass, commits them in the private git; publishing is a separate step. Leave `--patch` out. Lifted files (`lifted/`) and `target/` aren't tracked.

## The tool gate: a patch to gameprinter

```text
cd your-gameprinter-worktree
$GP_HOME/bin/gp tool gate --patch "$GP_WORKER_OUT/fix.patch" --onto main -m "gp-nes: ..."
```

A worker started with `--rust` has its own build on `PATH` in place of the published gp, so this names the published one. `gp tool gate` puts a worktree at the branch tip, scores the Donkey Kong canary's base there (the game gate's no-op through the tip's gp), applies the patch, and in order:

0. before the tip is touched, looks for the programs `just check` runs (just, node, npm, npx, python3, git, ca65, ld65) on PATH, and after the patch is applied runs `cargo fmt --all --check`, so a missing tool or an unformatted patch fails in seconds (`just check` runs fmt again);
1. builds gp in the optimized `fastdev` profile, in the gate's own target folder, incrementally: the profile says `incremental = true`, and the gate removes a caller's `CARGO_INCREMENTAL` (and `CARGO_PROFILE_FASTDEV_INCREMENTAL`) from what it starts, so a small patch rebuilds in seconds (`GP_GATE_NO_INCREMENTAL=1` keeps the caller's setting, for a gate that hits a broken incremental cache);
2. copies the build to `<gate home>/tool/canary/bins/gp-<hash>`, refuses a tree that differs from its index (a file changed and not staged, or one under crates/ or third_party/ that git neither tracks nor ignores), since the commit holds the index and the canary must score what lands, takes the canary's key (below) and makes its module kit with that copy (`gp game kit`, into the score's home) while nothing else runs in the tree, and then runs three lanes at once, each in a process group of its own:
   - `just check` from the patched tree, with `GP_GATE_RUNS_TESTS=1` so it leaves its test run to the tests lane instead of running every test twice, and `GP_GATE_RUN` set to the gate's process id, so that a `just check` honours `GP_GATE_RUNS_TESTS` only while the gate that set it runs;
   - `cargo test --workspace`, run by the gate itself so a patch can't drop tests by changing the justfile, built in a target folder of its own (`<gate home>/tool/target-tests`: a `--workspace` build resolves syn and a few other shared dependencies with more features than `-p gp-cli`, which in one folder made each rebuild tetanes-core, gp-nes and gp-cli for the other), with the slow tests on (`GP_SLOW_TESTS=1`): each slow test in a run of its own beside one run of the rest, which skips those names, as `just test` does ([docs/workers.md](workers.md)). The gp-cli tests that run gp run the one cargo builds there for them, from the same sources as the published build but with the workspace's features, not the published bytes;
   - the canary through the copy of the patched gp: every clip must score at least as well as on the tip.

   The first lane to fail stops the other two (their process groups get TERM, then KILL after 3 s) and its failure is the gate's. A patch passes only when all three pass, as when they ran one after another; when two things are wrong, the FAIL names whichever failed first. Each lane's output is kept in `<gate home>/tool/logs/`;
3. checks that the build and the tree are still what the canary scored. A check that changed the tree is a FAIL (phase `tree`) naming the files it changed: the kit was made before it, and the commit holds the index, so no score would stand for what lands. A check that built gp again from the same tree means the canary scores the new build, then, with a kit made from the tree as it is, and says `canary: gp changed during the checks, scored again`. Only bytes the canary scored are published;
4. commits, moves the branch and publishes the scored build;
5. starts, without waiting, the module builds of the hooked games in the games checkout with the new gp (`gp game build DIR`, one after another, output in `$GP_HOME/work/warm-modules.log`), after copying the module kit its canary made into `$GP_HOME/work/module-kit/`, so the first page open after a landing finds its module built. A failed build only means the page builds it on open, as before; a gate that starts meanwhile may wait for the machine's build lock on its first cargo call.

A gp built in the gate finds its module kit (gp-sim, gp-interp and the rest that Donkey Kong's module is built from, with Cargo.lock) in the gate's tree, so each side is scored while the tree holds its own sources: the tip before the patch is applied, the patched gp after. Each score runs in its own folder under `<gate home>/tool/canary/`, named by the score's key (`tip-<commit>-<key>`, `patch-<patch hash>-<key>`), so a score never meets a kit an earlier score of another tree left there: gp keeps its kit by its own bytes alone, and a tree that differs only in code gp doesn't link (gp-web, the wasm-only parts) builds the same gp. The kit is made in that folder before the score starts. Each score runs with only PATH, HOME, TMPDIR, RUSTUP_HOME, CARGO_HOME and GP_JOBS kept from the gate's environment and GP_HOME, GP_ROM_DIR and GP_TOOL_SRC set by the gate, so the two sides differ only in the gp and the kit. The one thing the scores share is the machine's tape folder (`~/.gameprinter/work/tapes`, from HOME): the original's side of each clip, keyed by the ROM, the clip's start and presses, and the hash of the emulators' sources and of gp-cli's files that start the original and record its draws (crates/gp-cli/build.rs). A patch to any of those is scored on the emulator; any other patch reads the tapes an earlier score wrote, so only the rebuilt game runs.

A canary score is kept as `<gate home>/tool/canary/base3-<key>.json`. The key is blake3 over everything a score depends on: the bytes of the gp that scored; the tool tree it read (the tree of the gate tree's index, each file that differs from the index, and every untracked or ignored file under crates/ and third_party/ with its bytes, since the module kit copies those folders as they are on disk, leaving out only a crate's own target/); the games tree's HEAD tree, its `git status --ignored`, and the bytes of each file there that differs from HEAD or that git doesn't track, ignored ones too; the ROM's bytes; `rustc -vV` in the tree, run with the kept environment alone as the score is, and the cargo settings in CARGO_HOME (`config.toml`); the kept environment's values and GP_ROM_DIR; and the score's arguments. A tip whose key matches a kept score isn't scored again (`canary: the tip's score is kept`). A landing keeps the patched score under the key of the published build and the tree it scored, which is the key the next gate computes for its tip, so a gate on a tip the gate landed doesn't score the tip either. A `--dry-run` keeps only the tip's score. `GP_GATE_CANARY_VERIFY=1` scores the tip even when a score is kept and fails the gate unless the two agree: run it after a toolchain change, and now and then, as the check on the key.

The gate notes each build it publishes in `<gate home>/tool/published.json` (commit and blake3). A tip's published build is scored as the tip only when it is noted there with its bytes; one the gate didn't publish (a `just publish` from someone's checkout, built from whatever that checkout held) is left alone, and the gate builds the tip and scores its own build.

The patch is applied, or merged three ways onto a newer tip, by the game gate's rules ("Patch" above). When the merge took the tip's CLI.md, the merged build writes it again before `just check`. A failing test step names the tests cargo says failed.

The canary reads the `games` checkout beside the repo (`--games DIR` or `$GP_GAMES_DIR` to name another) and needs the ROM in `$GP_ROM_DIR`; with no ROM it says so and passes without it. Each scored build keeps a home of about 1.1 GB under `<gate home>/tool/canary/`; the gate keeps the two newest and removes the rest. A failed step ends the gate with its last lines and nothing landed. `--dry-run` runs the whole gate and lands nothing. The first gate on a tip builds and publishes that tip first, which is what the canary compares against, so later gates on the same tip are faster. When the tip's score isn't kept, it is made before the patch is even tried, so a patch that fails to apply or build still waits for it: the tip's gp reads its kit from the tree, so the tip can't be scored while the tree holds the patch.

### Timings

Each step of a tool gate run prints `tool gate: step NAME: S s` and adds a line to `<gate home>/tool/steps.jsonl`: `{"at", "run" (the patch's hash), "step", "secs", "ok", "since_start"}`. The steps are checkout, tip build and tip publish (when the tip isn't published), canary (tip), apply, hook, fmt, build, canary kit (the patched gp's key and kit, before the lanes), test compile, one `tests/LABEL` a test run, `lane check`, `lane tests` and `lane canary` (each lane's end, in seconds from the lanes' start), lanes (all three), canary (again) when it ran again, commit, try, land, publish and gate (the whole run). Inside `just check`, `scripts/check/step.sh` prints `check-step: NAME S s` for each step, and the gate keeps those as `check/NAME`. `just check` runs fmt and Moss Hop's asset lift first, then its steps as five lanes at once through `scripts/check/parallel.sh` (rust: clippy, the generated files, Moss Hop, the originals and the script checks; players: play-web, play-web-arcade, check-roms; web: editor, web-parity; page: mechanic-page, which builds its players into its own `target/play-web-mech`; tests: `just test` unless the gate runs them). Each lane's steps run in order and stop at the first failure, each lane's output is printed whole when it ends with a `check-step: lane-NAME S s` line (its log is also in `target/check-lanes/`), and the first lane to fail stops the others and is printed last. After `just check` and after the tests the gate adds a `note` line naming the files of the tree they changed (against the index, and under crates/, third_party/, .cargo/ and the Cargo files by time), since such a file makes cargo build gp again; the note refuses nothing.

## Published builds

The tool gate publishes each landing's gp at `$GP_HOME/bin/gp-<commit>` and points the link `$GP_HOME/bin/gp` at it. The link only moves forward: a build of an older commit is published but not linked. The eight newest builds are kept. Published builds are `fastdev`: a debug gp takes the game gate about 10 minutes cold where `fastdev` takes 3.5.

```text
gp=$($GP_HOME/bin/gp self path) && "$gp" corpus score ...
```

`gp self path` prints the file behind the link, to hold one build through a long job, and exits 1 when none is published. `just publish` publishes the build of the commit checked out here, for a first setup.

## Append-only files

Some files every lane appends to. Two patches made on one tip each add rows at the end, so the second's three-way merge conflicts there. A conflict in an append-only file is resolved by its rows (`crates/gp-cli/src/gate_append.rs`): every line the base has stays as it is, and at each place both sides added lines, both sides' lines are kept, in date order by the first cell (`| YYYY-MM-DD |`), each side's own order kept and the tip's first on a tie; a line both added is kept once. A side that changed or dropped a line the base has is still a conflict, and the patch fails naming the file. The rest of the file merges as git merges it.

| File | What appends to it |
|---|---|
| `docs/research/deterministic-inference.md` | The notes log: a dated row from each lane that finds a rule by a signal a tool could read |

A file joins the list by a row here and in `APPEND_ONLY` in gate_append.rs; a test holds the two lists together.

## Affected clips

Brought from game-printer's docs/gate.md, edited to gameprinter's layout. A score costs what a patch can move. Each clip's run records what it read (`gp_sim::reads`): the spec's kinds, levels, machines, counters and cameras note each element a clip touches (anything that sees a whole list notes all of it), and the hooks note each call by the hook's name. `gp corpus score` keeps them in the score's JSON (`reads`, by clip), next to a fingerprint of everything else the score read (`fingerprint`, crates/gp-cli/src/affected.rs):

| Part of the fingerprint | What it covers | Changed: |
|---|---|---|
| `gp` | This gp build | every clip |
| `mapping`, `contract`, `rom`, `input` | The state mapping, fidelity.toml, the ROM and the run's input | every clip |
| `tolerances`, `catalog`, `pictures` | The tolerances, the mechanic catalog and the model's decoded pictures | every clip |
| `spec` | The spec but its lists' elements | every clip |
| `lists` | Each kind, level, machine, counter and camera, by name, as a hash of the compiled element | the clips that read it |
| `hooks` | Each hook, as a hash of the code it can reach in the game's `ext/` crate (below) | the clips that called it |

`gp corpus score GAME --run R --reuse EARLIER.json` compares the fingerprint with an earlier score's. When a part every clip reads changed, every clip is scored. Otherwise only a clip whose earlier reads meet an element or hook whose hash changed, a new clip, one whose recipe changed, one that couldn't be scored or was stopped, and one with no reads recorded. Every other clip keeps its earlier row. The output's `affected` says what changed and how many clips were scored and kept.

A hook's hash covers the code it can reach (crates/gp-cli/src/hook_closure.rs). In game-printer build.rs hashed every template's hook crate into the binary; a game's hook crate is now its package's `ext/`, outside gp's build, so gp reads it when it scores. The crate is read with syn: each fn, const, static, type, struct, enum and trait is a unit, and each item of an `impl` belongs to the type it is for. A unit reaches the units its names resolve to, by module as Rust resolves them, and every method of a type it names. A hook is the type `exts()` registers under its name. Doc comments, comments and test modules change nothing. The crate's other files and its Cargo.toml are in every hook's hash, and a crate that can't be read so (an `exts()` of another shape, a `#[path]`, a file that doesn't parse) makes every hook the hash of the whole `ext/` tree, which is what every hook was before 2026-11-12. Matching is by name, so a hook reaches more than it calls, never less: in Donkey Kong a change to `Taken::step` (m100.rs) moves dk_taken, dk_elevators, dk_idle and dk_sound, 4 of its 17 hooks, where it moved all 17 before.

A clip exact in the earlier score stops at its first wrong frame while the mapping is the earlier score's: a split decides it is worse, so the rest of its run isn't needed.
