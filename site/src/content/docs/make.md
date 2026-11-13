---
title: Make your own game
description: Start from Moss Hop, the small test game, and change it. Early, and on your machine only.
prompt: |
  Read this page. Then help me make a copy of templates/mosshop and change it the way I describe next, keeping gp model check passing. Tell me what gp interp compare says changed.
---

An original game needs no ROM from anyone else: every part of it is yours. **This path is early.** You can change a game's rules and measure the effect, but there's no way yet to play your game in a browser or publish it for others to play.

The place to start is Moss Hop, a small platformer written for testing the tool. Its source and its model are both in the `gameprinter` repo, and it's yours to copy.

## See it work

From your `gameprinter` checkout, after [installing `gp`](../install/):

```sh
just mosshop
gp assets lift target/mosshop/mosshop.nes --map assets/mosshop.toml -o /tmp/moss-assets --force
gp model check templates/mosshop --assets /tmp/moss-assets
gp interp compare templates/mosshop --rom target/mosshop/mosshop.nes \
  --baseline baselines/mosshop/coins-and-pit.toml --assets /tmp/moss-assets
```

`just mosshop` assembles the game into a ROM. `gp interp compare` plays one recorded run on the original and on the model, and says how closely they agree. The other recorded runs are in `baselines/mosshop/`. `gp baseline check baselines/mosshop` replays each recorded run on the original and checks its frame hashes and RAM values; it finds each run's ROM in `$GP_ROM_DIR` by its hash (or takes `--rom`), skips a run whose ROM isn't there, and with `--video` draws the picture too and fails if any hash changes.

`gp assets lift` finds the asset map by the ROM's hash (in `assets/` here, or a game's `assets/` in the games checkout) when `--map` isn't given, and writes to `~/.gameprinter/work/assets/SOURCE` when `-o` isn't. `gp assets pack DIR --base ROM -o FILE` writes a copy of the ROM with the lifted files packed back in, and lists the bytes that changed: the way an edited table or tile gets back into a ROM to try on the original.

## Change it

Copy `templates/mosshop` somewhere of your own and edit `model.toml`. These are good first numbers to change:

| Path | Now | What it does |
|---|---|---|
| `entities.player.jump.gravity` | 0.25 | Pull while rising |
| `entities.player.jump.speed` | -5 | Launch speed, with up as negative |
| `entities.player.run.run_speed` | 2 | Speed with B held |
| `entities.slug.patrol.step` | 1 | Pixels a slug moves each step |

Fractions are written as quoted text, such as `gravity = "0.375"`, and have to be exact multiples of 1/65536, because game logic is fixed-point. A value like 0.4 is refused.

Run `gp model check` on your copy after each edit. Run `gp interp compare` on it to see the effect: a changed jump makes the runs that jump stop matching, which is the measurement you're after.

## Run and compare in more ways

`gp interp run` plays the model alone on an input and prints the frame count, a digest of every frame's state and the state at the last frame by model path:

```sh
gp interp run templates/mosshop --baseline baselines/mosshop/jump-height.toml --assets /tmp/moss-assets
```

The input is an input log (`--input FILE`), a baseline's holds (`--baseline`), or a recording in the store (`--recording ID`, as `gp record` makes one). `--profile normal` runs it in a profile from `profiles.toml`, `--level NAME` starts at another level, and `--state FILE` starts from a saved model state instead of power-on. `--hashes`, `--frames` and `--apu-log` write each frame's state hash, the frames played and the hooks' sound writes ([the APU log](https://github.com/lex00/gameprinter/blob/main/docs/formats/apu-log.md)) for another runtime to check against. A model with hooks runs in its game's module (`--module FILE`); `--allow-missing-ext` runs it without one, its hooked entities doing only what their components say.

`gp interp compare` takes the same inputs, and prints each mapped path's agreement and then the lockstep's report. `--mode free` never puts the original's state back into the model, so the drift shows; `--from FRAME` starts both games at that frame of the original; `--mapping FILE` uses another state mapping; `--draws FILE` gives the model the original's random draws ([the draws script](https://github.com/lex00/gameprinter/blob/main/docs/formats/draws.md)).

`gp model fmt` rewrites `model.toml` and `fidelity.toml` in their one canonical form, and `--check` only says whether they are. `gp model check` also reads every evidence ref of the form `file:line` and checks that the line holds its quote; refs are relative to the git work tree, or to a game's package folder, or to `--root DIR`.

An original whose data is written by hand (`source.how = "authored"`, an `authored.toml`) is built with `gp template build DIR`: a data image, its asset map and the lifted files, into `~/.gameprinter/work/assets/SOURCE` unless `-o` says otherwise. It exits 1 when the asset map or `template.toml`'s pins are stale; `--write` rewrites them. For a template derived from a game (`[derive] from` in `template.toml`), `gp template check DIR --original` holds each of its stages to the game's: the share of cells and of ladders in the same places, refused at 0.6 and 0.5, and any data line of its built files that the game's lifted files hold. The game's files are lifted in memory from your ROM and never written.

## Derived templates, baselines and the level kit

A template made as a small delta on another (a hack of a game, or an original built on one) names its base and a delta file under `[derive]` in `template.toml`. `gp template derive DIR` makes its model, contract, state mapping and the other per-game files from the base's, with the base's source renamed to its own and the delta merged over them, and exits 1 while one is stale; `--write` rewrites them. It reads files only, so it works the same for an NES game and an arcade romset.

`gp interp reach DIR --record` writes a run through each level as a model baseline in `DIR/baselines/`. `gp interp baselines DIR` plays every baseline there on the model alone, through the game's module (`--module`) or the built-in interpreter, and exits 1 when a digest or an expectation differs; `--write` rewrites the ones that differ after a change meant to change play.

`gp level kit GAME` writes `level-kit/kit.toml` and `level-kit/level-kit.md`: what a designer needs to make a stage by hand, from the model, the asset map and the hand-written `level-kit/rules.toml` ([the level kit](https://github.com/lex00/gameprinter/blob/main/docs/level-kit.md)). `gp game new` writes a stub rules file and the first kit.

`gp chunks GAME` reads the model's levels from their lifted assets and finds the groups of cells they are built from: the format's own objects (a Donkey Kong stage's rows and columns of tiles), connected pieces, gaps, and patterns mined from the grid, each with what it does for the player. Its catalog goes to `~/.gameprinter/work/chunks/GAME/chunks.json`, never into a git checkout, since it holds the game's levels ([level chunks](https://github.com/lex00/gameprinter/blob/main/docs/chunks.md)). `--module` gives the game's built module for the reachability tests.

`gp catalog GAME` sets what the model holds against the pattern library ([catalog patterns](https://github.com/lex00/gameprinter/blob/main/docs/catalog-patterns.md)): each entity's patterns, as components or hooks (`--module` reads the hooks from the game's module), the asset map sorted by pattern, RAM slot arrays from the game's labels against the mapping's `[slots]`, and with `--recording ID` the tracked objects of a recording fitted to the behaviour patterns.

## What has to be yours

An original game can't use another game's art, music, levels, characters or name. Copying how something moves is a much greyer area than it sounds. This site isn't legal advice, so talk to a lawyer before you serve a game built on someone else's.

## Not built yet

A way to start a new game from nothing, a catalogue of game mechanics to build from, playing your own game in the browser, and publishing it where others can play.
