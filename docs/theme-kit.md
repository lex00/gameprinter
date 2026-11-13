<!-- Brought from game-printer's docs/theme-kit.md with the theme commands (wave 8
of the port). In gameprinter a game's folder is the games checkout's
games/GAME (game-printer's templates/GAME): read templates/GAME below as
games/GAME, templates/donkey-kong-oe as games/donkey-kong. A command
names a game by its folder or by its template id. `gp theme kit` writes
into the game's themes/kit/ unless given -o, and the THEMES tab of the
mechanic page always gives -o, so nothing is written into the games
checkout from there. The publishing index is themes/index/GAME.toml in
the games checkout. The notes on Rivet Rodeo, retired and deleted on
2026-11-11, are kept as the record of how the checks came to be. -->

# The theme kit

Every template ships a theme kit and a paper theme, so anyone (or any agent) with their own copy of the game can make it an original look and sound. The user's decisions, 2026-11-03: "we need an AI theming kit for each game that comes with a paper cutout theme and skills to generate a new theme"; "if the theme kit is strong enough, any agent should be able to load up their rom for them and generate them a theme". Original assets are never distributed: a theme is original art and sound, with no character's likeness, name or logo and no original melody, even re-synthesized. The steps an agent follows are in `.claude/skills/theme-generation/SKILL.md`.

| Part | Where | What |
|---|---|---|
| The kit | `templates/GAME/themes/kit/kit.toml`, written by `gp theme kit GAME` | Each art slot's role, size, frames, timing, color limit, anchor and a plain description, and each sound role's cue, length and loop, with the model's slot fingerprint ("The slot fingerprint", below). From the model alone, with no tile numbers, layouts or pictures, and no character's name |
| The geometry | `templates/GAME/themes/kit/geometry.toml`, written once by hand, optional | Boxes and danger frames the model's hooks read from their numbers (`ext_params`), which its components don't say: a held tool's head, a touch test the model folds into another body's box. `gp theme kit` reads the numbers from the model and puts each box into kit.toml ("Gameplay geometry", below) |
| The prompt words | `templates/GAME/themes/kit/words.toml`, written once by hand | Each art slot's neutral label, a description in plain function words and a pose phrase a frame, each sound role's description, and the words no prompt may hold ("Prompt words", below). `gp theme kit` puts them into kit.toml |
| The local part | `$GP_HOME/work/theme-kits/GAME/`, written by `gp theme kit GAME --rom ROM` | `local.json`: the tiles each sprite frame is cut into, with the console colors each tile is drawn in, the words each layout's cells spell where the game's font reads them, each frame's visible body in the original (a box) and each stage's floor lines and ladders. `reference.png`: the original frames in gray, for sizes and poses. Derived from the lifted assets; it never goes in the repo, and it is never an image reference for new art |
| The paper theme | `templates/GAME/themes/paper/`, written by `gp theme paint GAME -o DIR` | Paper cutouts in every art slot and an original tune for every sound role, all text files (`.pix`, `.tune`). Two versions of a game can share one: Donkey Kong's is in `templates/donkey-kong-oe/themes/paper/`, and Original Edition's folder holds only `same_as.toml` pointing at it ([the theme format](formats/theme.md#one-theme-for-two-templates)) |
| Style cards | `themes/styles/<name>.toml`, or `$GP_HOME/themes/styles/<name>.toml` for personal ones, written by `gp theme style new` | How a theme looks, apart from any game: the look, the cast in the maker's words, palette hints, the style board and turnarounds once made ("Style cards", below) |
| Sheets | `gp theme sheet GAME --style NAME -o DIR`, outside the repo | Starting grids for an image model, a prompt file per grid and model, the style board and turnaround prompts, and `sheets.json` ("Sheets", below) |
| Fitting | `gp theme fit GAME [--art DIR] [--sheets DIR] -o THEME` | Art made elsewhere (any size, any colors), or the filled grids, fitted to the slots: sprite frames cleared, cropped, scaled and placed by the anchor, cut into the game's tiles by tone; pictures kept whole ("Fitting", below) |

A finished theme is shared with `gp theme publish` ([theme publishing](theme-publishing.md)): every check runs again, the theme and the look drawn from it become a signed package, and the game's themes index lists it, so the game's settings screen offers it for download.

`gp template check` fails a template without `themes/kit/kit.toml`, a kit whose prompt-facing text holds a word its words file denies, or a `themes/paper/` theme that checks (a `same_as.toml` pointer there passes when its target checks and names the template's model in `for`), and `gp game new` writes both.

## Art slots

| Kind | Id | What the theme gives |
|---|---|---|
| `sprite` | `entity.<name>` (or `entity.<role>_<n>` where the entity's name is a character's), `effect.<n>`, `screen-sprite.<screen>.<n>` | One picture a frame, the slot's size. A game drawn from tiles cuts each into its tiles, so each 8 by 8 tile keeps three tones and transparent: dark, middle and light. A tile two frames share takes the first frame's art |
| `picture` | `picture.<slot>.<entry>` | One picture of a layout entry, drawn whole in any colors (format 0.1.3's restyle `entry`): a stage, the score display's fixed part, a title screen |
| `font` | `font` | The digits 0 to 9, a glyph a frame |

Descriptions come from the role's words, the entity's size and its model description with every name left out: a capitalized word that isn't a common sentence starter, a word of the model's title, and a word of an entity's name that isn't a plain word for a thing (`theme_kit::GENERIC`). Counters in a clip's condition are shown with names hidden (`x_pose`).

## Gameplay geometry

A theme changes how a game looks, never how it plays, but a picture can make it play unfairly: a hazard drawn smaller than the box that kills, a hero drawn a few pixels off its feet, a platform whose top edge isn't the floor. The user's aim, 2026-11-04: the kit gives designers the gameplay geometry, so themes play fairly. Each sprite slot's `geometry` in kit.toml, from the model alone (`crates/gp-cli/src/theme_geometry.rs`):

| Key | What | From |
|---|---|---|
| `boxes` | Each box in the picture's pixels from its top left, `[x, y, w, h]`, with its `kind`, what it does and the frames it holds in: `touch` (meets the hero: harms it, or is taken), `solid` (stands on floors, stops at walls), `grab` (takes a ladder), `hit` (smashes what it meets) | The entity's `hitbox`, `tile_hitbox` and `climb.grab`, moved by the sprite's offset; the boxes `geometry.toml` names. A `touch` box there takes the model's place |
| `danger` | The frames that harm the hero, or for a tool the frames it smashes in, and `danger_what` | Every frame of an entity whose `contact` kills or hurts; `geometry.toml`'s `[[slot]]` |
| `feet` | The row under the feet, from the picture's top, for the hero and anything that harms on touch | The touch box's bottom |
| `min_size` | The smallest visible body: the touch box, and at least 6 pixels each way | The touch box |
| `contrast` | What it must stand apart from: the readability check's rule for its role's group (`gp_theme::readability`) | The role |

kit.toml also has `[tolerance]` (the numbers below) and a `[[surfaces]]` entry a level: where a body stands (the top edge of each ground box), the rows inside a cell a surface tile's top edge sits at (the model's `tile_surfaces`), what can be climbed and where a body falls out. The local part adds each frame's visible body in the original (`bodies`) and each stage picture's floors (each girder's pieces, `[x0, row, x1]`), the step heights between pieces and its ladders (box, the floors it joins, the top, and whether it is broken: its box stops more than 4 pixels short of the floor above). Those come from the lifted art and level, so they stay local.

Donkey Kong's `geometry.toml` gives the hammer's head boxes (raised in its first frame, swung forward in its second, from the hook's `up_*` and `front_*` numbers less where it draws the hammer, `draw_up_*` and `draw_front_*`), its danger frames while held, and the touch tests as the screen shows them: the barrels' and fires' box from the hook's `smash_*` numbers (4, 3, 9 by 10 in a 16 by 16 picture) and the hero's 6 by 6 box (5, 4), which the model folds into one column of the hero's (the note on `entities.fire`). Rivet Rodeo has the same file.

The geometry is used in three places:

| Where | How |
|---|---|
| `gp theme sheet` | Each cell gets a dashed box (`#A000A0`, cleared by fit with the ground) at the original's body in its frames, from the local kit, and the gray stand-in figure fills it. The grid's prompt says to keep the figure's whole visible body inside it and fill it, and for each harmful subject that its harmful part must be plain to see and at least `min_size`. A stage's prompt says the top edge of each platform is the floor, flat and unbroken. The dashed box is a size and a place, no shape; `keep` in sheets.json |
| `gp theme fit` | A loose picture is scaled to fill the original's body in its frame and stands on that body's bottom, centered on its middle; before, it filled the slot and stood on the slot's bottom. A sheet's cell keeps its place against the cell's box as before |
| `gp theme check` | With the lifted assets, each themed sprite frame's visible body against the original's and the slot's geometry, and each stage's floors (below) |

The check, `geometry:` in its output and `geometry` in its JSON:

| Flag | When | Tolerance |
|---|---|---|
| too big, too small | The visible body's width or height differs from the original's by more than `size` | 4 px |
| off its box | Its middle is more than `edge` from the original's | 2 px |
| stands above or below the feet | Its bottom row is more than `feet` from the original's, in a frame where the original stands on the slot's `feet` row | 1 px |
| its feet below or above the floor it stands on | A hero, foe or character placed on a stage (its spawn place and the sprite's offset): its bottom row is more than `feet` from the floor piece under its middle column, the floor nearest the original's feet from 8 rows under them to half the body's height over them. Held in the base frame always and in other frames where the original stands on that floor, so art whose original sinks into its floor too is flagged | 1 px |
| draws where the original draws nothing; draws nothing where the original draws | A frame left empty in one but not the other | |
| under the smallest body | Smaller than `min_size`, when no size flag says so already | |
| its touch or hit box is N% drawn on | In a danger frame, under `cover` of the box lies in the visible body. Refused when none of it does | 0.5 |
| a floor's top edge off the floor line | More than a tenth of a floor's columns (a column every 2 pixels, none under a ladder) have their top edge more than `floor` from the original's, where the original has one within 8 rows | 1 px |
| a step or stacked tile the hero walks through; the feet stand on air; no girder drawn under the surface | At every x of every floor piece (none under a ladder; where two pieces share a column, the higher), the drawn top of the girder against the row the feet stand on: the top of the ink that reaches the surface, followed up over gaps of up to 2 rows (at most 24), more than `floor` over it, the first ink up to 8 rows under it more than `floor` under it, or none. Neighbouring columns of one kind are one flag, with the floor, its x range, the drawn rows and the surface rows. A column the original draws the same way (at it or within `floor` columns of it) is not flagged, so a thing the original stands on the floor (the drum) is the original's | 1 px |
| a ladder drawn on under half its rows | Where the original's ladder is drawn on half or more | |
| a surface tile's top edge | With no stage picture, a tile of `tile_surfaces` whose top edge is more than `floor` from the original tile's | 1 px |

A flag the original's own art has too is not raised: the check runs each frame against itself first and drops what that finds. The floor check is the exception: it holds the drawn feet to the stage's floor, not to the original's feet. It was added on 2026-11-04, when the user saw Rivet Rodeo's rescue figure (the robot on the top girder, `entity.npc_912`) drawn 9 px through its floor: it has no touch box, so no `feet` row, and its own art sinks as far, so the check said nothing. The stage script put it at Donkey Kong's place, row 25, under a top girder 9 rows higher than Donkey Kong's; it now stands at row 16 (scripts/rivet-rodeo/stage.py, `PAULINE`), and the baselines were written again. On the three paper themes the floor check flagged that figure's base frame on Rivet Rodeo and nothing on Donkey Kong or Original Edition (their rescue figure stands over no floor piece the stage gives). The test is `theme_geometry::tests::a_figure_drawn_through_its_floor_is_flagged`. A themed picture's backdrop is its most common color and every color within 12 of it (CIE76), so paper's speckles aren't ink. Only a harmful box with nothing drawn on it is refused (exit 1); the rest are warnings. The tests are `theme_geometry::tests` (made-up 16 by 16 sprites off their box, too big, too small, below their feet and drawn in an empty frame; a flag the original shares; a made-up floor a pixel and three pixels low, and a ladder left out) and `theme_fit::tests::a_loose_picture_fills_the_originals_body`.

The paper themes, checked on 2026-11-04 (`gp theme check templates/GAME --theme templates/GAME/themes/paper`, with a fresh lift) before and after the painter's fix below:

| Game | Flags before | After | What is left |
|---|---|---|---|
| Donkey Kong | 30 | 3 | The big foe's two side poses 2.5 px off the middle (its frames share tiles, so they share one box); the fire source's unlit frame shows a 9 by 1 sliver of a tile the lit frames share |
| Original Edition | 30 | 3 | The same three |
| Rivet Rodeo | 30 | 3 | Three floors' top edges off on 9 of 89 and 8 of 71 columns: girder cells stacked over the floor (below, "Floors drawn against the surface"); 0 since |
| Moss Hop | 0 | 0 | Not repainted |
| Super Mario Bros. | 95 | 95 | Not repainted: its paper was painted before the fix |

Before, the painter drew each frame to fill its whole picture with its own pose. The user saw sprites "cut out" on the public page (2026-11-04), and the check found why: a shape ran into the picture's edge (the robot's arms, the tool swung forward, the flames of the unlit fire source), the rolling hazard stood 2 px under the original's feet, and frames that share a tile showed halves of different poses or a corner of clear (the hero's walk, the big foe). `gp theme paint` now paints, for a game drawn from tiles whose lift gives the original's bodies (`theme_paper::paint_tiles`):

| Rule | Why |
|---|---|
| Frames that share a tile, directly or through others, are one group: one pose and one box, the union of the original's bodies in the group | A shared tile shows the first frame's art, so two poses would meet at its edge as halves |
| The shape fills that box, not the picture | It keeps the original's place, size and feet and never runs into the edge |
| A frame the original leaves empty is left empty, and its tiles to the frames that draw | The unlit fire source drew flames |
| A group that shows a tile mirrored in one frame and plain in another is painted the same both ways | The flame's sway and the big foe's arms turned to clear in the mirrored frames |
| A cell drawn from the game's blank tile takes no ink; the shape gets its own edge there | A corner of the big foe was cut square |
| After all frames, each frame's ink is put into its own slot's tiles where they are clear | A tile at another place in another frame showed clear over the shape |
| The swung tool lies across its picture; the figure's arms rise to the shoulders, never over the head, and its legs stay apart | A T on its side, and arms and head run together |

It prints how many frames still show another frame's art (1 on Donkey Kong: a 5-pixel sliver in a points effect). The before and after of each sprite are in the worker's folder (`$GP_HOME/work/workers/kitG/crops/`). Tests: `theme_paper::tests::frames_that_share_a_tile_are_one_group`, `theme_geometry::tests::a_shape_cut_off_by_the_pictures_edge_is_flagged`. The check also flags a frame whose ink runs into an edge of its picture (3 or more pixels along it) that the original's body doesn't reach.

Since 2026-11-04 it also flags a part cut straight at a tile's edge: on 6 or more rows in a run (columns, for the top and bottom), the ink stops at a multiple of 8 px where the original's goes on more than `edge` further (`theme_geometry::cut_flat`, test `a_part_cut_at_a_tiles_edge_is_flagged`). A hand stopped where the frame draws the blank tile reads this way. The user saw Rivet Rodeo's big foe "sometimes half of his hands" (2026-11-04). There the cause was the hands themselves: each was an oval a tenth of its box's width across, so in the narrow poses (34 and 40 px boxes, the bundle's frame-17-0.png and frame-20-0.png) it was a 7 px sliver or a flat stub against the body, ending at x 39 to 40. A big foe's hand is now a round mitt sized by the frame's height, which a slot's frames share, so it is as big in every pose, its outer edge where the oval's was; where the frame's cells there are the blank tile, it moves, or shrinks to six tenths at least, to where it fits whole (`brute_hand`, `hand_place`; test `the_big_foes_hands_are_whole_and_one_size_in_every_box`). Until 2026-11-05 only a frame that shared no tile with another moved its hands (the next paragraph). Rivet Rodeo's paper was painted again (tiles.pix); its geometry flags are 0 before and after, and its originality is at most 0.503 on art and 0.534 on shape (the big foe 0.271 and 0.246). On the old Rivet Rodeo paper the cut check flags nothing at 6 rows; at 4 it flagged ellipse tops that happen to start at y 16, so the run is 6.

Donkey Kong's and Original Edition's paper had the fault the cut check names: their big foe's seven frames were one tile group, painted as one shape with the first frame's hands, and in clip4.0 and clip5.0 a low hand was cut at y 24, where those frames draw the blank tile (the user, on the public page in ROM mode, 2026-11-05: the big foe "has vanishing hands in some poses"). Moving the hands frame by frame removed the flags but showed 28 to 36 px of other frames' hands in 3 frames through shared tiles. Since 2026-11-05 the painter works the hands out from the slot's tile map (`group_hands`). Frames whose tiles are the same at every place are one view (frames 0 and 1). A tile two views show holds one picture, so a hand takes ink only in its view's own tiles, or in a shared tile where every view that shows it holds the same hand at the same place; a hand's reach is its disc and a pixel round it, which the paper finish changes. Each view takes its pose's hands, whole, where the fewest other views must hold them. On Donkey Kong (the same tile maps on JU and Original Edition) frames 0 and 1 own only one row of tiles on each side (y 16 to 23), too low for a whole hand of 10.9 px, so their left hand is held in common with frame 6 and their right with frame 5, at y 17, through the tiles the three show over that row; frames 3, 5 and 6 have their other hands in tiles of their own. The blank tile no longer joins frames into a group (`tile_groups`), so the two side poses (clip1.0 and clip3.0, frames 2 and 4), which share no other tile, are painted in boxes of their own and are no longer 2.5 px off. In frame 2 no right hand of full size is whole at the box's side (the cells over its low right corner are the blank tile), so it is 8.9 px across. A hand that can't be whole moves up or down with its outer edge kept, then gets smaller (to six tenths), and only then moves in: moved in, frame 2's figure drew 38 px of the original's 43. A smaller hand keeps its outer edge (before, its middle moved in by what it lost). `gp theme paint` prints each hand held in common and each smaller one. Checked on 2026-11-05 with `gp theme check --original` on JU and Original Edition: the big foe's geometry flags are 0 (4 before), the oil drum's 1 flag is as before, and the big foe's originality is 0.371 on art and 0.165 on shape (0.314 and 0.170 before). Rivet Rodeo's paper changes in its big foe's frames 0 and 1, which share every tile and had kept the ovals: they have the mitts now. Its geometry flags are 0 before and after, and its big foe's originality is 0.271 on art and 0.239 on shape (0.246 before). The test is `theme_paper::tests::a_big_foes_hands_take_only_tiles_no_other_pose_shows_otherwise`; the public page's check shoots each ROM's big foe frames in paper (`rom-TAG-paper-big-foe-frames.png`).

Since 2026-11-05 (worker pic50) a moving ladder (an `npc` whose words say ladder: Original Edition's 50m) is painted as two rails and rungs filling its box, and the belt ends that turn (an `npc` whose words say end caps, four cells far apart in one name-table body) as the same small wheel in every 8 by 8 cell, so a tile the frames show at other cells holds one picture (it showed 112 to 118 px of other frames' art when painted as a figure). A tiles slot restyled whole is now found by its slot before its asset, since Original Edition's title and 50m both draw CHR bank 1 (`title_tiles` and `tiles_50m`, one asset): by asset alone, 50m's big foe took the title's paper tiles and drew nothing. Tests: `theme_paper::tests::a_ladder_that_moves_is_two_rails_and_rungs_its_box_tall`.

### Floors drawn against the surface

The user, playing Rivet Rodeo's paper page on 2026-11-04: "there are some double floor tiles that he walks right through. they look like he would have to jump over them". The floor line check held a floor only to the original's, and only when more than a tenth of its columns missed, so it named three floors and no place. The surface check (`theme_geometry::surface_flags`, the table above) holds the drawn top at every x to the row the feet stand on. On Rivet Rodeo's paper it flagged 6 runs, each a girder drawn 7 px over the surface: floor 0 x 128 to 151 (row 208 over 215), floor 2 x 56 to 79 (152 over 159) and floor 4 x 152 to 175 (88 over 95), each split by a ladder's columns. On Donkey Kong's and Original Edition's papers it flagged floor 0 x 138 to 142 (208 over 215). Both were painted again with the fix (and the hats above): the floor flag is gone and their other 3 flags (the big foe's two side poses, the fire source's sliver) are the same as before. The rescue figure has no spawn place in Donkey Kong's model (a hook draws it), so the stand check doesn't reach it; on a frame of paper over the real 25m (`gp theme video`) its feet are on the short girder under it, where the original's are.

The cause was the painter's stage. It draws each ground box as a strip 8 rows deep from its floor line, then draws the cells the game draws that the boxes leave out by their shape, and a cell counted as the boxes' only when more than 8 of its pixels were covered. A sloped girder piece whose floor line is the last row of its cell (row 7 of 8) shows one row of girder there; with the strip's shadow that was 8 pixels, so the cell stayed, and three in a row were painted as a strip of ground at the top of the cell, 7 px over the floor. A cell any of whose pixels the boxes' girder rows reach is now the girder's (`level_picture`). Rivet Rodeo's paper, painted again, has 0 geometry flags (3 before); its stage's originality score is 0.284 (0.276 before) and readability is unchanged. The model's levels are unchanged. The test is `theme_geometry::tests::a_stacked_tile_on_a_sloped_floor_is_flagged`: a made-up sloped floor with a tile stacked over 8 columns and 4 columns left undrawn, which the floor line check passes (6 of 64 columns) and the surface check flags.

## Prompt words

An image model given the words "who throws the barrels", "oil drum" and "girder" draws the famous cast, whatever the style (the research note `asset-generation.md`, "Risks", 2026-11-03). So the words a prompt is built from don't come from the model's descriptions, which keep the game's own terms for the people working on it. They come from the game's words file, `templates/GAME/themes/kit/words.toml`, written once per game by someone who has watched the template's clips:

```toml
format = "0.1.0"
deny = ["barrel", "girder", "oil drum", "hammer"]

[[art]]
id = "entity.barrel"
label = "rolling hazard"
description = "a small round hazard the big foe sends rolling down the sloped platforms"
poses = ["rolling, seen from the side", "breaking apart: a large hollow ring"]
note = "the game's barrel"

[[sound]]
role = "walk"
description = "a soft tick on every second step"
```

| Key | Meaning |
|---|---|
| `deny` | Words (or phrases) no prompt-facing text may hold, matched whole, without case, with an `s` or `es` plural: the original's names and its iconic things. They are hidden in the kit's counter names too (`drum_flame` -> `x_flame`) |
| `art.label` | A short neutral name, printed on each cell of a sheet. Without one, the role's (`hero`, `foe`, `big foe`, `hazard`) |
| `art.description` | The description a prompt starts from, in plain function words: "the big foe at the top who rolls hazards down", "a fire source at the bottom", "sloped platforms" |
| `art.poses` | One plain phrase a frame, in the kit's frame order (`base`, then `clipI.J`): "walking, legs apart", "climbing, seen from behind, left hand up". `clip14.0` means nothing to an image model; these do |
| `art.note`, `sound.note` | For people only: the game's own term. Never copied into the kit |
| `sound.description` | The sound role's description |

The prompt-facing text is each art slot's label, description and poses and each sound role's description. Slot ids, timing, cues and anchors are keys and rules: `gp theme sheet` never prints them on a sheet or into a prompt. Slot ids come from the model's names alone, so a words file never moves one. `gp theme kit` refuses a words file with a pose count other than the slot's frames or a denied word in the result, `gp template check` refuses such a kit, and the test `theme_words::tests::templates_kits_hold_no_denied_word` checks every template that has a words file. `gp game new` writes one with a placeholder pose a frame. Donkey Kong, Moss Hop and Super Mario Bros. have theirs (2026-11-03). The paper painter reads the model's own words, not these, so its themes don't change.

## Style cards

A style card says how a theme looks, apart from any game, so one card makes a theme for Donkey Kong, Moss Hop or a new game alike (the user, 2026-11-03). `gp theme sheet GAME --style NAME` puts it into every prompt it writes: the style board's and the turnarounds' first, then each grid's, which tags the board and the turnarounds. The code is `crates/gp-cli/src/theme_style.rs`.

```toml
format = "0.1.0"
name = "riso-site"
title = "Risograph site poster"
author = "Game Printer"
license = "CC0-1.0"
look = "thick ink outlines, risograph print colors, a 1950s work-site safety poster, chunky rounded proportions"
cast = "the hero is a small site worker in a hard hat; foes are clockwork machines and living tool crates"
palette = ["#1d1d3a", "#ff4f5e", "#ffd23f"]
board = "boards/riso-site.png"

[[turnaround]]
subject = "hero"
image = "boards/riso-site-hero.png"
```

| Key | Meaning |
|---|---|
| `format` | `"0.1.0"` |
| `name` | Lowercase letters, digits and `-`, starting with a letter; the file's name |
| `title`, `author`, `license` | For people; the license an SPDX expression, default `CC0-1.0` |
| `look` | The look in a few lines: lines, colors, period, proportions |
| `cast` | The cast in the maker's own words |
| `palette` | Optional palette hints, `#rrggbb` |
| `board` | Optional: the style board, once made, relative to the card's folder or absolute. With it, `gp theme sheet` asks for no new board |
| `turnaround` | Optional: a picture per subject, by the subject's label as the kit has it (`hero`, `big foe`). With one, no new turnaround of that subject is asked for |

Shareable cards are `themes/styles/<name>.toml` in the repo; they hold no game's name, so `gp theme style new` (without `--personal`) and the test `theme_style::tests::shared_cards_check_and_name_no_game` refuse a card holding a word any template's words file denies. Personal cards are `$GP_HOME/themes/styles/<name>.toml`, and a name is looked for there first. `gp theme style list` lists both; `gp theme style show NAME` prints one. The repo ships `riso-site`; the test `theme_sheet::tests::one_card_makes_sheets_for_three_games` makes sheets from it for Donkey Kong, Moss Hop and Signal Failure (a game with no words file yet).

## Sheets

`gp theme sheet GAME --style NAME -o DIR` writes, outside the repo, what an image model needs to draw a theme. The code is `crates/gp-cli/src/theme_sheet.rs`. An image model asked for "a sprite sheet" gives no steady grid, frames of differing sizes and no background to key out (`asset-generation.md`, 2(a)), so the grid is ours and the model paints inside it.

| File | Holds |
|---|---|
| `sheets.json` | The manifest (below). Paths in it are relative to DIR |
| `refs/style-board.gen4_image.txt` | The style board's prompt. Make it first |
| `refs/turn-<subject>.gen4_image.txt` | A turnaround's prompt per main subject (the hero, a bystander, a large foe or boss) |
| `sheet-NN.png` | A starting grid: 1024x1024, 1536x1024 or 1024x1536 |
| `sheet-NN.gpt_image_2.txt` | The grid's prompt for gpt_image_2: the style card, the rules, each subject's description, then a line a cell (`D07 hero: long stride, arms thrust forward`) |
| `sheet-NN.gen4_image.txt` | The same for gen4_image, within its 1000 characters (cells past the limit are "as labeled") |
| `pictures/<id>.gen4_image.txt`, `pictures/<id>.guide.png` | A whole picture's prompt (a stage, the score display, a title) and, for a stage, the geometry guide from the local kit (platforms orange, ladders blue) when it may be sent (below) |

A grid has a sheet for each main subject (split evenly when over 16 cells) and for any slot with 8 or more drawings; the other slots share props sheets of up to 16 cells. Frames whose tiles the local kit cuts the same are one drawing, one cell. The font and slots of 8 by 8 or less are listed under `code`: drawn in code, since a model's picture that small is mush. A cell is bordered in black on a magenta (`#FF00FF`) ground, with gutters of magenta between cells and a white label strip above it (the cell code, the slot's label and the pose words). Inside it are a thin box at the slot's size times the sheet's scale, the line the figure stands on, and a gray pose figure drawn from the pose words alone: a stick figure for a body (arms up, forward or down, legs together or apart, turned for a spin), a block shape for the rest (a ring, a flame, a bar, a tool, a number). With a local kit, a dashed box marks the original's body in the cell's frames and the figure fills it ("Gameplay geometry"): a size and a place, no shape. Nothing else in a grid comes from the original. Every prompt is checked against the game's denied words before anything is written. Donkey Kong gives 6 sheets of 58 cells (the hero 14 and 14, the rolling hazard 10, the big foe 6, the one to rescue 2, props 12), 4 refs and 3 pictures, with 3 slots in code.

### sheets.json

Stable as `gp-sheets 1`; a runner (the Runway one) reads it and `gp theme fit --sheets DIR` reads it back.

```json
{
  "format": "gp-sheets 1",
  "game": "donkey-kong-oe",
  "style": {"name": "riso-site", "card": "/abs/themes/styles/riso-site.toml"},
  "key": "#FF00FF",
  "refs": [
    {"id": "style-board", "kind": "style-board", "slots": [], "model": "gen4_image", "ratio": "1920:1080",
     "prompt_file": "refs/style-board.gen4_image.txt", "out": "refs/style-board.png", "refs": []},
    {"id": "turn-hero", "kind": "turnaround", "subject": "hero", "slots": ["entity.player"],
     "model": "gen4_image", "ratio": "1440:1080", "prompt_file": "refs/turn-hero.gen4_image.txt",
     "out": "refs/turn-hero.png", "refs": ["style-board"]}
  ],
  "sheets": [
    {"id": "sheet-04", "image": "sheet-04.png", "size": [1536, 1024], "scale": 14, "model": "gpt_image_2",
     "prompts": {"gen4_image": "sheet-04.gen4_image.txt", "gpt_image_2": "sheet-04.gpt_image_2.txt"},
     "out": "sheet-04.out.png", "refs": ["style-board", "turn-hero"],
     "cells": [{"cell": "D01", "slot": "entity.player", "label": "hero", "frames": [0], "frame_names": ["base"],
                "rect": [18, 50, 252, 280], "body": [32, 78, 224, 224], "size": [16, 16],
                "anchor": "top left at -5, -4 px from the body's place; drawn facing right, mirrored to face left",
                "baseline": 302, "pose": "standing, facing right, arms at its sides"}]}
  ],
  "pictures": [
    {"id": "picture-stage_screens-screen_25m", "slot": "picture.stage_screens.screen_25m", "size": [256, 240],
     "model": "gen4_image", "ratio": "1440:1080", "prompt_file": "pictures/picture-stage_screens-screen_25m.gen4_image.txt",
     "out": "pictures/picture-stage_screens-screen_25m.out.png",
     "guide": "pictures/picture-stage_screens-screen_25m.guide.png", "refs": ["style-board", "guide"]}
  ],
  "code": [{"slot": "font", "why": "8 by 8: ..."}]
}
```

| Field | Meaning |
|---|---|
| `refs[]` | Pictures to make first, in order. `image`, when present, is a picture the card already has (absolute): use it and make nothing. Else write the result to `out` |
| `refs[].id` | Named in a later job's `refs`. A prompt tags it as `@` and the id with `-` as `_` (`@style_board`, `@turn_hero`); `@grid` is the sheet's own `image`, `@guide` a picture's `guide` |
| `sheets[].model`, `prompts` | The model to call first, and a prompt file for each model |
| `sheets[].out` | Where the runner writes the filled grid. Any size: fit scales it to `size` (area averaging) before cutting cells |
| `cells[].rect` | The cell's inside, `[x, y, w, h]` in the grid: what fit cuts |
| `cells[].body`, `baseline` | The box (slot size times `scale`) and the row it stands on, in the grid |
| `cells[].frames`, `frame_names` | The kit frames the drawing is |
| `pictures[].out` | Where the runner writes the whole picture; fit scales it to `size` |
| `pictures[].rom_derived` | `true` when the guide is made from the person's ROM; absent otherwise |

Tags are 3 to 16 characters, Runway's limit (the coordinator, 2026-11-03, from the runner's dry run: `turn_the_one_to_rescue` was 22). A turnaround's id is a short key by its subject's role, `turn-hero` (the player), `turn-ally` (a bystander) or `turn-foe` (a large foe or boss), with `2`, `3` and on when two subjects share a role; the label stays in the prompt's words. `gp theme sheet` refuses to write a manifest with a tag outside 3 to 16, and the test `one_card_makes_sheets_for_three_games` checks it.

A stage's guide is the lifted level's platforms and ladders, so for a template that ports a commercial game it is made from the person's ROM: the original's layout. Such a guide is never sent to a service by default (the coordinator, 2026-11-03). `gp theme sheet` writes it only with `--rom-guides`, prints a warning, and marks the picture `rom_derived: true`; `gp theme generate` refuses a manifest with one unless it is given `--rom-guides` too. Without a guide, the stage's prompt describes the stage in words alone. A guide from an original template's own files (Rivet Rodeo, `how = "authored"`) is written by default and isn't marked.

To retry some cells, `gp theme sheet GAME -o DIR --retry D03,F02` writes `retry-sheet-NN.gpt_image_2.txt` for each grid holding any of them, which asks the model to paint only those cells again on the filled grid (its `out`, sent as `@grid`) and keep the rest. Write the result over the same `out` and fit again: a cell is cut by its `rect` alone, so the others stay as they were.

## Fitting

`gp theme fit GAME --sheets DIR -o THEME` reads the filled grids through `sheets.json`; `--art DIR` reads loose pictures, `ART/<slot id>/<frame name>.png` (or `<n>.png`), `ART/<slot id>.png` for a picture or a one-picture slot, `ART/font/0.png` to `9.png`, and `ART/sound/<role>.tune` (or `.wav`). Given both, a loose picture wins over a cell. Each frame goes through the steps of `asset-generation.md`, 2(e) (`crates/gp-cli/src/theme_fit.rs`):

| Step | What |
|---|---|
| Background | Clear where the picture has alpha; else the key color wherever it is (strong red and blue, little green: the magenta, the box, the line) when the edge is mostly key; else a flood from the edges over the edge's most common color |
| Figure | The opaque pixels' box. One scale per slot, the largest any of its frames needs (to fit the original's body, for a loose picture), and never smaller than the grid's box, so every frame of a slot keeps one size |
| Place | By the anchor: a cell's figure keeps its place against the cell's box and line; a loose picture's fills the original's body in its frame and stands on its bottom ("Gameplay geometry"), or without the local bodies stands on the slot's bottom, centered |
| Downscale | Area averaging: a slot pixel is opaque when half its footprint is, colored by the mean of what's opaque in it (nearest before 2026-11-03) |
| Clean | Lone opaque pixels dropped |
| Tones | One lightness split for all of a slot's frames: thirds between the 2nd and 98th percentiles of the slot's lightness, so a color keeps its tone from frame to frame (each frame's own range before 2026-11-03, which made tones flicker) |

The tiles are then cut as before: three tones a tile, the tones' colors the art's mean per role, a tile two frames share taking the first frame's art. `--contact FILE` writes each slot's fitted frames at 1x and 4x, a row a slot, for a person to look at.

## Sound roles

A model names its game's sound requests as sound roles (core `present.sound_roles`): each a counter holding the request, a mask or a value, whether the request is held (a tune the game keeps set) or taken each frame, and whether it loops. Donkey Kong's are the driver's request bytes $FC to $FF:

| Role | Request | Kind |
|---|---|---|
| title_tune, game_start, stage_intro, clear, pause | $FD bits 7, 0, 3, 1, 6 | music |
| stage_music, hammer_music, hurry_music | $FC = $02, $40, $20, held | music, loops |
| jump_over | $FD bit 5 | sfx |
| walk, jump, smash, death | $FF bits 3, 2, 1, 0 | sfx |
| thump | $FE bit 7 | sfx |

Every role can be a `.tune`, a template's only sound format (it holds text only): Donkey Kong's paper theme has a `.tune` for all 14, which the test `theme_paper::tests::every_sound_role_is_a_tune` holds. Music is notes; a short effect can be notes or an effect by parameters in sfxr's manner (`sfx zap 0.5 wave=square freq=C6 slide=-36 decay=12`, docs/formats/theme.md, "Sound replacements"), so a jump's rising sweep or a thud's filtered noise is a line of numbers in the theme rather than a recording. Runway's sound effect model gives WAV files only, which a template can't hold and the melody check reads poorly (`asset-generation.md`, section 4).

## The paper painter

`gp theme paint` paints from the kit. For a game drawn from tiles whose levels are update strings (Donkey Kong), it draws each sprite frame by its role and description words (a figure for the hero, a brute for a large enemy, a paper roll for a small one, a flame, a drum, a mallet, a small teal robot, its arms waving in turn, for a character the hero doesn't fight), cuts it into the game's tiles, and colors each role's tones with palette rules; it draws each stage from the level's ground and ladder boxes as kraft strips and paper rungs, and the other cells the stage draws by their shape: a block two by two or more as a paper card, a row of three or more as a strip, a column of two or more as a ladder, and a lone cell (a slope's step, a ladder's end, a stray) not at all. The score display gets its words in its own font, a thin frame where the game's own marks box a number in, and nothing for a label it can't read. A box of numbers in one row gets its labels set apart, as Donkey Kong's status box sets M, BONUS and L apart: each label over its number, a line between neighbours and a frame round the box. A row is such a box when two or more of its numbers each follow a one-letter label on the same row (the label moves up; before, Rivet Rodeo's paper HUD read L2B4900N1), or when the game's own marks cover the cells over a number (a word it draws as a picture, Donkey Kong's BONUS). A letter is spelled out as its counter's name when the name starts with it, is one plain word and fits over the number (B over `bonus` reads BONUS). A title screen gets the theme's own title (`--logo`, or `--logo-file` for a name kept in one file, as Rivet Rodeo's is in scripts/rivet-rodeo/title.txt) in the theme's font at twice the menu's size, one color with a one-pixel shadow (larger, deeper-shadowed letters were hard to read, the user said of Rivet Rodeo's page on 2026-11-03), the menu (the longest run of evenly spaced lines the game's font reads), and the theme's credit line (`--credit`, default MADE WITH GAME PRINTER) in place of the game's own words (a maker's or a country's line); every line it can't read is left out. Rivet Rodeo's paper theme says RIVET RODEO (scripts/rivet-rodeo/title.txt, since 2026-11-04). Donkey Kong's and Original Edition's said RIVET RODEO, the logo they were painted with; since 2026-11-04 they say the painter's default, PAPER PLAY. The game's own name, DONKEY KONG, is refused there by the originality check's names (a theme painted with it read "DONKEY KONG" and was refused). The hero and a big foe wear a cowboy hat in every frame (`hat`: a dark crown pinched at the top, a light band and a brim with its tips turned up), drawn inside the figure's box, so the geometry is the same; the user asked for it on 2026-11-04. Rivet Rodeo's paper is painted with it, and Donkey Kong's and Original Edition's since 2026-11-04, when the coordinator made Donkey Kong's paper (drawn over the real 25m from a visitor's ROM) the main paper theme. The two were painted by the same command and were the same in the files they shared; since 2026-11-04 they are one theme, painted from Original Edition with `gp theme paint templates/donkey-kong-oe -o templates/donkey-kong-oe/themes/paper --for donkey_kong`, which writes `for = ["donkey_kong_oe", "donkey_kong"]`. Every `.pix` and `.tune` it wrote was byte for byte the files of both folders before. `--for MODEL` (repeatable) adds a model to `for` after the painted one; paint the model with the most slots, since a model passes over a restyle of a slot it lacks. Each sound role gets a tune from its kind and words, its melody from a pentatonic scale picked by a hash of the role's name. The font's letters and signs are drawn in its own font too, so text screens read in it. The font (`theme_paper::glyph`) is our own, drawn in code and CC0 like the theme: a narrow face, most glyphs 4 dots wide (M, V, W, X and Y take 5) and 7 tall, each centered in its 8 by 8 cell, set proportionally in a line (a glyph's width and a dot) and centered on the cells the game's font drew the line in. Until 2026-11-03 it was a 5 by 7 face of the common LCD kind, the same that Rivet Rodeo's own pictures draw (scripts/rivet-rodeo/art.py), so the originality check refused Rivet Rodeo's paper font at 0.967 against its own art; Donkey Kong's font was never its source (0.556 against it). The digits now score 0.383 against Donkey Kong's lifted font, 0.371 against Super Mario Bros.', 0.492 against Rivet Rodeo's and 0.488 against Moss Hop's (refused at 0.75), and every glyph reads as itself at 1x and 2x (`the_theme_font_reads_as_itself`). A game whose levels are built from objects (Super Mario Bros.' area objects) has no stage picture to paint; its level tiles are painted metatile by metatile from each metatile's item role (ground as a kraft block, a platform as a strip, a block the hero hits as a card with a mark, a pickup as a disc, a goal as a post, water as waves, scenery as a few paper specks), colored by a lightness ramp per role so every area's palette gets paper, and its entities get a sprite per role. The score display drops a word of the game's title (Super Mario Bros.' MARIO), as the kit drops names. A game it can't draw either way gets a sprite per entity role and a cell per metatile role, as Moss Hop's paper does.

Since 2026-11-05 (Donkey Kong's 75m on the public page), a character the hero doesn't fight whose description says platform is a card plank filling its box, not a robot (75m's elevator platforms), and an enemy whose description says spring is a paper coil between two plates, as tall as its box, so a pressed frame is squat. A slot whose frames are all another slot's (the same tiles at the same places) is painted once, in the slot with the most frames (`same_frames_as`): 75m's big foe shows three of 25m's big foe's poses from the same tiles, and painted again in a box of its own frames it drew over 25m's tiles in other poses. The stage picture paints a thing on its own as a gem standing on the bottom of its block: the drawn cells of one palette block (two by two cells, aligned, as the console's attributes give a palette) in a palette none of the boxes' cells use, two or more of them, with no drawn cell of that palette just outside the block. These are Donkey Kong's prizes on 50m, 75m and 100m; a ladder in such a palette runs on out of its block, and 25m's oil drum is in the ground's palette, so it stays a card. On 75m before this, the umbrella was a blank card and the purse two cells of ladder. Checked on 2026-11-05 with `gp theme check --original` on JU and Original Edition: 0 refused; the platform scores 0.416 on art and 0.619 on shape, the spring 0.286 and 0.563, the 75m stage 0.172 and 0.206. The 75m stage has 6 geometry flags: the columns' bottom ends stand up to 24 px over floor 0 under them (as the original's column ends do; a fall there is a death), and three short floors' top edges are 6 to 8 px off on 1 to 4 columns.

### Pose pins

Frames that share tiles are painted as one group, in one pose, in the box of the original's bodies in its frames, and painted the same both ways when one of its frames is mirrored. Until 2026-11-07 the pose was the group's first frame's place in the slot's frame list, so a frame the base gained (Donkey Kong: Original Edition's mirrored walk and hammer poses) moved every pose after it, and a mirrored copy of a pose changed its box and its symmetry: Rivet Rodeo's hero walked, played and jumped in other poses with nothing about him changed. Now each group's pose, box and mirroring are pinned in `poses.toml` beside the theme's theme.toml, by the tiles its first frame shows. A paint reads the pins first; a group they name keeps them, and one they don't is painted as before and pinned. Change a pose number there to repaint that group in another pose. Rivet Rodeo's pins are the numbers its paper theme had at cb650de3: the current painter, run on that commit's template, paints its sprite sheet byte for byte and wrote them.

The same day: a frame drawn only from other frames' tiles at other places (OE's hero walking off a girder's end, from tiles 2, 4, 3 and 5) joins no group and isn't painted, and neither is a frame an earlier painted slot shows (OE's 100m fires on the barrels' tiles) or, in a group painted one way, a mirrored copy of a plain frame. One HUD cell drawn by two fields (OE's `bonus` and `hud_bonus`) gets one label, the always-drawn field's. Frames that are rows of one figure (OE's thrower going up with Pauline, five bodies of six sprites) are painted whole and cut back into rows. And a character the hero doesn't fight that is a large foe's size, or such a figure as wide as one, is painted as that foe (its shape, and its colors' tones under its own role's rule): the thrower going up is the red thrower.

## Slot names

`gp theme check` refuses a restyle whose slot or entry is in no model the theme is for, and names the nearest real one, since a runtime would pass over it with no word ([the theme format](formats/theme.md#slot-names)). A theme for both Donkey Kong versions (`for = ["donkey_kong", "donkey_kong_oe"]`) may fill Original Edition's extra slots, such as `title_tiles` and `stage_screens_50m`: on JU each gets a note that JU passes over it, and the check passes. Original Edition's paper theme with `for` set to both models, checked on JU on 2026-11-04, gave 7 restyles, 0 refused and 2 notes; every template's own themes pass (`theme_slots::tests::every_templates_themes_pass`). It is now the shared paper theme, and `gp theme check` on it gives the same on each model: 7 restyles, 0 refused, 2 notes (`title_tiles` and `stage_screens_50m` only in donkey_kong_oe); readability ok on JU and warn on Original Edition, geometry 3 flags and 0 refused on each, as before (1 since 2026-11-05, the oil drum's; "Gameplay geometry", above).

## The slot fingerprint

kit.toml's `slots` is the fingerprint of the model's art slots and sound roles, so a theme can record which version of a game it was checked on. It is the BLAKE3 hash of one line a slot, sorted, after a line naming the fingerprint's version (`gp-slots 1`) and a line for the screen's size:

```text
screen 256x240
art entity.barrel sprite 16x16 frames=18 colors=tile-3+clear
art picture.stage_screens.screen_25m picture 256x240 frames=1 colors=any
sound stage_music music loops=true
```

| Part | In the fingerprint |
|---|---|
| Art slot | Its id, kind, width by height, frame count and color limit (`tile-3+clear` for a game drawn from tiles, else `any`; a picture is always `any`) |
| Sound role | Its role, kind (`music` or `sfx`) and whether it loops |
| Left out | Every word: labels, descriptions, poses, frame names, timing, cues, anchors, the color limit's wording and a sound's usual length. A words file, or a gp that words the kit differently, gives the same fingerprint |

The code is `theme_kit::slot_lines` and `gp_package::index::slots_digest`. `gp theme kit` writes it beside `model` and prints it. Each model has its own: Donkey Kong JU's and Original Edition's differ, since Original Edition has more slots. A theme's index line records the fingerprint of each model it was checked on ([theme publishing](theme-publishing.md#the-index)).

| Test | What it checks |
|---|---|
| `theme_kit::tests::the_slot_fingerprint_is_the_same_on_every_run` | A made-up kit's lines and its fingerprint, written out in the test, so a machine that hashes it differently fails |
| `theme_kit::tests::the_slot_fingerprint_changes_with_a_shape_and_not_with_words` | A words file and reworded fields leave it as it is; a slot's size, frames, id or removal, a sound role's loop, name or removal, art not cut into tiles and another screen each give another |
| `theme_kit::tests::templates_kits_hold_their_models_fingerprint` | Every template's kit.toml holds the fingerprint its model gives now, with and without its words file; JU's and Original Edition's differ |

A template whose model changes a slot's shape fails the last test until `gp theme kit` writes its kit again.

## The originality check

`gp theme check templates/GAME --theme DIR --original [--rom ROM]` compares the theme with the player's own lift of the game and refuses one too close to the original (`crates/gp-cli/src/theme_original.rs`):

| Measure | What it is | Refused at |
|---|---|---|
| Art agreement | Over the pixels opaque in either picture, the share with the same tone (dark, middle, light), the theme's tones matched to the original's in the best of the six orders, the themed picture scaled to the original's size. A sprite slot is all its frames, each put together from its tiles; a picture is the entry's tiles drawn. The best over every shift of up to 2 pixels each way, plain and mirrored (the same move for all of a slot's frames) | 0.75 or more, on a slot with 24 or more such pixels |
| Shape | The outlines (opaque pixels with a clear neighbour) of the two: the F1 of the share of each outline's pixels on the other's, the best over the same shifts and mirror. It catches a copy repainted inside. Not judged for the font and 8 by 8 slots, whose shapes their meaning sets (a 0, an arrow) | 0.85 or more |
| Logo | Art agreement and shape over the box of the title screen's cells the game's font can't read (the original's logo) | As art |
| Names | Text read from a title or score display picture: its pieces of one color, and of everything well darker or lighter than its ground, split into letters, each matched to the theme's font and the game's own letters at any size. Refused when the text holds the model's title words or a name the kit leaves out (`theme_kit::names_of`), as a word or run together. An original's (`source.how = "authored"`) own title is not a name: Rivet Rodeo's paper says RIVET RODEO, and the Donkey Kong names its derived model keeps are still refused | Any |
| Melody | The longest common run of note steps (semitones between notes in a row) of the replacement and of each tone channel of the original cue (the model's own sound, its hooks' APU writes, from the frame the role starts in the template's clips), over the longer's count, so a transposed copy scores 1 | 0.6 or more, when both have 6 or more steps |
| ROM bytes | A run of 16 or more of the ROM's bytes (not all one value) in any file of the theme | Any |

The shifts, the mirror, the shape, the logo and the names were added on 2026-11-03 from the research note `asset-generation.md` ("Risks": a copy shifted a pixel or mirrored, and a name in title art, were missed). The tests `shifted_mirrored_and_recolored_copies_are_refused` (a made-up 16 by 16 sprite shifted by up to 2, mirrored, its tones swapped, and all three at once, all refused; repainted flat inside, refused by shape; a round blob in the same box, passed) and `a_name_in_a_picture_is_read_and_refused` hold them. On 2026-11-03 Donkey Kong's paper theme scored at most 0.556 on art (the digits) and 0.520 on shape (the smashing tool) and read RIVET RODEO and its menu with no name; the same theme painted with the title DONKEY KONG was refused on the title's text; SMB's paper scored at most 0.558 and passed. The reader knows pixel fonts only: a model's painted lettering in another typeface isn't read, so a person still looks at a title.

The thresholds are first settings, tested in the module: a copy, and a copy with its tones swapped, score 1.0; a transposed melody scores 1.0; a theme whose sprite tiles are the original's own is refused on 10 of 11 slots (Donkey Kong, 2026-11-03; the eleventh draws from the background tiles, which it kept). Donkey Kong's paper theme scores at most 0.383 (the digits, in the font of 2026-11-03; 0.556 in the one before) on art and 0.125 on a judged melody. Short effects (a tick, a jump) have under 6 steps and aren't judged: a sound that short owns no melody.

## Generating with Runway

`gp theme generate GAME --style NAME` makes a theme's art with Runway's API from start to end, so an agent with a key can go from a style card to a checked theme with no person drawing. The user's request, 2026-11-03: a fully automatic option at about $3 a theme, with the coordinator asking the user before each paid run. The API's facts (models, credits, limits, terms) are in `$GP_HOME/work/workers/researchR/asset-generation.md` and the sources `crates/gp-cli/src/theme_runway.rs` names. The code is `crates/gp-cli/src/theme_generate.rs`.

```sh
gp theme generate GAME --style NAME --dry-run              # every call, its model and credits, the total in dollars; no call
gp theme generate GAME --style NAME                        # asks on the terminal, then runs
gp theme generate GAME --style NAME --yes --budget 300     # no question; stops before passing 300 credits
gp theme generate GAME --style NAME --only turn-hero,A07   # some refs, sheets, slots or cells
```

| Step | What |
|---|---|
| Sheets | `gp theme sheet GAME --style NAME -o RUN/sheets` when RUN/sheets has no sheets.json, or the folder `--sheets` names. It reads `gp-sheets 1` only |
| Refs | The style board and each turnaround in order, on the model sheets.json names (`gen4_image`). A ref with `image` is the card's own picture: it is sent as it is and no call is made |
| Sheets | Each grid on `gpt_image_2` (quality `--quality`, default high, background opaque so the magenta stays), with the grid as `@grid` and its refs, each tagged by its id with `-` as `_` (`@style_board`, `@turn_hero`) |
| Pictures | Each picture on `gen4_image`, with `@guide` when sheets.json gives one |
| Results | Each to its entry's `out` in the sheets folder, as the runner contract says |
| Fit, check | `gp theme fit GAME --sheets DIR -o RUN/theme --contact RUN/contact.png`, then `gp theme check GAME --theme RUN/theme --original` |
| Retries | For each slot the check refuses, each of its cells on `gen4_image_turbo` (2 credits, the sheet's refs) pasted into its sheet's `out` at its `rect`, and a refused picture made again; then fit and check again, up to `--retries` rounds (default 2). What is still refused after that is for a person to pick |

`--no-fit` stops after the downloads. `--concurrency` (default 2) is the tasks in flight at once: Runway's first tier runs 1 or 2 a model group, and more wait on its side as THROTTLED, which costs nothing.

### The key

The key is read from the first of these that has one. gp never prints it: curl reads it on its standard input, never from its arguments, and every message from a call passes through a filter that replaces the key and any bearer token with `[key]`.

| Where | How to set it |
|---|---|
| `RUNWAY_API_KEY` | `export RUNWAY_API_KEY=...` in the shell that runs gp. Simplest; the variable is seen by every program the shell starts |
| The macOS keychain, service `game-printer-runway` | `security add-generic-password -s game-printer-runway -a "$USER" -w` (it asks for the key, so it isn't in the shell's history). gp reads it with `security find-generic-password -s game-printer-runway -w`. `GP_RUNWAY_KEYCHAIN=0` skips it |
| `$GP_HOME/secrets/runway` | `mkdir -p ~/.gameprinter/secrets && (umask 077 && cat > ~/.gameprinter/secrets/runway)`, paste the key, then Ctrl-D. gp refuses the file while anyone but its owner can read it (`chmod 600`) |

### Cost before money

| Rule | How |
|---|---|
| The estimate | `--dry-run` prints a line a call (model, ratio, quality, references, credits), the total in credits and dollars (a credit is $0.01), what the ledger holds already, the cap and the slots drawn in code, and writes `plan.json` and `estimate.txt` in the run folder. It needs no key and sends nothing. It also lists what the API or the rules would refuse (a ratio a model hasn't, a prompt over its limit, a tag that isn't 3 to 16 letters, digits or `_`, a file that may not be sent), and exits 1 when there is any |
| Asking | A paid run without `--yes` prints the estimate and the account's balance (`GET /v1/organization`, which costs nothing) and asks for `yes` on the terminal. Run without a terminal, it refuses unless `--yes` is given |
| The cap | `--budget CREDITS` is the most the run folder's ledger may hold. Before each call, the credits spent or held so far plus the call's are compared with it, and the run stops before passing it. Without `--budget` the cap is the estimate and a quarter more, for retries |
| The balance | A run whose estimate is over the account's balance stops before the first call |

Donkey Kong with kitTK's sheets of 2026-11-03 (riso-site): 13 calls, 176 credits ($1.76): 4 refs and 3 pictures on `gen4_image` at 8, 6 sheets on `gpt_image_2` high at 20. Cap 220 with the retries' quarter.

### The run folder

`$GP_HOME/work/theme-runs/GAME/STYLE/` unless `--run DIR`:

| File | Holds |
|---|---|
| `ledger.jsonl` | A line for every step of every call: `submitting` (synced to disk before the request is sent), `submitted` (the task id), `rejected` (an HTTP refusal, which costs nothing), `ended` (the status, the credits it cost, the failure code), `downloaded` (the result's path and SHA-256), `stopped`. Each line has `spent`, the running total. A call is keyed by its id and the hash of its request (model, ratio, prompt, references) |
| `plan.json`, `estimate.txt` | The last plan and estimate |
| `requests/` | The body of every request sent, with no key |
| `run.lock` | The process id of the run using the folder; a second run on it is refused while the first is alive |
| `theme/`, `contact.png` | The fitted theme and its contact sheet |

Run the same command again to continue. A call whose result is downloaded is never made again (unless `--again`); a task that was in flight is polled, not made again; a task that succeeded and wasn't downloaded is fetched again for a fresh URL. A call that changed (a new prompt, a new reference) has a new key and is made. The one gap is a kill between the request and the line that names its task: the call is counted as held at its estimate and made again, so it may be paid twice. That window is the time of one HTTP reply.

### Errors and refunds

As Runway documents them (https://docs.dev.runwayml.com/errors/errors/ and errors/task-failures/, read 2026-11-03):

| What | What gp does |
|---|---|
| HTTP 429, 502, 503, 504, or no reply | Waits with exponential backoff and up to 50% jitter, and tries again; after 8 refusals in a row the run stops, to be continued later |
| HTTP 401 | Stops: the key is wrong |
| HTTP 400, 404, 405 on a request | That call fails; the calls that need it are blocked |
| A task PENDING, THROTTLED or RUNNING | Polled from `--poll-ms` (2 s) up, half again each time, to 15 s, with jitter |
| FAILED with `INTERNAL*`, `INPUT_PREPROCESSING.INTERNAL`, `THIRD_PARTY.UNAVAILABLE` or no code | Refunded; made again up to `--retries` times |
| FAILED with `SAFETY.*` or `ASSET.INVALID` | Not made again. `SAFETY.INPUT.*` keeps its credits, as the task's `cost.credits` says |
| SUCCEEDED | The cost is the task's `cost.credits`; the output is downloaded at once, since its URL expires in 24 to 48 hours |

### What is sent

Only the sheets folder's files (grids, a stage's geometry guide, prompts), the run's own results, and a style card's own pictures. gp refuses to send a file under the lifted assets (`$GP_HOME/work/assets`), the ROMs (`$GP_ROM_DIR`), a theme kit's local part (`$GP_HOME/work/theme-kits`), or one named `reference*`. A reference made by an earlier call is sent as a data URI of the downloaded file when it is under the API's 5 MB, else as a fresh URL of its task's output.

### Tests

game-printer's `crates/gp-cli/tests/theme_generate.rs`, not carried here, ran gp against a mock of the API on 127.0.0.1: a run in order with each result downloaded, a refunded failure made again and a safety refusal not, rate limits, a run killed with a task in flight and continued with no second request, the budget's stop and its continuation, a paid run with no terminal and no `--yes`, and a wrong key that shows in no output and a right one in no file. No test calls Runway.
