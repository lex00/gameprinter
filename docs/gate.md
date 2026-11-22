# The gates

Nothing lands on `main` in `gameprinter` or `games` except through a gate. A gate takes a patch, tests it in a worktree in the gate's own home (`$GP_GATE_HOME`, default `~/.gameprinter/gate`), and moves the branch only when it passes. Run the gate from your worktree and pass `--onto main`: without it the gate lands on the branch checked out where it runs, which in a worktree is your task branch. It tests the tip of that branch, not your tree, and a failed gate leaves the branch and your tree alone.

Every commit a gate makes passes the ROM-blocking hook. A patch with a ROM's bytes, a game image, or anything else the hook refuses stops the gate with nothing committed. A patch can't change the hook: the tool gate refuses a patch to `.githooks/` or `scripts/romguard/`, and only the person edits those.

One exception: `site/` in `games`, the list of games, holds no game code, and the game gate only takes a game's folder. It lands directly, through the ROM-blocking hook.

## The game gate: a patch to a game

```text
gp model gate GAME --patch "$GP_WORKER_OUT/fix.patch" --onto main -m "GAME: ..."
```

`gp model gate` scores the game's corpus on the tip of `games`, applies the patch, builds the game's module if its hooks changed, scans the package for ROM data, checks the model, and scores the corpus again. If no clip got worse, the commit lands on the branch. A patch that removes a clip from the corpus has to name it with `--removes`, and one that changes a clip's recipe with `--replaces`, or the gate refuses it. `--dry-run` says PASS or FAIL and lands nothing. A patch made on top of cards not landed yet names them with `--after ASK` (repeated): the gate refuses it while one of them waits, holds a yes or runs, and once one was refused or failed, and gp serve's runner runs a YES to it only after they land (docs/loop.md, "A game row's worker starts from the pending patches"). A dry run with `--after` says which haven't landed and goes on, on the tip without them.

In order, a try of a patch (or of a batch's patches, below) on the tip:

| Step | What |
|---|---|
| Patch | Applied as it is (`git apply --index`). A patch made on an older tip that no longer applies is merged onto the tip three ways (`git apply --3way`) and the merge is gated; a merge that conflicts is a FAIL naming the files. Three kinds of file have a rule for a conflict: CLI.md, which gp writes of itself, takes the tip's side and the tool gate writes it again with the merged build; an append-only file ("Append-only files" below) keeps both sides' added rows; a game's model.toml or fidelity.toml is merged through its canonical text (`gp model fmt`'s), so a patch made on a hand-laid layout lands as the facts it changes. `--no-rebase` turns the merge off: the patch must apply as it is. A patch may change only the game's folder |
| Hook | The ROM-blocking hook over what is staged, before each commit, so a ROM's bytes never become a commit |
| Removed clips | A clip the patch removes from corpus.toml fails unless `--removes` names it |
| Changed clips | A clip already in corpus.toml whose recipe the patch changes (its fields but `name` and `why`, or the contents of the presses files it reads) fails unless `--replaces` names it: a clip made shorter or played another way can still score 100% while it holds less |
| Module | The game's module built for its hook code, once per hook code and kit |
| Package scan | A needs-rewrite item the tip doesn't have is a FAIL |
| Model | `gp model check` against the lifted assets (an arcade map's slots are held to the asset being there), and `gp model fmt --check`: model.toml and fidelity.toml stay in canonical form (the old guard.py's `models`) |
| Loads | No ROM the package loads only to play (a `[[rom.entry]]` whose role isn't `scored`, or a template.toml `[[loads]]`) is named as template.toml's `[source]` or corpus.toml's `[target]` (the old `ju_rom.py`): no tool may score or gate on one |
| Themes | Each theme's files against their pins |
| Hook tests | When the patch changes the hook crate (or the package's Cargo.lock), the crate's own tests, built from the kit in the build sandbox for this machine (`cargo test -p gp-ext-game`, about 12 s cold) |
| Corpus | Every clip scored and compared with the tip's: a clip at 100% stays there, no bar a clip met is lost, no clip that scored stops scoring. A patch to the model's `present` sections and themes alone is held to the picture too. Every clip held worse is listed, and any one fails the change unless the game's `tolerate.toml` names it ("Tolerated" below) |
| Approved | Each look the owner approved (the game's `approved.toml`) played again: no locked frame further from the original ("Approved looks" below) |
| Land | Committed in the gate's worktree and fast-forwarded onto the branch, else the branch's ref is moved if it is still at the tip scored |

### Tolerated

A clip may score worse on a change only for a stated reason: a known lag, a choice the owner made. The reason lives in the game's folder, in `tolerate.toml`, one table per clip (an ID, or a pattern with `*`):

```toml
[[tolerate]]
clip = "tapper.attract.*"
why = "the board's one-frame sprite lag (issue #42)"
```

A tolerance with no `clip` or no `why` fails the gate. The file is read as the patch leaves it, so a patch that adds a tolerance shows it in its diff, and lands only with the person's yes like any patch. Each regression a tolerance lets pass is a line of the gate's (`tolerated: tapper.attract.001: was 100% ..., tolerated: the board's one-frame sprite lag`) and is in `--json` as `tolerated`, so the card says it.

### Approved looks

When the owner says yes to a look (a quest-log row's clip and frames, judged on the screens in the game's own pictures), `gp mechanic approve GAME ROW` locks it: the row's clip (a corpus clip, or clip N of a row once it is in the corpus as GAME.ROW.NNN) is played as `gp picture` plays it, and each of its frames from the row's `from` to `to` is written to the game's `approved.toml` with its pixels apart from the original on each layer (the screen measure's distances):

```toml
[[look]]
row = "get-ready-screen"
clip = "tapper.l1.001"
frames = [70, 88]
look = "game"
approved = "2026-11-20"
apart = [
  [70, 0, 0],
  [71, 12, 0],
]
```

The gate plays each locked clip again on every change, whatever the patch touches, and fails it (phase `approved`) when a locked frame is further apart from the original on either layer than when approved, or isn't compared any more. Closer passes. No tolerance moves a lock; the owner's yes again (`gp mechanic approve` once more, which replaces the row's lock) does, and the file is read as the patch leaves it, so a patch that edits a lock shows it in its diff. Only the game's own look is locked yet: a theme's look is refused.

### Clips go in on their own

`gp model gate GAME --import` takes the clips the mechanic sessions have that the corpus hasn't into the corpus through the gate, and needs no yes from a tool (gp_run): a clip only added changes nothing already held, and the gate fails the import if any clip scores worse. With `--replaces` or `--removes` it changes clips already there and asks, as before.

While gp serve's runner is on and the loop isn't halted, it does this itself every two minutes for each game with mechanic sessions: a dry `gp corpus import`, and when that would add clips, `gp model gate GAME --import`. So every clip the person records and every clip a worker makes in a session goes into the corpus with no card. A set of clips that failed isn't tried again until the sessions give another. What it did last is in `$GP_HOME/work/loop/kept/GAME.json`, and in the runner's status (`kept`).

### A new game

A patch may add a game that isn't on the tip yet: GAME names its folder (`gp model gate my-game --patch ...` from a checkout of `games`, though the folder isn't there), and the patch creates it, with its package.toml, and changes nothing outside it. The ROM is the one the patch's package.toml names. The tip has no score for the game, so the gate holds the patch to the rest: model check valid, the package scan clean (any needs-rewrite item fails), the hook crate's tests pass, and the corpus scores, every clip making a score, whatever it is. That score is printed as the game's first baseline and kept in the gate's home (`scores/state-<commit>-<run>.json`, and `first_baseline` in `--json`); once it lands, the next gate scores the tip with the game on it and compares as usual. A patch to a game the tip doesn't have that doesn't create its package.toml fails, saying so.

### Batches

Model gates on one home run one at a time, and so do tool gates, but a model gate and a tool gate run at the same time: they land in different repos, and each kind has its own lock and queue ("The locks" below). A gate that waits leaves a ticket in its kind's queue (the home's `queue/` for model gates, `tool/queue/` for tool gates) with a copy of its patch. The gate whose turn comes takes up to seven waiting patches on the same repo, game, branch and run into its batch, in the order they asked: each patch is tried alone on the tip first (it applies, the hook passes), then the Markdown-only ones (the free lane: no score reads them, so the hook is their check) are committed, then the rest are gated together on top of them. A batch that fails is split in halves: the first half is settled, then the second on what the first left passing, and a first half that all passed means the failure is in the second, so it isn't tried whole again. A patch that fails on its own (doesn't apply, the hook, an unnamed removed clip) drops out without a split. What passed lands at once, and each waiting gate prints its own patch's outcome, with the batch's lines. A gate with `--dry-run` or `--no-rebase` batches with no other. A gate whose batch-taker dies before it answers gates its patch itself.

A batch commits each patch in the gate's worktree before the package scan and the score, so a FAIL can leave unlanded commits there; nothing lands on the branch unless the whole state passes.

When the branch's tip moves while a gate runs, the gate runs again on the new tip, up to five times in all, merging the patches onto it as above.

Tool gates batch the same way, with each other only (their tickets' run is `tool gate`). The gate holding the lock waits 3 s if it found the lock free, so the gates started with it can take their tickets, then takes up to seven waiting tool patches on the same repo and branch. Each is applied and committed on the tip in turn; one that doesn't apply beside the others, or that the hook refuses, is left out and gates itself after. The rest are built and checked once (`just check` and the tests) and land together, each its own commit, with one publish. A batch that fails gives every other patch back to its own gate and gates this gate's patch alone, so a bad patch costs the others one more run, never their landing.

The loop's runner starts every yes'd `model gate` and `tool gate` card at once (any other yes still runs one at a time), so cards the person said yes to together land in one gate run.

### A dry run's pass, used by the YES

A model gate `--dry-run` that passes keeps its pass in the gate's home, `passed/KEY.json`, with the tip's score and its own beside it. KEY is the BLAKE3 of the patch's bytes, its message and author, its `--removes` and `--replaces`, the game and the branch; the file names the tip, the gp the gate ran (its file, size and time), the game kit's hash and the commit the dry run made on the tip. The gate after the person's YES, when its patch has such a pass on the very same tip with the same gp and kit, and its batch is that patch alone, lands that commit with no scoring again and says so (`dry_run_reused` in its JSON, and a line naming when the dry run passed); the status file is written from the kept scores. Anything else (the tip moved, a new gp published, the kit changed, the commit gone) scores the patch again as before, with a line saying why. A pass is used once, or dropped when it can't be. A working copy with no git of its own is always scored, since its landing writes curves back to the copy.

### The locks

A home has two locks. `gate.lock` is the model gate's (it works in the home's `tree/`, `kit/`, `modules/` and `scores/` and lands in the games checkout), and `tool.lock` is the tool gate's (it works in `tool/` and lands in gameprinter). A tool gate's publish moves `$GP_HOME/bin/gp` while a model gate may be running; the model gate finds the file of the gp it started on once, its links followed, and starts every gp of its run from that file, so it finishes on the gp it started on. A tool gate of a gp from before the two locks held `gate.lock` and worked in `tool/tree`: a tool gate that finds such a gate holding `gate.lock` (its holder file says `tool gate`) waits for it, and `gp loop clean --gate` leaves the tool gate's build folders while either runs. Such a gate can also start after this one took `tool.lock`, since `gate.lock` is free then, so the tool gate's tree is one per lock: a gate under `tool.lock` works in `tool/tree-tool-lock` and reads a batch's patches to `tool/batch-tool-lock`, and never shares a tree with an older gp's (issue 45: an older gp's dry run held `gate.lock` and ran in `tool/tree` beside the server's landing, which failed in 0.1 s on git's `index.lock`).

The gate holding a lock writes its holder file beside it (`gate.holder.json`, `tool.holder.json`): its pid, its parent's pid, when it took the lock, what it is (`tool gate`, `tool gate dry run`, `model gate`, `model gate dry run`) and the steps file it logs to (the tool gate's `tool/steps.jsonl`; the model gate has none). While it holds the lock it watches itself. A dry run whose parent pid changed (the worker or MCP server that started it ended) stops, since no one reads its verdict; a landing run doesn't, because the runner starts landings to outlive gp serve. A tool gate that has logged no step for 30 minutes stops (`GP_GATE_IDLE_S` sets the limit in seconds; a normal run's longest gap is about 15). Stopping ends the lanes' process groups, so the gate ends with a FAIL that says why; one that hasn't ended 30 s later exits, which frees the lock. `gp loop health` prints who holds each lock, and says PROBLEM naming the pid when the holder logged no step for 30 minutes (a model gate: held over 2 hours), or when the lock is held by a gate of an older gp that wrote no holder file.

### A yes kept

The person's yes to a gate card is used only by the patch's own failure: a check that failed, or a patch that doesn't apply. A gate that couldn't land for a cause outside the patch says so: its FAIL has `"not_the_patch": true` in `--json`, with the phase:

| Phase | What stopped it |
|---|---|
| `ready` | Before the tree is touched: git's `index.lock` is in the gate's tree (another git, or a gate of another gp, works there), or, for a run that lands, the checkout where the branch is checked out has uncommitted changes to a file the patch changes, which `git merge --ff-only` would refuse (issue 45) |
| `checkout` | The gate's tree couldn't be reset and checked out at the tip |
| `land` | The checks passed and git refused the landing |
| `tip` | The tip moved on every try |

gp serve's runner reads that and puts the yes back: the card is a yes again, with `kept` (how many times), `kept_why` and `kept_until_ms`, and the runner runs it again a minute later, up to 3 times. A fourth such failure uses the yes like any other, its last line saying it couldn't land 4 times, and the card's row goes back to next with why in its note. Both gates do this; in a batch of model gates, each patch's gate is told.

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

The check levels (`just check-change`, `just check-batch`, scripts/check/level.py) and the flaky quarantine (flaky.txt, nightly.txt) are not brought back: they chose which tests a gate try ran to save time in the old gate's batches of tool patches. The tool gate runs all of `just check` and the whole test suite for every patch it lands (alone or in its batch), and a quarantine list would let a patch skip tests, which the tool gate exists to stop.

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

A worker started with `--rust` has its own build on `PATH` in place of the published gp, so this names the published one. `gp tool gate` puts a worktree at the branch tip, applies the patch, and in order:

0. before the tip is touched, looks for the programs `just check` runs (just, node, npm, npx, python3, git, ca65, ld65) on PATH, and after the patch is applied runs `cargo fmt --all --check`, so a missing tool or an unformatted patch fails in seconds (`just check` runs fmt again);
1. builds gp in the optimized `fastdev` profile, in the gate's own target folder, incrementally: the profile says `incremental = true`, and the gate removes a caller's `CARGO_INCREMENTAL` (and `CARGO_PROFILE_FASTDEV_INCREMENTAL`) from what it starts, so a small patch rebuilds in seconds (`GP_GATE_NO_INCREMENTAL=1` keeps the caller's setting, for a gate that hits a broken incremental cache);
2. runs two lanes at once, each in a process group of its own:
   - `just check` from the patched tree, with the gate's target folder and profile, so every step runs the patched gp. `GP_GATE_RUNS_TESTS=1` makes it leave its test run to the tests lane instead of running every test twice, and `GP_GATE_RUN` is set to the gate's process id, so that a `just check` honours `GP_GATE_RUNS_TESTS` only while the gate that set it runs;
   - `cargo test --workspace`, run by the gate itself so a patch can't drop tests by changing the justfile, built in a target folder of its own (`<gate home>/tool/target-tests`: a `--workspace` build resolves syn and a few other shared dependencies with more features than `-p gp-cli`, which in one folder made each rebuild tetanes-core, gp-nes and gp-cli for the other), with the slow tests on (`GP_SLOW_TESTS=1`): each slow test in a run of its own beside one run of the rest, which skips those names, as `just test` does ([docs/workers.md](workers.md)). The gp-cli tests that run gp run the one cargo builds there for them, from the same sources as the published build but with the workspace's features, not the published bytes.

   The first lane to fail stops the other (its process group gets TERM, then KILL after 3 s) and its failure is the gate's. A patch passes only when both pass, as when they ran one after the other; when two things are wrong, the FAIL names whichever failed first. Each lane's output is kept in `<gate home>/tool/logs/`;
3. commits, moves the branch and publishes the build.

The game check is Moss Hop's, inside `just check`: Moss Hop's ROM built from its source (`just mosshop`, with ca65 and ld65), its assets lifted and packed, `gp baseline check` on baselines/mosshop, the model checked and in canonical form, `_interp-mosshop` (each baseline replayed in the interpreter, `gp interp compare`) and `gp template check`. All of it runs through the patched gp and needs no ROM and no games checkout. `just check` reads no ROM and no games checkout, so the gate tests the tool and never a ported game; games are tested by their own `gp model gate`. The browser checks (the editor, web parity, play-web, the mechanic page, and the deploy and vendor script tests) are `just check-web`, which CI runs after `just check` and which you run by hand when you change the page, the editor or the play and web crates.

The patch is applied, or merged three ways onto a newer tip, by the game gate's rules ("Patch" above). When the merge took the tip's CLI.md, the merged build writes it again before `just check`. A failing test step names the tests cargo says failed. A failed step ends the gate with its last lines and nothing landed. A fmt failure's first line names every file rustfmt would change. `--dry-run` runs the whole gate and lands and publishes nothing.

A dry run that passes keeps its pass in `<gate home>/tool/passed/TREE`, TREE the git tree of the tip with the patch applied. A gate that lands and makes that same tree within a day (the same patch on a tip that hasn't moved) builds it for publishing and lands it without running fmt, `just check` and the tests again, and says so in its lines. A tip that moved, or a batch of several patches, makes another tree, which is checked. A loop worker on a tools row builds once, runs only the tests it added or changed, and leaves the suite, `just check` and clippy to the dry run.

### Timings

Each step of a tool gate run prints `tool gate: step NAME: S s` and adds a line to `<gate home>/tool/steps.jsonl`: `{"at", "run" (the patch's hash), "step", "secs", "ok", "since_start"}`. The steps are checkout, apply, hook, fmt, build, test compile, one `tests/LABEL` a test run, `lane check` and `lane tests` (each lane's end, in seconds from the lanes' start), lanes (both), commit, try, land, publish and gate (the whole run). Inside `just check`, `scripts/check/step.sh` prints `check-step: NAME S s` for each step, and the gate keeps those as `check/NAME`. `just check` runs fmt and Moss Hop's asset lift first, copies gp to `target/check-bin/gp` (the lanes run that copy, `GP_CHECK_BIN`, since the tests' cargo builds write another gp, its features unified otherwise, to `target/` while the other lanes run), then its steps as five lanes at once through `scripts/check/parallel.sh` (rust: clippy, the generated files, Moss Hop, the originals and the script checks; players: play-web, play-web-arcade, check-roms; web: editor, web-parity; page: mechanic-page, which builds its players into its own `target/play-web-mech`; tests: `just test` unless the gate runs them). Each lane's steps run in order and stop at the first failure, each lane's output is printed whole when it ends with a `check-step: lane-NAME S s` line (its log is also in `target/check-lanes/`), and the first lane to fail stops the others and is printed last. The page lane's waits for a build (two minutes or more: the players, the preview, a game's module) are a Mac's; `GP_CHECK_BUILD_WAIT=N` lets them take N times as long on a slower machine, and the check workflow on GitHub sets 3 for its four-core runner. It changes how long the check waits, not what it checks. After `just check` and after the tests the gate adds a `note` line naming the files of the tree they changed (against the index, and under crates/, third_party/, .cargo/ and the Cargo files by time), since such a file makes cargo build gp again; the note refuses nothing.

## Published builds

The tool gate publishes each landing's gp at `$GP_HOME/bin/gp-<commit>` and points the link `$GP_HOME/bin/gp` at it. The link only moves forward: a build of an older commit is published but not linked. The eight newest builds are kept. Published builds are `fastdev`: a debug gp takes the game gate about 10 minutes cold where `fastdev` takes 3.5.

Beside each build the gate copies the page's own files from the tree it built in: `$GP_HOME/bin/gp-<commit>.pages/` holds `mechanic/` (the game's page, the loop page, the journeys) and `editor/dist/` (the editor). A published gp serves those, and falls back to the checkout's files only when its build has none. The gate's tree holds each patch it tries, dry runs included, so serving from it would show the person a page that hasn't landed; with the copy, their page changes only when a landing publishes and gp serve moves onto the new build. A build's page files are pruned with it.

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

A hook's hash covers the code it can reach (crates/gp-cli/src/hook_closure.rs). In game-printer build.rs hashed every template's hook crate into the binary; a game's hook crate is now its package's `ext/`, outside gp's build, so gp reads it when it scores. The crate is read with syn: each fn, const, static, type, struct, enum and trait is a unit, and each item of an `impl` belongs to the type it is for. A unit reaches the units its names resolve to, by module as Rust resolves them, and every method of a type it names. A hook is the type `exts()` registers under its name. Doc comments, comments and test modules change nothing. The crate's other files and its Cargo.toml are in every hook's hash, and a crate that can't be read so (an `exts()` of another shape, a `#[path]`, a file that doesn't parse) makes every hook the hash of the whole `ext/` tree, which is what every hook was before 2026-11-12. Matching is by name, so a hook reaches more than it calls, never less: in a game whose `ext/` registers 17 hooks, a change to one type's `step` moves only the hooks that name that type, where it moved all 17 before.

A clip exact in the earlier score stops at its first wrong frame while the mapping is the earlier score's: a split decides it is worse, so the rest of its run isn't needed.
