---
title: Themes and levels
description: Give a game a new look, in a repo of your own, or make a new stage.
prompt: |
  Read this page. Then help me make a large-art theme for Donkey Kong in its own repo, using lex00/turnips as the pattern, in the style I describe next. Use no art, music or names from any existing game, and check the result with gp template served before I publish it.
---

A theme changes how a game looks and nothing about how it plays. Every body keeps its place and its hit box. A player picks a theme under Settings, Theme, and can go back to the original's art at any time.

## A theme with large art

The way to make a theme today is a finished look: pictures drawn bigger than the game's own, at the place and size of the picture each one stands for, and a rules file that says how to draw them. [`lex00/turnips`](https://github.com/lex00/turnips) is one, a clay look for Donkey Kong, and the one to copy.

| File | What it holds |
|---|---|
| `package.toml` | The theme's name, version and title |
| `theme/look.txt` | `gp-look 1`, the theme's id and title, and `art art.txt` |
| `theme/art.txt` | The rules, `gp-art 1`, one per line |
| `theme/art/` | The pictures |

The game reads the rules in Rust (`gp_present::art` in gameprinter), so the web page and the native player draw the same frame. The page only loads the pictures and paints what the game returns.

### The rules

Pictures are named by the build's own file names (`frame-82-0.png`, `hud-0.png`), levels by their level picture's name (`level-main.png`), and art by its path in `theme/`. Sizes and places are in the game's pixels.

| Line | What it does |
|---|---|
| `scale 6` | Canvas pixels to a game pixel, so the art keeps its detail |
| `limit off` | Draws without the console's sprite limit |
| `pic BUILD ART W H DX DY` | Draws `ART` in place of the build's picture, `W` by `H`, moved by `DX`, `DY` |
| `timed BUILD FROM TO ART W H DX DY` | Another picture while a body has shown `BUILD` for `FROM` to `TO` frames. Add `every=N` for a repeat, `after=PIC`, `wait=N` or `hide` |
| `held BUILD...` | Not drawn while a `hide` pose holds it, such as a ball in a hand |
| `face BUILD...` | Turned the way the body really moves, after `face_turn N` pixels (6 by default) |
| `near ART W H DX DY`, `near_when`, `near_other` | A pose shown while another body stands level with this one |
| `elsewhere LEVEL DX DY BUILD...` | Moves those pictures on every level but one |
| `prop LEVEL ART X Y W H`, `prop_over`, `prop_burst` | A picture in front of the bodies that pass behind it, with a burst where one lands |
| `drift LEVEL Y RISE EVERY`, `drift_art`, `drift_at` | Pictures that rise and fade, like smoke |
| `stage SCALE R G B R G B` | Stages built from pieces over a gradient backdrop |

A stage is never a picture in the theme. The `stage`, `stage_piece` and `level` lines say how to build each one from pieces (a plank, a ladder rail, a rope, a wheel) over the level picture the player's own ROM gives, finding floors, ladders and ropes by colour. So a theme holds no layout from the original.

Lettering is part of a theme too: draw your own letters and use `pic` lines for the HUD, the score pop-ups and the title screens.

### Check it holds nothing from a ROM

No file may hold bytes from the ROM, and no picture may be traced from the original's art. Check every file against the ROMs the game names:

```sh
gp template served ~/games/donkey-kong --folder my-theme/theme
```

Your work has to be your own. If you used an AI tool to make any of it, name the tool in your README, and make sure its terms let you share the result.

## Get it in the game

Keep your theme in a repo of your own, with its pictures in Git LFS. Players get themes from the registry the page reads, [`lex00/gp-registry`](https://github.com/lex00/gp-registry): open an issue there with your repo's link. Once it's published for the game (`gp registry publish ... --for lex00/donkey-kong`), the page at [nes-donkey-kong.intentius.workers.dev](https://nes-donkey-kong.intentius.workers.dev) installs it with the game and offers it under Settings, Theme, beside Original and Turnips. Nothing on the page changes for that: the page asks the registry what it offers for the game each time it loads.

The page opens in its own theme, Turnips. A link with `&c=YOUR/THEME` opens it in yours instead.

You can also [publish it to a registry of your own](../registry/), but the game page only reads its one registry, so players wouldn't see it there.

## A theme from the game's theme kit

The other way to make a theme is the one game-printer had: a theme folder (`theme.toml` with pictures as `.pix` files and tunes as `.tune` files) fitted to the game's own sprite slots, at the game's size. Every game has a theme kit, `themes/kit/` in its folder: each art slot's role, size and frames, and each sound role's cue, with no names or pictures from the original. The commands:

| Command | What it does |
|---|---|
| `gp theme kit GAME --rom ROM -o DIR` | Writes the kit, and from your ROM its local part (the tiles each frame is cut into) under `~/.gameprinter/work/theme-kits/` |
| `gp theme style list` | The style cards: how a theme looks, apart from any game |
| `gp theme sheet GAME --style CARD -o DIR` | Starting grids and prompts for an image model |
| `gp theme generate GAME --style CARD --dry-run` | The image model calls a theme would take and their cost. A real run pays Runway, so it needs `--yes` and your own key |
| `gp theme fit GAME --sheets DIR -o THEME` | Fits filled grids (or `--art DIR`) to the game's slots |
| `gp theme paint GAME -o THEME` | A paper theme, painted in code |
| `gp theme check GAME --theme THEME --original` | The readability check, the style and geometry checks, and the originality check against your own ROM |
| `gp theme video GAME --input LOG --theme THEME -o FILE.png` | The model played on an input, drawn with the theme |
| `gp theme publish THEME --game GAME --dry-run` | Every check and the package scan, and what would be published |

`GAME` is the game's folder (`~/games/donkey-kong`) or its name in your games checkout. The page's **THEMES** tab does all of this for the game it's on: it shows the kit, lists the game's themes and any folder you link, starts the commands as background work (never a paid call: generate and publish are dry runs there), and shows the checks. Its clip review plays a clip of the game with the theme on beside the same clip in the original's art, frame for frame. Click the theme's side to mark what's wrong; a refinement round fits the theme again from its art and checks it, and says what became of each mark. The docs in the repo are `docs/theme-kit.md` and `docs/theme-publishing.md`.

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

A large-art look (like Turnips) in the THEMES tab's clip review, a way to send a theme to the registry from the command line, and level packages a player can pick.
