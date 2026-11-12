# The farms: a decision model plays by search

`gp farm` and `gp farm-topdown` play a game by search from savestates. The emulator is deterministic and saves its state for free, so the search is Go-Explore: an archive of cells (where the player is, whether it stands, how fast it goes), each with a savestate and the input log that reached it. Each round picks a cell near the frontier (less visited ones more often), asks the decision model which move to try first, simulates its most likely moves, and keeps any that reach a new cell or an old one sooner. A death or a stall is dropped. The model only orders the moves; the emulator says what works.

The decision model is Laya, served on 127.0.0.1:7720 by `just laya` (the server `gp mechanic demo` uses). `--decide uniform` (farm) or `--uniform` (farm-topdown) orders the moves at random from `--seed` instead, as the baseline, and needs no server.

The commands came from game-printer (farm.rs, blind.rs, blind_topdown.rs) with their flags and outputs. Its research write-ups (docs/research/laya-level-play.md, topdown-extension.md there) are about other games and stay there. Both read the NES's RAM and sprite table, so an arcade game is refused in words.

## On a game's package

```sh
gp farm donkey-kong --level main --decide uniform --rounds 2000
gp farm donkey-kong --level main --check 3b27      # a recording's furthest x in the level, held out
gp farm donkey-kong --level main --save            # the winning path as a clip in session farm-main
```

The game's package (in `--games`, `$GP_GAMES_DIR`, or a `games` folder beside the current one) gives the ROM (roms.toml, found in `$GP_ROM_DIR`), the player's x and y bytes (the first `ram` byte of `entities.player[0].x` and `.y` in mapping.toml), the level key (observe.toml `[level]`: its RAM bytes, the `when` condition and the names) and, if mapping.toml has one, the `[cells]` grid. Jump hold lengths come from the person's clips in `$GP_HOME/mechanics/GAME`. Play starts where stage 0's start runs first move the player; the level named there must be `--level`. Results go to `$GP_HOME/work/farm/GAME/LEVEL` (log.ndjson, result.json; `-uniform` beside them for the baseline).

game-printer read the player's bytes, with an x high byte, from the game's ROM dataset (`gp dataset rom`, not in gameprinter yet). A mapping field names no high byte, so progress is the low byte alone, which is enough on a single screen and not on a scrolling level. Game mode in game-printer ran on Super Mario Bros. only; its plan and question to the model named that game's hero and now say "the player". On 2026-11-12 none of stage 0's start runs reached play on Donkey Kong, so `gp farm donkey-kong` stops there with "no start run reaches play", as the old binary does.

## Blind

```sh
gp farm --blind ~/roms/nes/mosshop.nes --decide uniform --rounds 1500
gp farm --blind ROM --probe probes.json      # search for each goal in turn
gp farm --blind ROM --session mosshop/blind  # show the search in a mechanic session
gp farm bench                                # the train games in bench/farm-blind.toml
gp farm bench --ledger "what changed"        # and a row in docs/research/farm-ledger.md
```

Blind mode knows nothing of the game: play start from stage 0's start runs (or a person's path, `GP_BLIND_START_PATH` and `GP_BLIND_START_FRAME`), the player's x as the byte Right raises and Left lowers, y and the jump button as the byte a button sends up and back, scenes from the RAM that holds still, death as the pad no longer moving the player, and a level end as a jump to another scene. Each run writes `$GP_HOME/work/farm/blind/NAME/<ms>/` (log.ndjson, result.json, findings.json, path.log) and points `latest` at it; findings.json names each learned byte with the rule that found it and a confidence. `--session GAME/MECHANIC` writes the queue row, the busy line and the best run so far as a clip (the old tool also measured that clip with `gp mechanic contacts`, which gameprinter hasn't got yet).

The bench keeps Moss Hop and Donkey Kong (game-printer's also named other games). A ROM the bench lists as held out runs only under `gp farm bench --held-out`.

## Top-down

```sh
gp farm-topdown ROM --uniform --seconds 300
```

For a top-down game: it plays from power-on through a title and a class select, finds the player's x and y on both axes from the d-pad and their high bytes, finds a health byte that drains while the player stands, and searches with eight-way moves for area (new 16 px cells, in each maze). The search runs for `--seconds` of wall time, so its counts depend on the machine. It writes result.json, path.log and pictures to `--out` (default `$GP_HOME/work/farm/topdown/NAME/<ms>`).

## Compared with game-printer

On 2026-11-12, old binary against new on the same ROMs: `gp farm --blind` on Moss Hop with `--decide uniform --rounds 80` gave the same result.json and findings.json (but for run folders and seconds); on Donkey Kong (JU) both stop at play start. `gp farm-topdown --uniform --seconds 20` on Moss Hop and Donkey Kong learned the same bytes (x, y, high bytes, health candidates, class menu); the search's tallies differ by the rounds each build fitted into 20 seconds.
