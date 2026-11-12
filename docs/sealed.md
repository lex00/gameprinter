# Sealed runs

A sealed run measures the model on play no worker has seen, so no worker's tool reads one. Sealed runs are kept under `$GP_HOME/sealed/GAME/`: each `.log` there, with the `.toml` beside it saying where it starts (its stored path and start frame), and the clips the old mechanic page sealed under `sealed-run/clips/`. The code is `crates/gp-cli/src/sealed.rs`, ported from game-printer's sealed.rs and the "Sealed runs" part of its docs/contributing-mechanics.md.

gameprinter's mechanic page doesn't seal a run yet; a sealed run is put under `$GP_HOME/sealed/GAME/` by hand, or comes from the old home.

## The guard

The refusal is one guard that every reader passes through (`gp_evidence::guard`, installed by `gp` at start, `sealed::Guard`):

| Where | What it refuses |
|---|---|
| The store's recording lookup (`gp_evidence::recordings::load_manifest`) | A stored recording whose input log from power-on plays a sealed run |
| The clip set reader (`clipset::read` and `read_game`, a plan or an older `clips.toml`) | Leaves out a clip whose presses from power-on and its own presses play a sealed run, and says how many on stderr: `gp corpus score`, `gp game score`, `gp clip score` and `log`, `gp model gate` and every other reader of a game's clips |
| `gp corpus import` | Leaves out a session named `sealed*`, a clip that is a sealed run's, and a clip that plays one, beside the guard |

Every refusal has the same message: `refused: WHAT is sealed (WHY). Sealed runs are scored only by the coordinator, with gp sealed score`.

A play plays a sealed run when it is of the same game, the two are the same input from power-on into the sealed run's own frames, and the shared part of those frames holds a press. So a copy of a sealed run in a mechanic's session, a clip cut from inside it, or a log from power-on that runs through it is refused. A clip from the same play that starts after the sealed run ends is kept, and so are a play that shares only the wait before it and a clip that shares only frames with nothing pressed, such as a title screen's. Another game's sealed runs never block a play (`sealed::applies`):

| Both sides | Same game when |
|---|---|
| Listed in a game's `roms.toml` in the games checkout | The source IDs are the same |
| Either one unlisted, both ROMs known | The headerless ROM hashes are the same |
| Neither ROM known | The names are the same (a game's or the sealed folder's) |
| Nothing tells them apart | Always: a play that names no game and no ROM is held to every sealed run, and so is any play against a sealed run of an unlisted game with no ROM |

A sealed run's game is its folder under `$GP_HOME/sealed`, and its ROM is the `rom_sha256` in the `.toml` beside it, else every `.nes` in the nearest folder that holds one, from the run's own up to its game's folder.

With nothing under `$GP_HOME/sealed`, the guard reads nothing and refuses nothing.

## The coordinator's two commands

Two readers see what the guard refuses, and both are the coordinator's. A worker never runs them (docs/workers.md).

`gp sealed leaks GAME` lists every stored recording, mechanic-session clip and corpus clip of the game that plays one of its sealed runs. It reads `$GP_HOME/sealed` itself but prints only each leak's kind, its ID (a recording's first 12 hex digits, a session clip's `SESSION:N`, a corpus clip's ID) and the name of the sealed run it matches, never frames or presses. The corpus is the game's package in the games checkout: GAME is its folder's name or a source ID its `roms.toml` lists (`--games` names the checkout).

`gp sealed score GAME RUN` scores a sealed run as `gp clip score` scores a clip, straight through and with resync. RUN is N for clip N the page sealed (`sealed-run/clips/NNN`), `SESSION:N` for a clip of another folder there, or a log's name. The model is the game's own, or `--model DIR`; a model with hooks runs in its module (`--module`), with its lifted assets (`--assets`, default `$GP_HOME/work/assets/SOURCE`), and the ROM is the one the package names, or `--rom`. It writes the files under `$GP_HOME/sealed/GAME/scores/RUN/` and adds a line to `$GP_HOME/sealed/GAME/ledger.txt`: the time, the run, the model's hash, the frames together straight through and with resync, and the longest stretch. Nothing goes to `$GP_HOME/work` or the start cache.

```text
2026-11-12T05:22:13.402118Z 1 model d7b5b84c0aba straight 100.0% resync 100.0% longest 48.0 s
```
