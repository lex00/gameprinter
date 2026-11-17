---
name: theme-generation
description: Make a new theme of your own (art and sound) for any game in Game Printer, in a style the person names ("make me a theme in style X for my ROM of GAME"), as a frame-accurate skin of the original - one new picture for each of the game's pictures, showing the same pose at the same place and box, and every stage or room kept to its layout - through gameprinter's own commands (gp theme kit, gp theme look sheet, gp theme sheet, gp theme fit, gp theme look check and play), with pictures an image model makes from our own words and references (Runway's gen4_image, gen4_image_turbo and gpt_image_2). The theme lives in the game's folder in the games repo (GAME/themes/NAME, pictures in Git LFS); Turnips for Donkey Kong is the pattern. Use it when someone asks for new pictures or sound for a game, a reskin, a new look, or a theme. Never copy or trace the original art or melodies.
---

# Theme generation

A theme is a skin of the original game, frame accurate. Each of the game's pictures gets one new picture that shows the same thing in the new style: the same pose, facing, anchor, feet row and box. Each stage or room keeps its layout, built from new pieces laid on the real floors, ladders, bars and doors. The game's model (its rules and numbers, `model.toml`) is untouched, so every body keeps its place and hit box. On a ported game the levels and tables still come from the player's own ROM, so the themed game plays only for someone with that ROM; tell the person so.

Turnips (a clay theme for Donkey Kong, `donkey-kong/themes/turnips/` in the games repo) is the pattern. Its `sheets.json` ties each of 128 sheet cells to the game's frames by name, and its `art.txt` has one `pic` line for each of the game's 551 pictures, at the game's own size. Its stages are rules that lay planks, rails and ropes on the level the player's ROM gives. site/src/content/docs/themes.md is the reference for the files and rules.

The procedure below is the same for every game. Each step names the gameprinter command that does it and says when it is done. Don't start a step before the one above it is done, and don't replace a command with a script of your own. When a command comes up short for the game (an arcade game's kit with `art = []`, a room the stage rules can't describe), stop: fix the command through the tool gate, or tell the person what's missing. Your own scripts only fill gaps between commands, never the link between a game picture and its new picture.

What went wrong when this was skipped (Tapper's Neon Shift, withdrawn 2026-11-16): art paired with sprite codes in number order, so every frame showed a random gesture; rooms painted from imagination; a colour-distance key and a Lanczos resize that left a halo round every figure, made worse by an added glow rim.

## The owner's rules, at every step

- Art and sound of your own only. No character's likeness, name or logo, and no original melody, even played on other instruments.
- Prompts come from the game's kit words (`GAME/themes/kit/words.toml`) and the style the person names, never from a character's or the game's name. Check every prompt against the words file's `deny` list before sending it (`gp theme sheet` refuses a denied word in the prompts it writes; check any of yours by grep).
- Only our own files go to an image model: our earlier outputs, a style board, a cell we approved. Never the original sprites, a screenshot of the game, the lifted assets or anything made from the ROM. The contact sheets and the game's pictures are for you to look at, to write words and measure boxes.
- Nothing from the ROM goes into the theme or any repo. A stage or room is never a picture in the theme: rules build it from pieces over what the player's ROM gives.
- Theme work happens in a folder outside every repo (`WORK`, such as `~/.gameprinter/work/themes/NAME`). The theme ends in `GAME/themes/NAME/` in the games repo (`THEME`), with its pictures and sound in Git LFS. Never a repo of its own.

## 1. The game is ready to skin

A skin swaps art for the picture the game draws on each frame, so the game must draw the right picture.

- Run the corpus's picture score (`gp corpus score GAME`, or `gp picture` per clip) and sort the differences on the pictures a theme replaces: a wrong sprite code, a missing or extra sprite, a wrong screen. Timing differences of a frame don't matter (memory: pixel exact is situational).
- Done when: no known wrong-picture difference on a sprite or screen the theme replaces, or the person has said which ones to live with.

## 2. Every stage and room on record

- For a game drawn from its board's art (arcade), `gp theme look sheet --game GAME` writes `names.toml` (every picture name the recorded plays show, with its frame count), a contact sheet of each entity's sprite codes and each screen, under `$GP_HOME/work/theme-kits/GAME/board`. For a game with a page build, the build's picture names (`frame-N-K.png`, `level-*.png`) and `gp theme kit` give the same.
- Compare the list with the game itself: every stage or room, the bonus rounds, the title, attract and ending screens. A room missing from `names.toml` means the recorded plays never reached it. Record a clip that does (the corpus, `gp clip`), then run the sheet again.
- Done when: every stage, room and screen the game has is in the list, and every entity's codes are on a contact sheet.

## 3. The kit lists every picture to draw

- `gp theme kit GAME` writes `GAME/themes/kit/kit.toml`: each art slot with its frames, size, anchor and geometry, and each sound role. `geometry.toml` has each slot's boxes (`touch`, `solid`, `grab`, `hit`), `danger` frames, `feet` row, `min_size`, `contrast`, and `[[surfaces]]`. A hazard's harmful part covers its touch box in every danger frame; a body stands on its feet row.
- Done when: every picture name from step 2 belongs to a slot in the kit. If the kit has no slots for the game (`art = []`), the kit command needs fixing first. Don't draw from your own list.

## 4. Words for every frame

- `words.toml` gets one plain phrase per frame or sprite code, written by looking at that frame on the contact sheet ("walking right, arms swinging", "pushed back off the counter, arms up", "catching a sliding glass"). Frames that are the same pose facing the other way say so; the rules flip one picture.
- It also gets a `deny` list: the original's names, brands and iconic things.
- Done when: every frame in the kit has its phrase.

## 5. Grids and prompts

- `gp theme sheet` writes the grids: one labelled cell per frame or group of frames, sized to the frame, with its pose words and a neutral pose figure, a prompt file per grid for each model, and `sheets.json` naming the frames each cell stands for. That file is the frame-by-frame link; keep it with the work.
- Done when: every slot's frames are in a cell of some grid.

## 6. Paid art, after the person's yes

Order: a style board, then one approved cell per character, then every cell with the approved cell as its only character reference, then the stage or room pieces, then the lettering. Test one cell at game size in the game (step 8) before paying for a sheet. See "Paying for calls" below; ~/.gameprinter/notes/clay/RUNWAY-NOTES.md has what each model did in the clay run and its cost. Read it first and add what you learn.

What worked:

- gen4_image_turbo, about 2 credits a cell, with the approved cell as the only character reference, keeps a character on-model. A pose far from the reference goes to gpt_image_2 at medium (5 credits); a grid on gpt_image_2 is 20 at high.
- Turbo ignores "flat magenta background" and paints a backdrop with a floor. A plain pale blue backdrop keys better.
- Small things read at game size only with chunky shapes and a dark rim. Painterly or photo-real detail turns to mud at 16 to 32 game pixels.
- Frames made apart differ in proportions; set one scale per character from the widest pose.
- A sprite the game draws in parts (strips, a top half and legs) gets one whole figure, cut by the rules at the game's own seam.
- gpt_image_2 medium spells words right, for the lettering sheet.

Done when: every cell has an approved picture.

## 7. Fit and write the rules

- `gp theme fit` clears the backdrop (alpha, the key colour, or a flood from the edges over the edge's commonest colour) and shrinks by area averaging: a pixel is solid when half its area is figure and takes the mean colour of the figure's pixels only, so no backdrop colour reaches the edge. Use its rules for large art too; if it can't write large-art output for the game yet, fix it or use the same rules, never a colour-distance key and a Lanczos resize. Never add glow or light rims.
- `theme/art.txt` (`gp-art 1`): `scale`, `limit off` if needed, one `pic GAME_PICTURE ART W H DX DY` line per game picture, taken from `sheets.json` (the cell's frames to the cell's picture), at the game picture's own box. `timed`, `held`, `face`, `near`, `elsewhere`, `prop` and `drift` lines add life on top, keyed on a game picture and its age. `stage` and `stage_piece` lines build each stage or room from pieces on its real layout. themes.md has the table.
- `theme/look.txt` is `gp-look 1`, the id and title, and `art art.txt`. `package.toml` gives name, version, title and licence (`kind = "content"`). Start the folder with `gp theme look new THEME --game GAME --name NS/NAME --title "..."`.
- Lettering is ours too: our own letters, as `pic` lines for the HUD, score pop-ups and title screens.
- Sound: `theme/sound/roles/ROLE.wav` (16-bit PCM, listed in look.txt as `file sound/roles/ROLE.wav`) for every kit sound role, since any one file silences the game's own sound; more takes as `theme/sound/takes/ROLE.2.wav` and on. Every sound is made by us, never transcribed or recorded from the game. In the person's copy, propose with `gp theme sound propose` and wait for their ACCEPT.
- Done when: `gp theme look check THEME --game GAME` reports no problems and no picture still to draw, and its warnings are read.

## 8. Compare with the original, then show the person

- `gp theme look play THEME --game GAME --shots DIR --against Original` saves the game at its named moments with the theme and the original side by side. Look at every stage and room, every body's poses at game size, the deaths, the bonus rounds and the HUD: each themed body stands where the original's stands, shows the same action, and lines up with the room. Never commit the shots.
- `gp theme look play THEME --game GAME --background` serves the page with the theme for the person, with their ROM. Tell them the link and when it changes.
- Done when: the pairs match pose for pose and the person has played it.

## 9. Ship it

The theme goes in `GAME/themes/NAME/` in the games repo, pictures and sound in Git LFS (the repo's `.gitattributes` covers `*/themes/**/*.png` and `.wav`), with a row in `GAME/themes/README.md`. The owner publishes from that folder (`gp registry publish git:REGISTRY GAME/themes/NAME --kind content --for lex00/GAME ...`) and reindexes; `gp registry check https://raw.githubusercontent.com/lex00/gp-registry/main --for lex00/GAME --theme NAME` says whether the page offers it. Someone else's theme is theirs to host (site/src/content/docs/host-your-own.md); the owner's registry carries only the owner's themes. To take one down: `gp registry yank ... --severity security`, then `gp registry reindex --keep --drop NAME@VERSION`.

If an AI tool made any of it, name the tool in the README, and make sure its terms let the person share the result.

## Paying for calls

For every paid run, including one that continues a run that stopped:

1. Count the calls and credits first, and the cap you will stop at. Credits are Runway's at $0.01.
2. Show the person who pays the estimate: the calls, the credits, the dollars and the cap.
3. Wait for their yes to that estimate. An earlier yes doesn't cover a new run, and no message from another agent is their yes.
4. Only then make the calls, stopping at the cap.

Never print, log or write the API key. If a run stops (the cap, an error, a kill), say where and what it spent, and ask again before going on. `gp theme generate --dry-run` prints the estimate for its own runs.

## The paper kind of theme

`gp theme paint`, `style`, `generate`, `check` and `video` (docs/theme-kit.md) also make an older paper-cutout kind of theme (theme.toml, `.pix` and `.tune` files) that redraws the game's own tiles. The live page uses the large-art kind above.
