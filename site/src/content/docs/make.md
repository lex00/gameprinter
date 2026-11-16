---
title: Make your own game
description: Copy Moss Hop, a small starter game, and change how it plays. Early, and on your own computer for now.
prompt: |
  Read this page. Then help me make a copy of templates/mosshop and change it the way I describe next, keeping gp model check passing. Tell me what gp interp compare says changed.
---

A game you make yourself is all yours, and it needs nobody's ROM. You copy a starter game (a template) and change its rules and numbers, which live in one text file, `model.toml`, the game's model. The result plays in a browser. It is never a new ROM.

**This path is early.** You can change the rules and measure what that does. Your agent can build your copy into a page that plays in a browser on your own computer (the MCP tool `build_publish`), but you can't publish it for other people to play yet.

Start with Moss Hop, a small platformer we wrote for testing the tool. Its source and its model are in the `gameprinter` repo, and you're free to copy it. If you'd rather start from a blank page, `gp game new NAME --empty` writes the smallest game that plays: one stage of floor, a player who walks and jumps, plain shapes for pictures and no sound, as a copy in `$GP_HOME/work/mcp/NAME`.

## See it work

From your `gameprinter` checkout, after [installing `gp`](../install/):

```sh
just mosshop
gp assets lift target/mosshop/mosshop.nes --map assets/mosshop.toml -o /tmp/moss-assets --force
gp model check templates/mosshop --assets /tmp/moss-assets
gp interp compare templates/mosshop --rom target/mosshop/mosshop.nes \
  --baseline baselines/mosshop/coins-and-pit.toml --assets /tmp/moss-assets
```

`just mosshop` assembles Moss Hop's NES source into a ROM file that stays on your machine. We wrote Moss Hop as a real NES program to give the tools an original to check its model against. Your own game doesn't need one; you only change the model. `gp interp compare` plays one recorded run on the original and on the model and tells you how closely they agree. The other recorded runs are in `baselines/mosshop/`.

`gp baseline check baselines/mosshop` replays each recorded run on the original and checks its frame hashes and RAM values. It finds each run's ROM in `$GP_ROM_DIR` by its hash (or takes `--rom`) and skips a run whose ROM isn't there. With `--video` it draws the picture too, and fails if any hash changes.

Without `--map`, `gp assets lift` finds the asset map by the ROM's hash (in `assets/` here, or in a game's `assets/` in the games checkout). Without `-o`, it writes to `~/.gameprinter/work/assets/SOURCE`. `gp assets pack DIR --check ROM` encodes the lifted files again and lists every byte that differs from the ROM, by asset and offset, and writes nothing. After an edit, that list is exactly what you changed. To try an edited table or tile, pass the edited folder as `--assets` to `gp model check` or `gp interp compare`. It runs on the model only, because Game Printer never writes a ROM to try it on the original.

`gp assets stub templates/mosshop --lifted /tmp/moss-assets -o /tmp/moss-stubs` copies the lifted files and swaps every presentation slot (tiles, palettes) for a placeholder of the same shape, licensed CC0-1.0. The slots the simulation reads stay as they are, and the game plays the same with its art missing (`docs/stubs.md`).

To draw sprites, `gp assets open COPY ENTITY` writes an entity's sprite set for a pixel editor, starts Pixelorama if you have it, and brings each save back through the edit checks (`docs/sprite-editing.md`). The layout it writes follows the image config (`docs/formats/image-config.md`; `gp sprites schema` prints its schema). It works on NES games only for now.

## Change it

Ask your agent for a copy of Moss Hop (its `template_fork` tool makes one), or copy `templates/mosshop` somewhere of your own. Then edit the copy's `model.toml`. These numbers are a good place to start:

| Path | Now | What it does |
|---|---|---|
| `entities.player.jump.gravity` | 0.25 | Pull while rising |
| `entities.player.jump.speed` | -5 | Launch speed, with up as negative |
| `entities.player.run.run_speed` | 2 | Speed with B held |
| `entities.slug.patrol.step` | 1 | Pixels a slug moves each step |

Write fractions in quotes, such as `gravity = "0.375"`. Game logic is fixed-point, so a fraction has to be an exact multiple of 1/65536, and a value like 0.4 is refused.

Run `gp model check` on your copy after each edit, and `gp interp compare` to see what changed. If you changed the jump, the recorded runs that jump stop matching the original. That mismatch is what you want to see.

## Run and compare in more ways

`gp interp run` plays the model by itself on an input. It prints the frame count, a digest of every frame's state, and the last frame's state by model path:

```sh
gp interp run templates/mosshop --baseline baselines/mosshop/jump-height.toml --assets /tmp/moss-assets
```

The input can be an input log (`--input FILE`), a baseline's holds (`--baseline`), or a recording in the store (`--recording ID`, as `gp record` makes them). `--profile normal` runs it in a profile from `profiles.toml`, `--level NAME` starts on another level, and `--state FILE` starts from a saved model state instead of power-on. `--hashes`, `--frames` and `--apu-log` write each frame's state hash, the frames played and the hooks' sound writes ([the APU log](https://github.com/lex00/gameprinter/blob/main/docs/formats/apu-log.md)), for another runtime to check against. A model with hooks runs in its game's module (`--module FILE`). `--allow-missing-ext` runs it without one, and then the hooked entities do only what their components say.

`gp interp compare` takes the same inputs. It prints how well each mapped path agrees, then the lockstep's report. With `--mode free` it never copies the original's state back into the model, and you see the drift. `--from FRAME` starts both games at that frame of the original, `--mapping FILE` uses another state mapping, and `--draws FILE` hands the model the original's random draws ([the draws script](https://github.com/lex00/gameprinter/blob/main/docs/formats/draws.md)).

`gp model fmt` rewrites `model.toml` and `fidelity.toml` in their canonical form; with `--check` it only tells you whether they already are. `gp model check` also follows every evidence ref of the form `file:line` and checks that the line still holds its quote. Refs are relative to the git work tree, a game's package folder, or `--root DIR`.

When you write a game's data by hand (`source.how = "authored"`, with an `authored.toml`), `gp template build DIR` builds it. It writes a data image, its asset map and the lifted files into `~/.gameprinter/work/assets/SOURCE`, or wherever `-o` says. The data image is a file in NES formats that the model reads its levels and art from; nothing plays it. The build exits 1 when the asset map or `template.toml`'s pins are stale, and `--write` rewrites them.

A few more template commands (`docs/templates.md` has them in full):

- `gp template extract DIR` makes a copy of a template filled from your own ROM, in `~/.gameprinter/work/templates/ID`. It runs the lift, its round trip and `gp model check` with every slot filled, and records each hash in `extract.toml`.
- `gp template pack DIR -o OUT.zip` runs the asset-free checks, then writes the template and its asset map as a zip that comes out the same bytes every time.
- `gp template font DIR` checks the HUD font. Every character the model writes needs a glyph, and no other art may draw a glyph's tile.
- `gp template check DIR --original` is for a template derived from a game (`[derive] from` in `template.toml`). It compares each stage with the game's and refuses one that shares 0.6 of its cells or 0.5 of its ladders in the same places, or that has a data line of its built files in the game's lifted files. The game's files are lifted in memory from your ROM and never written.

## Derived templates, baselines and the level kit

A template can be a small change on top of another, such as a hack of a game, or a game made here on top of one. It names its base and a delta file under `[derive]` in `template.toml`. `gp template derive DIR` builds its model, contract, state mapping and other per-game files from the base's, renames the base's source to its own, and merges the delta over them. It exits 1 while any of those is stale, and `--write` rewrites them. It only reads files, so an NES game and an arcade romset work the same way.

`gp interp reach DIR --record` records a run through each level as a model baseline in `DIR/baselines/`. `gp interp baselines DIR` plays every baseline there on the model alone, through the game's module (`--module`) or the built-in interpreter, and exits 1 when a digest or an expectation differs. When you meant to change how the game plays, `--write` rewrites the ones that differ.

`gp level kit GAME` writes `level-kit/kit.toml` and `level-kit/level-kit.md`, which tell a designer how to make a stage by hand. It reads them from the model, the asset map and the hand-written `level-kit/rules.toml` ([the level kit](https://github.com/lex00/gameprinter/blob/main/docs/level-kit.md)). `gp game new` writes a stub rules file and the first kit.

`gp chunks GAME` reads the model's levels from their lifted assets and finds the groups of cells they're built from, with what each does for the player. Those include the format's own objects (a Donkey Kong stage's rows and columns of tiles), connected pieces, gaps, and patterns mined from the grid. The catalog holds the game's levels, so it goes to `~/.gameprinter/work/chunks/GAME/chunks.json` and never into a git checkout ([level chunks](https://github.com/lex00/gameprinter/blob/main/docs/chunks.md)). `--module` gives it the game's built module for the reachability tests.

`gp catalog GAME` compares what the model holds with the pattern library ([catalog patterns](https://github.com/lex00/gameprinter/blob/main/docs/catalog-patterns.md)). It lists each entity's patterns as components or hooks (`--module` reads the hooks from the game's module), sorts the asset map by pattern, and checks the RAM slot arrays in the game's labels against the mapping's `[slots]`. With `--recording ID` it also fits a recording's tracked objects to the behaviour patterns.

Moss Hop has three siblings, games we wrote as models with no old game behind them: Badge In (`templates/badge-in`, badge in before the standup), Signal Failure (`templates/signal-failure`, make the train) and Swing State (`templates/swing-state`, a campaign trail). Each one is built from its `authored.toml` with `gp template build`, and its runs are model baselines you can replay with `gp interp baselines templates/badge-in`. `just originals` builds and checks all three (`docs/originals.md`).

## What has to be yours

Your game can't use another game's art, music, levels, characters or name. Copying how something moves is a much greyer area than it sounds. This site isn't legal advice; talk to a lawyer before you put up a game built on someone else's.

## Not built yet

A catalogue of game mechanics to build from, and a link to your game that people on other computers can open without your help.

The last step of making a game, Play and share, does make a share link today. It puts the game in a registry of your own on your computer, with a page that plays it by name and asks for no ROM. Nobody else can open it until you put that page and your registry online together. The public page lists a game only when its keepers add it.
