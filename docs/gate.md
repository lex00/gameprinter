# The gates

Nothing lands on `main` in `gameprinter` or `games` except through a gate. A gate takes a patch, tests it in a worktree in the gate's own home (`$GP_GATE_HOME`, default `~/.gameprinter/gate`), and moves the branch only when it passes. Run the gate from your worktree and pass `--onto main`: without it the gate lands on the branch checked out where it runs, which in a worktree is your task branch. It tests the tip of that branch, not your tree, and a failed gate leaves the branch and your tree alone.

Every commit a gate makes passes the ROM-blocking hook. A patch with a ROM's bytes, a game image, or anything else the hook refuses stops the gate with nothing committed. A patch can't change the hook: the tool gate refuses a patch to `.githooks/` or `scripts/romguard/`, and only the person edits those.

## The game gate: a patch to a game

```text
gp model gate donkey-kong --patch "$GP_WORKER_OUT/fix.patch" --onto main -m "Donkey Kong: ..."
```

`gp model gate` scores the game's corpus on the tip of `games`, applies the patch, builds the game's module if its hooks changed, scans the package for ROM data, checks the model, and scores the corpus again. If no clip got worse, the commit lands on the branch. A patch that removes a clip from the corpus has to name it with `--removes`, or the gate refuses it. `--dry-run` says PASS or FAIL and lands nothing.

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
gp tool gate --patch "$GP_WORKER_OUT/fix.patch" --onto main -m "gp-nes: ..."
```

`gp tool gate` puts a worktree at the branch tip, applies the patch, and in order:

1. builds gp in the optimized `fastdev` profile, in the gate's own target folder;
2. runs `just check` from the patched tree;
3. runs `cargo test --workspace` itself, so a patch can't drop tests by changing the justfile;
4. runs the Donkey Kong canary, the game gate's no-op scored through the patched gp and through the tip's gp: every clip must score at least as well;
5. commits, moves the branch and publishes the new build.

The canary reads the `games` checkout beside the repo (`--games DIR` or `$GP_GAMES_DIR` to name another) and needs the ROM in `$GP_ROM_DIR`; with no ROM it says so and passes without it. Each scored build keeps a home of about 1.1 GB under `<gate home>/tool/canary/`, and the gate doesn't remove them; the person empties that folder when disk is short. A failed step ends the gate with its last lines and nothing landed. `--dry-run` runs the whole gate and lands nothing. The first gate on a tip builds and publishes that tip first, which is what the canary compares against, so later gates on the same tip are faster.

## Published builds

The tool gate publishes each landing's gp at `$GP_HOME/bin/gp-<commit>` and points the link `$GP_HOME/bin/gp` at it. The link only moves forward: a build of an older commit is published but not linked. The eight newest builds are kept. Published builds are `fastdev`: a debug gp takes the game gate about 10 minutes cold where `fastdev` takes 3.5.

```text
gp=$($GP_HOME/bin/gp self path) && "$gp" corpus score ...
```

`gp self path` prints the file behind the link, to hold one build through a long job, and exits 1 when none is published. `just publish` publishes the build of the commit checked out here, for a first setup.
