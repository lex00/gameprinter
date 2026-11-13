# The model interpreter

The model interpreter runs a game model directly, with no engine code generated from it: a hand-written model becomes playable and checkable, and previews, scores and validation have something to run. It implements the runtime protocol (crates/gp-runtime), so every tool that drives the original game through the protocol drives the model the same way. This page came from game-printer; what changed is that a model with hooks now runs through its game's module (docs/modules.md), the Super Mario Bros., Super Mario Bros. 2 and Mega Man 2 parts are gone with those games, and so are the Bevy generator and the old capture run's commands.

Two crates make it up:

| Crate | What it is |
|---|---|
| `crates/gp-sim` | The deterministic simulation: a compiled game (`Spec`), the state that changes (`World`), one function per system, and `step`, which runs a frame. It reads no model text and no files. A game's hook crate and its module link the same crate, so they and the interpreter run the same code |
| `crates/gp-interp` | The interpreter: compiles a model and its lifted assets into a `Spec`, runs it as a `gp_runtime::Runtime`, names its state by model path, records and replays model runs, and compares a run with the original game through a state mapping (docs/validation.md, "The state mapping") |

## Run it

`just check` compares each Moss Hop baseline (`baselines/mosshop/*.toml`) on the original and in the interpreter with `gp interp compare`, after lifting Moss Hop's assets. `just web-parity`, also part of `just check`, plays the same baselines in the web build, `gp_web.wasm` under Node, and in the native interpreter, and every frame's state hash must be the same ([the page's parity](web.md#parity)).

| Command | What it does |
|---|---|
| `gp interp run MODEL --input LOG` | Runs the model through an input log and prints the frame count, the digest of its per-frame state hashes, and its state at the last frame by model path. `--baseline FILE` takes the input from a baseline's holds and `--recording ID` from a recording in the store. `--profile NAME` runs the model in one of its profiles ([profiles](formats/profiles.md)), and the output names the profile. `--level NAME` starts at another level, `--state FILE` from a canonical model state. `--hashes FILE`, `--frames FILE` and `--apu-log FILE` write the state hashes, the frames played and the hooks' APU writes |
| `gp interp compare MODEL --rom ROM --baseline FILE` | Runs the original and the model on the same input and compares them every frame through `MODEL/mapping.toml`. Prints each mapped path's agreement and exits 1 if any path disagrees, then the lockstep ([below](#lockstep-distance-and-resync)): each path's distance, its splits and the mechanics they belong to. `--mode resync` (the default) or `free`, `--from FRAME` to start at a frame of the original, `--draws FILE` to give the model the original's random draws ([draws](formats/draws.md)). `--input LOG` or `--recording ID` instead of `--baseline`, `--mapping FILE` for another mapping |
| `gp clip score GAME [CLIPS]` | Scores corpus clips from their start, straight through and with resync, with the original's draws when the mapping asks for them. `gp clip log` writes a clip's input log from power-on |
| `gp interp translate MODEL --rom ROM --input LOG --frame N` | The original's state at frame N, or in a snapshot with `--snapshot FILE`, as a model state: a note for every part, loaded, approximated or missing. `--out FILE` writes the canonical state, which any runtime of the model loads with `load-state` |
| `gp interp moments MODEL --rom ROM --input LOG [--kind KIND]` | The game's own moments in a run of the original and what the model does at each ([moments](#moments)) |
| `gp interp slots MODEL --rom ROM --input LOG` | Measures the model's enemies against the original's enemy slots, one body a slot life |
| `gp interp scenarios`, `draws`, `odds`, `reach` | The scenario ladder ([below](#scenarios-one-press-at-a-time)), the original's random draws, the random-choice check, and whether each level's goal can be reached |

Not here yet: `gp record --model` (a model run recorded in the store) and `gp replay` of model recordings. The library has both (`gp_interp::recording`); the commands wait on running a model with hooks through its module. `translate`, `moments` and `slots` take `--recording ID` but refuse it for now; give `--input` or `--baseline`. `run` and `compare` read a recording's input from the store.

Every `gp interp` command takes `--assets DIR`, the lifted assets the model reads, defaulting to `$GP_HOME/work/assets/<model.source>`, where `just check` puts Moss Hop's. A model that names hooks needs `--module FILE`, its game's built module (`gp game build`, [modules](modules.md)); `--allow-missing-ext` on `run` and `compare` runs it in the built-in interpreter instead, those entities doing only what their components say. With `--json`, `compare` prints the frame counts and `agrees`, and a `fields` list with each path's counts and, when it disagrees, its `first` difference. Both `compare` and `run` also print `ext_hooks`, the label of the hooks they ran, and `ext_not_run`, the hooks that weren't run.

The original starts by the package's platform: gp-nes for an NES image, gp-arcade for a MAME romset (Tapper). What only the NES has, such as reading random draws from the 6502's code, refuses an arcade original in words.

On Moss Hop's eight baselines every one of the 20 mapped paths in `templates/mosshop/mapping.toml` agrees on every frame, and the screen measure finds both layers the same. Between them the runs take in the jump arc, both patrols, the bump block, the coins and the pit. The player stomps, dies in both ways and starts over, loses the game and wins it.

## What a frame does

A frame takes one input frame. The model reads controller 1 only, and reads it every frame, so it has no lag frames.

1. The frame count goes up by one, and the input is taken. An action is held when its button is down, and pressed when it's down and wasn't the frame before.
2. The frame is frozen if any rule, or any entity's own machine, is in a state with `freeze`. That's decided now, once, together with the systems a current state's `stops` names and the counters its `holds` names.
3. The systems run in `model.systems` order. A frozen frame runs only `rules` and `entity_states`; a system a state stops doesn't run, though a stopped `counters` still counts the counters with `runs_when_frozen` and a stopped camera `by_move` still notes its entity's move and place. A state entered during the frame stops the systems in its `stops_at_once` from then on.

Events fire as they happen. Counters that count an event change at once, and a rule whose current state has a transition on it takes it at once, with its actions, so the systems after it see the new state. Freezing is the exception. A state entered or left during a frame freezes or frees the world from the next frame, not the systems after it in the same one. Moss Hop's main loop works that way, since it checks `STATE` once at its top. A player that dies or reaches the goal touches nothing more in that frame, as `touch_enemies` skips when `STATE` isn't 0.

| System | Each frame |
|---|---|
| `rules` | Each machine counts a frame in its state, then takes its first `after` transition whose time is up and whose `if` holds |
| `entity_states` | The same for each active entity's own machine |
| `counters` | Each `every` change counts a frame and changes its counter when its count comes round |
| `spawn` | Entities waiting within `spawn_ahead` of the view appear, and those `despawn_behind` it go |
| `platformer.player` | Each player runs, then jumps and falls, then meets the pit or the goal |
| `platformer.enemies` | Every other active entity patrols, walks, flies or follows its path, then falls and hops |
| `platformer.contact` | Each player's touches with entities that have a `contact` component, in entity order, on the frames their `contact.every` and `phase` allow |
| `exits` | The player takes an exit whose take box it's in |
| `camera` | Each camera follows the first entity of its kind, then holds it inside `keep_in_view` |

| Part | What it does |
|---|---|
| Running | A held direction moves the speed toward the top speed (the run speed while the run action is held) by `acceleration`, or sets it at once without one, and a speed past the top speed drops to it. Against the motion on the ground it moves by `skid_deceleration` (or `acceleration`). With nothing held the speed goes toward 0 by `deceleration` on the ground and is kept in the air; without a `deceleration` it stops at once, on the ground and in the air, as Moss Hop's player does. Right wins when both are held. Without `air_control` an entity in the air keeps its speed. Each whole pixel moved is checked against solid tiles under the tile hitbox and against the level's sides, and a blocked entity stops at the last open pixel |
| Jumping | The jump action pressed while standing sets the vertical speed to `speed`. So does a press up to `coyote_frames` frames after walking off a ledge, and a press made in the air up to `buffer_frames` frames before landing, on the landing frame. With `bands`, the first band whose `below` is above the horizontal speed at the start of the frame gives the speed and both gravities until the entity lands. Gravity is added every frame, on the ground too; `gravity_held` replaces it in the held phase, from takeoff until the action is let go or the entity stops rising. The speed is capped at `max_fall`, or with `max_fall_from_half` set to it only once its whole part is at least `max_fall` and its fraction at least one half |
| Landing | Moving down or not at all, the row of pixels just under the tile hitbox is checked; solid ground there puts the entity on the tile's top with no speed. Moving up, the hitbox's top row is checked, and a solid tile stops the entity under it |
| Pits and goals | A player whose top edge is at or below `kill_below` dies (`player.died`), or, in a level with `pit_event`, fires that event on the frame its top edge first gets there and falls on. Otherwise one whose left edge is at or past `goal.x` reaches the goal (`goal.reached`) |
| Walking and falling | An entity with `walk` moves its speed a frame the way it faces. Blocked by a tile, it stays where it was and turns, and with `turn_at_ledges` it does the same when it was standing and finds no ground under its leading bottom corner. A walker starts facing the player. One with `fall` falls and lands as a jump does, with its own gravity and fall limit. A kicked entity turns to face away from the player |
| Patrols | A patrol moves on frames whose number is a multiple of `period`, counting from power-on, where frame 1 is the first step. It walks right from its spawn first, steps `step` pixels, and turns at `patrol_end` and at its spawn |
| Stomps | Hitboxes touch when they share a pixel, with positions rounded down. A falling player whose bottom row is within `stomp_depth` rows of an entity's hitbox top stomps it. Then `enemy.stomped` fires and the player's speed becomes `bounce`, and the entity dies unless it has `keep_on_stomp` |
| Other touches | `kill-player` and `hurt-player` fire their events and end the player's touches for the frame. `collect` removes the entity and fires `item.collected`, and `kick` fires `enemy.kicked`. Dead, removed and waiting entities keep their place in the list, so their paths don't change |
| Cameras | A camera sits at the followed entity's position less `lead`, inside the level with `clamp_to_level`. Without `scroll_back` it never moves left or up once the level has started. With `by_move` it moves by the entity's whole-pixel move instead, by where the entity was in the view on the frame before ([the model](model.md)), and keeps that place for the next frame |
| Actions | `restart-level` puts every entity at its spawn and every camera at its start, sets the level's `counters` and fires `level.started`. `next-level` does the same in the level's `next`. `reset-game` sets every counter back to its start first. `kill-player` fires `player.died`. `change:<counter>:<by>` changes a counter as an event's change does. `remove`, in an entity's machine, takes that instance out of the level, and `become:<entity>` replaces it by a new instance of that entity |
| Flying, hopping, paths | `fly` moves its speed a frame the way the entity faced when it appeared (by `heading`), through tiles. `hop` sets the vertical speed on any frame the entity ends standing. `path` moves out from the spawn by `distance` and back at `speed`, waiting `pause` frames at each end and, with `wait_near`, at the spawn end while the player's x is less than that many pixels away |
| Exits | The `exits` system takes the first exit of the level whose take box the player's hitbox overlaps (sharing a pixel), with its action held and standing if it must be. A `start` exit starts its level as `next-level` does. Any other puts the level's entities at their spawns, the player (keeping its own state) at `enter` with no speed and the cameras on it, without setting counters or firing `level.started`; the systems after it don't move that frame |

Patrols only match an original whose frames line up with the model's. With the state mapping's `polled` alignment they do: Moss Hop's slugs move when its NMI count is even, and the model's frame number is that count.

### Entity machines and `ext`

An entity's own machine runs once per instance. Each of its states is compiled into the entity as that state makes it, with `set` merged in by `gp_model::Entity::in_state`, so a flattened walker's state can have a walk speed of 0 and another contact. Events reach the instances they concern. The player's events (`player.died`, `player.hurt`) reach the player. A stomp or a kick reaches the entity it concerns. So does a hit, a defeat or a stand, and a hurt the entity did (`enemy.hurt_player`). `item.collected` and `item.lifted` reach both the item and the player, and so does `item.thrown`. One of the model's own events (`model.events`) fired for an entity reaches that entity only. Every other event reaches all of them. A state with `freeze` freezes the world from the next frame while any active instance is in it, as a game rule's does.

An `ext` hook names hand-written behavior for what the components can't say. Hooks are Rust that implement one trait, `gp_sim::Ext`. The interpreter and a game's module link the same hook crate (it depends on `gp-sim`, and on crates beside it that depend on `gp-sim` only, such as `gp-cycles`), so they agree frame for frame. A runtime finds hooks by the names models give them, in a registry (`gp_sim::Exts`, a `BTreeMap<String, Box<dyn Ext>>`) passed to `gp_sim::step_with_ext`; `Interp::with_exts` gives the interpreter one. `step` runs no hooks. An entity whose hook the registry doesn't have does only what its components say, one with no components stands where it spawned, and `Interp::ext_hooks` lists the hooks that weren't run.

```rust
pub trait Ext: Send + Sync {
    fn step(&self, cx: &mut ExtCx<'_>);
    fn appear(&self, _cx: &mut ExtCx<'_>) {}
}
```

A hook runs once a frame for each active instance of an entity that names it (as its machine's state makes it), in that entity's system: `platformer.player` for a player, `platformer.enemies` for the rest, after the entity's components there. On the frame a level's `appear_still` holds an instance still, `appear` runs in place of `step`, for a game that sets an object up as it appears; by default it does nothing. The hook gets an `ExtCx` with these fields and methods, among others (`crates/gp-sim/src/ext.rs` has them all).

| Field or method | What it is |
|---|---|
| `spec`, `frame`, `id` | The game, the frame number, and this instance's ID, its index in the world's bodies |
| `me: &mut Body` | This instance, to change: position, speed, facing, state, and four `i64` words, `ext`, for the hook's own counters and timers. They're in the canonical state, so hashes cover them |
| `world: &World` | The world as it stands, read only: other bodies, cameras, counters, rule states. This instance and the generator in it are as they were before the hook ran |
| `rng: &mut Rng` | The world's random number generator, the only randomness allowed |
| `event(name)`, `kind(name)`, `held(action)`, `pressed(action)`, `solid_at(x, y)`, `tile_at(x, y)` | Lookups by the model's names, the input, and the level's tiles as they stand (a tile's kind, changes included) |
| `param(name)`, `table(name)`, `counter(name)`, `set_counter(name, value)` | The model's `ext_params` and tables, and its counters |
| `draw(site)` | A random byte at a named draw site, which a draws script can replace with the original's ([draws](formats/draws.md)) |
| `fire(event, entity)`, `fire_own(name)`, `spawn(kind, x, y, facing_left)`, `remove(id)`, `set_tile(x, y, tile)` | Requests, applied in order when the hook returns. `fire_own` fires one of the model's own events for this instance only; `set_tile` changes a cell until the level is placed again |

A fired event is handled as the components' events are: counters, then the game's machines, then each entity's machine. A spawned body starts alive in its kind's initial state, and it stands if there's ground under it. It goes at the end of the world's bodies. Its ID is the number of bodies before it, so the order of requests decides the IDs, the same in every runtime. The system loop that's running doesn't reach it, since a system steps the bodies that were there when it started; the systems after it in the frame do, and every system from the next frame. Bodies are never reordered or reused: a removed or dead one keeps its ID. `restart-level` and the other actions that start a level drop every spawned body with the rest. The hook's rules are the simulation's: fixed-point only, no clock, no state outside the world, no iteration over a hash map. A test hook in `crates/gp-sim/src/systems.rs` counts in its words, spawns and fires, and two runs reach the same hash.

### Hooks in gp

A game's hooks live with the game, in a crate of its own beside its `model.toml`: `games/donkey-kong/ext/` in the games checkout is `gp-ext-dk`, with hooks such as `dk_throw`, `dk_barrel`, `dk_fire` and `dk_conveyor` (its `src/lib.rs` lists them all). It depends on `gp-sim` and `gp-cycles` only. gp links no game's hook crate. `gp game build` compiles the hook crate, the interpreter and the presentation into the game's module, and the tools run a model with hooks through that module (`--module`). `crates/gp-exts` holds only the demo hook, `demo_turn` for the model `hooks_demo`, which the tests use; the page's build swaps a game's crate in for `crates/gp-ext-game`, which is empty here.

`gp interp run`, `gp interp compare`, `gp validate --runtime interp` and the scoring commands run the hooks the module has. A model that names a hook with no module given is refused, since its entities would run on their components alone. `--allow-missing-ext` runs it anyway and lists the hooks that weren't run. A cache keyed by the model keys by the module file's hash too, so results made with other hooks aren't reused.

A model recording names the hooks it ran in its runtime version: `1 (state GPSM <version>; hooks <crate path> <version> <hash>)`. The hooks' hash is the first 12 hex digits of the BLAKE3 hash of the hook crate's code (its `Cargo.toml`, `build.rs` and the `.rs` files under `src/`, not dotfiles or other files), which `crates/gp-exts/build.rs` computes. A change to the crate's code changes the version, so recordings made with the old hooks, or with none, replay as stale rather than diverged. The recording format and the runtime protocol don't change: the runtime version is free text in both.

### Levels

A level's tiles come from one of three places. A `grid` slot with a `tileset` and a `solid_property` gives each cell from a metatile grid, as Moss Hop's level does. A `layout` whose slot is a `ppu-strings` asset (core `level.layout_ppu_strings`) gives the tile numbers its string draws on the first name table, one a cell, 32 by 30 cells of 8 pixels, which `tile_kinds` and `tile_surfaces` read; Donkey Kong's stages are drawn this way. A level with `empty = true` has no solid cells. `spawn_objects` is refused: this tool has no spawn-object list format since the games that had one were dropped.

### Spawning by the view

With `spawn_ahead`, every entity but the player waits, out of the level, until its spawn's x is within that many pixels right of the first camera's view. It appears then, a walker facing the player. The `spawn` system checks every frame against the view as it stands when the system runs, so whether that's before or after the camera moves that frame depends on `model.systems` (Moss Hop's runs `spawn` first). With `despawn_behind`, an active entity other than the player whose right edge is more than that many pixels left of the view goes for good. Restarting the level puts every entity back to waiting.

A level with the core's draw and slot rules (docs/model.md) runs them in `gp_sim::systems::spawn`: the `drawn` spawns its draw has reached, the others by `spawn_ahead` within the free `spawn_slots` (in the spawns' order with `spawn_in_order`, unless the draw's `pause` holds them), then the draw's next column. A body that appears with `appear_still` is in the frame's `still` list, and the enemies system skips it that frame. `gp_sim::systems::reached` says whether the view has reached a spawn by its rule, which a translation from the original uses to drop the waiting spawns the original has already placed, and `start_draw` places the draw with each level and each translated start. Despawning with `despawn_at = "position"` compares the position's whole pixels, and a kind's own `despawn_behind` takes the level's place. Moss Hop uses none of these.

## Determinism

Everything in `gp-sim` is integer or fixed-point arithmetic: `Fx` is a signed 64-bit integer counting 1/65536ths, the same raw value as `gp_model::Fixed` and the protocol's `fixed(s64)`, and its arithmetic wraps on overflow. Entities are stepped in index order, and nothing iterates a hash map. The random number generator (SplitMix64) keeps its state in the world, seeded from the spec. No clock is read, and systems run in the order `model.systems` gives in full.

### Canonical state

`World::canonical` writes the canonical state in little-endian order. It starts with the magic `GPSM` and the version. The version is computed, never counted: it is an FNV-1a hash of the code between the two layout marks in `crates/gp-sim/src/world.rs` (`// state layout: begin` and `end`, around `World::canonical` and `World::from_canonical`), with whitespace and `//` comments left out (`gp_sim::layout`). Changing a field that is written or read changes the hash and rewording a comment does not. Two lanes that change the layout therefore get different versions, and a merge of both gets a third. In game-printer it was a hand-counted number until 2026-10-30, last 19.

After the version, the state holds these parts:

| Part | Bytes |
|---|---|
| The frame count, the level, the level last started (which `restart-level` starts after the player took an exit), the random number generator's state | 8, 4, 4, 8 |
| The controller bits held this frame and the frame before | 1, 1 |
| The entities: a count, then for each its kind, its machine's state and time, `x`, `y`, `vx`, `vy`, flags (bit 0 on the ground, 1 facing left, 2 alive, 3 waiting, 4 in a jump's held phase, 5 a path on its way back, 6 climbing, 7 waiting to respawn once its spawn is out of the view's reach), the jump band plus one, frames in the air since walking off a ledge, frames left of a buffered jump, the four `ext` words, frames a path still waits, where the entity was placed (`x`, `y`, a path's spawn end), the damage it has taken, then frames of invulnerability, of knockback and walked from standing, the body it rides, carries and is carried by (each its ID plus one, 0 for none), frames of lifting left, frames toward a `shoot.every` and frames the run action still counts (`run_hold`), then the speed's rest below a step (`vx_frac`, 8 bytes) and a byte of the run's flags (bits 0 and 1 the way it last moved, 0 not yet, 1 right, 2 left, 3 both; bit 2 `decel_hold`; bits 3 and 4 the side `run.push_masks` last pushed it to, 0 none, 1 right, 2 left; bit 5 facing both ways with `run.face_both`; bit 6 `stepped_up`, a move across that stepped it up a sloped surface with `climb.down_through_step`), then the gravity its falls use with `jump.keep_fall_gravity` (`fall_gravity`, 8 bytes, 0 without it), then whether it bounced on the last frame (`bounce`, 0 or 1), then the speed the run last left with `run.speed_before_walls` (`seen_speed`, 8 bytes, 0 without it) | 4, then 176 each |
| The cameras: a count, then each one's `x` and `y`, then the followed entity's whole-pixel place in its view when it last moved (`seen_x`, `seen_y`), then the frames a `push_hold` still holds it (`hold`), then (0 without `keep_move`) the followed entity's last whole-pixel move across and down (`move_x`, `move_y`) and its whole-pixel place when the camera noted that move | 4, then 68 each |
| The rules: a count, then each one's state and time | 4, then 8 each |
| The counters: a count, then each value | 4, then 8 each |
| The `every` changes: a count, then each one's frames counted | 4, then 4 each |
| The changed tiles: a count, then each cell's index and kind, in cell order | 4, then 5 each |
| The cells a head hit holds solid (`hold`): a count, then each cell's index, the kind it takes next and the frames left, in the order hit | 4, then 9 each |
| Frames left of an exit's scroll, and whether a stepped camera holds the player | 4, 1 |
| The level's draw: the steps of the view the drawn columns have reached, and the frames left of the set being drawn, both 0 in a level without `draw` | 8, 4 |
| The level's numbered slots (`slot_rest`, none without it): a count, then each slot's body (its index plus one, 0 when free) and what its last body left, the fraction of its place across, its speed up and down and the fraction of its height, then the slot's own word, which hooks set (`ExtCx::set_slot_word`) | 4, then 36 each |
| The counter changes waiting for their `after`, in the order their events fired: a count, then each one's counter (its index), amount and frames left | 4, then 16 each |
| The shot slots of a `shoot.in_slots` (none without it): a count, then each slot's body (its index plus one, 0 when free) and the fractions its last shot left of its place across, its place down and its speed down, then the shots taken so far, whose remainder by the count is the slot whose turn it is | 4, then 28 each, then 4 |
| The pairs of walkers `walk.bump` found touching (none without it), the earlier body first: a count, then each pair's two body indexes | 4, then 8 each |

The state holds everything that changes, so it's the snapshot too. `restore` refuses bytes of another version, length or game. The state hash is its BLAKE3 hash, and the protocol's `state-hash` is that hash's first 8 bytes. In a profile other than `retro` the hash is of `gp-profile `, the profile's name and a zero byte before the state (`World::hash`, from `Spec::profile`), so runs in two profiles never share a hash; the state's bytes are the same in every profile.

The unit tests in `crates/gp-sim/src/systems.rs` run a small game through a death and a restart, a jump, and counters with bounds, and round-trip the canonical state. `crates/gp-interp/src/tests.rs` builds small models whose assets are made in the test and runs each part above: the grid, spawning and despawning by the view, a walking, falling enemy whose machine flattens it on a stomp and removes it, a player's skid, jump bands, coyote time and jump buffer, fliers, hoppers and paths, and the lockstep, the slots and the carry.

## State by model path

`query`, `sample` and run-until conditions name the interpreter's state with [model paths](formats/model-path.md), under the same names the model uses:

| Path | Value |
|---|---|
| `frame` | Frames since power-on |
| `level` | The current level's name |
| `rng` | The random number generator's state |
| `entities.<kind>[i].id` | The entity's ID, as events name it |
| `entities.<kind>[i].x`, `.y` | Position, the top left corner, in pixels (fixed-point) |
| `entities.<kind>[i].vx`, `.vy` | Speed in pixels per frame (fixed-point) |
| `entities.<kind>[i].on_ground` | Standing on something solid |
| `entities.<kind>[i].fall_gravity` | For an entity with `jump.keep_fall_gravity`: the gravity its falls use, kept from its last jump |
| `entities.<kind>[i].seen_speed` | For an entity with `run.speed_before_walls`: the speed without its sign, in whole steps, as the run last left it before a wall stopped it, which the run's speed tests and the jump's bands read |
| `entities.<kind>[i].pushed` | For an entity with `run.push_masks`: `"none"`, or `"right"` or `"left"`, the side a wall or its feet pushed it to on its last frame |
| `entities.<kind>[i].facing` | `"left"` or `"right"` |
| `entities.<kind>[i].alive` | False once stomped, collected, removed or gone behind the view |
| `entities.<kind>[i].waiting` | True until it appears (`spawn_ahead`) |
| `entities.<kind>[i].state`, `.time` | Its own machine's state and the frames since entering it, for an entity with a machine |
| `entities.<kind>[i].ext[0]` to `[3]` | Its mechanic words, for an entity with a hook or a component that counts in them (`hop.pattern`, `leap`, `shuttle`, `orbit`, `orbit_part`) |
| `entities.<kind>[i].home_x`, `.home_y`, `.path_back`, `.wait` | For an entity with a `path`: its spawn end, whether it is on its way back (or waiting at the far end to go back), and the frames it still waits |
| `cameras.<name>.x`, `.y` | The view's top left corner (fixed-point) |
| `cameras.<name>.seen_x`, `.seen_y` | For a camera `by_move`: the followed entity's whole-pixel place in the view when the camera last moved, which it reads on the next frame |
| `cameras.<name>.hold` | For a camera `by_move` with `push_hold`: the frames it still holds still after a push |
| `rules.<name>.state` | The current state's name |
| `rules.<name>.time` | Frames since entering it |
| `rules.<name>.states.<state>` | True while in that state |
| `counters.<name>` | The counter's value |
| `input.<action>` | Whether the action is held this frame, which the next frame counts as held the frame before; a translation loads it from the original |
| `ticks.<counter>[i]` | The frames counted toward the counter's `i`-th `every` change: 0 on the frame it changes, up to one less than `every` |

An entity type's instances are a list in spawn order, so `entities.slug[1].x` is the second slug's position, and the grammar's patterns and filters work as on a model (`entities.slug[*].x`, `entities.slug[alive=false]`), and a path to a table returns every value under it. A condition compares a fixed-point value by its whole part, so `entities.player[0].y >= 128` reads pixels, and a flag is 0 or 1 there. `set` changes a counter, or one of an entity's `x`, `y`, `vx`, `vy`, `on_ground` and `alive`. `load-state` loads a canonical state whole, as `restore` does, and returns no notes; a translation of the original's state ([below](#the-originals-state-as-a-model-state)) sets the rest of the mapped paths (facing, states, cameras, rules) in the world it builds. `explain` names the model parts behind a value, such as `entities.player.jump` for a player's `vy`. Events carry the model's names (`player.died`) and the entity's ID, and each frame ends with a `frame` event whose `polled` is true. The `patch` request answers `needs-reload`, since a changed model is loaded again, and a reset in the input starts the game over from its power-on state, with the frame count going on.

## Recordings

`gp_interp::recording` makes a recording whose source is the model rather than a ROM: `{"kind": "model", "name", "bundle"}` ([recordings](recording.md)). The bundle is a store object holding the model's canonical text and the lifted asset files the simulation reads, so a replay needs nothing but the store. The runtime is `gp-interp` with a version that holds the interpreter's behaviour version and its canonical state's version, followed by `; hooks ...` for a model with hooks (see [Hooks in gp](#hooks-in-gp)). A change to how the interpreter steps bumps the behaviour version, and a change to what its state holds changes the computed state version. Recordings made before either replay as stale. Not here yet: the commands that make and replay these (`gp record --model`, `gp replay` of a model run); the library and its tests are here.

Checkpoints are canonical states. A recording also keeps a frame trace and an event stream (below), so the trace and event readers can read model runs. The input log marks every frame polled. The `power_on` field holds the defaults, which nothing reads.

The trace holds a view of the state by model path rather than the canonical bytes (`gp_interp::modeltrace`). The canonical state changes length as bodies spawn and despawn, and a trace has one field table. The table (state `GPSP`, version 1) names every path the run ever names, found by running the input once before recording, in the order the run first names them:

| Group | Paths |
|---|---|
| The game | `frame`, `level`, `rng` |
| Each entity | `id`, `x`, `y`, `vx`, `vy`, `on_ground`, `facing`, `alive`, `waiting` (and `state` and `time` for one with a machine) |
| The rest | The cameras, rules and counters, and each `every` change's count |

An entity instance has a `.present` flag too, false on a frame it isn't in the world. Numbers take 8 bytes (fixed-point values in 65536ths), flags 1 and names 24, zero padded. The frame hash's `state` is the hash of that view and its `ram` is the canonical state's hash, so a trace check covers the trace and a replay the canonical state.

The event stream holds the model's own events, such as `player.died` and `enemy.stomped`, with the entity each happened to, frame by frame. A replay regenerates the trace's and the stream's chunks and compares their hashes with the manifest's.

## Comparing with the original

`gp_interp::compare::compare` runs two runtimes on the same input and compares them through a state mapping. It reads the original with RAM selectors and the model with model paths. With the `polled` alignment the model's power-on state is compared with the original's frame just before the first frame it reads its controller, and after that the model steps once for each frame the original reads it, with that frame's input. On Moss Hop the model's frame 0 is the original's frame 5. The report keeps every compared frame's values on both sides. The lockstep goes on from this comparison with distances, resync and starting anywhere.

## Lockstep: distance and resync

`gp_interp::lockstep` runs the original and the model on the same input, lined up as the comparison above lines them up, and measures how far apart they are on every mapped path every frame. The user asked for it on 2026-10-29: put the original and the rebuilt game both at the start of level 1, drive both with the same button presses, and measure how far apart they are on every metric, every frame, with the model put back on the original's path at each split or left to drift, switchable.

| Part | What it is |
|---|---|
| Distance | For a number, the absolute difference in the path's units (pixels, pixels a frame, the counter's units), whole parts only for a `whole` field. For a flag or a name, 0 when equal and 1 when not |
| Tolerance | From the model's fidelity contract ([the contract](formats/contract.md)): a metric rule over the path's part with a tolerance named for its field (`x = 2`, in the field's units). Otherwise 0, so the path must be exact. A path the contract leaves free is still measured and marked free |
| Split | A frame on which a path's distance is past its tolerance. Each one keeps the frame (the model's and the original's) and both values before anything is put back |
| Resync (`--mode resync`, the default) | At a split, the original's state is translated into the model's (below) from the model's own state, and loaded with the runtime protocol's `load-state`. So every split is a place the model parts from the original by itself, never a consequence of an earlier one |
| Free run (`--mode free`) | Nothing is put back. A path's split is listed when it goes past its tolerance after being within it, and the distances show the drift |
| Per path | Frames within the tolerance, the mean and the largest distance, and the first split |
| By mechanic | Each split path goes to the mechanic that owns it in the [mechanic catalog](mechanics.md#the-catalog): the mechanics the path's part carries whose record writes the value (`self.x` for an entity's `x`), its own components and hooks before the core's. A counter's is `core.counters`, a camera's `core.camera`. The list is the per-mechanic score and to-do list |
| Start here (`--from FRAME`) | The first aligned frame at or after the original's frame FRAME is translated into the model's power-on state (with the model's frame count) and both go on from there |
| Frame pairing | How the model's frames pair with the original's ([below](#frame-pairing)) |
| Scope end | With the mapping's `[scope]`, the run stops at the first of the original's frames out of the template's scope, which is not compared; it and the input's frames after it are counted as out of scope |

`--json` gives it under `lockstep` with these keys, among others (`measured`, `mode`, `stretches`):

| Key | Holds |
|---|---|
| `frames` | Each measured frame as the model's and the original's |
| `paths` | Each path with `level`, `tolerance`, `within`, `mean`, `max`, `first_split`, `owners` and the `distance` series |
| `splits`, `mechanics` | The splits and the mechanics they belong to |
| `start` | The start, with its notes |
| `resync_notes` | What a resync loads and can't |
| `pairing` | The first compared frame (`model`, `original`), the `offset` there, every lag frame skipped (`lag`) and the `last_offset` |
| `scope` | With `[scope]`, the first frame out of scope (`original_frame`) and the frames left out (`frames`); null without one or when the run never left it |
| `screen` | The screen measure ([below](#the-screen-measure)) |

### Frame pairing

A model frame and the original's frame it is compared with seldom have the same number: the original runs frames before the model starts, and a lag frame (one on which the original doesn't read its controller) is skipped by the model, so the original is one frame further ahead after it. Pairing frames by hand led to two false claims about Donkey Kong in one evening in game-printer. So every compare gives the pairing: `gp interp compare` prints `pairing: model frame n is the original's n+5 (model 0 = original 5); no lag frames skipped`, or lists the lag frames as spans with the offset after them, and gives the same under `lockstep.pairing` in JSON. A split, a first split and a first wrong frame always name both frames. `gp corpus score` and `gp clip score` keep each clip's pairing, and `gp model gate` prints the failing clip's pairing on its FAIL line. The lag frames are those between two compared frames; those before the first are in the offset. The code is `gp_interp::lockstep::pairing`.

A resync run is deterministic. The model's state hash at each measured frame is in the report, two runs give the same hashes, and loading a resync's state into a fresh model reaches the hash the run had there (`crates/gp-interp/src/tests.rs`).

### Bodies: enemies as measures

The mapped paths name the player, the camera and the counters. `gp_interp::bodies` makes every body but the player a measure of its own, paired across the two games for the run, so the loop can target an enemy's mismatch like the player's. The user asked for it on 2026-10-30, watching a clip: "is the analysis able to notice when enemies change direction?"

| Part | What it is |
|---|---|
| A mapped body | The mapping's own `entities.<kind>[i]` fields of a kind that isn't the player's name the same body in both games, so they pair it for the run and its measures are those fields. Moss Hop's two slugs are paired this way, with no measure added |
| A slot body | A mapping's `[slots]` life (the original's object in one slot) paired with a living model body of its entity (alive and appeared). A new body on one side is paired with a new one on the other, or with a body only the other game has so far, of the same kind, the nearest first by distance across plus up, within the view's width. A start translated from the original places each slot object as a body exactly at the slot, so on the first frame the nearest body is the one the start carried in. The pairing holds for the run: a life that ends ends its side, and a new life in the same slot is a new body |
| A body one game has | Its own body: its being alive differs while only one game has it ("the original has a barrel the model doesn't", or the reverse) |
| Measures | A slot body's are named `bodies.<kind>[n].<measure>`, the run's n-th body of that kind: its slot values `x` and `y` (and `vx`, `vy` when the slots map them), compared on whole pixels when the original keeps no fraction; `moving`, the way it last moved across (from the slots' `vx` when mapped, else from its place frame to frame, ignoring jumps of more than 128 px); `phase`, for an entity with a `path` component: moving out or back (`up`, `down`, `left`, `right`), paused at the far end (`out`) or home (`hidden`) after 4 frames without moving, or `still` before it has moved; and `alive`. A measure is compared only on frames both games have the body, but for `alive`; the frames before a body is seen count as together |
| Owners | The mechanic that writes the measure ([by mechanic](#lockstep-distance-and-resync)): a body's `x` and `y` its movers, its `moving` what turns it (`self.facing`, `self.x` or `self.vx`), its `phase` what moves it along its path. Tolerances are exact, and the drift thresholds are the player's (2 px for a place, a flag or a name never drifts) |
| Events | Each game's turns ("turns left"), phase changes ("rises", "sinks", "stops out", "hides"), a mapped body's state changes, and appearing and going ("appears", "is gone"), by frame. The i-th of one kind of event in the original is set against the i-th in the model; the first pair on different frames, or with one missing, is the body's first difference: "the original's pacer turns left at frame 40, the model's at 44" |
| Splits | A body measure splits as a mapped path does. In resync mode it doesn't resync on its own, since a resync keeps the model's own bodies |

The body measures follow the mapped paths in the lockstep's `paths` and in each frame's values, so the clip score's measures, tracks, drift point and owners take them as they take the player's. A body's track in a clip's score says why it is wrong when its first difference is on or before the track's first wrong frame, as in `Pacer: wrong from frame 41 (the pacer turns left 4 frames late)`. A clip's score lists its `bodies`. Each has a kind and a name ("the second barrel"), and the score says whether it is paired or which game has it alone. It also holds the body's measures and both games' events, then the first difference. The pairing reads only the two games' values, slots in order and model bodies by ID, in whole numbers, so the same run pairs the same way every time (`crates/gp-interp/src/tests.rs`: a model with a pacer that turns 4 pixels further splits on its track 4 frames late).

### The screen measure

The mapped paths cover only what the mapping names. The user said on 2026-10-29 that the loop "seems not to be noticing all the other screen pixel movement". The screen measure (`gp_interp::screen`) compares the two games' pictures on every measured frame. It judges two layers on their own, since "the background drifting apart is a separate and easily noticeable thing isolated from getting the character movement correct" (the user, the same day).

| Part | What it is |
|---|---|
| The original's picture | The emulator's frame, 256 by 240, in the NES palette the model's pictures are drawn with, so the same NES color is the same RGB on both sides. Its sprite pixels come from OAM and the pattern tables at the end of the frame (`gp_nes`'s `sprite_mask`), with the eight-a-line limit and the left-edge clipping. An arcade original has no sprite table the tools read yet |
| The model's picture | The interpreter's scene drawn on the CPU (`gp_present::raster`) as the preview draws it: the clear color, the level and its cycling colors, the HUD's background part, the bodies, then the HUD's sprites. The bodies go over the HUD's background, as the NES draws sprites over the status row; a body behind the background stays under it. It is drawn into the original's whole frame, and only the model's `visible` part is compared. Themes aren't drawn |
| Lag | A NES game writes its sprites and scroll in the vertical blank after the frame that moved them, so its picture of frame N shows its RAM of frame N - 1. The model's picture of the frame before is compared with it, and the first measured frame, which has no model picture before it, is skipped |
| `screen.background` | The pixels neither game covers with a sprite: the level's tiles, the HUD and the backdrop. When much of it differs (more than 0.5%), the best offset that lines the two up is looked for, 48 px across and 16 up and down; if it at least halves the differing pixels, the layer is "shifted": "the background is 3 px further left in the rebuilt game". Owned by `core.camera` |
| `screen.sprites` | Every pixel either game covers with a sprite. Compared in the frame's own space and again with the model's picture moved by the background's shift; its distance is the smaller, so a character that is right in the world but drawn under a wrong camera isn't counted wrong twice. Owned by presentation |
| Distance | The pixels that differ, by more than a color tolerance on any channel (0, exact, by default) |
| Boxes | The differing pixels grouped: 8 by 8 tiles holding any are joined with the tiles around them, and each group's box is the smallest one around its pixels, with its pixel count. Each box is named by what the model draws there: a body's kind for a sprite box (`barrel`), a HUD field for a background box (`the score`), and "something the model doesn't draw" for a sprite box over none of the model's. At most 12 a frame, the rest counted |
| Split, drift | A layer splits on a frame with more differing pixels than its tolerance (0). It drifts with a box of at least 4 by 4 pixels, or more than 0.5% of the visible area different |

Neither layer belongs to a mechanic the loop tunes, so the refinement loop never fits a number on them and treats them as a signal only. `gp interp compare` prints a line for each (`screen: Characters: the same on every frame.`, or how many frames are the same and the first different frame with its boxes, and for the background its shift), and `--json` gives `lockstep.screen`: the visible part, and for `background` and `sprites` the owner, `within`, `skipped`, `max`, `first_split`, `first_drift`, `first_shift` and each frame's `distance`, `pixels`, `shift`, `shifted` and `boxes`. A scenario's and a clip's score has `screen`: each layer's frames the same, its first different frame and that frame's boxes, the two lines, and `spans`, each layer's boxes by frame for the page to draw. When the model's pictures can't be drawn (no lifted pictures), the report says why and measures the rest. The same input gives the same counts and boxes; everything is whole numbers.

On Moss Hop's eight baselines both layers are the same on every frame.

### The picture-gap detector

`gp picture GAME --run RUN` (`crates/gp-cli/src/picture.rs`) finds the classes of picture gap over every clip of a game's corpus before anything is fixed. Each clip is made again (as `gp corpus score` makes it) and played in lockstep with the screen measure, and each box of each compared frame is put down to one cause by what is on screen there. The evidence is the model's bodies and their looks and the level's cells in view. The original's sprite table (OAM) and backdrop color add to it, as do the HUD's rows and the background's shift.

| Group | A box is put down to it when |
|---|---|
| fallback box (no sprite) | A model body over it has no sprite in its state, so it draws as a box |
| animation frame or flip | A body is over it in both games and the shapes differ; "flip" when the model's shape mirrored is the original's |
| palette or color step | The shapes are the same and the colors differ; on the background, most differing pixels on a cycling color's mask (a level color step) or on the backdrop where the original shows its own (the backdrop) |
| HUD | It lies in the HUD's rows: a field by name, the fixed tiles, or a HUD sprite |
| background tiles | It is over a cell the rebuilt game changed, a cell the original changed, or a block the original bumps (a sprite in the original over a block cell) |
| draw order and priority | One game draws the sprite behind the background (the OAM priority bit, or the model's `behind`) and the other doesn't, or two model bodies overlap there |
| scroll offset | The background is only shifted on the frame (the screen measure's shift) |
| drawn in one game only | The original has sprite pixels there and the model no body ("original only", with the OAM tile numbers), or the reverse ("rebuilt only") |
| flicker | The model draws a body where the original shows nothing, and the original had more than eight sprites on one of its lines or showed it the frame before |
| placed elsewhere | The two shapes line up when the model's is moved by at most 4 px each way; the detail says by how much. This is usually behaviour (a body or the camera apart), not the picture |
| other | Anything else: the level picture differs |

For each group and detail it reports the clips and frames, the frames it alone holds down (the layer's only cause on that frame, so fixing it alone would make the frame the same) and the pixels, with the first example clip and frame by clip ID. It also gives the corpus's fidelity as `gp corpus score` counts it, every clip scored again with the build it runs on, and each clip's behaviour, so a change to the picture can be checked to leave behaviour alone. It never writes the corpus or the model. `--clip ID` takes one clip and `--top N` the details printed a group. With `--clip`, `--shots FRAMES --shots-dir DIR` writes those compared frames (the report's frame numbers, `12,40-45`) as PNGs: the original, the model and the pixels that differ side by side at 2x. Everything is whole numbers in clip ID order, so the same corpus with the same model and build gives the same table. An arcade original's boxes are put down to causes without a sprite table.

In game-printer on 2026-11-05 it found Donkey Kong's 50m, 75m and 100m drawn with placeholder boxes (the big foe, the moving ladders, the pies), in bank 0's tiles and 25m's colors. With the pictures and palettes taken from the ROM's own banks and strings, the fallback boxes went to none and the background was the same on 88% of frames where it had been 60%, with behaviour the same on every clip.

### The original's state as a model state

`gp_interp::translate` reads the state mapping in reverse. From the original as it stands (an input replayed to a frame, or a snapshot restored), it reads every mapped field and writes the value into a copy of a model world: the model's own state for a resync, its power-on state to start somewhere. It never guesses, and every part of the model's state gets a note (the runtime protocol's `load-note`), by model path.

| Note | When |
|---|---|
| `loaded` | The original's value, as the mapping gives it |
| `approximated` | Taken in part. A `whole` field keeps the model's fraction under the original's whole part. A rule's or entity's state that changes is entered with what its `set` changes, and noted `loaded`; its time is 0, noted `approximated`, unless the mapping maps the time. A field whose case holds with an `approximated` reason is noted with it. The level, when the original is in another one, is started afresh, so what the mapping doesn't hold is as the level starts. The frame count is the model's own |
| `missing` | The mapping has nothing for it, so the base world's value stays: the level when the mapping can't say, the random number generator, each entity kind the mapping holds nothing of (`entities.barrel[*]`, with how many the model has there) unless its `[slots]` carry it over (below), each unmapped field of a mapped entity, unmapped cameras, rules and counters, each unmapped count toward an `every` change (`ticks.<counter>[i]`, so the counter changes on the model's own count), the mechanic state the state view doesn't name (jump bands and buffers, timers, hook words), and the level's changed cells |

The level comes from a `level` field when the mapping has one, and otherwise from the first `[[slots.level]]` whose condition holds. An entity the mapping names that the model's world doesn't have is refused as missing, since a field can't add a body. A mapping with `[slots]` carries the original's slot objects instead (`translate::from_original`): the model's own entities but the player go (those that have appeared, those a hook or a spawn made, and those still waiting whose spawn the view already reaches, which the original has spawned too), and each living slot object of a kind the slots name becomes a body of its entity at the slot's values, facing the player, noted `approximated` ("1 from the original's slots, at x 312"). Every slot field the mapping names loads, as the same field of a mapped entity does: the facing, the machine state, the hook's words, a path's spawn end, way and wait, and the x fraction a `whole` field keeps. What the slots don't hold is as a new body has it. A body at an end of its path heads for the other end, and the model's frame count moves on to the next count whose low byte is the slots' `clock`, as `gp interp slots` runs its bodies, so the components that move on some frames move on the original's. A resync keeps the model's own entities (`translate::keeping_entities`), so the enemies a run is measuring aren't put back at every split. The result is a canonical model state, so the interpreter and a game's module load the same bytes.

### Carrying the original's objects

A start past a level's start carries every loaded object by one rule over the model and the mapping (`gp_interp::carry`). What each body keeps between frames, by its components, must be read by its kind's slot fields. Every value read must load. A start is refused only for an object it can't carry, and its reason names the object and the field. `gp_interp::starts::cant_start_here` applies it, for the starts found in an input and for `gp capture carry`. `gp capture carry GAME_DIR` (the package folder, such as `games/donkey-kong`) lists, for each of the game's clip starts and with `--every N` for frames sampled through their presses, every object a start can't carry, grouped by cause (an unmapped kind, a missing field, a timer or phase not read, a state not named, or refused by the mapping), and counts the starts each gap alone holds back.

### Refining from the lockstep

`gp model refine GAME --lockstep` is the refinement loop driven by the lockstep, clip by clip from the game's corpus. The user asked for it on 2026-10-29 ("it starts off kinda ok but even from frame 1 it could use refinement loop"), and chose two rules: numbers are fitted on their own, shapes are asked about; and the loop runs until it is stuck, with an exact target (tolerance 0, subpixels included, unless the contract gives one).

```text
gp model refine GAME --lockstep [--model DIR] [--clip ID]... [--against SCORE] [--window N] [--rounds N] [--moment KIND:N] [--mechanic NAME]... [--drift FIELD=N,...] [--run RUN] [--module FILE] [--assets DIR] [--rom ROM] [--json]
```

`--model DIR` is the working copy whose model.toml and questions.toml the loop rewrites: a copy of the game's folder outside any git checkout. First the clips are checked against the ROM: each is played on the original and its curve compared with the one recorded for it in `--against`'s score; a clip whose curve differs is reported and not used. Then each clip the model doesn't play exactly gets the loop, from the original's state at the clip's start. A value it keeps must leave every other clip used no worse.

| Step | What happens |
|---|---|
| Measure | The original and the model run in lockstep in resync mode from the clip's start. The window is the original's frames from the first wrong frame for N frames. The first wrong frame is the first frame on which any measure is past its exact tolerance, not the drift point (the user, 2026-10-29: "its like ONE frame in things are already WRONG"). The camera (`cameras.*`, `core.camera`) is its own track: it follows the bodies, so its measures count only once no other measure is wrong. `--window` takes frames or seconds (`5s`); the default is 300 frames, 5 seconds, so a round fits a stretch of play rather than a few steps |
| Owners | The measures wrong on the first wrong frame, and only the mechanics that own them ([by mechanic](#lockstep-distance-and-resync)). If the player's speed across is wrong on frame 1, only `platformer.run`'s numbers are tried; the jump's wait until a jump measure is the first wrong one. The round says it: "Wrong from frame 1: the player's speed across (original 0.0625, rebuilt 0.0371). Trying only platformer.run's numbers." |
| Numbers | Each number those mechanics set for the measure's part: every parameter that is a fixed-point number, a frame count or a count (`walk_speed`, `run_acceleration`, `gravity`, `max_fall`, a counter's `every`), and the numbers in a list parameter's items (a jump's bands). Booleans, tables, actions and choices are shapes, never changed |
| Trials | Each number alone, run through the window only, from the original's state at the clip's start (or a moment's, half a second before it), translated into the model, with the same presses from there. The model's numbers never decide where a trial starts (the user, 2026-10-29: "if im starting at frame 100, it has to come from the rom state first, then try to replicate it with the rust version, then issue the commands and compare"). The search has two passes. The impact pass tries every number at drastic values: a quarter, a half, twice and four times its value, and zero, in its range and step. It ranks the numbers by whether one of those brings the first wrong frame closer to the original's, then by how much they move the measure's curve. A number that moves nothing is "none" and is not fine-tuned; one that moves the curve only away from the original's is "the wrong way", a sign for the shape question. The fine pass works on the top three that move it toward the original's: their values the original's own numbers give, then a bracket that halves the gap between the best and the drastic values either side of it, then steps of a whole, a sixteenth, a 256th, a 4096th and a 65536th out from the best so far. When two values score the same, the one that changes its number least relative to its old value is kept; the round lists the others as `ties` |
| Score | First how late the first wrong frame is, then how many times a measure splits on a frame of the trial up to the window's end, then the distances added up. While a body is wrong only the bodies' measures count; once they match, only the camera's. Only a later first wrong frame counts. A window that is exact from the original's state is passed, and the loop goes on at the next first wrong frame |
| Check | The best few trials are run over the whole clip. One is kept when its first wrong frame is later, no measure's first split moves earlier, every frame before the window is still together, and frames together over the whole clip don't drop. While a body is wrong the camera's measures are left out of these checks |
| Code | A number the game's code gives outranks a fit (the user, 2026-10-29: "known code values should be considered"). Its fact is code-evidenced when evidence on it or on a part above it is stated and cites a line (`file:line`, as `gp model derive` reads, [derivation](derivation.md)). The code's value is always the one a trial starts from, and a fit that only ties with it is never kept. When the best fit wants to move such a number, the loop stops on a `conflict` question ahead of any fit: "The game's code says entities.player.run.walk_speed is 1 (games/mosshop/mosshop.s:31), and the fit ... wants 1.5: check the mapping first" |
| Stop early | After the impact pass, the loop stops at a missing mechanic without the fine pass when either detector fires: no number moves the measure at all at any drastic value ("nothing moves it"); or every number tried is the game's code and no drastic value makes the first wrong frame any later ("the code's numbers"). A round with no number to try says "nothing writes it". The question names the detector and what the original does, read from its values. `GP_REFINE_EARLY_STOP=off` turns it off |
| Solve | After the early stop's detectors, the deterministic solvers (`crates/gp-cli/src/solve/`) read the clip's free-run score and propose exact values with no search. `arith` classes the diff from the first wrong frame: a constant offset, a linear drift (a rate off by a fraction, with the smallest power-of-two denominator that explains it), a lag, a sign flip, a step every N frames, a 256 wrap, one side that stops, or wrong from the start. `timers` cuts a count (a timer, a hold, ticks) at its change points: a different start, period or reload, a count that stops, steps aligned to the frame count modulo N, or the same count some frames apart. Each proposal runs through the window from the original's state; the best that makes the first wrong frame later gets the Check, and when it passes it is written with its evidence and the fine pass is skipped. A proposal on a code-evidenced number is never written. `gp solve GAME --run RUN` prints each non-exact clip's class and proposals, `--check` scores each clip's best proposal on every clip, and `--missing` adds the body and state detectors. `GP_REFINE_SOLVE=off` turns it off |
| Write | The kept number is written into the copy's model.toml as an observed fact with evidence: the clip, the window's frames, and the original's values there. Earlier evidence on the number is added to |
| Next | When the window is exact, the next round's window starts at the next first wrong frame. When no number makes the first wrong frame later, the loop stops there |
| Ask | The stuck window becomes a `shape` question in questions.toml ([questions](mechanics.md#questions)): the first wrong frame and the closest the impact pass came, the window, each measure wrong there with both games' values frame by frame, why no numbers fit, and a general mechanic to add or change, never a game's own hack. An option the mechanic already offers and the model doesn't set comes first. Answering it approves or declines the proposal and writes no fact |

`--progress-json` writes each window, pass, trial and kept value as a JSON line on standard error. It exits 1 when it stops on a question.

Not here yet from game-printer's loop: a positional input (a recording, baseline or input log) in place of the corpus, and `--from FRAME`, `--until FRAME` and `--metrics PREFIX`. `--clip` and `--moment` narrow it instead.

#### Where they drift apart

A window stops where the two games drift apart (the user, 2026-10-29: "once they drift past a threshold it should stop and refine. even if the loop is 5 seconds, once it drifts enough it should stop and work, even if at 3 seconds"). Each round runs the model free from the window's start and finds the first frame a measure is past its drift threshold; the window then ends 10 frames after that frame, and only those frames are fitted.

| Measure | Drift threshold, unless told otherwise |
|---|---|
| A position (`x`, `y`, `stage_x`, `stage_y`) | 2 px |
| A speed (`vx`, `vy`) | 0.5 px a frame |
| A counter, a flag or a name | 1 |

A contract tolerance that allows more raises it; `--drift x=4,vy=1` (a field or a whole mapped path, in its units) sets it outright. The threshold decides only where a window stops: the fit still aims for exact. Each round's `drift` in `--json` names the frame and the measure, with its distance and threshold, and a sentence: "Stopped at 1.1 s: entities.player[0].y 2.25 px apart (past 2 px). Working on 1.0 to 1.3 s" (`gp_interp::lockstep::drift_thresholds`, `drift_point`).

### Moments

The game's own moments in a run: its jumps, landings, deaths, lives lost, counter changes and level changes, each numbered by kind, so a person can say "refine the very first jump" (the user, 2026-10-29). `gp_interp::moments` reads them from the mapped measures a lockstep keeps, for any game:

| Kind | Read from |
|---|---|
| Jump | An entity's `on_ground` going from true to false with upward speed (`vy` below 0, or `y` going up without `vy`), to its next landing; its height is how far `y` rose from takeoff to the top |
| Landing | `on_ground` going from false to true |
| Death | An entity's `alive` going false; on the model's side also an event whose kind ends in `.died` |
| Life lost | A `lives` counter going down or a `deaths` counter going up |
| Counter | Any mapped counter changing, except a clock (a counter the model changes `every` so many frames, such as a time counter) |
| Level | The level the mapping reads changing |

Each moment names the mechanic that owns its kind for its part, by the catalog (a jump's `vy` is `platformer.jump`'s). `moments::check` then runs the model on each of the original's moments from the original's state half a second (30 frames) before it, on the same presses, free, to half a second after, in one pass of the original with a fork at each start, and finds the model's own moment of the same kind there: "jump 1 at frames 61-100 (1.0 s): rises 47.5 px; the model too, on the same frame, 47.5 px", or "the model doesn't". A check whose frames already cover a moment answers it too, so a landing shares its jump's.

```text
gp interp moments MODEL --rom ROM --input LOG|--baseline FILE [--kind KIND] [--mapping FILE] [--module FILE] [--assets DIR] [--json]
```

`--json` gives `moments` (each with `kind`, `n`, `part`, `mechanic`, `from`, `to`, seconds, `peak`, `height`, `before` and `after` for a counter or a level, `focus`, the frames a focus on it covers, and `model`, the model's moment with its `offset` in frames and its `height`) and `kinds`, how many of each the original has and the model does too. Not here yet: the page's moments view (`GET /api/capture/{run}/moments` answers that `gp interp moments` gives them).

`gp model refine --lockstep --clip ID --moment jump:1` narrows the loop to one moment of that clip, with its windows in the moment's `focus` frames, and only the numbers of the mechanic that owns its kind are tried (`--mechanic NAME` names others) and only the measures it owns are held, so a jump's windows aren't stopped by the clip's other measures. A value it keeps is then measured on every other moment of the same kind in the clip, each from the original's state before it, with the model before the loop and after, and the `also` key in `--json` has how many are closer (`better`), further (`worse`) and the same, with "also better on 2 of 2 other jumps, worse on 0, the same on 0" in words. Each one it makes worse becomes a `worse` question in questions.toml naming its frames and the measures apart there.

### Scenarios: one press at a time

A scenario, called a sequence on the page, is a few frames of presses run on both games from the same start: the original's state at one frame. The user asked for them on 2026-10-29: "button presses in isolation to begin with, then combining them according to whats known about the game, if it has double jump etc. then combining with directional. each scenario can be about a very small sequence or single press". Both games always take the same commands from the original's state: the original replays the input to frame N, the model loads the original's state there translated (as `gp interp translate` does), and both take the scenario's presses (`gp_interp::lockstep::start_at` and `lockstep_from`).

```text
gp interp scenarios MODEL --rom ROM --recording FILE [--at FRAME] [--tier N] [--bar exact|99%] [--played FILE]... [--refine ID | --auto] [--module FILE] [--assets DIR] [--json]
```

`--recording` here takes a baseline file or an input log file: the input played to the start.

The function `gp_interp::scenario::ladder` generates the ladder from the model's input actions (`[input.actions]`) and the player's mechanics in the catalog, for any game. Each scenario has an ID and a name in plain words ("Jump, held 15 frames"). It has a tier and the mechanic it is about with that mechanic's numbers, and its presses as button spans in the input log format (`a@0-14`). It runs its presses and then 40 frames with nothing pressed, at most 120 frames (2 seconds).

| Tier | Scenarios |
|---|---|
| 1 | Nothing pressed for 60 frames; then each button that isn't a direction: tapped for 1 frame, held 5, 15 and 30 frames, tapped twice |
| 2 | A second jump in the air (named "no double jump" when no mechanic of the player jumps in the air, so it checks that nothing happens), a jump held through the landing, a jump again on landing, a jump pressed just before landing when the jump has `buffer_frames`, the run button with the jump, any other button in the air, two buttons together |
| 3 | Left and right tapped, held 15 and 60 frames, each with the run button; right then left; right with a jump at frames 0, 10 and 30; a running jump; a turn in the air; right in the air after a standing jump; up and down only when a mechanic reads them (a climb, a crouch) |

| Part | What |
|---|---|
| Start | `--at FRAME`, or a quiet frame found in the input (`quiet_start`): the player in the level, on the ground and still for 10 frames, nothing alive within 80 px, by the mapped measures and the original's state translated. Failing still, the first frame on the ground with nothing near, and the sentence says the start is moving |
| Result | Free run over the scenario's frames: exact, or the first split (the scenario's frame, the measure, both values, the owning mechanic, the scenario's own mechanic's measure first) in plain words: "Jump, held 15 frames: the rebuilt player goes 0.8125 px higher, from frame 33 (jump)" |
| Score | For each measure: frames together of the total, the first split, the largest and mean distance, the distances in doubling bins, the owning mechanics; and a headline, "9/10 measures, 96%" |
| Pass bar | `--bar exact` (the default): no split on any mapped measure. `--bar 99%`: that share of every measure's frames within tolerance |
| Needs | A scenario that presses one button needs "nothing pressed" to pass first; any other needs every scenario that presses each of its buttons alone. A played scenario needs the tier-1 scenarios of the mechanics whose measures the original moves in it, and can be opened by hand. Each scenario is locked, open, failing or passed, and lists what it needs and what it unlocks |
| Next | The first open or failing scenario in ladder order: where refinement should start |
| History | Each model (by its text's hash) adds its scores to the history, after every refine round too, so a scenario shows its trend, "was 86% -> now 100%", and when it got worse |
| Cache | By the model, its assets, the mapping, the contract, the ROM, the input, the start and the scenario, under `$GP_HOME/work/scenarios/` |

`--refine ID` runs the refinement loop on one scenario of a working copy: its input is the input to the start and then the scenario's presses, every run starts from the original's state at the start, only the scenario's mechanic's numbers are tried and only the measures it owns are held, and a fit is written only if no other scenario gets worse. Afterwards the whole ladder is scored again: "Jump, tapped for 1 frame now passes; also fixes 16 other scenarios, none worse." `--auto` repeats that on its own: the next open or failing scenario is refined until it passes, everything is scored again, and it stops when all open ones pass or one is stuck, with that scenario's question.

A played scenario has the same format with `"source": {"kind": "played", "when", "by"}`, its own start (the input and frame, or a snapshot) and its presses as an input log, and it runs the same way. A played start can name its own input: `"start": {"input", "frame", "log"}`, where `log` is an input log file from power-on (a path relative to `$GP_HOME`, or absolute) that the original plays to the frame in place of the ladder's input. Results are cached with the log's text in the key. `gp serve` saves one with `POST /api/capture/{run}/scenarios/played`.

The scenario format is 0.4.0 (`gp_interp::scenario::SCENARIO_VERSION`). Its versions added each result's first wrong frame per track (`tracks`) and the picture measure (`screen`) in 0.2.0, a start's optional `model_frame` in 0.3.0, and each result's `bodies` and a track's optional `reason` in 0.4.0. A reader of an older version reads a newer one, since the new fields are optional. Cached results are keyed by the version, so they are scored again.

#### Many starts

The ladder runs from one quiet start; `gp_interp::starts::find_starts` finds many in an input without anyone choosing them. One pass plays the input on the original and, on each frame it reads the controller after the mapping's start holds, reads the mapped measures:

| Start | When |
|---|---|
| A level's start | The first frame of a level the input reaches, by the mapping's `level` field or its slots' level conditions (a model of one level is in it always), with the player in play on the ground for 10 frames and nothing alive within 80 px. One standing still for 10 frames within 3 seconds of the level's first frame is taken first; failing that, the first frame on the ground, and the sentence says the start is moving |
| Next to something | Up to three more in the level, at least 120 frames apart, each the first frame on the ground next to something not met yet in the level: a pit (a column 8 to 72 px ahead with nothing to stand on down to the level's bottom), a wall (a solid tile up to 24 px ahead at the player's height), a block (a solid tile up to 72 px over the player's head), or an enemy of a kind not met yet 16 to 160 px ahead (by the mapping's slots for a game that keeps enemies in slots, by the translated bodies otherwise) |

Every start is a frame where every rule of the translated state is in its first state, so the player isn't dying or past the goal, whose time in the state the mapping doesn't carry over. A level played again in the same input adds nothing. Each start keeps the model's frame there as the lockstep counts it (one frame for each polled frame after the mapping's start), so a start taken from a snapshot of the original is the start `start_at` takes by playing the input to it. A start the rebuilt game can't begin from says why and is kept: the original's state there doesn't translate, the mapping can't say which level it is, the player isn't in the level, or the original has an object a start can't carry ([above](#carrying-the-originals-objects)).

`scenario::clip_sequences` adds walks to the ladder: right held for each of a list of seconds ("Walk right 3 s"), the same with the run button, and a run then a jump at 1 second, each with the settle after it. `scenario::level_words` names a level in game words: the HUD label, the game's title for a model of one level, with the level's own words after a label two levels share.

### The clip corpus

The user asked on 2026-10-29 for "a tree/matrix of clips that could be generated in any game, or by genre. pressing a button, pressing and holding, etc. but the choices should be based on what the game is and what characters can actually do", consistent enough to build an alignment corpus from. `gp_interp::corpus` builds it: a grid of what the player presses by the situation the press happens in, derived from the game model, with starts found in recordings of the original, timings measured on the original and a measure of which numbers each clip can move. Not here yet: `gp capture corpus`, the command that wrote it into an old capture run. A game's scoring corpus in gameprinter is its corpus plan ([the scoring corpus](formats/scoring-corpus.md)).

A corpus is a grid. One side is a press pattern (a tap, a hold from a fixed list of 1, 2, 4, 8, 16, 32 and 64 frames, a release, two taps, two buttons together, a hold through the landing, a press placed just before the original's landing or just after it leaves a ledge, a turn); the other is the context it happens in. A cell is (mechanic, pattern, context), named `MECHANIC:PATTERN:CONTEXT` (`platformer.jump:jump.hold-16:ground.still`), and a clip is a cell at one start, `CELL@START`.

| Step | What |
|---|---|
| Actions to mechanics | Each `[input.actions]` action leads to the player's mechanics whose `action` parameter or `reads` name it; a parameter ending in `_action` (`run_action`) makes it a modifier, read only with a direction; an exit's `take.hold` makes it `core.exits`' in an exit's box |
| Mechanics to numbers | Each fixed-point number, frame count and count a mechanic's use sets, and the numbers in a list's items (a jump's `bands`), as the refinement loop changes them |
| Mechanics to contexts | `ground.still` always; `ground.walk_top` with `platformer.run`, `ground.run_top` with its `run_action`; `ground.band[i]` and `ground.edge[i].under`/`.over` for each jump band; `ground.ledge` with tile collision; `air.rising` and `air.falling` with a jump; `enemy.near[KIND]` for each entity with `platformer.contact`; `exit` when an exit holds an action; `ladder`, `carrying`; `state[S]` for each of the player's machine states after its first; `water` for a water mechanic, or as the platformer genre's probe |
| Contexts to cells | The patterns each mechanic's class allows there, and nothing pressed in each context a recording can give |
| Genre probes | For the platformer genre, moves the model lacks: jump tapped in water (swim), the run button tapped in each power state (shoot), down held (crouch). A probe runs on both games and is never fitted |
| Mapping checks | On the original alone, first: x moves by vx every frame at walking top speed, y by vy in a held jump, vy is 0 on the ground |
| Mixed | Two clips of seeded pseudo-random presses (4 to 16 frames each) over every button a mechanic reads |

A cell's ID never holds a frame number the model computes. A timing is an anchor measured once on the original (`jump.top`, the frames a held jump takes to reach its top; `jump.edge[i]` and `jump.band[i]` for the bands), and a press placed by an event (`land`, `leave`) is placed per clip at the original's own frame. So refining the model never renames or moves a clip, and adding a mechanic only adds cells, which `crates/gp-interp/src/corpus/tests.rs` holds on Moss Hop with a climb added to a copy.

Four rules prune, and each pruned cell stays in the file with its reason:

| Rule | Removes |
|---|---|
| Dominated | A jump held longer than `jump.top`: the same as held to the top, until the landing |
| No effect by the record | A button no mechanic reads in that context: the run button alone, held (one tap stays as a check that nothing happens), down outside an exit's box |
| Checks | A press the records say does nothing (a second jump in the air with no double jump, an early press with no `buffer_frames`, a late one with no `coyote_frames`) stays as a check |
| Identical outcome | Of the active cells of one context whose outcome on the original, every mapped measure on every frame from the context's first start, is the same, the first in the file's order stays |

`corpus::scan` finds each context's starts in one pass of each recording: standing still, at walking or running top speed, at a ledge, in the air rising or falling, near an enemy, in an exit's box, in water, on a ladder, carrying, in a machine state. It keeps two starts the rebuilt game can begin at in each context (from different levels when it can) and one it can't, for its reason. A clip excites a number when the number doubled or halved changes the model's mapped measures on some frame. A number is covered when two independent clips move it, and two numbers of one mechanic that some clip moves together are told apart by a clip that moves one without the other, or two that move both in different ratios. The minimal set is a greedy cover, and `corpus::coverage::rank` orders the clips for the background queue: the mapping checks first, then starred clips, then the minimal set, then the rest, probes last.

## External runtimes

An external runtime is a program that can't load the original's state, but can start at a level start the way a player does and log a trace, one line a frame. `gp_interp::external` scores one with the lockstep, as the model is scored. Its first user, a probe of Super Mario Bros. Remastered, went with that game; no command here uses it.

| Part | What |
|---|---|
| `Trace` | The trace: a header line (the probe's version, the program, its version, the level, the settings it ran with), then rows from `f = -1`, where the player stands as it gets control before any press, then `f = 0, 1, ...`, each a frame's end with its presses in `in`. Every column but `f` is the program's own |
| `TraceMap` | A TOML file of `[[field]]`s: a model `path`, the trace's `column`, `scale` and `offset` for a number (`column * scale + offset`), `round` to the nearest whole, `flag` for a flag, `values` for a name |
| `TraceRuntime` | The trace through its map as a `gp_runtime::Runtime`: each step takes the next row and checks its presses, `query` answers the map's paths and `level`, and `load-state` loads nothing and says so (`missing`) |
| `probe_presses` | The presses to give the program: the input of each frame the original reads its controller, which is when the model steps |

## As WebAssembly

A game with hooks runs as one Wasm module: the interpreter, the presentation and the game's hook crate compiled together by `gp game build`, which `gp_module` runs under wasmtime with a deadline on every call and a cap on memory ([modules](modules.md)). A game with no hooks runs in the page's own build, `gp_web.wasm`, whose frame hashes `just web-parity` holds to the native interpreter's on Moss Hop ([parity](web.md#parity)).
