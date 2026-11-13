# Signal Failure

An original template: the commute as a platformer, a game of one to three minutes. The commuter has to make the train before the doors close, over the tracks and the ticket barriers, past fare inspectors who turn at the platform edge, hopping pigeons, delay announcements and platform screen doors on a timer of their own. Patience is the lives counter.

Its movement has no source: acceleration, slowing, a held jump, coyote time and a jump buffer, all set by its author. It is the template to start from for a game whose feel is its own.

## Files

| File | Holds |
|---|---|
| `template.toml` | The manifest: source `how = "authored"`, the pins `gp template build --write` keeps current, and the slots |
| `model.toml` | The model (docs/model.md), canonical |
| `fidelity.toml` | The contract |
| `authored.toml` | The assets, written by hand: metatiles, levels, palettes, and the stub tiles the build generates |
| `baselines/<level>.toml` | One model baseline per level: an input that plays it from its start to its goal (docs/originals.md, "Model baselines") |
| `../../assets/signal-failure.toml` | The asset map the build writes. Don't edit it |

## Build, play and check

```sh
gp template build templates/signal-failure            # the data image and lifted files, into $GP_HOME/work
gp model check templates/signal-failure               # schema, evidence, contract
gp model fmt --check templates/signal-failure
gp interp reach templates/signal-failure              # every goal reachable by the player's own moves
gp interp baselines templates/signal-failure          # each level's recorded run still plays the same
gp template check templates/signal-failure            # asset-free, pins current
```

`just originals` runs all of these on the three originals (docs/originals.md).

## Change recipes

Every change ends the same way: `gp template build templates/signal-failure --write` if `authored.toml` changed, `gp model fmt templates/signal-failure`, then `gp interp reach templates/signal-failure --record` to record new runs, and `gp interp baselines templates/signal-failure` to check them. A change to movement, an enemy or a level changes the digests, so the runs are recorded again rather than edited by hand. Read `git diff templates/signal-failure/baselines` before committing: a run that got much longer or now ends in another state says the change did more than intended.

| To | Change |
|---|---|
| Edit a level | Its `rows` in `authored.toml`, one character per 16x16 cell, using the metatiles' `char`s. Keep 15 rows. Move the goal (`levels.<name>.goal.x`) with the goal tiles. `gp interp reach` must still say "reachable" |
| Add a level | A `level-grid` asset in `authored.toml`; a slot `[assets.<name>]` with `from = { source = "signal-failure", asset = "<name>" }` and role `layout`; the slot in `template.toml`'s `[slots]`; a `[levels.<name>]` table with one player spawn; `next` from the level before it; and raise the win condition (`if` on `counters.trains` in `rules.game`) |
| Retune the jump or the run | `entities.commuter.jump` and `.run`. Keep the evidence note honest: a borrowed value that changes is no longer the source's, so say so in its note. `gp interp reach` prints the new envelope; check the widest gap still fits it |
| Add an enemy | An entity with a role and its components (docs/model.md, "Platformer extension") and a spawn in a level. A stomp needs `stomp_depth` at least the player's `max_fall` (4), or a fast fall can pass through it |
| Change the time per level | `levels.<name>.counters.departure` |
| Restyle | Palettes in `authored.toml`, or art through the MCP tools (`assets_generate`, `assets_place`) on a working copy; tiles are generated stubs until then. Names and descriptions are presentation: rename freely, but keep the model paths an agent's edits use |
| Answer it | A reply game is a fork that takes the other side: same mechanics, swapped roles and names. Fork it with `template_fork`, rename the entities and levels in the copy, and record its own baselines |
