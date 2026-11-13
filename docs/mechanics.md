# Mechanics

A mechanic is a named composition of primitives with a version and parameters. Entities, levels and the game all carry them. A running jump, a patrol and a camera that never scrolls back are examples. `gp model mechanics` lists the ones a model uses, from records the code declares.

Ported from game-printer's docs/mechanics.md: the catalog first, then measuring feel, questions and the refinement loop, which came with their commands.

## The rule for game-specific code

Every fix for a specific game is a general mechanic, primitive or asset kind that a later game can use (the user's rule, 2026-10-28). A hook in a template's `ext/` crate is allowed while no component says what it does, but it must describe itself (`Ext::describe`) and name the general mechanic it is an instance of (`like`). The catalog lists every hook record whose `like` no component runs yet as debt, so the debt is visible per model and shrinks as hooks become components (Milestone 2, step 8).

## The catalog

```text
gp model mechanics <model> [--all] [--paths] [--module FILE [--assets DIR]] [--json]
```

It prints every mechanic the model uses and the class of every model path. The same output has the debt and the problems found by holding the model to the records. The command exits 1 when the model and a record disagree. `--all` adds every component record, used or not; `--paths` lists each path with its class and mechanic.

gp links no game's hook crate, so a model whose hooks are in its game's module gives their records through the module: `--module` names the built module (`gp game build`, or `~/.gameprinter/work/modules/GAME.wasm`), and `--assets` the lifted assets it is loaded with (by default `$GP_HOME/work/assets/SOURCE`). Without `--module`, such a model's hooks have no record, and the catalog lists them as opaque. The first line names the hooks as `module` and the first 12 characters of the module's hash.

Records are generated from code, never written beside it:

| Mechanics | Where the record is |
|---|---|
| Platformer components (`run`, `jump`, `walk`, `patrol`, `fall`, `fly`, `hop`, `path`, `contact`) and the level's pit and goal | `gp_sim::mechanic::COMPONENTS`, beside the systems that run them, named `platformer.<component>` |
| Core mechanics: tile collision, entity modes, the game's rules, counters, the camera, spawning by view, exits | The same list, named `core.<name>` |
| `ext` hooks | `Ext::describe` in the game's hook crate, usually in its `records.rs`, read back across the module boundary (`gp_interp::describe`) |

A record has these parts (`gp_sim::mechanic::Record`):

| Part | What it says |
|---|---|
| `name`, `version` | Which mechanic, and the version of the extension or hook crate that declares it |
| `carrier` | entity, level or game |
| `system` | The system in `model.systems` it runs in |
| `class` | verb, mover, touch, collision, spawner, timer, economy, flow, camera, hazard or projectile |
| `implementation` | The code that runs it |
| `like` | For a hook, the general mechanic it is an instance of: `oscillate`, `shuttle`, `leap`, `hop`, `pursue`, `orbit`, `throw`, `projectile`, `leave-view`, `boss`, `player-physics`. A hook whose pattern is a component (`platformer.<like>`) isn't debt: it stays only for the game's own arithmetic |
| `params` | Each parameter's name, type (`fixed`, `frames`, `count`, `bool`, `action`, `choice`, `list`, `rect`), unit, allowed range, and whether it's optional |
| `tables` | For a hook, each `ext_tables` entry marked `mechanic` (how the mechanic works, which a formula could replace) or `content` (choices a clean room replaces) |
| `state`, `reads`, `writes`, `emits`, `consumes`, `spawns` | Its interface, for composition |
| `assumes`, `after`, `excludes` | What it needs of the world and of other mechanics |
| `built_from` | Its primitives: quantity, integrator, timer, input binding, condition, mode, tile collision, body contact, spawn and remove, event, random draw, sensing |
| `invariants`, `measures`, `envelope` | What must hold, what validation measures, and what it makes reachable |

These are the problems it finds:

| Problem | Severity |
|---|---|
| An entity's `ext_params` (its states' `set` included) name a parameter its hook's records don't declare, or omit a required one | error |
| An `ext_tables` entry its hook's records don't mark as mechanic or content | error |
| An entity carries two components that exclude each other | error |
| A hook gp has no crate for, or one that describes nothing (opaque) | warning |
| A mechanic whose system `model.systems` doesn't list | warning |

A test holds the component records to the platformer schema: every field of every component's schema is a parameter of its record and the reverse, and the records' versions are gp-model's. Moss Hop's model has no hooks and no errors. Donkey Kong's, with its module, has 13 mechanics and no errors.

## Measuring feel

```text
gp model propose <model> --recording ID... [--frames A..B] [--no-questions] [--json]
```

`gp track` finds the recording's objects, and `gp-extract` measures them from their whole-pixel positions and the controller:

| Parameter | How |
|---|---|
| `run.walk_speed`, `run.run_speed` | Runs on the ground of at least 16 frames with one direction held, the run button held or not: the speed over each run's last half, the median of runs at least half as fast as the fastest |
| `run.acceleration`, `run.run_acceleration` | From a standing start, the distance D a run falls short of moving at top speed v from its first frame: `v^2 / (2 (D + v/2))`. Proposed when the model has one or most standing starts show one |
| `jump.speed`, `jump.gravity`, `jump.gravity_held` | Each press of the jump action while standing, as an arc of heights until it lands; a quadratic fitted to the arc gives gravity as twice its square term and the takeoff speed from its linear term (by the model's `move_first` order). With held gravity, the held and rising part and the released part are fitted apart. With `bands`, each jump goes to the band its takeoff speed falls in |
| `jump.max_fall` | The mean drop a frame where the arc falls at its steepest for three frames or more |
| Rise by hold | Each jump's rise for the frames the action was held, listed for comparison |
| `patrol.step`, `patrol.period` | An enemy track's most common move and most common frames between moves |
| `walk.speed`, `fly.speed` | An enemy track's pixels over its frames |

Each value is rounded to the grid the original likely used (sixteenths of a pixel for speeds, 256ths for gravity and acceleration) and proposed by model path with its samples, its spread, the model's fact now, and whether they agree within the quantity's tolerance (speeds 1/16 or 3%, gravities 1/64 or 10%, pixels 0.5, frames exactly). An enemy track is an entity by its object ID through the state mapping's slot kinds, or the model's only moving enemy. The agreement with the model's stated facts is in the JSON's `accuracy` (`agree`, `compared`, `mean_relative_error`).

On Donkey Kong (games/donkey-kong, the corpus input dk.clear-25m.001 recorded with `gp record`) it proposes the walk speed 0.625 from 11 ground runs, which agrees with the model's 0.6667 within 3%; game-printer's propose gives the same on its template.

## Questions

Where measurement can't decide, the tools ask. Questions are kept in the working copy's `questions.toml`, beside model.toml and never in the repo. Each has an ID that stays the same across runs, along with its candidates and what would settle it:

| Kind | When |
|---|---|
| `conflict` | The samples of one recording round to two values with a third of the samples each, or two recordings measure different values (`recordings:<path>`) |
| `disagree` | A measurement with three samples or more disagrees with the model's fact; the candidates are the model's and the measurement's |
| `which-entity` | An enemy track has no object ID the mapping names, and the model has more than one moving enemy |
| `shape` | No numbers make a lockstep window exact (`gp model refine --lockstep`): the candidates are approving the proposed general mechanic and keeping the mechanic as it is. The answer writes no fact; an approved one is built as a component option, and the loop runs again |

```text
gp model questions <model> [--answer ID=N]... [--json]
```

lists them, and `--answer` chooses candidate N (from 1), writing it into model.toml as a stated fact with confidence 100 and a ref to the question.

## The refinement loop

```text
gp model refine <model> --recording ID... [--model COPY] [--against ...] [--rom FILE] [--assets DIR] [--module FILE] [--rounds N] [--frames A..B]
```

Each round validates the model (`gp validate`, the interpreter as the subject), measures the recordings, writes the answered questions as stated facts and each measured fact that disagrees with the model where a failing check measures that part (the recording and validation agree the model is wrong) as an observed fact with the recordings as refs, and asks the rest. It stops when a round writes nothing, or after `--rounds` (default 4); a last validation follows a round that wrote facts. It exits 0 when the checks pass; its JSON gives the rounds (`iterations`), the facts applied and the open questions. With `--lockstep` it is the corpus loop instead (site/src/content/docs/improve.md).

In game-printer, on Moss Hop with the walk speed made 2 and the gravity 0.3125, round 1 failed 11 checks and wrote both back, and round 2 passed all 21. On Donkey Kong's dk.clear-25m.001 the loop runs one round on both tools: 81 checks pass, 81 fail (counters and rules the recording's run shows differently), nothing measured explains them, and nothing is written.
