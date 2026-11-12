# Capture: clips the code chooses

Capturing a game's presentation (`gp sprites capture`, `gp sound capture`) is [capture-presentation.md](capture-presentation.md).

`gp capture dense` and `gp capture carry` came from game-printer's capture runs (`gp serve`'s capture loop). gameprinter has no capture runs until wave 7 of the port brings them back, so for now each reads a game's package folder: its scored ROM (found in `$GP_ROM_DIR` by hash, or `--rom`), `mapping.toml`, and the starts of its clips (its corpus plan, else `clips.toml`), each run on the original once. `crates/gp-cli/src/capture/mod.rs` (`Inputs`) is the one place a capture run will plug in.

## Dense clips

```sh
gp capture dense games/donkey-kong --budget 4000 --seed 1 --into /tmp/dk-dense
```

A few short clips that each run much more of the original's code (docs/research/dense-clips.md). From each start, a coverage-guided search mutates press sequences on gp-explore's loop and keeps one when it reaches a feature no kept one has: a CPU address run, a branch way, a mapped measure's value bucket, an enemy slot's kind and state. Greedy set cover, a clip's price its frames, picks `--keep`, and the first `--write` of its order are written into `--into` (default `$GP_HOME/work/dense/GAME-dense-SEED`) as a clip set: `clips.toml` with `dense` recipes (their presses, start and coverage hash) beside the start presses files, and `dense.json`, the report (what the game's own clips cover, what the search's first 1, 3, 5 ... clips cover). The game read from is unchanged. The same seed, ROM and game give the same clips for any `--jobs`.

`--no-search` leaves out the search. The recording windows (`tas`), `--pick`, `--write-set` and `--repick` read a capture run's recordings and prune, so they are refused until wave 7.

## Carry

```sh
gp capture carry games/donkey-kong [--every 60]
```

What the game's clip starts can't carry into the rebuilt game (`gp_interp::carry`): every loaded object a start can't carry and why, grouped by cause, with the starts each gap alone holds back. `--every N` also samples, through each start's presses from power-on, every Nth polled frame in play with the player on the ground. It needs the model's lifted assets (`--assets`, default `$GP_HOME/work/assets/<model.source>`). `--write` (into a capture run's `starts.json`) waits for wave 7.

## Not here yet

`gp capture steer` (routine, branch, cited, directed and element clips, docs/research/code-steered-clips.md) reads a capture run's recordings, its generated clips and their scores, and its directed method the capture run's starts: it comes with wave 7. Its press search is gp-explore's `drive` with a `Toward`-like plan, which `gp explore --target` already runs.

These read gp-nes's code coverage, so an arcade game is refused in words until gp-arcade has a code map (wave 6).
