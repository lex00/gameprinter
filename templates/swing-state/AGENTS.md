# Swing State

An original template: a campaign as a platformer, a game of one to three minutes. The candidate runs the trail from a town hall through the debate to election night, stomping hecklers (who keep shouting on foot), timing pundits who rise from their lecterns, jumping attack ads aimed at them and picking up small-dollar donations. Approval is the lives counter and days until the vote are the clock.

The mechanics are composed from two sources (docs/originals.md, "Composition"):

| Part | Taken from | Converted |
|---|---|---|
| The candidate's run and jump, with its bands and held gravity | Super Mario Bros., `entities.mario` | Taken whole; the candidate is the scale |
| Hecklers: walk, fall, hop, and a stomp that leaves a walker (`become`) | Super Mario Bros., `entities.paratroopa_green` | Unchanged |
| Pundits rising from lecterns | Super Mario Bros., `entities.piranha_plant.path` | Unchanged |
| Attack ads | Super Mario Bros., `entities.bullet_bill.fly` | Aimed at the player instead of left |
| Pollsters' patrols, from a spawn table per level | Moss Hop, `entities.slug` | Kept at half the player's walk: 3 pixels every 4 frames |

## Files

| File | Holds |
|---|---|
| `template.toml` | The manifest: source `how = "authored"`, the pins `gp template build --write` keeps current, and the slots |
| `model.toml` | The model (docs/model.md), canonical |
| `fidelity.toml` | The contract |
| `authored.toml` | The assets, written by hand: metatiles, levels, palettes, spawn tables, and the stub tiles the build generates |
| `baselines/<level>.toml` | One model baseline per level: an input that plays it from its start to its goal (docs/originals.md, "Model baselines") |
| `../../assets/swing-state.toml` | The asset map the build writes. Don't edit it |

## Build, play and check

```sh
gp template build templates/swing-state            # the data image and lifted files, into $GP_HOME/work
gp model check templates/swing-state               # schema, evidence, contract
gp model fmt --check templates/swing-state
gp interp reach templates/swing-state              # every goal reachable by the player's own moves
gp interp baselines templates/swing-state          # each level's recorded run still plays the same
gp template check templates/swing-state            # asset-free, pins current
```

`just originals` runs all of these on the three originals (docs/originals.md).

## Change recipes

Every change ends the same way: `gp template build templates/swing-state --write` if `authored.toml` changed, `gp model fmt templates/swing-state`, then `gp interp reach templates/swing-state --record` to record new runs, and `gp interp baselines templates/swing-state` to check them. A change to movement, an enemy or a level changes the digests, so the runs are recorded again rather than edited by hand. Read `git diff templates/swing-state/baselines` before committing: a run that got much longer or now ends in another state says the change did more than intended.

| To | Change |
|---|---|
| Edit a level | Its `rows` in `authored.toml`, one character per 16x16 cell, using the metatiles' `char`s. Keep 15 rows. Move the goal (`levels.<name>.goal.x`) with the goal tiles. `gp interp reach` must still say "reachable" |
| Add a level | A `level-grid` asset in `authored.toml`; a slot `[assets.<name>]` with `from = { source = "swing-state", asset = "<name>" }` and role `layout`; the slot in `template.toml`'s `[slots]`; a `[levels.<name>]` table with one player spawn; `next` from the level before it; and raise the win condition (`if` on `counters.stops` in `rules.game`) |
| Retune the jump or the run | `entities.candidate.jump` and `.run`. Keep the evidence note honest: a borrowed value that changes is no longer the source's, so say so in its note. `gp interp reach` prints the new envelope; check the widest gap still fits it |
| Add an enemy | An entity with a role and its components (docs/model.md, "Platformer extension") and a spawn in a level. A stomp needs `stomp_depth` at least the player's `max_fall` (4), or a fast fall can pass through it |
| Change the time per level | `levels.<name>.counters.days` |
| Restyle | Palettes in `authored.toml`, or art through the MCP tools (`assets_generate`, `assets_place`) on a working copy; tiles are generated stubs until then. Names and descriptions are presentation: rename freely, but keep the model paths an agent's edits use |
| Answer it | A reply game is a fork that takes the other side: same mechanics, swapped roles and names. Fork it with `template_fork`, rename the entities and levels in the copy, and record its own baselines |
