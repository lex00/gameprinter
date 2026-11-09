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

`just mosshop` assembles the game into a ROM. `gp interp compare` plays one recorded run on the original and on the model, and says how closely they agree. The other recorded runs are in `baselines/mosshop/`.

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

## What has to be yours

An original game can't use another game's art, music, levels, characters or name. Copying how something moves is a much greyer area than it sounds. This site isn't legal advice, so talk to a lawyer before you serve a game built on someone else's.

## Not built yet

A way to start a new game from nothing, a catalogue of game mechanics to build from, playing your own game in the browser, and publishing it where others can play.
