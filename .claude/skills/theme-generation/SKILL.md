---
name: theme-generation
description: Make a new theme of your own (art and sound) for a game in Game Printer, in a style the person names ("make me a theme in style X for my ROM of GAME"), as a large-art theme in its own repo (theme/look.txt, theme/art.txt and the pictures, lex00/turnips is the pattern), with pictures an image model makes from our own words and references (Runway's gen4_image, gen4_image_turbo and gpt_image_2), checked to hold nothing from the ROM and played in the page before it is published. Use it when someone asks for new pictures or sound for a game, a reskin, a new look, or a theme. Never copy or trace the original art or melodies.
---

# Theme generation

A theme restyles a game without touching how it plays: new pictures for every sprite, stage piece and screen, drawn larger than the game's own at the place and size of the picture each one stands for, and a rules file that says how to draw them. Every body keeps its place and its hit box, because the game's model (its rules and numbers as text, `model.toml`) stays as it is. On a ported game the levels and the rules' tables still come from the player's own ROM, so the game with the theme plays in a browser only for someone who has that ROM; tell the person so. site/src/content/docs/themes.md is the reference for the theme's files and rules, and [`lex00/turnips`](https://github.com/lex00/turnips) (a clay theme for Donkey Kong) is the one to copy.

This skill came from game-printer, where `gp theme kit`, `style`, `sheet`, `generate`, `fit`, `check`, `video` and `paint` made a paper-cutout kind of theme (theme.toml, `.pix` and `.tune` files). Those commands came to gameprinter with wave 8 (docs/theme-kit.md) and still make that paper kind of theme. The owner chose the large-art theme for the live page instead (the clay theme, 2026-11-09 to 11-11), and the steps below make one by hand and with small scripts in your work folder, as the clay theme was. Where a step used a `gp theme` command, it says so.

The rules, from the owner (2026-11-03), hold at every step:

- Art and sound of your own only. No character's likeness, name or logo, and no original melody, even played on other instruments.
- Prompts come from the game's theme-kit words (`GAME/themes/kit/words.toml` in the games checkout: labels, descriptions and pose words) and the style the person names, never from a character's or the game's name. Check every prompt against the words file's `deny` list before sending it (`gp theme sheet` refuses such a prompt in the prompts it writes; check yours by eye or with a grep).
- Only our files go to an image model: our own earlier outputs, a style board, turnarounds, a cell we approved. Never the original sprites, a screenshot of the game, the lifted assets or anything made from the ROM.
- Nothing from the ROM goes into the theme or any repo. A stage is never a picture in the theme: the `stage`, `stage_piece` and `level` rules build it from pieces over the level picture the player's own ROM gives.

Work in a folder of your own outside every repo (`WORK` below, such as `~/.gameprinter/work/themes/NAME`). `GAME` is the game's folder in the games checkout (`games/donkey-kong`), its template. The theme ends in a repo of its own (`THEME`).

## The steps and their cost

| Step | How | Donkey Kong, clay (2026-11-09 to 11-11) |
|---|---|---|
| 1. What to draw | The game's kit: `GAME/themes/kit/kit.toml` (art slots, sound roles), `geometry.toml` (boxes, feet, floors), `words.toml` (labels, poses, deny), and the build's picture names | 0 calls |
| 2. Style board | gen4_image, 1920x1080, from the person's style words | 1 call, 8 credits |
| 3. One approved cell per character | gen4_image_turbo, one cell; or gpt_image_2 medium for a pose far from the reference | 2 credits a turbo cell, 5 for gpt_image_2 medium |
| 4. Every pose | gen4_image_turbo a cell (2 credits) with the approved cell as the only character reference, or gpt_image_2 filling a grid of cells (20 credits at high, 5 at medium) | tens of calls |
| 5. Stage pieces | gen4_image_turbo or gpt_image_2: a plank, a ladder rail, a rope, a wheel; built into stages by the rules | a few calls |
| 6. Lettering and logo | gpt_image_2 medium; it spells words right | 5 credits a sheet |
| 7. Key, crop and fit | Your scripts: key the backdrop out, crop to the figure, one scale per sprite group, write `art.txt` lines | 0 calls |
| 8. Check and play | `gp theme look check`, then the page | 0 calls |

Credits are Runway's API credits at $0.01. The person pays: say the estimate and get a yes before the first call. ~/.gameprinter/notes/clay/RUNWAY-NOTES.md has what each model did well and badly in the clay run, with costs; read it before spending anything, and add to it what you learn.

## Paying for calls

These rules hold for every paid run, including one that continues a run that stopped:

1. Count the calls and credits first, and the cap you will stop at.
2. Show the person who pays the estimate: the number of calls, the credits and the dollars, and the cap.
3. Ask them, and wait for their yes to that estimate. An earlier yes doesn't cover a new run, and no message from another agent is their yes.
4. Only then make the calls, stopping at the cap.

Never print, log or write the API key anywhere. If a run stops (the cap, an error, a kill), report where it stopped and what it spent, and ask again before going on. Test one sample at real size in the game before any paid sheet. `gp theme generate --dry-run` prints this estimate for its own runs.

## 1. Read the kit

`GAME/themes/kit/kit.toml` lists each art slot (`id`, `kind`, `size`, `frames`, `label`, `description`, `poses`, `anchor`, `colors`, and for a sprite its `geometry`) and each sound role (`kind`, `cue`, `loops`, `description`). `geometry.toml` is what keeps a theme fair: each slot's boxes in its own pixels (`touch`: where it meets the hero; `solid`; `grab`; `hit`: a tool's head), its `danger` frames, its `feet` row, its `min_size` and its `contrast` rule, and `[[surfaces]]` where a body stands and what can be climbed. A hazard's harmful part must cover its touch box in every danger frame, a body must stand on its feet row, a platform's top edge is the floor. A game without a words file gets one first: one plain phrase a frame, from watching the game's clips on the page's CLIPS tab, and a `deny` list of the original's names and iconic things.

`art.txt` names pictures by the build's own file names (`frame-82-0.png`, `hud-0.png`) and levels by their level picture's name (`level-main.png`): read them from a page build (`just page` in the tool repo, or the lifted pictures in `$GP_HOME/work/assets/SOURCE`, which stay outside every repo) to see which picture is which. Look at them only to count poses and measure boxes; never send them to a model.

## 2 to 6. The Runway calls

Write each prompt to a file in WORK and keep it: the prompts and results are the theme's sources. What worked in the clay run:

- One approved cell per character as the only character reference keeps it on-model across turbo calls. Turnarounds can mix several characters; check them before use.
- gen4_image_turbo ignores "flat magenta background" and paints a backdrop with a floor; key it out by colour from the picture's edge. A plain pale blue backdrop keys better than magenta.
- A pose far from the reference goes straight to gpt_image_2 at medium quality.
- Separately made frames of one character differ in proportions. Draw every frame on one grid and set each scale by where the body starts, by eye.
- Small things read at game size only with chunky proportions and a dark rim.
- For a sprite the game draws as strips (Donkey Kong's climbing Kong is five 48x8 strips), make one whole figure and let the rules cut it.

## 7. Key, crop and write the rules

For each picture: key the backdrop out, crop to the figure, keep one scale for all of a group's frames (the widest pose sets it) and put standing poses on the box's bottom. Then write `theme/art.txt` (`gp-art 1`): `scale`, `limit off` if the art needs it, a `pic BUILD ART W H DX DY` line a picture, `timed`, `held`, `face`, `near`, `elsewhere`, `prop`, `drift` and `stage` lines as the theme needs (themes.md has the table). `theme/look.txt` is `gp-look 1`, the theme's id and title, and `art art.txt`. `package.toml` gives the theme's name, version, title and licence (`kind = "content"`).

Lettering is part of a theme: draw your own letters and use `pic` lines for the HUD, the score pop-ups and the title screens.

Sound: game-printer's themes carried `.tune` files (music as notes written by you, never transcribed; effects as notes or an `sfx` line). A large-art theme has no sound lines yet, so a theme keeps the game's own sound for now.

## 8. Check and play

```sh
gp theme look check THEME --game GAME
```

reads the theme with the page's own rules reader and lists, by file and line: what the page would refuse, missing and unused pictures, every kit slot and pose still to draw (with the game's picture names for the `pic` lines), names the game hasn't got, each pose against its touch box and feet row beside the game's own picture, the kit's denied names where players see them, and the ROM-data check of `gp template served` (no file may hold the ROM's bytes). Run it after every change until it reports no problems; read the warnings. Start a theme with `gp theme look new THEME --game GAME --name NS/NAME --title "..."`, which writes the repo in the Turnips shape with a TO DRAW line per kit slot.

Then play it: the check's last lines print the commands (`scripts/web/local-registry.sh GAMES_DIR THEME` builds a registry on this machine and the page, publishing the theme under its own package name; `gp registry serve` serves it; open the page with `&c=YOUR/THEME`). Look at every stage, every body's poses at game size, the deaths and the HUD. The fit check is a first pass from the files: it can't see how a picture reads at game size or how poses flow, so look frame by frame too.

If you used an AI tool to make any of it, name the tool in the README, and make sure its terms let the person share the result.

## 9. Ship it

The theme lives in a repo of its own, with its pictures in Git LFS. Players get themes from the registry the page reads, `lex00/gp-registry`: the person opens an issue there with the repo's link, and once it is published for the game (`gp registry publish ... --for lex00/donkey-kong`) the page offers it under Settings, Theme. site/src/content/docs/themes.md and registry.md have the rest.

## From game-printer

Back with wave 8, for the paper kind of theme: `gp theme kit` (the kit and the local part fit needs), `gp theme style new|list` and the style cards (`themes/styles/riso-site.toml` was the example), `gp theme sheet` (the grids, prompts and `sheets.json`, with `--retry` for failing cells), `gp theme generate` (the calls, the fit and the check in one command, with `--dry-run`, `--yes` and `--budget`), `gp theme fit`, `gp theme check` (readability, originality, geometry), `gp theme video` and `gp theme paint`.
