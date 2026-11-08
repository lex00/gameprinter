# Changing Moss Hop

Instructions for an agent working in a copy of this template (made with `template_fork`). Moss Hop is a small side-scrolling platformer: a moss creature runs and jumps through one level, stomps slugs and reaches a burrow. The template is open and ships no game data, so everything here runs on a machine with cc65 and the repo.

Tools are the ten `gp mcp` tools ([docs/mcp.md](../../docs/mcp.md)). The scenario tests are `scenarios.toml` in this directory, run with `gp host scenarios COPY` ([docs/host.md](../../docs/host.md)).

## Before the first edit

1. `surface_describe` the copy. Five slots: `tiles` and `palettes` are art, `level`, `metatiles` and `slugs` are read by the simulation.
2. Run the scenarios on the untouched copy. All 6 pass. Keep that output: after an edit, the scenarios that now fail are the edit's measured effect.
3. Edit with `model_edit` by model path. It checks the model and puts the old file back if the check fails.

## The numbers

| Path | Shipped value | What it does |
|---|---|---|
| `entities.player.jump.gravity` | 0.25 | Pull while rising. The jump's height goes as 1/gravity |
| `entities.player.jump.speed` | -5 | Launch speed (up is negative) |
| `entities.player.jump.max_fall` | 4 | Terminal fall speed |
| `entities.player.run.run_speed` | 2 | Speed with B held |
| `entities.player.run.walk_speed` | 1 | Walking speed |
| `entities.slug.patrol.step` | 1 | Pixels a slug moves each time it steps |
| `entities.slug.contact.bounce` | -3 | The player's bounce after a stomp |
| `entities.slug.contact.stomp_depth` | 7 | How far above a slug the player's feet must be to stomp |

Values are fixed-point: write fractions that are a multiple of 1/65536, such as `0.2109375` (27/128). `0.2` is refused.

## Recipes

Each recipe says what to edit, which scenarios should change, and what to do about them. A scenario that fails after an edit is not an error to hide: it names a behavior that moved. Either the move is the point of the edit (update or re-record the scenario) or it is a mistake (undo the edit).

### Make the jump higher or lower

1. `model_edit` `entities.player.jump.gravity`. The apex goes as 1/gravity: 20% higher is 0.25 * 0.8333 = 0.2083, and the nearest fixed-point value the model keeps is `0.2109375`.
2. `run_preview` with `holds: ["a@50-60"]`, `frames: 130` and `paths: ["entities.player[0].y"]` at the frames you care about. The shipped apex is y 128.5 (47.5 pixels above the ground at 176).
3. Run the scenarios. With `0.2109375`, `jump-height` fails (the player is at y 121.08 at frame 69, outside 124 to 133), and `finish-level` and `slug-death` fail because the shipped inputs no longer line up with the slugs and the burrow. `jump-airtime`, `stands-still` and `walks-right` pass.
4. Update `jump-height` for the new numbers (edit its `at` and `min`/`max`), or delete it and record a new one (below).
5. Raise the contract's level for the jump to `free` with `contract_edit`, since the jump is now deliberately different: `{"op": "set", "at": "entities.player.jump", "rule": {"path": "entities.player.jump", "level": "free", "note": "..."}}`.
6. The shipped runs no longer win. Look for inputs that do, and record them (below).

### Make the player faster

1. `model_edit` `entities.player.run.run_speed` (2 to 3, for example).
2. Run the scenarios. `finish-level` fails: the shipped run reaches the slugs at a different frame, the player dies and `goal.reached` never fires.
3. Look for inputs that win and record them. `walks-right` doesn't hold B and passes.

### Make the slugs faster

1. `model_edit` `entities.slug.patrol.step` (1 to 2).
2. Run the scenarios. `finish-level` fails (the player dies and never reaches the burrow). Look for inputs that win and record them.

### Change how a stomp feels

`entities.slug.contact.bounce` and `stomp_depth` change the stomp. No shipped scenario depends on them, so record one that does: stomp a slug (`finish-level`'s inputs reach one at frame 169), read `entities.player[0].vy` at the next frame, and pin it.

### Give the game new art

Art slots do not change the simulation, and `assets_place` proves it by running every baseline before and after.

1. `assets_generate` with `progressive: true` and `plan_only: true` shows the order: palettes, a base sheet, then the player, then the slugs.
2. `assets_generate` with `progressive: true` does it, with the consistency critic reviewing each piece. The reply lists each step's score and its problems. Read the problems: a warning such as `contrast.backdrop` means the sprite is hard to see against the sky.
3. To redo one sprite, call `assets_generate` with `slot: "tiles"`, `entity: "player"`, a different `seed` and `critic: "enforce"`. A piece the critic fails is not placed.
4. All 6 scenarios still pass after an art change. If one fails, the edit touched more than art.

### Record the variant's own baselines

The shipped baselines and `scenarios.toml` describe the original game. Once the jump or the slugs changed, give the copy runs of its own:

1. Search for inputs that clear the level in the edited model: `gp host search COPY --baseline finish-level --goal rules.game.state=clear --progress 'entities.player[0].x'`. It moves the baseline's holds until the goal is met, and prints the holds.
2. Record them: `gp host record COPY variant-finish-level --frames 300 --hold ...`. This appends a scenario to the copy's `scenarios.toml` that pins the final counters and states, the event counts and the state hash.
3. The copy now has a run that wins on its own rules. Later edits that change that run make the scenario fail.

### Publish

`build_publish`, after `validate` passes or its failures are explained by `contract_edit` levels you set on purpose. The manifest records what differs from the template (`changes`), so a viewer can tell this copy from the original. To get a playable link, `gp host serve` and queue a build job ([docs/host.md](../../docs/host.md)).

## Do not

- Edit the `level`, `metatiles` or `slugs` assets with `assets_place`: the simulation reads them.
- Loosen an Exact contract rule to make `validate` pass. Set a rule to Free only for the thing you changed on purpose.
- Delete a scenario to make a run pass without saying why in the commit or the note.
