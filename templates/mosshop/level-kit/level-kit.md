# Level kit: Moss Hop (`mosshop`)

Written by `gp level kit mosshop` from the model, the asset map and `level-kit/rules.toml` (docs/level-kit.md). Edit the rules file, not this one. It holds rules, sizes, counts and boxes, never a table of the game's ROM. `.claude/skills/level-design/SKILL.md` says how to design a stage from it by hand.

A stage is one level two screens wide (32 by 15 cells of 16 px). The hero runs and jumps from the left, stomps or avoids slugs that patrol between two places, bumps a ? block, picks up coins, and reaches the burrow at the right. Falling below the level costs a life.

## The levels

| Level | Size | Bounds | Goal |
|---|---|---|---|
| `main`: Two screens wide, ending at the burrow, with a ? block and five coins | 32 by 15 cells of 16 px | the level is 512 by 240 px; the screen 256 by 240; a body below row 208 is out | the hero's left edge at x 468 or past |

What `main` starts with (the model's spawns, places in pixels from the level's top left):

| Thing | Role | x | y |
|---|---|---|---|
| `player` | player | 24 | 176 |
| `slug` | enemy | 0 | 176 |

## What's data and what's fixed in code

A stage changes what is data. What the code fixes, a stage keeps to: change it and the stage no longer runs on the game's own engine.

| | What | Rule | Slot | Source |
|---|---|---|---|---|
| data | The level | A grid of 32 by 15 cells, a character a cell from the legend .MSB()?uo (sky, moss over soil, soil, stone block, the burrow's two halves, a ? block, a used block, a coin). The used block `u` is never written: a bumped ? block becomes one | `level` | the asset map; games/mosshop/mosshop.s, level_row |
| data | The metatiles | 9 metatiles of 4 tiles each, with a solid byte and a kind byte: solid cells stop the hero, kinds mark the ? block and coins | `metatiles` | the asset map |
| data | The slugs | 2 slugs, each a start and an end x in pixels: it walks right from start to end and back, on row 176 | `slugs` | the asset map; the model's spawn table |
| code | The level's size | 32 by 15 cells; the camera follows the hero across, never back past the level's left edge |  | the model's cameras.main and the asset map |
| code | The goal | The hero's left edge at x 468 or past: the burrow must stand there |  | the model's levels.main.goal |
| code | Falling out | A body below row 208 is out and the hero loses a life (player.fell) |  | the model's kill_below and pit_event |
| code | The hero's start | x 24, y 176: the start cell must have solid ground under it |  | the model's spawn |

## Limits

| What | Limit | Why | Source |
|---|---|---|---|
| Cells | 32 by 15, 480 cells |  | the asset map (level: width 32, height 15) |
| Slugs | 2 |  | the asset map (slugs: 2 rows) |
| Metatiles | 9 |  | the asset map |
| A jump's reach | about 50 px up, 40 px across walking and 80 running (the hero's moves below; Badge In, which takes this jump, measures 48, 40 and 80 with gp interp reach); a gap wider or a step higher can't be crossed | every level must stay reachable: gp interp reach checks it | the model's run and jump |

The data slots the levels read, as the asset map holds them:

| Slot | Kind | Format | Size | What |
|---|---|---|---|---|
| `level` | level-grid | level-grid | 32 by 15 cells |  |
| `metatiles` | metatiles | metatiles | 9 rows of 1 column | Sky, moss over soil, soil, stone block, the burrow's two halves, the ? block, the used block it becomes, and a coin |
| `slugs` | table | table | 2 rows of 2 columns | Each slug's patrol: start and end |

## The hero and the things in a stage

From the model. A gap the hero must jump, a ladder it must reach and a fall it must survive are measured against these.

| Move | The model's numbers |
|---|---|
| fall | at most 4 px a frame |
| jump | up at 5 px a frame, gravity 0.25 px a frame each frame: about 50 px high and 40 frames in the air on flat ground (v*v/2g and 2v/g) |
| jump across | about 40 px walking, 80 running |
| run | 2 px a frame |
| walk | 1 px a frame, 60 px a second at 60 frames |

| Thing | Role | Size | Touch box (x, y, w, h from its place) | A touch |
|---|---|---|---|---|
| `player` | player | 16 by 16 | 2, 0, 12, 16 | the hero's box |
| `slug` | enemy | 16 by 16 | 2, 4, 12, 12 | kill-player |

The touch boxes are the model's, as its touch tests read them. The theme kit (`themes/kit/kit.toml`, each slot's `geometry`) gives each in its picture's pixels, as the screen shows it.

## Play measures to land near

None yet (see Open).

## Originality

`gp template check T --original` compares a stage derived from this game with the original's, lifted in memory from your ROM. It is refused at cells 0.6 or ladders 0.5. What counts as copying:

- the layout: 60% or more of the cells either level fills (ground, ladder) the same kind in both, at the best shift of up to 4 cells each way, mirrored or not
- the ladders: 50% or more of them within a tile across of one of the original's, between the same floors counted from the bottom, at that shift
- the data: any line of 24 characters or more of the stage's built files that the original's lifted files hold too (gp template served's rule)
- tracing: a stage drawn over the original's picture, or laid out from a guide made from the ROM, is a copy whatever it scores
- the art: a theme's sprites and pictures are held to the theme kit's originality check (docs/theme-kit.md, "The originality check")

## How to verify

| Step | How |
|---|---|
| 1. Reachable | gp interp reach templates/mosshop: the goal reachable by the hero alone |
| 2. The scenarios | gp host scenarios on a copy (templates/mosshop/AGENTS.md): a scenario that fails after the change names a behavior that moved |
| 3. The ROM | Moss Hop builds from its source with cc65; a new level is written in games/mosshop/mosshop.s's level_row lines and the build's asset test checks the offsets |

## Open

- The play measures a Moss Hop level should land near aren't taken: gp level measure is for stages of girders and ladders; a measure of a scrolling level (the jumps' margins, gp level difficulty) would give them
- The room left in the ROM for a bigger level isn't measured: the grid's size is fixed at 32 by 15 in the source
- Originality has no base to compare with: Moss Hop is an original, so a stage of it is held to nothing but its own checks
