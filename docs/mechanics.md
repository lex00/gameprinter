# Mechanics

A mechanic is a named composition of primitives with a version and parameters. Entities, levels and the game all carry them. A running jump, a patrol and a camera that never scrolls back are examples. `gp model mechanics` lists the ones a model uses, from records the code declares.

Ported from game-printer's docs/mechanics.md: the catalog only. The capture, measuring and refinement sections described commands gameprinter doesn't have.

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
