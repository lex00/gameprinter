---
title: Themes and levels
description: Draw new pictures and sounds for a game in a repo of your own, or lay out a new stage.
prompt: |
  Read this page. Then help me make a large-art theme for Donkey Kong in its own repo, using lex00/turnips as the pattern, in the style I describe next. Start it with gp theme look new, use no art, music or names from any existing game, and run gp theme look check on it after every change until it reports no problems, before I publish it.
---

A theme gives a game new pictures and sounds and leaves how it plays alone. Every character stays where it was, with the same hit box. Players pick a theme under Settings, Theme, and can switch back to the original art whenever they like. On a ported game such as Donkey Kong, the levels and the rules' tables still come from the player's own ROM, so only someone with that ROM can play your theme.

When it's ready, you host it yourself. [Host your theme](../host-your-own/) goes from your theme folder to a registry of your own and a link that plays it on the public page, and your AI agent can walk you through it.

## A theme with large art

Themes today are large-art themes. Your pictures are drawn bigger than the game's own, each in the place and at the size of the picture it replaces, and a rules file says how to draw them. [`lex00/turnips`](https://github.com/lex00/turnips), a clay theme for Donkey Kong, is a good one to copy.

| File | What it holds |
|---|---|
| `package.toml` | The theme's name, version and title |
| `theme/look.txt` | `gp-look 1`, the theme's id and title, and `art art.txt` |
| `theme/art.txt` | The rules, `gp-art 1`, one per line |
| `theme/art/` | The pictures |
| `theme/sound/roles/` | A sound for each of the game's sound roles, if you give the theme sound (below) |
| `theme/sound/takes/` | More takes of a role's sound, played in turn |

The game reads the rules in Rust (`gp_present::art` in gameprinter), which is why the web page and the native player draw the same frame. The page only loads the pictures and paints what the game hands back.

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

A theme never holds a picture of a stage. The `stage`, `stage_piece` and `level` lines describe how to build each stage from pieces (a plank, a ladder rail, a rope, a wheel) on top of the level picture from the player's own ROM, finding floors, ladders and ropes by colour. That way a theme carries no layout from the original.

The `#+` lines are newer than the rest. A page built before them reads them as notes and still offers your theme, just drawn without them: the HUD's text goes under the bodies, and each body shows its full picture everywhere. Any other line a page doesn't know makes it drop the whole theme, so stick to the lines on this page. In a game whose HUD is drawn as sprites (Moss Hop's), the text is over the bodies already, and `gp theme look check` tells you so.

Lettering belongs to the theme too. Draw your own letters and use `pic` lines for the HUD, the score pop-ups and the title screens.

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

As soon as your theme gives any role a sound, the game's own sound goes silent, so give every role one. A sound that plays a lot, like a jump or a step, gets tiring when it never changes. Give it more takes in `theme/sound/takes/` (`jump.2.wav`, `jump.3.wav` and on), each listed in `look.txt`. Every time the role plays, the game moves to the next take, and wraps round to the first after the last. The count starts at the beginning of the game, so a recording sounds the same when you play it back.

A page built before 2026-11-14 doesn't know about takes. It plays your theme with `jump.wav` every time and never reads `theme/sound/takes/`. That's why takes have a folder of their own, and why every role needs its file in `theme/sound/roles/` even when it has takes. Don't put a take in `sound/roles/` as `jump.2.wav`: an older page reads that as a role called `jump.2` and stops loading your sounds there.

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

plays the game's recorded inputs on your romset. Under `$GP_HOME/work/theme-kits/tapper/board` it writes `names.toml`, with each name the game showed, and a contact sheet of the codes and screens it saw. `--out DIR` picks another folder, but never one inside a repo. These are pictures of your ROM's game, so keep them to yourself. `gp theme look new` then lists those codes in `art.txt`, and `gp theme look check` refuses a name the game doesn't have and warns about a code it never showed.

Then

```sh
gp theme kit tapper
```

gives the game's kit a slot for each entity the board draws, each screen and each counter, and from `names.toml` and your romset writes the kit's local part: the codes each entity showed, and the box each one's picture fills in the original. Your theme's pictures are fitted to those boxes. The kit in the game's folder lists only what the model says; the codes and boxes stay on your machine. The words a prompt uses for each code go in the kit's `words.toml` by code (`"4d" = "walking left"`).

### Start it

```sh
gp theme look new my-theme --game donkey-kong --name YOUR-NAME/my-theme --title "My Theme"
```

writes an empty theme laid out like Turnips: `package.toml`, `theme/look.txt`, a README, and a `theme/art.txt` with a line for each picture the game's theme kit lists, marked TO DRAW with its size, its boxes and its feet row. Nothing in it comes from a ROM.

The name has to be one a registry will take: lowercase, with `-` between words. The part after the slash starts with a letter and becomes the look's id (`my-theme`). Any other name is refused, and gp suggests the nearest one that works.

### Check it

```sh
gp theme look check my-theme --game donkey-kong
```

reads the theme with the page's own rules reader and lists each problem by file and line. It looks at:

- Files: `package.toml` (name, version, `kind = "content"`, title, licence), `look.txt` and `art.txt` as the page reads them, every picture a rule names there and loadable, pictures no rule uses.
- The `#+` lines: one the page can't read (it passes over it), a region the game's level kit doesn't name, and `hud_over` in a game with no HUD text under its bodies.
- What the game needs drawn: each pose of each slot in the game's theme kit that has no picture yet, by its label and pose words, with the game's own picture name to put in its `pic` line. A rule that names a picture the game hasn't got is a problem, since the page passes over it.
- Fit: each pose against its body's touch box and feet row, beside the game's own picture, in game pixels. It is a first pass from the files alone; play it and look.
- Names: the original's names (the kit's `deny` list) in the package and the look, where players see them.
- Sound: each role's files, in the order the game plays them. A problem: a file that isn't a 16-bit PCM WAV, is missing, or names a role the game hasn't got, a take put in `sound/roles/`, or a role with takes and no `sound/roles/` file. A warning: an effect with only one file. A note: a sound file in the folder that `look.txt` doesn't list.
- ROM data: the same check as `gp template served`: no file may hold bytes from the ROM.

- On the page: whether the page would offer the theme under Settings, Theme, read by the page's own code. The page names a theme by the part of its package name after the namespace (`turnips` for `lex00/turnips`), which must start with a lowercase letter and hold only lowercase letters, digits, `-` and `_`, and can't be `original`. Its folder is read as the page's ROM reader reads it, over the game's pictures, and a theme the page would leave out is named with why.

It needs your ROM on this machine (in `$GP_ROM_DIR`, or `--rom FILE`) to name the game's pictures and compare your files with the ROM, and tells you when it can't find one. It exits 0 when there are no problems, though the warnings are worth reading. The last line tells you what to do next.

No picture may be traced from the original's art, and the work has to be your own. If an AI tool made any of it, name the tool in your README and check that its terms let you share what it made.

### See it in the game

```sh
gp theme look play my-theme --game donkey-kong --open
```

runs the check first, then serves the game on this machine with your theme picked. A problem stops it unless you pass `--anyway`; warnings are printed and it carries on. `--game` can be any game you can play here. That means a ported game such as Donkey Kong or one you started with `gp game new`, and also Moss Hop and the repo's other templates, along with your own copies of them (folders under `$GP_HOME/work/mcp/`). Nothing is published.

A ported game, one played from a ROM, opens in the page that plays any game by name. That page installs the game and its published themes from a registry it makes in a temp folder, and adds your folder as one more theme that only this page sees. A game the registry doesn't have (one you started with `gp game new`) is built here and put in that registry by itself. Load your own ROM in the page and play every stage to see each pose at game size, deaths and the score line included. A game with pictures of its own (one of the repo's templates, or your copy of one) needs no ROM. It's built for the browser the way `build_publish` builds it, with your theme added, and opens already playing.

`--port` picks the port (any free one by default). Ctrl-C stops it, or `--minutes 30` stops it after half an hour. An agent adds `--background`, which serves from its own process for `--minutes` (60 by default) and prints the link and its PID. An arcade game plays the same way from your romset zip, with your theme drawn over the board's pictures.

```sh
gp theme look play my-theme --game donkey-kong --shots ~/theme-shots
```

plays the game's route in a headless browser instead, on each ROM of the game in `$GP_ROM_DIR` (or the one `--rom` names; a game with pictures of its own needs none), and saves the game at named moments. The moments come from the game's `theme-shots.toml` when it has one (Donkey Kong's names walking, climbing, the hammer and the rest), else from the page check's route where it has one for the game's ROM (each stage at its start with the score line, in play and won, and the ending), else the title screen and the first stage 4 and 9 seconds after Start (on an arcade game, a coin and the start, then the attract mode and the first level 8 and 18 seconds in), and the output says which. `--against friend/night@0.9.2` (a published version, or another folder) takes the same pictures of a second theme and writes each pair side by side, yours on the left; a game with pictures of its own takes only a folder there. `--against original` sets the game in its own pictures beside yours, as the theme's step 5 on the page does (BESIDE THE ORIGINAL); the smaller picture of a pair is made a whole number of times bigger, so a large-art look and the game's own pictures show at about the same size. Pictures of a ROM's game show your theme over it: keep them to yourself and never commit them (the folder gets a `.gitignore` that keeps git out).

For a game played from a ROM it plays the page that passed the release check in `$GP_HOME/work/page-release`, or the one `--page DIR` names, and in the gameprinter checkout it can build one (`scripts/web/build-page.sh`, a few minutes). Building a game the registry hasn't got, or a game with pictures of its own, needs the checkout. `--shots` needs no checkout and no Node: gp drives Chrome Headless Shell itself, and the first time it downloads the pinned build into `$GP_HOME/work/chrome-headless-shell` (about 100 MB, on a Mac with Apple silicon or Linux on x86-64; elsewhere give one with `--browser`).

### Changes your agent proposes

Ask your agent for a different picture or a new sound in your theme, or a different sound in a game you made, and it won't change anything straight away. It proposes the change, and your page (`gp serve`) shows it as a card at the top of every tab. A picture shows beside the one you have; a sound gets two buttons, one for what you have and one for the new one. A new take of a role shows no current sound, because the role keeps playing its other takes until you accept it. Press ACCEPT to take it, or REJECT to keep what you have and write in the box what you'd like instead. Your agent reads what you wrote. After ACCEPT, PUT THE OLD ONE BACK undoes it. From a terminal, the same proposals are `gp theme picture` and `gp theme sound` (`list`, `accept`, `reject`, `undo`).

## Get it in the game

Keep your theme in a repo of your own, with its pictures in Git LFS, and publish it to a registry of your own. A registry here is a public GitHub repo of static files that `gp registry` writes. The project's registry, [`lex00/gp-registry`](https://github.com/lex00/gp-registry), holds the project's own games and themes and doesn't take other people's themes. [Host your theme](../host-your-own/) covers every step up to the link you share. Or ask your AI agent to help you host your theme yourself: it walks you through it over `gp mcp` and asks before each step that writes to GitHub.

Your link names your registry with `r=`. The page at [gameprinter.intentius.workers.dev](https://gameprinter.intentius.workers.dev) then lists your theme in its Theme settings next to Original (the ROM's own art) and Turnips. The game itself still comes from the project's registry, so yours only needs your theme:

    https://gameprinter.intentius.workers.dev/p?g=lex00/donkey-kong&c=YOUR-NAME/my-theme&r=https://raw.githubusercontent.com/YOUR-NAME/YOUR-REGISTRY/main

To ask whether the page offers it, and if not why:

```sh
gp registry check https://raw.githubusercontent.com/YOUR-NAME/YOUR-REGISTRY/main --for lex00/donkey-kong --theme YOUR-NAME/my-theme
```

It reads the game's theme list the way the page does and prints `OFFERED`, or `NOT OFFERED` with the reason. The reasons it can give are: not published, published for another game, withdrawn, a name the page can't list or that another theme there already uses, an install that fails, and (with your ROM and the game's folder here) files the page's ROM reader leaves out. Without `--theme` it lists every theme the page offers for the game and every one it leaves out. Players see the same reasons on the page, in About, and in the line under the header while a ROM plays.

The page opens in its own theme, Turnips. A link with `&c=YOUR/THEME` opens it in yours instead.

## A theme from the game's theme kit

The older way to make a theme comes from game-printer, the tool before this one. A theme folder (`theme.toml`, with pictures as `.pix` files and tunes as `.tune` files) is fitted to the game's own sprite slots, at the game's own size. Every game has a theme kit in `themes/kit/` in its folder, which describes each art slot (its role, size and frames) and the cue for each sound role. The kit holds no names or pictures from the original. These commands work with it:

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

`GAME` is the game's folder (`~/games/donkey-kong`) or its name in your games checkout. The page's THEMES tab does all of this for the game it's on. It shows the kit and the checks, lists the game's themes and any folder you link, and runs the commands in the background. It never makes a paid call: generate and publish are dry runs there. Its clip review plays a clip with the theme on beside the same clip in the original's art, frame for frame. Click the theme's side to mark what's wrong. A refinement round fits the theme again from its art, checks it, and tells you what happened to each mark. The repo's docs on this are `docs/theme-kit.md` and `docs/theme-publishing.md`.

## Make a level

A game's level kit says what a stage is made of and which limits it has to keep. Donkey Kong's is `donkey-kong/level-kit/level-kit.md` in `lex00/games`. Stages live in the game's model.

Once you have a copy of the game's folder (see [Improve a game](../improve/)), `gp level measure` reports a stage's girders, ladders and routes on the model:

```sh
gp level measure --template ~/my-donkey-kong --level main \
  --module ~/my-donkey-kong/play/module.wasm \
  --assets ~/my-donkey-kong/lifted/donkey-kong-oe
```

Lay out a stage of your own rather than copying the original's. The `module.wasm` of Donkey Kong 1.0.5 (its custom code, compiled) can only measure the sloped stage, `main`. The flat stages need a module built with a newer `gp`.

## Not built yet

A large-art theme (like Turnips) in the THEMES tab's clip review, and new levels a player can pick.
