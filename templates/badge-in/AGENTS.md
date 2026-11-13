# Badge In

An original template: the return-to-office mandate as a platformer, a game of one to three minutes. The associate has to badge in at the far end of each floor before the standup, past middle managers who want a minute, a printer that jams when stomped and slides when kicked, and reply-all chains flying down the corridor. Three floors: the lobby, the open plan and the hot desks.

The mechanics are composed from two sources (docs/originals.md, "Composition"):

| Part | Taken from | Converted |
|---|---|---|
| The associate's run and jump | Moss Hop, `entities.player` | Taken whole: same tile size and frame scale |
| Middle managers (walk, fall, stomp) | Super Mario Bros., `entities.goomba` | Walk kept at 1/3 of the player's walk, as the goomba's is of Mario's |
| The printer's jam, kick and slide | Super Mario Bros., `entities.koopa_green.machine` | Slide at twice the player's walk, as the shell's is; a sliding touch kills instead of hurting |
| Reply-all chains | Super Mario Bros., `entities.bullet_bill.fly` | Speed equal to the player's walk, as the bullet's is |

## Files

| File | Holds |
|---|---|
| `template.toml` | The manifest: source `how = "authored"`, the pins `gp template build --write` keeps current, and the slots |
| `model.toml` | The model (docs/model.md), canonical |
| `fidelity.toml` | The contract |
| `authored.toml` | The assets, written by hand: metatiles, levels, palettes, and the stub tiles the build generates |
| `baselines/<level>.toml` | One model baseline per level: an input that plays it from its start to its goal (docs/originals.md, "Model baselines") |
| `../../assets/badge-in.toml` | The asset map the build writes. Don't edit it |

## Build, play and check

```sh
gp template build templates/badge-in            # the data image and lifted files, into $GP_HOME/work
gp model check templates/badge-in               # schema, evidence, contract
gp model fmt --check templates/badge-in
gp interp reach templates/badge-in              # every goal reachable by the player's own moves
gp interp baselines templates/badge-in          # each level's recorded run still plays the same
gp template check templates/badge-in            # asset-free, pins current
```

`just originals` runs all of these on the three originals (docs/originals.md).

## Change recipes

Every change ends the same way: `gp template build templates/badge-in --write` if `authored.toml` changed, `gp model fmt templates/badge-in`, then `gp interp reach templates/badge-in --record` to record new runs, and `gp interp baselines templates/badge-in` to check them. A change to movement, an enemy or a level changes the digests, so the runs are recorded again rather than edited by hand. Read `git diff templates/badge-in/baselines` before committing: a run that got much longer or now ends in another state says the change did more than intended.

| To | Change |
|---|---|
| Edit a level | Its `rows` in `authored.toml`, one character per 16x16 cell, using the metatiles' `char`s. Keep 15 rows. Move the goal (`levels.<name>.goal.x`) with the goal tiles. `gp interp reach` must still say "reachable" |
| Add a level | A `level-grid` asset in `authored.toml`; a slot `[assets.<name>]` with `from = { source = "badge-in", asset = "<name>" }` and role `layout`; the slot in `template.toml`'s `[slots]`; a `[levels.<name>]` table with one player spawn; `next` from the level before it; and raise the win condition (`if` on `counters.floors` in `rules.game`) |
| Retune the jump or the run | `entities.associate.jump` and `.run`. Keep the evidence note honest: a borrowed value that changes is no longer the source's, so say so in its note. `gp interp reach` prints the new envelope; check the widest gap still fits it |
| Add an enemy | An entity with a role and its components (docs/model.md, "Platformer extension") and a spawn in a level. A stomp needs `stomp_depth` at least the player's `max_fall` (4), or a fast fall can pass through it |
| Change the time per level | `levels.<name>.counters.clock` |
| Restyle | Palettes in `authored.toml`, or art through the MCP tools (`assets_generate`, `assets_place`) on a working copy; tiles are generated stubs until then. Names and descriptions are presentation: rename freely, but keep the model paths an agent's edits use |
| Answer it | A reply game is a fork that takes the other side: same mechanics, swapped roles and names. Fork it with `template_fork`, rename the entities and levels in the copy, and record its own baselines |
