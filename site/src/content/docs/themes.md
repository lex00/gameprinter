---
title: Themes and levels
description: Give a game new pictures, in a repo of your own, or make a new stage.
prompt: |
  Read this page. Then help me make a large-art theme for Donkey Kong in its own repo, using lex00/turnips as the pattern, in the style I describe next. Start it with gp theme look new, use no art, music or names from any existing game, and run gp theme look check on it after every change until it reports no problems, before I publish it.
---

A theme is new pictures and sounds for a game. It changes how the game looks and nothing about how it plays: the game's model, the text file of its rules ([what a model and a template are](../)), stays as it was, so every body keeps its place and its hit box. A player picks a theme under Settings, Theme, and can go back to the original art at any time. On a ported game such as Donkey Kong the levels and the rules' tables still come from the player's own ROM, so the game with your theme plays in a browser only for someone who has that ROM.

## A theme with large art

The way to make a theme today is a large-art theme: pictures drawn bigger than the game's own, at the place and size of the picture each one stands for, and a rules file that says how to draw them. [`lex00/turnips`](https://github.com/lex00/turnips) is one, a clay theme for Donkey Kong, and the one to copy.

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

### Start it

```sh
gp theme look new my-theme --game donkey-kong --name YOUR-NAME/my-theme --title "My Theme"
```

writes an empty theme in the Turnips shape: `package.toml`, `theme/look.txt`, a README, and a `theme/art.txt` with one line for each picture the game's theme kit lists, marked TO DRAW with its size, its boxes and its feet row. It writes nothing from a ROM.

### Check it

```sh
gp theme look check my-theme --game donkey-kong
```

reads the theme with the page's own rules reader and lists what is wrong by file and line:

- Files: `package.toml` (name, version, `kind = "content"`, title, licence), `look.txt` and `art.txt` as the page reads them, every picture a rule names there and loadable, pictures no rule uses.
- What the game needs drawn: each pose of each slot in the game's theme kit that has no picture yet, by its label and pose words, with the game's own picture name to put in its `pic` line. A rule that names a picture the game hasn't got is a problem, since the page passes over it.
- Fit: each pose against its body's touch box and feet row, beside the game's own picture, in game pixels. It is a first pass from the files alone; play it and look.
- Names: the original's names (the kit's `deny` list) in the package and the look, where players see them.
- ROM data: the same check as `gp template served`: no file may hold bytes from the ROM.

It needs your ROM on this machine (in `$GP_ROM_DIR`, or `--rom FILE`) to name the game's pictures and to compare your files with the ROM, and says so when it hasn't got one. It exits 0 when there are no problems; warnings are worth a look. Its last line says what to do next.

No picture may be traced from the original's art, and your work has to be your own. If an AI tool made any of it, name the tool in your README and make sure its terms let you share the result.

### See it in the game

The check's output ends with the commands that play your theme on this machine, from a registry in a folder and a page built to install from it. Run them in the gameprinter checkout.

```sh
scripts/web/local-registry.sh ~/games my-theme
target/debug/gp registry serve "$GP_HOME/registry" --page target/page-local
```

then open `http://127.0.0.1:8377/p?g=lex00/donkey-kong&c=YOUR-NAME/my-theme` and play every stage, looking at each pose at game size.

## Get it in the game

Keep your theme in a repo of your own, with its pictures in Git LFS. Players get themes from the registry the page reads, [`lex00/gp-registry`](https://github.com/lex00/gp-registry): open an issue there with your repo's link. Once it's published for the game (`gp registry publish ... --for lex00/donkey-kong`), the page at [nes-donkey-kong.intentius.workers.dev](https://nes-donkey-kong.intentius.workers.dev) installs it with the game and offers it under Settings, Theme, beside Original (the ROM's own art) and Turnips. Nothing on the page changes for that: the page asks the registry what it offers for the game each time it loads.

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

`gp level measure` reports a stage's girders, ladders and routes on the model, once you've made a copy of the game's folder as described in [Improve a game](../improve/):

```sh
gp level measure --template ~/my-donkey-kong --level main \
  --module ~/my-donkey-kong/play/module.wasm \
  --assets ~/my-donkey-kong/lifted/donkey-kong-oe
```

A new stage should be your own layout, not a copy of the original game's. The `module.wasm` of Donkey Kong 1.0.5 (its custom code, compiled) can only measure the sloped stage, `main`; the flat stages need one built with a newer `gp`.

## Not built yet

A large-art theme (like Turnips) in the THEMES tab's clip review, a way to put a theme in the page's registry yourself (today you open an issue there; `gp registry publish` works only on a registry you run), and new levels a player can pick.
