# Workers

A worker is an agent given one task, working in a git worktree of its own, on a machine other workers share. Point a new worker at this page; its brief then says only the task, the branch and what to report. gp serve's runner starts workers on the loop's open steps this way, each told its step ([loop.md](loop.md)). This page and [docs/gate.md](gate.md) are enough to take a task and land it, in `gameprinter` (the tool) or `games` (the games).

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
| `CARGO_TARGET_DIR` | This worktree's own target folder, `$GP_HOME/work/target/BRANCH/HASH`. Never another worktree's: cargo mixes checkouts that share a folder. Sharing happens through the compile cache below instead, and for a loop worker through a warm start (below) |
| `GP`, `GP_BIN`, `PATH` | The published gp, first on `PATH`, unless `--rust`: `$GP` runs the games repo's `just check`. A worker not changing Rust builds nothing and runs this build |
| `GP_PROFILE` | `fastdev`, the optimized profile the gate also publishes. Workers changing Rust build with it: `cargo build --profile fastdev -p gp-cli`, then run `$CARGO_TARGET_DIR/fastdev/gp`. A debug build runs the game gate three times slower |
| `CARGO_BUILD_JOBS` | The cores divided by the loop's workers in a `gameprinter` worktree now (this one among them), at least 2. The load average used to set it, and on a machine busy scoring that left -j 2 |
| nice level | From the machine's load: 10 under half load, 15 under full, 19 past it |

`$GP_HOME/bin/locked` comes first on `PATH`, so every `cargo` goes through the locked cargo: one compile at a time on the machine, jobs from free memory, and compiled dependencies shared across worktrees through `sccache` when it is installed. Tests run outside the lock.

### A warm start

gp serve's runner makes a tools worker's target folder when it makes its worktree, as a copy-on-write clone of the tool gate's own target (`<gate home>/tool/target`, built from main's tip): `cp -c` (clonefile) on macOS, `cp --reflink=always` on Linux, so it takes no disk until a build writes. Each file of the worktree that is byte for byte the gate tree's takes the gate tree's file's modified time, so cargo finds fresh what the gate built from those bytes and compiles only what the worker changes; a file that differs keeps its own time and is built again. Where the disk can't clone, or a tool gate is building in its target at that moment, the folder starts empty as before, and the worker's record (`warm`) says why. `gp worker env` says when the folder started warm. A game's module build needs none of this: it already builds in one folder per game, `$GP_HOME/work/module-build/GAME`, kept between builds.

## Tests

Build with incremental compilation on: never set `CARGO_INCREMENTAL=0` in a worktree. After a one-line edit in gp-cli, build, test compile and clippy took 26 s with it on and 81 s with it off (dev profile). The first build after turning it on is a full rebuild; after that edits are quick.

While working, run the tests of the crate you change: `cargo test -p <crate>`. A few tests take 10 s or more each (the Tapper harvest, the arcade `game new` and lockstep tests, Moss Hop's static model, validate and author runs). They start with `if gp_arcade::skip_slow_test() { return; }`, so without `GP_SLOW_TESTS=1` they print "skipped: slow test, set GP_SLOW_TESTS=1" and pass. That takes `cargo test -p gp-cli` from 218 s to about 60 s. A new test that takes over about 10 s gets the same first line.

`just check` (through `just test`) and `gp tool gate` run every test, the slow ones included: each slow test in a run of its own beside one run of the rest, about two minutes for the whole workspace. Run `just test` yourself before landing a change to code a slow test covers.

## Dependencies

Each crate added to the workspace is built by every worker, every gate and CI, so a common job uses what the workspace already has. A test needs a crate even less: write the small check in the test, as `tests/schema_mapping.rs` checks the JSON Schemas with serde_json and a hundred lines of its own (jsonschema had brought 56 crates into the lock for that one file).

| Job | Use |
|-----|-----|
| JSON, TOML | `serde` with `serde_json`, `toml` |
| JSON Schema for a type | `schemars` (gp-model and eleven more) |
| Errors | `anyhow` in gp-cli, `thiserror` in the library crates |
| Arguments | `clap` (derive) |
| Hashes | `blake3` for content, `sha2`, `sha1` and `md-5` where a format names one |
| Hex, base64 | `hex`, `base64` |
| Dates | `time` |
| Temporary files and folders | `tempfile` |
| PNG | `png` |
| Zip, gzip | `zip`, `flate2` |
| Logging | `tracing` |
| Async (the MCP server only) | `tokio` |

No regex crate is in the build (regex comes only with criterion's benches): match a fixed shape by hand, as `fits_pattern` in that test does. A crate the table lacks is a choice to say in the commit message, with what it adds to `Cargo.lock`.

## The rules

`gp worker env NAME` prints these, and `--shell` writes them as comments.

- No `rm` in any form. List what should be deleted in your report; the person deletes it.
- Remove a worktree with `git worktree remove`, adding `--force` once its patch has landed: the change is still uncommitted in it. Empty a build folder with `cargo clean --target-dir DIR`.
- Never commit with `--no-verify`, and never push. A loop worker that runs `git push`, a gate from its shell, a gp_run call citing another step, or an edit of the person's games checkout is stopped by the runner as it runs ([loop.md](loop.md), "The runner's limits").
- Never release gp: no `just release`, `just release-on-main`, `chant release cut --yes` or `chant release publish`, and no release tag pushed. The owner starts a release ([docs/release.md](release.md)); `just release-plan` is a dry run anyone may use.
- Never open `$GP_HOME/sealed` or a sealed run's session.
- ROMs stay in `$GP_ROM_DIR`. Nothing made from a ROM (bytes, lifted files, a built game) goes into a commit or leaves the machine.
- A change lands only through a gate: `gp model gate` for a game, `gp tool gate` for the tool. See [docs/gate.md](gate.md).
- Not changing Rust? Run the published gp and build nothing. Changing Rust? Pass `--rust` and build in your worktree's target folder, in the `fastdev` profile. At most two workers change Rust at once.
- Write outputs in your own folder.
- Do one task, then stop with a report: what changed, the numbers you checked, and what is left.

## Taking a task

1. Make a worktree and run the setup above.
2. Change what the task asks, and make sure `just check` passes in that worktree (it needs `just setup` run in the repo).
3. Make a patch with `git diff > "$GP_WORKER_OUT/fix.patch"`, or `git format-patch` for a commit with its own message and author. A worker gp serve's runner started on a game's row has the game's pending patches (its model gate cards not landed yet) as commits on its branch above main, and its brief names them and the commit above them: build on them rather than making them again, diff from that commit (`git diff BASE`), never from main, and name the cards the patch stands on to the gate with `--after ID` (repeated), so it lands after them or is refused if one of them is ([loop.md](loop.md), "A game row's worker starts from the pending patches").
4. Land it with the gate for that repo ([docs/gate.md](gate.md)).
5. Write your report and stop.
