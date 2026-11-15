---
title: Themes and levels
description: Give a game new pictures, in a repo of your own, or make a new stage.
prompt: |
  Read this page. Then help me make a large-art theme for Donkey Kong in its own repo, using lex00/turnips as the pattern, in the style I describe next. Start it with gp theme look new, use no art, music or names from any existing game, and run gp theme look check on it after every change until it reports no problems, before I publish it.
---

A theme is new pictures and sounds for a game. It changes how the game looks and nothing about how it plays: the game's model, the text file of its rules ([what a model and a template are](../)), stays as it was, so every body keeps its place and its hit box. A player picks a theme under Settings, Theme, and can go back to the original art at any time. On a ported game such as Donkey Kong the levels and the rules' tables still come from the player's own ROM, so the game with your theme plays in a browser only for someone who has that ROM.

When your theme is ready, you host it yourself: [Host your theme](../host-your-own/) takes you from your theme folder to a registry of your own and a link that plays your theme on the public page. Your AI agent can walk you through it.

## A theme with large art

The way to make a theme today is a large-art theme: pictures drawn bigger than the game's own, at the place and size of the picture each one stands for, and a rules file that says how to draw them. [`lex00/turnips`](https://github.com/lex00/turnips) is one, a clay theme for Donkey Kong, and the one to copy.

| File | What it holds |
|---|---|
| `package.toml` | The theme's name, version and title |
| `theme/look.txt` | `gp-look 1`, the theme's id and title, and `art art.txt` |
| `theme/art.txt` | The rules, `gp-art 1`, one per line |
| `theme/art/` | The pictures |
| `theme/sound/roles/` | A sound for each of the game's sound roles, if you give the theme sound (below) |
| `theme/sound/takes/` | More takes of a role's sound, played in turn |

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
| `#+ hud_over` | The HUD's text is drawn over the bodies, so a body standing on a HUD row doesn't cover it. Without the line it is drawn under them, as before |
| `#+ hud_over ROW...` | The same for those rows of the HUD only, 8 pixels each from the screen's top (row 2 is y 16 to 24) |
| `#+ in REGION pic ...`, `#+ in REGION timed ...` | A `pic` or `timed` line that holds only while the body stands in a region of the stage the game's level kit names, such as a shorter picture on a low ledge. Moss Hop's kit names one, `start`, where the hero starts; Donkey Kong's and Tapper's name none yet |

A stage is never a picture in the theme. The `stage`, `stage_piece` and `level` lines say how to build each one from pieces (a plank, a ladder rail, a rope, a wheel) over the level picture the player's own ROM gives, finding floors, ladders and ropes by colour. So a theme holds no layout from the original.

The lines that start with `#+` are newer than the rest. A page built before them takes them for notes and still offers your theme, drawn without them: the HUD's text under the bodies, and each body's full picture everywhere. Any other line a page doesn't know makes it leave the whole theme out, so use only the lines on this page. A game whose HUD is drawn as sprites (Moss Hop's) has its text over the bodies already, and `gp theme look check` says so.

Lettering is part of a theme too: draw your own letters and use `pic` lines for the HUD, the score pop-ups and the title screens.

### Sound

A game names its sounds by role: Donkey Kong has `jump`, `walk`, `smash`, `stage_music` and ten more, each with a line in its theme kit (`themes/kit/kit.toml`) saying when it plays. Give a role a sound of your own as `theme/sound/roles/ROLE.wav`, a 16-bit PCM WAV, and list it in `theme/look.txt`:

```text
gp-look 1
id my-theme
title My Theme
art art.txt
file sound/roles/jump.wav
file sound/takes/jump.2.wav
file sound/takes/jump.3.wav
```

Once your theme gives any role a sound, the game's own sound is silent, so give every role one. A sound that plays often, such as a jump or a step, gets tiring when it is the same every time. You can give it more takes in `theme/sound/takes/` (`jump.2.wav`, `jump.3.wav` and on), each listed in `look.txt`. Each time the role plays, the game plays its next take, and after the last the first again. Which take comes next is counted from the start of the game, so a recording played back sounds the same as when it was played.

A page from before takes (built before 2026-11-14) plays your theme too, with `jump.wav` every time: it never reads `theme/sound/takes/`. That is why the takes have their own folder, and why every role needs its file in `theme/sound/roles/` even when it has takes. Don't put a take in `sound/roles/` as `jump.2.wav`: an older page takes that for a role named `jump.2`, and stops loading your sounds there.

Make every sound yourself, or with a tool whose terms let you share what it makes. Nothing may be recorded or copied from the game.

### An arcade game

An arcade game such as Tapper is drawn by its board, and its theme uses the same lines with the board's names in place of the build's file names:

| Picture | Name | Size |
|---|---|---|
| A screen | its slot in the game's model, such as `screen_saloon` | 512 by 480 |
| A sprite | `sprite-ENTITY-CODE`, the entity and the picture code it shows in hex, such as `sprite-mug-4d` | 32 by 32 |
| A counter's digit | `counter-NAME-D`, such as `counter-score-7` | 16 by 16 |
| A repeated counter tile | `counter-NAME`, such as `counter-lives` | 16 by 16 |

Sizes and places are the board's picture's pixels, 512 by 480; `scale 2` draws the art at twice that. Which codes each entity shows comes from your own romset, never from the theme or the game's package:

```sh
gp theme look sheet --game tapper
```

plays the game's recorded inputs on your romset. It writes `names.toml` with each name the game showed, and a contact sheet of the codes and screens it saw, under `$GP_HOME/work/theme-kits/tapper/board`; `--out DIR` picks another folder, though never one inside a repo. They are pictures of your ROM's game: keep them to yourself. `gp theme look new` then lists those codes in `art.txt`, and `gp theme look check` refuses a name the game hasn't got and warns of a code it never showed.

### Start it

```sh
gp theme look new my-theme --game donkey-kong --name YOUR-NAME/my-theme --title "My Theme"
```

writes an empty theme in the Turnips shape: `package.toml`, `theme/look.txt`, a README, and a `theme/art.txt` with one line for each picture the game's theme kit lists, marked TO DRAW with its size, its boxes and its feet row. It writes nothing from a ROM.

The name has to be one a registry takes. Write it in lowercase with `-` between words. The part after the slash starts with a letter and is the look's id (`my-theme`). Any other name is refused with the nearest one that works.

### Check it

```sh
gp theme look check my-theme --game donkey-kong
```

reads the theme with the page's own rules reader and lists what is wrong by file and line:

- Files: `package.toml` (name, version, `kind = "content"`, title, licence), `look.txt` and `art.txt` as the page reads them, every picture a rule names there and loadable, pictures no rule uses.
- The `#+` lines: one the page can't read (it passes over it), a region the game's level kit doesn't name, and `hud_over` in a game with no HUD text under its bodies.
- What the game needs drawn: each pose of each slot in the game's theme kit that has no picture yet, by its label and pose words, with the game's own picture name to put in its `pic` line. A rule that names a picture the game hasn't got is a problem, since the page passes over it.
- Fit: each pose against its body's touch box and feet row, beside the game's own picture, in game pixels. It is a first pass from the files alone; play it and look.
- Names: the original's names (the kit's `deny` list) in the package and the look, where players see them.
- Sound: each role's files, in the order the game plays them. A problem: a file that isn't a 16-bit PCM WAV, is missing, or names a role the game hasn't got, a take put in `sound/roles/`, or a role with takes and no `sound/roles/` file. A warning: an effect with only one file. A note: a sound file in the folder that `look.txt` doesn't list.
- ROM data: the same check as `gp template served`: no file may hold bytes from the ROM.

- On the page: whether the page would offer the theme under Settings, Theme, read by the page's own code. The page names a theme by the part of its package name after the namespace (`turnips` for `lex00/turnips`), which must start with a lowercase letter and hold only lowercase letters, digits, `-` and `_`, and can't be `original`. Its folder is read as the page's ROM reader reads it, over the game's pictures, and a theme the page would leave out is named with why.

It needs your ROM on this machine (in `$GP_ROM_DIR`, or `--rom FILE`) to name the game's pictures and to compare your files with the ROM, and says so when it hasn't got one. It exits 0 when there are no problems; warnings are worth a look. Its last line says what to do next.

No picture may be traced from the original's art, and your work has to be your own. If an AI tool made any of it, name the tool in your README and make sure its terms let you share the result.

### See it in the game

```sh
gp theme look play my-theme --game donkey-kong --open
```

runs the check first (a problem stops it unless you give `--anyway`; warnings are printed), then serves the game on this machine with your theme chosen. `--game` is any game you can play here: a ported game such as Donkey Kong or one you started with `gp game new`, and Moss Hop or another of the repo's templates or your own copy of one (a folder under `$GP_HOME/work/mcp/`). Nothing is published.

A game played from a ROM (a ported game) is served in the page that plays any game by name: it installs the game and its published themes from a registry made in a temp folder, with your folder as one more theme only this page sees. A game the registry hasn't got (one you started with `gp game new`) is built here and put in that registry alone. Load your own ROM in the page and play every stage to see each pose at game size, the deaths and the score line among them. A game with pictures of its own (a template of the repo's, or your copy of one) needs no ROM: it is built for the browser as `build_publish` builds it, with your theme added, and opens playing.

`--port` picks the port (any free one by default); ctrl-C stops it, or `--minutes 30` after half an hour. An agent adds `--background`, which serves from a process of its own for `--minutes` (60 by default) and prints the link and its PID. An arcade game plays the same way from your romset zip, your theme drawn over its board's pictures.

```sh
gp theme look play my-theme --game donkey-kong --shots ~/theme-shots
```

plays the game's route in a headless browser instead, on each ROM of the game in `$GP_ROM_DIR` (or the one `--rom` names; a game with pictures of its own needs none), and saves the game at named moments. The moments come from the game's `theme-shots.toml` when it has one (Donkey Kong's names walking, climbing, the hammer and the rest), else from the page check's route where it has one for the game's ROM (each stage at its start with the score line, in play and won, and the ending), else the title screen and the first stage 4 and 9 seconds after Start (on an arcade game, a coin and the start, then the attract mode and the first level 8 and 18 seconds in), and the output says which. `--against friend/night@0.9.2` (a published version, or another folder) takes the same pictures of a second theme and writes each pair side by side, yours on the left; a game with pictures of its own takes only a folder there. `--against original` sets the game in its own pictures beside yours, as the theme's step 5 on the page does (BESIDE THE ORIGINAL); the smaller picture of a pair is made a whole number of times bigger, so a large-art look and the game's own pictures show at about the same size. Pictures of a ROM's game show your theme over it: keep them to yourself and never commit them (the folder gets a `.gitignore` that keeps git out).

For a game played from a ROM it plays the page that passed the release check in `$GP_HOME/work/page-release`, or the one `--page DIR` names, and in the gameprinter checkout it can build one (`scripts/web/build-page.sh`, a few minutes). Building a game the registry hasn't got, or a game with pictures of its own, needs the checkout. `--shots` needs no checkout and no Node: gp drives Chrome Headless Shell itself, and the first time it downloads the pinned build into `$GP_HOME/work/chrome-headless-shell` (about 100 MB, on a Mac with Apple silicon or Linux on x86-64; elsewhere give one with `--browser`).

### Changes your agent proposes

When you ask your agent for a different picture in your theme, a sound for your theme, or a different sound in a game you made, it doesn't change it straight away. It proposes the change, and your page (`gp serve`) shows it as a card at the top of every tab: a picture with the one you have beside it, or a sound with two buttons that play what you have and the new one. A new take of a role says there is no sound there yet, since the role plays its other takes until you accept it. Press ACCEPT to take it or REJECT to keep what you have, and write in the box what you'd like instead; your agent reads your answer. After ACCEPT, PUT THE OLD ONE BACK undoes it. In a terminal the same proposals are `gp theme picture` and `gp theme sound` (`list`, `accept`, `reject`, `undo`).

## Get it in the game

Keep your theme in a repo of your own, with its pictures in Git LFS, and publish it to a registry of your own: a public GitHub repo of static files that `gp registry` writes. The project's registry, [`lex00/gp-registry`](https://github.com/lex00/gp-registry), holds the project's own games and themes and doesn't take others' themes. [Host your theme](../host-your-own/) is the whole road, step by step, up to the link you share. Ask your AI agent to host your theme yourself and it walks you through it over `gp mcp`, asking before each step that writes to GitHub.

The link names your registry with `r=`, and the page at [gameprinter.intentius.workers.dev](https://gameprinter.intentius.workers.dev) then offers your theme under Settings, Theme, beside Original (the ROM's own art) and Turnips. The game itself still comes from the project's registry, so yours holds only your theme:

    https://gameprinter.intentius.workers.dev/p?g=lex00/donkey-kong&c=YOUR-NAME/my-theme&r=https://raw.githubusercontent.com/YOUR-NAME/YOUR-REGISTRY/main

To ask whether the page offers it, and if not why:

```sh
gp registry check https://raw.githubusercontent.com/YOUR-NAME/YOUR-REGISTRY/main --for lex00/donkey-kong --theme YOUR-NAME/my-theme
```

It reads the game's theme list as the page does and prints `OFFERED`, or `NOT OFFERED` and why. The reasons it knows are these: not published, published for another game, withdrawn, a name the page can't list or that another theme there already has, an install that fails, and (with your ROM and the game's folder here) files the page's ROM reader leaves out. Without `--theme` it lists each one the page offers for the game and each one it leaves out. Players see the same reasons on the page, in About and, while a ROM plays, in the line under the header.

The page opens in its own theme, Turnips. A link with `&c=YOUR/THEME` opens it in yours instead.

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

A large-art theme (like Turnips) in the THEMES tab's clip review, and new levels a player can pick.
