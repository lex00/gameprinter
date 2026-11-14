# Workers

A worker is an agent given one task, working in a git worktree of its own, on a machine other workers share. Point a new worker at this page; its brief then says only the task, the branch and what to report. This page and [docs/gate.md](gate.md) are enough to take a task and land it, in `gameprinter` (the tool) or `games` (the games).

## Before the first task on a machine

Once, on the machine:

```text
cd gameprinter && just setup && just publish
cd ../games && just setup
```

`just setup` turns on the ROM-blocking git hook and installs the locked cargo at `$GP_HOME/bin/locked/cargo`. `just publish` builds gp (optimized) and publishes it at `$GP_HOME/bin/gp`, so workers have a gp to run before any gate has run. After that the tool gate publishes a new build each time it lands a patch. `$GP_HOME` is `~/.gameprinter`. `$GP_ROM_DIR` is where ROMs live (`/Users/alex/Documents/roms` on the maintainer's machine).

## Setup, at the start of each shell

Make a worktree and a branch for the task, then, in the worktree:

```text
git -C gameprinter worktree add ../wt-tool -b task/my-fix
cd ../wt-tool
export GP_HOME="${GP_HOME:-$HOME/.gameprinter}"
eval "$($GP_HOME/bin/gp worker env NAME --shell)"
```

A worker changing Rust passes `--rust`. Run it again at the start of each new shell; an agent's tool calls each start a fresh one. It works the same in a worktree of `gameprinter` or of `games`.

| What | Where it comes from |
|---|---|
| `GP_WORKER`, `GP_WORKER_OUT` | The name, and your private folder `$GP_HOME/work/workers/NAME`. Write outputs there, never in a shared folder |
| `CARGO_TARGET_DIR` | This worktree's own target folder, `$GP_HOME/work/target/BRANCH/HASH`. Never another worktree's: cargo mixes checkouts that share a folder. Sharing happens through the compile cache below instead |
| `GP`, `GP_BIN`, `PATH` | The published gp, first on `PATH`, unless `--rust`: `$GP` runs the games repo's `just check`. A worker not changing Rust builds nothing and runs this build |
| `GP_PROFILE` | `fastdev`, the optimized profile the gate also publishes. Workers changing Rust build with it: `cargo build --profile fastdev -p gp-cli`, then run `$CARGO_TARGET_DIR/fastdev/gp`. A debug build runs the game gate three times slower |
| `CARGO_BUILD_JOBS`, nice level | From the machine's load |

`$GP_HOME/bin/locked` comes first on `PATH`, so every `cargo` goes through the locked cargo: one compile at a time on the machine, jobs from free memory, and compiled dependencies shared across worktrees through `sccache` when it is installed. Tests run outside the lock.

## Tests

Build with incremental compilation on: never set `CARGO_INCREMENTAL=0` in a worktree. After a one-line edit in gp-cli, build, test compile and clippy took 26 s with it on and 81 s with it off (dev profile). The first build after turning it on is a full rebuild; after that edits are quick.

While working, run the tests of the crate you change: `cargo test -p <crate>`. A few tests take 10 s or more each (the Tapper harvest, the arcade `game new` and lockstep tests, Moss Hop's static model, validate and author runs). They start with `if gp_arcade::skip_slow_test() { return; }`, so without `GP_SLOW_TESTS=1` they print "skipped: slow test, set GP_SLOW_TESTS=1" and pass. That takes `cargo test -p gp-cli` from 218 s to about 60 s. A new test that takes over about 10 s gets the same first line.

`just check` (through `just test`) and `gp tool gate` run every test, the slow ones included: each slow test in a run of its own beside one run of the rest, about two minutes for the whole workspace. Run `just test` yourself before landing a change to code a slow test covers.

## The rules

`gp worker env NAME` prints these, and `--shell` writes them as comments.

- No `rm` in any form. List what should be deleted in your report; the person deletes it.
- Remove a worktree with `git worktree remove`, adding `--force` once its patch has landed: the change is still uncommitted in it. Empty a build folder with `cargo clean --target-dir DIR`.
- Never commit with `--no-verify`, and never push.
- Never open `$GP_HOME/sealed` or a sealed run's session.
- ROMs stay in `$GP_ROM_DIR`. Nothing made from a ROM (bytes, lifted files, a built game) goes into a commit or leaves the machine.
- A change lands only through a gate: `gp model gate` for a game, `gp tool gate` for the tool. See [docs/gate.md](gate.md).
- Not changing Rust? Run the published gp and build nothing. Changing Rust? Pass `--rust` and build in your worktree's target folder, in the `fastdev` profile. At most two workers change Rust at once.
- Write outputs in your own folder.
- Do one task, then stop with a report: what changed, the numbers you checked, and what is left.

## Taking a task

1. Make a worktree and run the setup above.
2. Change what the task asks, and make sure `just check` passes in that worktree (it needs `just setup` run in the repo).
3. Make a patch with `git diff > "$GP_WORKER_OUT/fix.patch"`, or `git format-patch` for a commit with its own message and author.
4. Land it with the gate for that repo ([docs/gate.md](gate.md)).
5. Write your report and stop.
