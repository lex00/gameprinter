# Genres and what each game adds

Brought from game-printer's docs/research/ as written on 2026-11-12. Its findings are game-printer's; doc paths gameprinter doesn't have (docs/model.md, the topdown, gridhop and events notes) are game-printer's, and Donkey Kong is games/donkey-kong in the games checkout here.

A running list of the genres the model has or plans, the games in each, and what a game adds to its genre. A game added to a genre that already exists gets a row here; a new genre gets its own design note.

| Genre | Extension | Note | Games |
|---|---|---|---|
| Platformer | `platformer` 0.4 | docs/model.md, "Platformer extension" | Super Mario Bros., Mega Man 2, Super Mario Bros. 2, Ninja Gaiden, Kirby's Adventure, Moss Hop, Donkey Kong |
| Top-down maze | `topdown` 0.1 | docs/research/topdown-extension.md | Gauntlet, Gauntlet II |
| Isometric grid hop | none yet (design) | docs/research/gridhop-extension.md | Q*bert |
| Sports events | none yet (design) | docs/research/events-extension.md | Track & Field |

## Donkey Kong in the platformer genre

Donkey Kong (`donkey-kong-ju`, mapper 0) is a platformer and uses the platformer extension. It adds, or leans on, these parts:

| Part | In Donkey Kong | In the platformer extension today |
|---|---|---|
| Ladders | Mario climbs up and down ladders, some of them broken, and can't jump while on one | `climb` exists; broken ladders (a top that can't be climbed past) would be a new option |
| Fixed-arc jumps | a jump's arc is fixed at takeoff: no air control, no height from holding the button | `jump` with `run.air_control = false` and no `gravity_held` |
| Rolling and falling hazards | barrels roll along girders, fall down ladders at random and at the ends, and fireballs climb | `walk` with `turn_at_ledges` covers rolling; dropping down a ladder by chance is new (a random choice from the seeded RNG) |
| A hammer with a timer | the hammer power-up destroys barrels for a while; Mario can't climb or jump while holding it | `invincible` while a counter holds is close; "can't climb or jump" while it runs is new |
| Single-screen stages, goal at the top | each stage is one screen; reaching the top (Pauline) clears it | the core's levels with a fixed camera, and a `goal` at a place rather than an x |

It plays like the arcade original and like other single-screen climbing platformers (Mario Bros., Donkey Kong Jr.).

### What the platformer extension needs for Donkey Kong

Measured from the user's clips and from probes run from the user's path: a walk of about 2 px every 3 frames with an instant turn and stop, girders that slope 1 px per 24 px, and a jump that rises 13 px and lands 40 frames later whether A is tapped for 1 frame or held for 31. A walking jump drifts 20 px forward and a standing jump 0, and holding Left or Right in the air changes neither.

Checked against the interpreter, two of the parts first proposed are already expressible:

| Part | Already expressible as | Why |
|---|---|---|
| A fixed arc | `run.air_control = false`, and a jump with no `gravity_held` | gp-sim's run system leaves the speed as it was in the air when `air_control` is off, so the takeoff drift carries through and the pad does nothing; with no held gravity the height doesn't depend on the hold |
| A broken ladder | level data: ladder cells that stop below the girder | `climb` stops when the body's middle leaves the ladder |

The parts that aren't, each a named feature of the platformer extension (a feature-list row, no version bump), built once its evidence is in:

| Feature | What it states | Status |
|---|---|---|
| `slope` | ground whose height steps by 1 px every N px across a tile kind | measured (1 px per 24 px), but its test from the clips needs Donkey Kong's level grid in the model, which isn't lifted yet |
| `power.locks` | while a power-up runs, the actions it takes away (jump and climb, for the hammer) | waits on the hammer sweep |
| `walk.drop_ladder` | a walker that takes a ladder down by a seeded random choice | waits on the barrel sweep |

The blind search had no up and down moves: on Donkey Kong it found the player and forward but stayed on the bottom girder (800 rounds, furthest x 176). It has `climb_up` and `climb_down` now.

Sources for reading only: the NES manual ([Internet Archive, "Donkey Kong (World)"](https://archive.org/download/NESManuals/Donkey%20Kong%20%28World%29_text.pdf)), [The Mushroom Kingdom's NES page](https://themushroomkingdom.net/mariopedia/game/dk-nes/full), and the [arcade guide at NinDB](https://nindb.net/guide/arcade/donkey-kong.html), which the NES version follows. Nothing from them is copied into the repo.

## Gaps

Q*bert and Track & Field are mapper 3 (CNROM). gp-nes runs them, but `gp record` refuses them: the canonical state defines mapper registers for NROM, MMC1 and MMC3 only. Adding CNROM is one CHR bank register in crates/gp-nes/src/canonical.rs (docs/research/topdown-extension.md, "CNROM").

## Quest log checklists

`gp game new GAME --rom ROM --genre G` seeds a new game's quest log with its genre's rows below, in order, each `next` with its line, and opens a mechanic session for each so its clips have somewhere to go. Each row is something a person records in `gp mechanic start GAME ROW`; the code answers what it does (.claude/skills/element-extraction/SKILL.md). `gp mechanic queue GAME ROW --status ...` changes a row and `gp mechanic leads GAME --add` adds the ones the clips missed. The last row of every genre is `sealed-run`, the held-out run no worker sees (docs/sealed.md). A genre's model extension, when it has one, is in the table at the top; a genre with none yet gets a core-only model.

Rows are taken from the Donkey Kong quest log (docs/research/dk-25m-retrospective.md) and each genre's note. In gameprinter the lists live in crates/gp-cli/src/mechanic/checklists.toml, which the binary reads when it is built, with an `other` list added and the `sealed-run` rows left out until the page seals runs; change a genre's list there.

### `platformer`

| Row | What to record |
|---|---|
| `title` | The title screen and the attract demo from power-on: the menu, the demo, and what Start does |
| `walk` | Walking both ways, turning and stopping: the speed, and how a walk starts and stops |
| `jump` | A standing jump, a running jump, a tap and a held jump: the height, the length and any control in the air |
| `climb` | Ladders or vines, if the game has them: up, down, getting on and off |
| `enemies` | Each enemy and hazard moving on its own, with the player out of its way |
| `enemy-contact` | Touching each enemy, from the side and from above |
| `power-up` | Taking each power-up and what it changes, until it runs out |
| `die` | Each way to die (an enemy, a fall, the timer) and what happens until play starts again |
| `score` | What gives points, and the extra life |
| `clear-stage` | Clearing the first stage, and the scene after it |
| `game-over` | Losing the last life: the game over and the way back to the title |
| `sealed-run` | A full run of the first stage, played however you like. Sealed at hand-off: no worker sees it |

### `topdown`

| Row | What to record |
|---|---|
| `title` | The title screen and the attract demo from power-on: the menu, the demo, and what Start does |
| `class-select` | Choosing a character before play, if the game has classes |
| `move` | Moving in each of the eight directions: the speed, starting and stopping |
| `walls` | Walking into walls and corners: sliding along a wall, stopping at a corner |
| `shoot` | Shooting each way: the shot's speed and what stops it |
| `enemies` | Each enemy moving on its own, and what makes them (generators) |
| `pickups` | Taking each pickup (food, keys, potions) and what it changes |
| `doors` | Opening a door, with and without a key |
| `health` | Health over time: what drains it and what restores it |
| `exit` | Taking an exit, and the level after it |
| `die` | Running out of health, and what happens next |
| `game-over` | The game over and the way back to the title |
| `sealed-run` | A full run of the first level, played however you like. Sealed at hand-off: no worker sees it |

### `gridhop`

| Row | What to record |
|---|---|
| `title` | The title screen and the attract demo from power-on: the menu, the demo, and what Start does |
| `hop` | A hop in each of the four directions: the frames it takes, its arc, and when the next press counts |
| `cells` | What a landing does to a cell, and how the level is cleared |
| `fall-off` | Hopping off the edge, and the lost life |
| `discs` | Riding a disc to the top, and an enemy that follows |
| `enemies` | Each enemy's hops, with the player out of its way |
| `caught` | Being caught by each enemy, and what happens until play starts again |
| `level-clear` | Clearing the first level, and the scene after it |
| `game-over` | Losing the last life: the game over and the way back to the title |
| `sealed-run` | A full run of the first level, played however you like. Sealed at hand-off: no worker sees it |

### `events`

| Row | What to record |
|---|---|
| `title` | The title screen and the attract demo from power-on: the menu, the demo, and what Start does |
| `run` | Pressing the two buttons in turn: how speed builds and drains |
| `timed-action` | The timed press of each event (a jump at the line, a throw): early, on time and late |
| `hold-angle` | Holding a button to set an angle, and letting go at several angles |
| `foul` | Crossing the line before the action |
| `qualify` | Reaching the qualifying mark, and the next event |
| `miss` | Missing the mark, and the tries left |
| `game-over` | Running out of tries: the game over and the way back to the title |
| `sealed-run` | A full run of the first event, played however you like. Sealed at hand-off: no worker sees it |
