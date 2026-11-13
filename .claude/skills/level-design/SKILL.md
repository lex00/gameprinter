---
name: level-design
description: Design a new stage for a game in Game Printer by hand from its level kit (GAME/level-kit/level-kit.md and kit.toml in the game's folder): read the format, keep to what the code fixes and to the limits, lay the stage out on paper or in a script, land the play measures within their tolerances, stay apart from the original's layout, and verify it on the model. Use it when someone asks for a new level, stage or layout for a game, or to check one they made.
---

# Level design

A stage is data the game's model reads: floors, ladders, things and where they start. The level kit says, for one game, what is data and what the game's code fixes, the limits with numbers, the play measures a stage of its kind should land near, the originality rules and how to verify. [docs/level-kit.md](../../../docs/level-kit.md) is the reference. Everything here works by hand; the tools only check.

This skill came from game-printer. GAME below is a game's folder in the games checkout (`$GP_GAMES_DIR`, or `games` beside the tool repo), such as `games/donkey-kong`. That folder is the game's template: `template.toml`, the model (the game's rules and numbers as text, `model.toml`, where the stages live) and no ROM data, and T is the folder of your stage's game. Moss Hop's kit is in this repo, `templates/mosshop/level-kit/`. Tapper has no kit yet.

The rules hold at every step:

- Never copy or trace the original's layout. Don't draw over its picture or lay a stage out from a guide made from the ROM. Design on a blank grid from the kit's rules and numbers.
- Never write ROM data into a repo. A stage is your own numbers.
- Keep to what the kit's code rules fix, or the stage runs only on the model, not on the game's engine.

## 1. Read the kit

The kit is in the game's folder: `GAME/level-kit/level-kit.md`, `kit.toml` and `rules.toml`. `gp level kit GAME` writes the first two from the model, the asset map and `rules.toml`, so after a change to `rules.toml`, run it again. Donkey Kong's kit still names the game by its old id, `donkey-kong-oe`.

Read `level-kit.md` top to bottom. Note:

| Section | What to take from it |
|---|---|
| The levels | The screen and level size in pixels, where a body falls out, the goal, what starts where |
| What's data and what's fixed in code | The parts you lay out (data) and the places and numbers you must keep (code). For Donkey Kong: one run a girder row above the bottom, so each girder slopes end to end; the fires' four ladders and the barrels' third ladder stay where the code has them |
| Limits | Counts and bytes: runs, shapes, ladders, broken ladders, tops, hammers, the space for the stage's picture |
| The hero and the things | How high and far the hero jumps, how fast it walks and climbs, the fatal drop, each thing's size and touch box. A gap wider than the jump carries, or a step higher than it rises, can't be crossed |
| Play measures | The targets and tolerances your stage should land within |
| Originality | The thresholds, and what counts as copying |
| How to verify | The commands, in order (some name commands that aren't here yet; step 4 below has the ones that are) |
| What fits on the real engine | What earlier stages found when they were packed into a copy of the ROM, before Game Printer stopped writing ROMs. Read it as a record: nothing checks a new stage this way now |
| Worked example | A stage explained against these rules |
| Open | What nobody knows yet: don't rely on it |

If the game has no kit (a new game), read the game's code for each data slot first (.claude/skills/element-extraction/SKILL.md) and write what you find into `GAME/level-kit/rules.toml`, with its source, in the shape docs/level-kit.md gives.

## 2. Lay the stage out on a grid

Draw the screen at the level's tile size (Donkey Kong: 32 by 30 cells of 8 px). Place, in this order:

1. What the code fixes: the goal's platform, the start, the places code holds (fires' ladders, a thrower's girder, a drum).
2. The floors, inside the format's rules (Donkey Kong: the bottom row flat then sloped, each other girder one slope of 24 px pieces a pixel apart, the slope places from the shared slope list), about the target gaps apart.
3. The ladders: full ones joining each floor to the next, broken ones as stubs, each where its foot is level (clear of a piece's step). Count against the limits.
4. The things: hammers, pickups, foes' paths, at heights near the targets.

Keep each number your own. Write the stage as a script that emits the slots, not as edited copies of lifted files. game-printer's example, Rivet Rodeo's `scripts/rivet-rodeo/stage.py`, went with Rivet Rodeo (retired by the owner on 2026-11-11); its shape is still the one to follow: one function a slot, each number with a comment saying which rule it keeps.

Your stage's game is a template folder of its own (T) with the base's model and its own stage files, and `[derive] from` in its `template.toml` naming the base (docs/templates.md, "Originals"). `gp template derive T --write` makes those files from the base's; keep the stage's slots your own.

## 3. Check the reach by hand

For each floor, the hero must reach the next by a full ladder, and every jump the route needs must be within the kit's jump: height under the rise, width under the carry, any drop under the fatal drop.

## 4. Verify

Run these in order and keep the output:

```sh
gp model check T
gp template build T --write && gp template check T        # for a stage whose source is authored
gp level measure --template T                             # each measure within its tolerance; --module M for a model with hooks
gp template check T --original                            # with the base game's ROM in $GP_ROM_DIR
gp level measure --template T --clears 3                  # the holds a model baseline takes
gp level difficulty --template T --level L                # how late or early each input can come
```

A measure out of tolerance names what to move: slopes and gaps move floors, the route and climbs move ladders, the barrels' share and the throw's time move barrel ladders, the jump window moves where foes meet the hero. Change one thing, measure again.

The kit's verify steps also name `gp template derive T --write` and `gp interp baselines T`; run them too.

There is no check on the game's own engine any more. `gp assets pack --base`, which wrote the stage's slots into a copy of the ROM to run beside the model, was removed on 2026-11-13, since Game Printer's games play in the browser on the model. A stage is checked on the model alone: the steps above, the kit's code rules kept by hand, and its "What fits" findings from the stages packed before. A rule the engine holds that the kit doesn't list yet won't show up; when the code is read and one is found, add it to the rules file.

## 5. Theme it

A theme keeps the stage's geometry. game-printer's `gp theme check T --theme DIR` flagged a platform whose top edge wasn't the floor line and a ladder left undrawn; it is back with wave 8, for a theme directory rather than a look. A large-art theme builds each stage from pieces over the level picture the player's ROM gives (site/src/content/docs/themes.md), so it follows a new stage's floors and ladders by itself; look at the stage in the page's PLAY tab under each theme. .claude/skills/theme-generation/SKILL.md makes the theme.
