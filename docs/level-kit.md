# The level kit

A level kit is what a designer needs to make a stage for a game by hand: the stage's format (what is data and what the game's code fixes), its limits with numbers, the play measures a stage of its kind should land near, the originality rules, how to verify a stage, and a worked example. The user, 2026-11-04: "level designers can hopefully succeed without the tool if the kit has enough for them to work with, even by hand they should have all they need". `.claude/skills/level-design/SKILL.md` is the procedure; the theme kit (docs/theme-kit.md) is its counterpart for art and sound.

```sh
gp level kit GAME [-o DIR]
```

GAME is a template: a game's folder holding template.toml, given as a path, as a game's name in the games checkout (`donkey-kong`, `tapper`), or as a template id under `templates/` (Moss Hop). In game-printer every game was under `templates/`; in gameprinter Moss Hop and the games made here are, and the ported games' kits live in their folders in the games checkout.

| File | Written by | What |
|---|---|---|
| `GAME/level-kit/rules.toml` | A person, once per game, as its code is read | What the model and asset map can't say: the format's rules, the limits, the target measures, how to verify, what fits on the real engine, a worked example and what is still open |
| `GAME/level-kit/kit.toml` | `gp level kit` | The machine-readable kit: the rules with what the model and asset map say |
| `GAME/level-kit/level-kit.md` | `gp level kit` | The same as a plain-language document |

`gp game new` writes a stub rules file whose parts are all open, and the kit from what the new model knows, for an NES game or an arcade romset alike. The test `level_kit::tests::committed_level_kits_are_current` fails while a kit under `templates/` isn't what `gp level kit` writes now, or holds an em dash or a character outside ASCII; the games checkout's kits are its own to keep current. The code is `crates/gp-cli/src/level_kit.rs`.

A kit holds rules, sizes, counts, boxes and descriptions, never a table of a game's ROM. CPU addresses name where a rule was read, as the model's notes and asset maps do; the bytes stay in the person's ROM.

## What comes from where

| Part | From |
|---|---|
| The levels | The model: each level's size in cells and pixels, the screen, where a body falls out, the goal, its spawns (the things with a picture; `levels` in the rules file picks the levels whose spawns the document lists, as Super Mario Bros.' 56 would bury the rest) and the asset slots it reads |
| The hero's moves | The player's components: walk and run speed, the jump's height and air time on flat ground (`v*v/2g` and `2v/g`, with the held gravity when the model has one), how far a jump carries, the fall speed, the fatal drop, the climb's speed |
| The things | Each entity with a picture: its size, its touch box and what a touch does. The theme kit's `geometry` gives the same boxes as the screen shows them |
| The data slots | Every slot a level names, and every slot a rule names: its format and size in the asset map (rows and bytes a row, a region's bytes and the entries that share it, a grid's cells) |
| Data and code | `[[data]]` and `[[code]]` in the rules file: `what`, `rule`, `slot`, `source` |
| Limits | `[[limit]]`: `what`, `limit` (with its number), `why`, `source` |
| Measures | `[[measure]]`: `name` (as `gp level measure` prints it), `what`, `target`, `within`, `example` (the worked example's value), `source` |
| Originality | The thresholds `gp template check --original` refuses at (`template_original::CELLS_REFUSE`, `LADDERS_REFUSE`, the shift `stage::SHIFT_REACH`) and what counts as copying, in words |
| Verify | `[[verify]]`: `step`, `how` |
| What fits | `[[fit]]`: findings from packing a stage into a copy of the ROM and running it on the real engine, kept as a record (Game Printer stopped writing ROMs on 2026-11-13) |
| The worked example | `[example]`: `template` and paragraphs of `text`; the kit adds the example template's spawns from its model |
| Open | `open`: what isn't known yet, each with the tool that would find it where one is named |

## The kits

| Game | What the kit holds | Open |
|---|---|---|
| Donkey Kong (`donkey-kong-oe`, until 2026-11-06 `donkey-kong-ju`) | 13 data rules (girder runs and shapes, ladders, tops, heights, gaps, the slope list, barrel ladders, broken ladders' stops, straight drops, hammers and the start, the stage picture, the demo), 10 fixed in code (2 runs on the bottom row and 1 on each other, sloped ends, the fires' 4 ladders, the third barrel ladder on girders 4 and 5, the throw's height, a pushed barrel's drop, the drum, the screen's edges, the goal and the top platform, the thrower's place), 11 limits (8 runs, 3 shapes, 13 ladders, 4 broken, 9 tops, 2 hammers, the screen, 1258 bytes for five screens with 449 for 25m), the 11 measures with their tolerances, 6 steps to verify and 3 findings on what fits; no worked example since Rivet Rodeo was deleted | 4 |
| Moss Hop (`mosshop`, in this repo) | The level grid (32 by 15 cells, the legend), its 9 metatiles and 2 slugs; the size, goal, fall and start fixed; the jump's reach | 3: no play measures, no room measured, no base for originality |

## Donkey Kong's rules, and where they came from

The format's rules were read from the model's notes and asset map, from worker packP's pack of Rivet Rodeo into a copy of Donkey Kong (2026-11-04, `$GP_HOME/work/workers/packP/`, a script outside the repo that writes Rivet Rodeo's lift into 25m's places), and from worker gcFit's notes while it redesigned Rivet Rodeo within the format (`scripts/rivet-rodeo/stage.py` on its branch). The two that shape a stage most: the walk's ground test reads the bottom row as two runs and each other row as one, so a girder above the bottom slopes from end to end; and the fires' four ladders and the barrels' third ladder are numbers in the code, so a stage that runs on the real engine keeps ladders where they are or changes code.

packP's findings on what fits: Rivet Rodeo's girders (9 runs, 54 bytes) are 6 bytes over 25m's 48; its fourth ground shape had to move with its pointer; its stage picture (569 bytes) fits only with 75m's and 100m's pictures dropped; its ladders, tops, stop boxes, barrel ladders, slope list, hammers, start and demo fit as data; the fires' ladders and the third barrel ladder fit only by changing 16 numbers in code. With it packed, runs of the same inputs on the copy first split from the model, where Donkey Kong's own don't, at model frames 640, 705, 811 and 1079.

## Verifying a stage

| Step | Command |
|---|---|
| The model's own checks | `gp template derive T --write`, `gp template build T --write`, `gp template check T` |
| The play measures | `gp level measure --template T` against the kit's table; `--clears 3` for a baseline's holds |
| Originality | `gp template check T --original`, with the base game's ROM in `$GP_ROM_DIR` |
| Baselines | `gp interp baselines T` |
| A theme | `gp theme check T --theme DIR`: the theme kit's geometry |

## Editing a stage

An agent reads and edits a stage for a person over MCP (`gp_run`), and the same commands work by hand. They read and write the stage in the kit's own terms: the slots its rules name, row by row, and each thing's start in the model. Only a person's copy is ever changed.

Where the slots are read from: a copy's own `assets/` when it has one; else the game's `level-data/` folder when it has one; else the lift of the person's ROM in `$GP_HOME/work/assets/SOURCE`. A `level-data/` folder holds a level that is authored data, not lifted: the asset map (`assets.toml`) and the files of the slots the simulation reads, in the lift's formats. Moss Hop's level is authored so (`templates/mosshop/level-data/`: `level.toml`, with the `metatiles.toml` and `slugs.toml` it needs). `gp level show`, `edit`, `undo` and `verify` read it with no ROM, no lift and no cc65, an edit of a copy writes the copy's `level-data/level.toml`, and such a game is its own original. The ROM is built from the same file (`scripts/mosshop/build.sh` writes its rows as `level.inc`, which `games/mosshop/mosshop.s` includes). `crates/gp-cli/tests/level_data.rs` reads and edits it in a home with no lift and no cc65, and `gate_mosshop.rs` holds the built ROM's lift to the three files byte for byte. The game's code and pictures still come from the assembly build: the baselines, the presentation and the gate's game check run the ROM.

| Command | What |
|---|---|
| `gp level list GAME` | The levels: what each is, its size, its starts, the slots its pieces are in, whether the kit's rules cover it, and whether the folder is a copy an edit may change |
| `gp level show GAME [--level L] [--data]` | One level as data: its girders (the ground's runs by row, each with its ends and rows and the girder it is on), ladders (the girders each joins, whether it reaches the one above, its top or stop), each thing's start, the kit's rules, limits and measures, and any rule it breaks now. `--data` adds every slot's rows and each start under `data`, the shape `--from` takes back |
| `gp level render GAME [-o FILE] [--art]` | A PNG by what each piece is, with no ROM art, numbered as the operations name the pieces; it can be shown anywhere. `--art` draws the model's own picture at the start of play from the person's lift under the pieces' outlines, and stays on the computer |
| `gp level edit GAME --op OP ... [--from FILE] [--dry-run]` | Named operations, run in order and written together, or the level's data replaced whole: checked before anything is written, kept for undo, and answered with the picture, the distance from the original's layout and the play measures against the kit's tolerances |
| `gp level undo GAME` | Puts the newest edit back |
| `gp level verify GAME [--level L]` | The kit's rules, whether the stage can be finished with the enemies in (by the kind of stage, below), every play measure against its tolerance (the barrels' routes and jump windows too, on a girder stage), and the distance from the original's layout. Exits 1 when a rule breaks, it can't be finished or it is too close to the original, and when it couldn't be played to its end (NOT CHECKED, below) |

On an arcade model the levels are in the board's own units when the state mapping says `board_units = true` (docs/formats/state-mapping.md, "An arcade original"): `gp level list` counts their cells in units (8 on MCR, 2 screen pixels each), `gp level show` gives the starts in them, and `gp level render` draws by what each piece is at 256 by 240 units, the NES's size; with `--art` it draws the board's own picture of the start of play, the bodies placed as the board draws them, under the pieces' outlines at the same size. An unmarked arcade model (Tapper's today) draws as before. `gp level verify` and the girder and ladder operations of `gp level edit` refuse an arcade level by name: the stage measure (`gp_interp::stage`) plays girders and ladders with the NES's pad, and no stage measure reads an arcade level yet.

`gp level verify` plays the level it names, reached as the game reaches it: power-on, Start, and each stage before it in the model's stage order (`[stages]`, else each level's `next`) ended at its goal. The hero is put at that stage's goal, so the model's own goal rule, its clear and the next stage's setup run; nothing is read from a ROM or a recording, and the output names the stages ended on the way. A stage after one with no goal can't be reached this way, and says so. How it plays the stage depends on what the stage is:

| The stage | How verify plays it | Finished when |
|---|---|---|
| Girders and ladders with a route up its ladders to the top | The direct route: walks and ladder climbs to the top girder, then played with the enemies in, waiting before each walk | The goal is reached |
| Girders whose ladders reach no route to the top, or whose ground carries the player on a body (platformer `level.ground.bodies`, Donkey Kong's 75m elevators) | A search in the whole world (`gp_interp::finish`): from each place the player stands, breadth first, it walks a short or a long way, walks to a ladder, jumps up or either way, climbs up or down, and waits, everything in play. Waiting is a move, so it waits for a carrier and rides it; beside a carrier's column a place is kept with the carriers' rows, so the same spot with the platform elsewhere is another place | The goal is reached |
| No goal: the game's rules end it some other way (Donkey Kong's 100m, every rivet pulled) | The same search, a leg at a time: each leg until one of the level's pickups (`level.pickups`) is taken, the next from there | Play leaves the stage with the player alive and the game goes on to another level; the frame play left it is the one said |
| Tiles with a platformer player and a goal (Moss Hop's) | The reach search, the player alone, then the playthrough with every enemy in (`gp_interp::reach`, as `gp interp reach`), from the stage's own start | The goal is reached |

The search's moves are a sample: "could NOT be finished" after the search means every place those moves reach was searched and none finished it, and a search that stops at its limit (12000 places a leg) says NOT CHECKED, never a pass. A stage it can't play at all (a level of fewer than two girders that isn't a tile stage, a model with no game machine to tell a stage's end by) is NOT CHECKED with the reason. The rules the kit states for the stages it covers (the barrels' and fires' code ladders, the goal reached by full ladders; on a tile stage the hero's start on ground and the goal inside the level) are held on those stages only; the kit covers a tile stage when one of its data rules names the level's grid slot. A game made here (its template says `how = "build"` or `"authored"`, Moss Hop) has no original's layout to be kept apart from, and says so.

On Donkey Kong's own stages and Moss Hop's (2026-11-14, one at a time on a debug build): 25m is finished at frame 1754 (81 s), 50m after 25m at frame 2205 (12 s), 75m after both at frame 2293, riding the elevators on 4 of the run's moves (2148 places searched, 169 s), and 100m after the three ends at frame 3642 with its 8 rivets and 2 of its 3 prizes taken, the game going on to 25m at frame 4416 (591 places, 39 s); each Donkey Kong stage also fails as the original's own layout, which verify refuses for a new stage. Moss Hop's stage passes, finished at frame 323 (477 places, 3 s). 75m's search is the slow one, about 6000 frames a second through Donkey Kong's hooks on a debug build: a release build of gp (about ten times faster), or searching toward the goal first instead of breadth first, would bring it under a minute

GAME is a copy's name (`template_fork`'s, under `$GP_HOME/work/mcp`), a folder holding `model.toml`, a name under `templates/` or a game in the games checkout. Reads take any of them; an edit takes only a copy in this home's work area that is outside any git work tree and not a package as `gp game fetch` writes it. A games checkout's original and a published package are refused in words, from the command itself, so the refusal holds over MCP and by hand. `gp_run` runs list, show, render and verify as reads, and edit and undo as writes to the person's copy, with no ask, as other edits to a copy are (crates/gp-mcp/src/policy.rs).

The operations:

| Operation | What it changes |
|---|---|
| `move girder N x=.. y=.. shape=.. step=.. count=.. dx=.. dy=..` | Row N of the ground's runs (`ground_runs`): its place, its pieces' shape, the step across and how many pieces |
| `move ladder N x=.. y=.. shape=.. dx=.. dy=..` | Ladder N (`ladders`). Its top (a full ladder's, in the climb's list) moves with it, and a broken ladder's stop goes to the girder above it. Moved across only (`x` or `dx`), it is fitted where it lands, as the game's own are: a full ladder runs from the girder above's row to its own girder's, with the shape of that height, and a broken one keeps its height with its foot on its girder. No shape of that height is refused with the heights there are |
| `move spawn ENTITY[#K] x=.. y=..` | The K-th start of that thing (from 0) in the copy's model |
| `make ladder N full` or `broken` | Gives the ladder a top and takes its stop, or the other way, within the game's lists' room (9 tops and 4 stops on 25m) |
| `set row SLOT ROW V,V,..` | One row of any slot the kit names as the stage's data (`play_heights`, `play_gaps`, `play_tables` ...); a table of one column takes several values for rows in turn |

The ladders and the runs are lists the game keeps at a fixed size in fixed places, so a stage always has the same number of each: `place` and `remove` of a ladder or a girder are refused in words, and a person's "remove that ladder" is a move. The tops and stops are lists with an end mark, so they shrink and grow up to their room.

Before anything is written an edit is held to:

- the format: every slot keeps its rows and widths, every value is a byte, a shape is one the slot has, and the level compiles;
- the screen: each girder piece starts on it, no run's x wraps past an edge, each ladder is on it;
- what the code fixes, from the model's numbers: the runs make the girders the code reads (as many, each row on the same girder as the original's rows), and each ladder an entity's code climbs (its params `ladder_<k>_x` with `_foot`, or `ladder_c<k>` with `ladder_cfoot<k>`: Donkey Kong's fires' four and the barrels' third) has a ladder at its middle and foot. A refusal names the kit's rule by its `what`;
- the start: the hero stands on a girder;
- the goal: full ladders lead from the start's girder to one the goal can be reached from, and, with the play measures on (the default), the hero walks and climbs there on the model. A ladder whose box reaches the girder above but that the hero can't climb on the model is named;
- the room of the stage's picture (below).

The rules the checks can't hold a stage to (the throw, a pushed barrel's drop, the drum, the screen's edges, the thrower's place on Donkey Kong) come back as `kept_by_hand`. When girders move, the reply names the other data the kit says follows them (the girder heights, the gaps under the girders' ends, the hammers' table, the demo), which an edit leaves to the person or agent (`set row`).

The stage's picture is play data on the model. Donkey Kong's model climbs by the picture's tiles (`climb.by_feet`: a climb holds while the hero's feet are in a ladder tile and ends on a girder or ladder tile's standing row, `tile_kinds` and `tile_surfaces`), so a ladder moved away from where the picture draws one can't be climbed. An edit that moves girders or ladders redraws the picture's play tiles in its PPU string (`stage_screens`, the level's `layout`): the cells the pieces covered are cleared, and each piece is drawn again with the tiles the original's picture uses for that kind and standing row, learned from the person's lift. Its other updates (palettes, attributes) stay as they are. The reply gives its bytes against what the original's took (448 for 25m): over that it fits only with other stages' pictures dropped, as the kit's limit says (894), and past that the edit is refused.

Each edit keeps every file it changed as it was, in the copy's `level-edits.jsonl`, the way a copy's sound edits keep theirs in `sound-edits.jsonl`; the first edit gives the copy its own assets (`<copy>/assets/`, made from the person's lift as the art tools do), so the lift in `$GP_HOME/work/assets/` is never written. The distance from the original's layout is the originality check's (`template_original::CELLS_REFUSE` and `LADDERS_REFUSE`, at the best shift of up to `stage::SHIFT_REACH` cells, mirrored or not), on cells drawn from the pieces' boxes, against the original's stage read from that lift in memory.

A game with no level kit (Tapper today) is answered in words by show, edit and verify; list and render still work. The code is `crates/gp-cli/src/level_edit.rs` and `level_picture.rs`.

## Regions of a stage

A large-art theme can draw a body's picture only in a region of the stage (`#+ in REGION pic ...`, docs/formats/theme.md, "Large-art rules"), so a tall hero can be shorter on a ledge just under the HUD (issue #31). The game's level kit names the regions:

| Part | What |
|---|---|
| The key | In `GAME/level-kit/rules.toml`, `[[region]]` tables: `name` (lowercase letters, digits, `-` and `_`, unique in the game), `level` (a level's name as the kit's `[[levels]]` give it, `main`), `box = [x, y, w, h]` in that level's pixels, `what` (in words: "the top girder, where the goal is") and `source` (where it was read, as the other rules give it). `gp level kit` refuses a name of other letters, a name given twice, a level the model hasn't got, an empty box, a box not inside its level and an empty `what`. It copies each into `kit.toml` as `[[levels.regions]]` (`name`, `box`, `what`) under its level, and lists them in level-kit.md under the levels |
| Who writes it | A person, in the rules file, as the rest of it: a coarse box drawn around a part of the stage the model already describes (a band of rows, a ledge), never a table copied from the ROM. A kit already holds boxes (each thing's touch box); this is one more |
| Who reads it | `gp theme look check` (`theme_look::Game::regions`, from `kit.toml` by `gp_web_lift::regions`): the names, to refuse a theme's region the kit hasn't, or every region of a game with no level kit. The build: gp-web-prep, and gp-web-lift for the public page (build-lift.sh ships `level-kit/kit.toml` in the lift's folder when the kit names a region), write the game's regions into the preview's `preview.json` as `regions: [[name, level picture, x, y, w, h]]`, the level picture by the build's own name (`level-main.png`), and leave the key out for a game with none. preview.js gives them to the game's module once, after its first load, with `gp_set_regions` (gp_web's `Preview::set_regions`, which calls `Art::set_regions` on every look resolved after, and keeps them through a restart in new settings) |
| A real user | Moss Hop's kit names one region, `start`: where the hero starts, `[0, 160, 64, 32]` on `main`. The page check's theme-in-region section draws a made-up theme with a shorter hero there and checks it is drawn only there |

Format notes, for a gp or a page older than the key:

- An older `gp level kit` refuses a rules file with `[[region]]` ("unknown field `region`"), since the rules file's reader takes no field it doesn't know (`level_kit::Rules`, `deny_unknown_fields`); the same holds for an unknown key inside a region. The games checkout's rules files can take the key once the gp that reads them does.
- Nothing but `gp theme look check` and the build reads `kit.toml`. A `kit.toml` with no region reads as before.
- An older page ignores a `regions` key in `preview.json` (it reads the keys it knows by name), and a module from before `gp_set_regions` is given none (preview.js asks the module first): either way it takes the `#+ in` lines for notes and draws each body's full picture everywhere. A newer page with a build that has no regions draws the same.
- An arcade game whose board draws its art (gp_board.wasm) isn't given regions yet: its `#+ in` lines hold nowhere.

## Limits

- The kit's data rule for Donkey Kong's stage picture says play never reads it. On the real engine that holds; on the model it doesn't (the climb reads its tiles), which is why `gp level edit` redraws it. The rules file is the games checkout's to correct.
- Nothing checks a stage on the real engine. That step packed the slots into a copy of the ROM with `gp assets pack --base` and ran `gp interp compare T --rom COPY --input LOG --mode free`; the pack was removed on 2026-11-13. A stage now passes on the model and the kit's rules alone, so an engine rule the kit doesn't list yet goes unseen until someone reads it in the code.
- The rules file is written by hand. What the code fixes was read by people and agents from the model's notes; a detector that finds a table's reader and the immediates it compares with would write it (docs/research/deterministic-inference.md, 2026-11-04).
- The space left in the ROM is counted by hand: no tool yet encodes a layout entry and reports its bytes.
- `gp level measure` measures stages of girders and ladders only, so Moss Hop's kit has no measures.
- An arcade game's kit (Tapper's) has its levels, the hero's moves and the things' boxes from the model; its rules file starts as the stub, as every new game's does.
- A kit is per template, and a template's model holds one stage of Donkey Kong (25m).
