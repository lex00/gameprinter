# The level kit

A level kit is what a designer needs to make a stage for a game by hand: the stage's format (what is data and what the game's code fixes), its limits with numbers, the play measures a stage of its kind should land near, the originality rules, how to verify a stage, and a worked example. The user, 2026-11-04: "level designers can hopefully succeed without the tool if the kit has enough for them to work with, even by hand they should have all they need". `.claude/skills/level-design/SKILL.md` is the procedure; the theme kit (docs/theme-kit.md) is its counterpart for art and sound.

```sh
gp level kit GAME [-o DIR]
```

GAME is a folder holding template.toml, a template id under `templates/` (Moss Hop), or a game's folder in the games checkout (`donkey-kong`, `tapper`). In game-printer every game was under `templates/`; in gameprinter only Moss Hop is, and the commercial games' kits live with their packages.

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
| What fits | `[[fit]]`: findings from packing a stage into a copy of the ROM and running it on the real engine |
| The worked example | `[example]`: `template` and paragraphs of `text`; the kit adds the example template's spawns from its model |
| Open | `open`: what isn't known yet, each with the tool that would find it where one is named |

## The kits

| Game | What the kit holds | Open |
|---|---|---|
| Donkey Kong (`donkey-kong-oe`, until 2026-11-06 `donkey-kong-ju`) | 13 data rules (girder runs and shapes, ladders, tops, heights, gaps, the slope list, barrel ladders, broken ladders' stops, straight drops, hammers and the start, the stage picture, the demo), 10 fixed in code (2 runs on the bottom row and 1 on each other, sloped ends, the fires' 4 ladders, the third barrel ladder on girders 4 and 5, the throw's height, a pushed barrel's drop, the drum, the screen's edges, the goal and the top platform, the thrower's place), 11 limits (8 runs, 3 shapes, 13 ladders, 4 broken, 9 tops, 2 hammers, the screen, 1258 bytes for five screens with 449 for 25m), the 11 measures of Rivet Rodeo's table with their tolerances, 6 steps to verify, 6 findings on what fits and Rivet Rodeo as the worked example | 4 |
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
| The real engine | Pack the slots into a copy of the ROM outside the repo (`gp assets pack` for slots that keep their size), then `gp interp compare T --rom COPY --input LOG --mode free` |
| A theme | `gp theme check T --theme DIR`: the theme kit's geometry |

## Limits

- The rules file is written by hand. What the code fixes was read by people and agents from the model's notes; a detector that finds a table's reader and the immediates it compares with would write it (docs/research/deterministic-inference.md, 2026-11-04).
- The space left in the ROM is counted by hand: no tool yet encodes a layout entry and reports its bytes.
- `gp level measure` measures stages of girders and ladders only, so Moss Hop's kit has no measures.
- An arcade game's kit (Tapper's) has its levels, the hero's moves and the things' boxes from the model; its rules file starts as the stub, as every new game's does.
- A kit is per template, and a template's model holds one stage of Donkey Kong (25m).
