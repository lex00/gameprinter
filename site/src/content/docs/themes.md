---
title: Themes and levels
description: Add a new look or a new stage to a game. Early, with no preview yet.
prompt: |
  Read this page. Then help me make a theme in its own folder, using the folder paper/ in lex00/games as the pattern, in the style I describe next. Use no art, music or names from any existing game, and run gp package scan on the result.
---

**This path is early.** You work by copying an existing package and editing text files. The theme kit, the automatic checks and the in-game preview from the old tool haven't been brought over.

## Make a theme

A theme is a small package of text. The one to copy is [`paper/`](https://github.com/lex00/games/tree/main/paper) in the `lex00/games` repo, the Paper look for Rivet Rodeo.

| File | What it holds |
|---|---|
| `package.toml` | The theme's name, licence and the game it's for |
| `theme/theme.toml` | Its title, licence and colour changes |
| `theme/*.pix` | Art, as text: a size, a palette, then a grid of characters |
| `theme/*.tune` | Sounds and music, as text: voices and notes |
| `theme/poses.toml` | Which pose each group of frames is drawn in |

Each `.pix` file has to keep the size and the tile layout of the one it replaces, because the game finds its art by position.

Your work has to be your own, with an open licence stated in the package. If you used an AI tool to make any of it, name the tool, and make sure its terms let you share the result.

Before you share a theme, check it holds nothing from a ROM:

```sh
gp package scan my-theme
```

## Get it in the game

Open a pull request on [`lex00/games`](https://github.com/lex00/games) with your theme in a folder of its own. If it's accepted, it's published from there, and players can open the game with it from a link.

You can also [publish it to a registry of your own](../registry/), but the game page only reads its one registry, so players wouldn't see it there.

## Make a level

A game's level kit says what a stage is made of and what limits it has to keep. Donkey Kong's is `donkey-kong/level-kit/level-kit.md` in `lex00/games`. Stages live in the game's model.

`gp level measure` reports a stage's girders, ladders and routes on the model, once you've made a working copy as described in [Improve a game](../improve/):

```sh
gp level measure --template ~/my-donkey-kong --level main \
  --module ~/my-donkey-kong/play/module.wasm \
  --assets ~/my-donkey-kong/lifted/donkey-kong-oe
```

A new stage should be your own layout, not a copy of an original's. The module in version 1.0.5 can only measure the sloped stage, `main`. The flat stages need a module built with a newer `gp`.

## Not built yet

A preview of an unpublished theme or level in the game, the theme kit and its checks, and level packages that a player can pick.
