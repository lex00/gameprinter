# Catalog patterns

> Brought from game-printer with wave 9 of the port. The pattern library and its rounds are game-printer's record of Super Mario Bros., Mega Man 2 and Super Mario Bros. 2 and are kept as they were; those games' code is cut here. In gameprinter, `gp catalog GAME` takes a folder, an id under `templates/` or a game in the games checkout; it reads the asset map the game's `template.toml` names, the labels `gp labels` finds, and the hooks of the game's module with `--module` (else those built into gp). The behaviour finder tracks NES and arcade recordings alike (gp track reads both), so on Tapper it fits the tracked sprites of a store recording. The `catalog.<template>.*` metrics aren't recorded, since `gp catalog` doesn't hand them to the run recorder yet.

This page is the pattern library: the general shapes behind what three NES games (Super Mario Bros., Mega Man 2's Wood Man stage and Super Mario Bros. 2's world 1-1) do, found by putting their hooks, asset maps, RAM layouts and facts side by side. Each pattern has its parameters, the games that instance it, what the model holds it as (a component, an asset kind, a hook that names it), and how capture finds it in any game. `gp catalog <template>` runs the finders and prints what they found beside what the model holds.

Status: Milestone 2, step 8 (the catalog loop). The rule behind it is the user's (2026-10-28): every fix for one game is a general mechanic, primitive or asset kind that a later game can use, and game specifics are data. A hook stays only as a last resort, and its `Ext::describe` names the pattern it is an instance of (`like`), so `gp model mechanics` and the scorecard can count it as debt.

[Mechanics](mechanics.md) is how mechanics are held as data and how the catalog checks a model against their records. The [scorecard](scorecard.md) counts debt per game. The research behind the primitives is [docs/research/mechanics.md](research/mechanics.md).

## How to read a pattern

| Part | What it says |
|---|---|
| Parameters | The numbers and choices a game gives the pattern. They are the component's fields when the pattern is a component |
| Instances | Which entity, asset or RAM block in which game is one |
| Held as | A platformer component (`platformer.<name>`), an asset kind or format, a mapping section, or a hook whose `like` names the pattern |
| Capture | How a tool finds an instance in a game it has never seen: labels (`gp labels`), the asset map's shapes, a trace signature from `gp track`, or a measurement |

## Behaviour patterns

Each enemy and moving object of the three games is one or more of these. The trace signature is what `gp catalog`'s behaviour classifier looks for in a track's positions (docs/formats/tracks.md); the labels are the routine names that tend to go with it.

### oscillate

Swinging around a center on one axis: each frame the speed moves by a force toward the center, up to a top speed either way.

| Part | Value |
|---|---|
| Parameters | `axis`, `center` (offset from where it's placed), `force`, `max_speed`, and `low_at`/`low_center` for a center that depends on where it's placed |
| Instances | Super Mario Bros.' red paratroopa (InitRedPTroopa, ProcMoveRedPTroopa) and its vertical lift (YMovingPlatform, by the game's byte arithmetic) |
| Held as | `platformer.oscillate` since platformer 0.4.1; the red paratroopa is data. The vertical lift stays in `smb_lift`, whose record names `oscillate` |
| Capture | Trace: a coordinate whose frame-to-frame change is a straight line in time with one slope, changing sign at a fixed position, and capped; the turning points are symmetric about the center. Measured: `force` is the slope of the speed, `max_speed` the cap, `center` the mean position. Labels: `Move*`, `*PTroopa`, `YMoving*` |

### shuttle

Moving back and forth along an axis by a counter: a step every few frames, turning after a number of steps or at the ends of a range, with an optional pause at each end.

| Part | Value |
|---|---|
| Parameters | `axis`, `step` (pixels), `every` (frames), `turn_after` (steps) or `distance`, `pause` |
| Instances | Super Mario Bros.' green flying paratroopa (XMoveCntr_GreenPTroopa, with a y sway by the frame counter's bit 6) and horizontal lift (XMovingPlatform), the piranha plant, Super Mario Bros. 2's Birdo pacing (still, left, still, right, 64 frames each), the hammer brother's shimmy |
| Held as | `platformer.shuttle` since platformer 0.4.1 (`axis`, `step`, `every`, `turn_after`, `sway`), and `platformer.path` (out by `distance` at `speed`, `pause`, `wait_near`) and `platformer.patrol` (`step` every `period` frames between two columns). The green flying paratroopa is a `shuttle` with a sway, the piranha plant a `path`. The horizontal lift stays in `smb_lift` (the game's byte arithmetic), Birdo's pacing in `smb2_birdo` and the hammer brother's shimmy in `smb_hammer_bro` (both a part of a larger hook) |
| Capture | Trace: `gp track`'s `turns` and `period`, with a constant speed between turns. Steps of a whole pixel on frames at a fixed interval give `step` and `every`. The positions at the turns give the range |

### leap

Launched from a rest point with a speed, pulled back by gravity, then waiting before the next launch.

| Part | Value |
|---|---|
| Parameters | `speed` (the launch), `gravity`, `max_fall`, where it starts (an absolute position or its spawn), the wait (an interval times a random choice), an optional slow rise before the drop |
| Instances | Super Mario Bros.' podoboo (MovePodoboo: leaps at -7 from below the screen, waits 6, 7, 14 or 15 intervals of 21 frames), Super Mario Bros. 2's falling log (rises 12 pixels, then drops a sixteenth faster every 8 frames) |
| Held as | `platformer.leap` since platformer 0.4.1 (`speed`, `gravity`, `max_fall`, `every`, `from`, `rise`, `interval`, `waits`, `below`); both are data. The podoboo's `waits` lists the 16 values of the game's LFSR nybble or'd with 6, so the model's generator draws them with the same odds |
| Capture | Trace: a parabola in y (second difference constant) that starts at the same position each time, with the object gone or still between parabolas. The constant second difference is `gravity`, the first frame's change the `speed`, the gaps between launches the wait. Labels: `*Podoboo`, `*Jump*`, `*Log*` |

### hop

Jumping again on landing, by a speed, a pattern of speeds in turn, or only on a beat.

| Part | Value |
|---|---|
| Parameters | `speed`, `pattern` (speeds in turn), `every` and `phase` (a beat, shifted per object), and, for a hop toward the player, a sideways speed |
| Instances | Super Mario Bros. 2's Tweeter (three small hops and a big one), the jumping Ninji (on a 64-frame beat by the frame counter plus eight times its slot, four speeds in turn), Birdo's hop every 256 frames; Mega Man 2's robbit (hops toward the player after a wait); the hammer brother's jumps between floors |
| Held as | `platformer.hop` with `pattern`, `every` and `phase` since platformer 0.4.1; the Tweeter and the Ninji are data. The robbit stays in `mm2_robbit` (`like = "hop"`): it also throws, and its hop carries a sideways speed while airborne |
| Capture | Trace: y repeats an arc that starts at the landing height; the arcs' rises cycle through a short list (the pattern), and the takeoff frames modulo a power of two cluster at one value per object (the beat). Measured: the rises give the speeds by the fall's gravity |

### pursue

Moving toward the player: re-aiming at a fixed period, by bursts, or accelerating toward the player's position.

| Part | Value |
|---|---|
| Parameters | `speed` or `force` and `max_speed`, the re-aim period, which axes, and what makes it start and stop |
| Instances | Super Mario Bros.' blooper (swims up in bursts, floats down toward the player), Lakitu (follows the player's x at a speed by distance, LakituDiffAdj), the spiny walking toward the player once it lands; Mega Man 2's batton (hangs, then swoops along the line to the player, re-aimed every animation cycle by the game's 16-bit division) |
| Held as | Hooks `smb_blooper`, `smb_lakitu`, `mm2_batton` (`like` `pursue` or `follow`). `platformer.walk` starts toward the player and `fly.heading = "player"` aims once, which are the simplest cases |
| Capture | Trace: the sign of the object's velocity follows the sign of (player - object) with a lag; the speed's magnitude is constant (aim) or grows to a cap (accelerate). The re-aim period is the gap between velocity changes. Labels: `*Chase*`, `*Follow*`, `calc_*velocity` |

### orbit

Parts placed on circles around a pivot by an angle that turns each frame.

| Part | Value |
|---|---|
| Parameters | `spin` (angle per frame and its direction), `parts`, each part's radius, the angle steps per turn, the position table that turns an angle into an offset |
| Instances | Super Mario Bros.' firebars (ProcFirebar, FirebarSpin, GetFirebarPosition with the lifted firebar tables): short and long, slow and fast, both directions |
| Held as | `platformer.orbit` (the pivot: `spin`, `angles`, `parts`, `part`, `place`, `skip`, `in_view`) and `platformer.orbit_part` (a part: `within`, and the quarter-turn tables `offsets`, `rows` and `signs` as `ext_tables`) since platformer 0.4.1. All five firebars and their fireballs are data |
| Capture | Trace: several objects of one palette whose positions keep a fixed distance from one point and whose angle about it changes by a constant each frame. The distances give the radii, the angle step the spin. Asset map: a table of small offsets whose length is a multiple of the number of angles in a quarter turn |

### throw

Making a projectile on a timer or a trigger, from an offset, with a speed.

| Part | Value |
|---|---|
| Parameters | the entity thrown, the timer (`every`) or trigger, the offset it appears at, its speed, how many may be out at once |
| Instances | Lakitu's spinies, the hammer brother's hammers, Bowser's fire and hammers; Mega Man 2's robbit carrots and Mega Man's buster; Super Mario Bros. 2's Birdo eggs and the player's throw of a carried enemy |
| Held as | `platformer.shoot` (entity, `action` or `every`, `ahead`, `max`) for Mega Man's buster; `platformer.carry` for the throw of a carried thing. The enemies' throws are inside their hooks (`smb_lakitu`, `smb_hammer_bro`, `smb_bowser`, `mm2_robbit`, `smb2_birdo`) |
| Capture | Trace: a new track starts at a fixed offset from an existing track, at a regular interval; the new track's kind is the projectile. Labels: `*Throw*`, `*Spit*`, `*Shoot*`, `Spawn*` |

### projectile

What a thrown thing does: fly straight, fall in an arc, level out, or become something else when it lands.

| Part | Value |
|---|---|
| Parameters | the starting speed on each axis, `gravity`, `max_fall`, a height it levels out at, what it becomes on landing |
| Instances | Mega Man's buster shot (straight), the hammer (held, then a ballistic arc), Bowser's flame (flies left and levels out at one of four heights), the spiny's egg (falls and becomes a walking spiny), Birdo's egg (straight) |
| Held as | `platformer.fly` (straight), `platformer.fall` (an arc), `become:<entity>` in a machine (what it becomes). The hammer, the flame and the egg's landing stay in hooks (`smb_hammer`, `smb_bowser_flame`, `smb_spiny`, `like = "throw"`) |
| Capture | Trace: a track that starts at another track's position; a constant vx with vy 0 is `fly`, a constant second difference in y is `fall`'s gravity, a y that settles to one of a few values is the levelling heights |

### leave view

Leaving the level once off the screen.

| Part | Value |
|---|---|
| Parameters | `margin` and `axis` |
| Instances | Mega Man 2's buster shot (gone 8 pixels inside either screen edge, entity_physics_core) |
| Held as | `platformer.leave_view` since platformer 0.4.1; `mm2_shot` is gone. The level's `despawn_behind` and `despawn_ahead` are the same for placed enemies, by the spawn distance |
| Capture | Trace: tracks end (the slot's active flag clears) at the same screen x on both sides, a margin from each edge |

### boss

A composition of the patterns above under a mode machine: pace, hop, throw and face the player, with hit points.

| Part | Value |
|---|---|
| Parameters | each part's own, the modes and the timers between them, `health` |
| Instances | Super Mario Bros.' Bowser (paces a random range, jumps, fire in worlds 1 to 5 and 8, hammers from world 6), Super Mario Bros. 2's Birdo (faces the player, paces, hops every 256 frames, spits eggs) |
| Held as | Hooks `smb_bowser` and `smb2_birdo`, `like = "boss"`, with `health` as a component |
| Capture | The parts are found one by one (shuttle, hop, throw). The mode switches are the frames where the classifier's best pattern changes for one track |

### carry and stand

The player standing on, lifting, carrying and throwing other objects.

| Part | Value |
|---|---|
| Parameters | `contact.stand`, `carry` (`action`, `lift_frames`, `offset`, `throw`), `carryable` |
| Instances | Super Mario Bros. 2's Shy Guys, Tweeters, Ninjis and eggs; Super Mario Bros.' lifts (carrying the player is not modeled) |
| Held as | Components since platformer 0.4.0 |
| Capture | docs/research/smb2-schema-gaps.md, "What core 0.4.0 settled": the player's y equals a slot's top while the slot moves; a slot's position follows the player's at a fixed offset after a button press |

### player physics

The player's run and jump, with the game's own arithmetic.

| Part | Value |
|---|---|
| Parameters | `run` (speeds, accelerations, skid, `start`, `air_speed`), `jump` (speed, gravities, `bands`, `release_cap`), `climb`, `knockback` |
| Instances | Mario (bands by takeoff speed), Mega Man (slow first steps, a jump cut), Super Mario Bros. 2's characters (gravity picked by the jump button every frame, a charged jump, collision points) |
| Held as | Components for Mario and Mega Man; `smb2_player` stays a hook (`like = "run-jump-carry"`): the game moves by sign and magnitude and lands only in a window of the tile (rows S2-16 to S2-18 of docs/research/smb2-schema-gaps.md) |
| Capture | `gp model propose` (docs/mechanics.md, "Measuring feel"); the asset map's physics tables give the stated values (below) |

## Asset patterns

The three asset maps hold 1,251 assets in 21 formats. They fall into a few kinds, which `gp catalog`'s table-shape classifier names from each entry's format, shape and name:

| Pattern | Formats | Shape | Instances | Model kind |
|---|---|---|---|---|
| Tile graphics | `chr-2bpp`, `mm2-chr-uploads` | 16 bytes a tile | All three: the CHR banks; Mega Man 2 uploads CHR RAM from PRG | `tiles` |
| Palettes | `palette`, `mm2-palettes` | 4 colors a palette, sets of 8 | Super Mario Bros.' per area type, Mega Man 2's per stage, Super Mario Bros. 2's per world | `palettes` |
| Metatile sets | `metatiles`, `mm2-metatiles` | 4 tile numbers (2 by 2) and an attribute | All three; Mega Man 2's are 4 by 4 of 2 by 2 | `metatiles` |
| Object lists (level geometry) | `smb-area-objects`, `smb2-level-objects`, `mm2-screens` | variable-length records with a terminator, one list per area or screen | All three | `level-objects` |
| Spawn lists | `smb-enemy-objects`, `smb2-enemy-objects`, `mm2-spawns` | records of (position, kind) | All three | `spawn-objects` |
| Pointer tables | `pointers` | split low and high bytes, or words, into a region | Super Mario Bros.' area and enemy pointers, Super Mario Bros. 2's 25 | (read by the decoders) |
| Per-kind property tables | `table` with `count` equal to the number of object kinds, several columns | one row an object kind | Mega Man 2's `enemy_ai_routine`, `enemy_spawn`, `boss_ai_flags`; Super Mario Bros. 2's `enemy_initialization` and `enemy_behavior_pointer` (142 kinds) | `table`, read by `ext_tables` or a slot's `entities` |
| Physics tables | `table` or `byte-rows` whose names say speed, velocity, acceleration, gravity, force or jump | a few rows of speeds | Super Mario Bros.' `player_physics`, `player_max_speed_*`; Super Mario Bros. 2's `player_control_acceleration`, `enemy_ninji_jump_velocity`, `character_stats`; Mega Man 2's `airman_jump`, `quickman_y_vel` | `table`; the values are a component's parameters, cited by evidence |
| Motion tables | `table` of small offsets by angle or step | a multiple of a quarter turn | Super Mario Bros.' firebar positions | `table`, an `ext_tables` entry marked mechanic |
| Sprite frames and OAM layouts | `byte-rows` named frames, sprites, oam | rows of tile numbers or of (y, tile, attribute, x) | Super Mario Bros.' `enemy_frames`, `player_frames`; Mega Man 2's `boss_portrait_oam` | `rows` (sprite frames); the `present` lane's OAM layouts |
| Collision tables | `table` or `byte-rows` named hitbox, bounding, collision, solid | boxes and solidity by kind or tile | Super Mario Bros.' `bounding_boxes`, Mega Man 2's `hitbox_width_lo` and `wall_solid_flag`, Super Mario Bros. 2's `pokey_hitbox` and `enable_collision_flag` | hitboxes and `tile_kinds` |
| Timing and chance tables | names with rng, random, delay, ranges, heights, times | waits, masks and choices | Super Mario Bros.' `bowser_ranges` and `bowser_flame_heights`, Mega Man 2's `cannon_fire_rng_mask` and `heatman_random_delay`, Super Mario Bros. 2's `birdo_spit_delay` | a component's `waits`, a hook's `ext_tables` marked content |
| Screen layouts | names with ppu_, nametable, layout, buffer | whole screens or PPU writes | Mega Man 2's `password_ppu_layout`, Super Mario Bros. 2's `title_layout` and the ending's screens | presentation |
| Text | `byte-rows` with a `charset`, `ppu-strings` | characters | Super Mario Bros.' names and messages, Super Mario Bros. 2's story | presentation |
| Music and sound | `smb-music`, `byte-rows` named note, envelope, freq, sfx | notes and lengths | All three | presentation |
| Level structure tables | `table` named area, world, screen, checkpoint, link | one row a level or screen | Super Mario Bros.' `world_areas`; Mega Man 2's `*_screen_links`, `*_checkpoints`; Super Mario Bros. 2's `level_area_starts` | level fields (`next`, exits) |
| Code and other bytes | `blobs` | anything | Mega Man 2 and Super Mario Bros. 2 | none |

The table-shape classifier is the asset-map half of capture: a new game's map, written from its labels (`gp labels` data labels with their sizes), sorts into these kinds before anyone reads a byte. A kind the model has no slot for is what to lift next; a physics or per-kind table whose values are not yet a fact is a fact to state.

## RAM patterns

| Pattern | Parameters | Instances | Held as | Capture |
|---|---|---|---|---|
| Object slot array | base address per field, slot count, stride, active flag, ID byte | Super Mario Bros.: 5 enemy slots (plus player, fireballs, blocks and misc objects as neighbours in one array, `SprObject_*`); Mega Man 2: 32 entity slots (`ent_*` at $0400 + 16 + i); Super Mario Bros. 2: 9 slots (`ObjectXLo` and friends) | A state mapping's `[slots]` (docs/formats/state-mapping.md) | Labels: a family of names sharing a suffix with different prefixes at consecutive addresses (`Player_X_Position`, `Enemy_X_Position`), or one name with a size above 1; the gaps give each class's slot count. Traces: `gp track --propose-slots` |
| Position as page, pixel and fraction | three bytes per axis | All three games keep x as a page byte, a pixel byte and a sub-pixel byte | `[slots.field]` `ram` (high first) and `frac` | Labels with `_Page`, `_Hi`, `_HighPos`, `_Lo`, `_Sub`, `_MoveForce`; the tracker's page byte |
| Velocity and sub-velocity | a speed byte (sign and magnitude or two's complement) and a fraction | All three | the body's `vx` and `vy` | Labels with `Speed`, `Vel`, `XMoveForce`; a byte whose sum each frame is the position's change |
| Frame counter | one byte | `FrameCounter`, `frame_counter`, `byte_RAM_10` | `[slots].clock` | Labels; the tracker's clock |
| Per-slot state and timers | a byte per slot | `Enemy_State`, `ent_state`, `EnemyState`, `EnemyTimer` | `[slots].state`, a hook's `ext` words | Labels in the slot family that change only on events |
| Game mode | one byte | `OperMode`, `game_mode`, `GameMode` | the rules' states, the slots' `moving` | Labels; a byte constant through gameplay that changes at deaths and level ends |

## Fact clusters

Facts come in clusters that recur across games: each cluster is one place in a game's code and data, and one part of the model.

| Cluster | Model part | Where the facts come from |
|---|---|---|
| Player run and jump | `entities.<player>.run`, `.jump` | A physics table (Super Mario Bros.' `player_physics`, Super Mario Bros. 2's `character_stats`), the move routine's constants, `gp model propose` |
| An enemy kind's motion | the entity's movers (`walk`, `path`, `oscillate`, `hop`, `fall`) | Its init routine's constants and its move routine; the slot comparison measures it (`gp interp slots`) |
| An enemy kind's touch | `contact`, `health`, `hits` | The collision routine and the per-kind tables (damage, hit points) |
| Spawning | `spawn_objects`, `spawn_ahead`, `despawn_behind` | The spawn list format and the spawn distance in the code |
| Level geometry | `layout`, `tile_kinds` | The object-list format and the solidity table |
| Flow | `rules`, `counters`, `exit` | The game-mode byte, the counters the HUD draws, the exits' code |

## The finders

`gp catalog` runs these on a template, with whatever the machine has: the asset map is in the repo, labels are under `$GP_HOME/work` (docs/formats/labels.md), and traces come from store recordings.

| Finder | Input | What it proposes |
|---|---|---|
| Model patterns | `model.toml` and the hooks' records | Each entity's patterns, from its components and its hook's `like`; which are data and which are hooks |
| Table shapes | `assets/<source>.toml` | Each asset's pattern (above), counted by pattern, against the slots the model fills |
| Slot arrays from labels | the labels file | Families of RAM labels that form object slot arrays, with their slot counts, against the mapping's `[slots]` |
| Behaviour from traces | a recording's frame trace, through `gp track` | Each track's best-fitting behaviour pattern with its parameters, grouped by kind, against the entity the mapping's `[[slots.kind]]` names for that ID |

## Rounds

Each round compares the games, turns the strongest patterns into components, asset kinds or finders, and is scored with `just scorecard` (the `scorecard.*` metrics in `$GP_HOME/metrics.sqlite`). Debt is `gp model mechanics`' count: hook records whose `like` names no component. Hooks is the number of distinct hooks the model still names.

| Round | Change | Super Mario Bros. hooks / debt | Mega Man 2 hooks / debt | Super Mario Bros. 2 hooks / debt |
|---|---|---|---|---|
| Start (fce1dec) | | 13 / 13 | 3 / 2 | 5 / 3 |
| 1 | Platformer 0.4.1: `oscillate`, `leave_view`, `hop.pattern`, `hop.every` and `hop.phase`. The red paratroopa, the buster shot, the Tweeter and the jumping Ninji are data. Every hook's `like` is a pattern name from this page | 12 / 11 | 2 / 1 | 3 / 3 |
| 2 | Platformer 0.4.1 `leap`: the podoboo and the falling log are data. `gp catalog` with its four finders; `gp_track::behaviour` | 10 / 10 | 2 / 1 | 2 / 2 |
| 3 | Platformer 0.4.1 `shuttle`, `orbit` and `orbit_part`: the flying green paratroopa, the five firebars and their fireballs are data. The hammer brother's `like` is `throw` | 8 / 7 | 2 / 1 | 2 / 2 |
| 4 | The table-shape classifier learns collision, timing and chance, and screen-layout tables; `just catalog` in the ROM tier | 8 / 7 | 2 / 1 | 2 / 2 |

Each round's numbers from `gp catalog` (parameter facts in general components against those in hooks, the asset maps' assets the table-shape classifier places, the slot arrays the labels give, and the behaviour finder on 30,000 frames of a store recording each) and `gp model derive` (the share of hand-written facts derived from what their evidence cites, all of them agreeing):

| Round | Game | Parameter facts, components / hooks | Assets classified | Labels: arrays, mapping's slots found | Behaviour: mapped kinds that agree | Derived |
|---|---|---|---|---|---|---|
| 2 | Super Mario Bros. | 652 / 117 (84.8%) | 87 of 96 into 14 patterns | 2; active, ID and state in `Enemy` (6 slots) of `Player:1 Enemy:6 Fireball:2 Block:2 Misc:9` | 3 of 4 on 782c557d | 2,446 of 3,296 (74.2%) |
| 2 | Mega Man 2 | 211 / 15 (93.4%) | 376 of 416 into 15 | 3; active and ID in a 16-slot sized array of 38 fields | 0 of 1 on 35a900f9 | 58 of 246 (23.6%) |
| 2 | Super Mario Bros. 2 | 181 / 16 (91.9%) | 458 of 530 into 15 | 1; active, ID and state in `Enemy` and `Object` (9 slots) | 2 of 2 on c31a0471 | 215 of 466 (46.1%) |
| 3 | Super Mario Bros. | 685 / 91 (88.3%) | as round 2 | as round 2 | 3 of 4 | 2,450 of 3,299 (74.3%) |
| 4 | Super Mario Bros. | 685 / 91 (88.3%) | 95 of 96 into 16 | as round 2 | 3 of 4 | 2,450 of 3,299 (74.3%) |
| 4 | Mega Man 2 | 211 / 15 (93.4%) | 402 of 416 into 18 | as round 2 | 0 of 1 | 58 of 246 (23.6%) |
| 4 | Super Mario Bros. 2 | 181 / 16 (91.9%) | 507 of 530 into 18 | as round 2 | 2 of 2 | 215 of 466 (46.1%) |

The behaviour finder's disagreements are findings. Super Mario Bros.' horizontal lift moves like a walker or shuttle, while the lift hook's one record says `oscillate` for all nine lift routines; Mega Man 2's batton is a faller by its tracks because it swoops down most of its life. Walkers the TAS passes before they turn fit `fly`, which the finder counts as agreeing with `walk`.

Round 1's comparisons, before and after, on this machine: `gp interp compare` on Mega Man 2's Wood Man baseline and Super Mario Bros. 2's enter-1-1 baseline, `gp interp slots` on Mega Man 2's baseline, on Super Mario Bros. 2's c31a0471 and on Super Mario Bros.' 782c557d and c8f4e908, and `gp interp compare` on Super Mario Bros.' run-right: every line is the same but the hooks' label. The red paratroopa's slots (6 lives on each recording) and the Tweeter's (5 lives, 542 frames) give the same numbers as their hooks did. `just conform-smb` (3,785 frames), `conform-mm2` (726) and `conform-smb2` (1,019) give the interpreter's state hash on every frame. The SMB lift and the green flying paratroopa were counted under `oscillate` before; the green paratroopa is a `shuttle` (a counter, not a force), so it stays debt, and the lift stays a hook that names `oscillate` for the game's byte arithmetic. Mega Man 2's robbit record asked for `buster_damage` and `hp`, which its `health` component holds since core 0.4.0, so `gp model mechanics` reported two errors; the record no longer asks.

Round 3 left every Super Mario Bros. comparison and slot line as it was, the firebars' spin state on every frame included, and `conform-smb` and `conform-smb2` agree on every frame. Round 4 changes no hook. A fifth round would make no progress by the same means, so the loop stops here: every remaining hook does its game's own arithmetic, which no general component reproduces frame for frame (below).

## What is general now, and what stays a hook

General since this loop, each a platformer 0.4.1 component with its record in `gp_sim::mechanic`, its schema in `schema/platformer.json` and its reading in docs/model.md: `oscillate`, `shuttle`, `leap`, `hop.pattern` with `hop.every` and `hop.phase`, `orbit` and `orbit_part`, `leave_view`. Ten entities of the three games moved from hooks to data with them, and seven hooks were deleted: `smb_paratroopa_red`, `smb_podoboo`, `smb_paratroopa_green_flying`, `smb_firebar`, `smb_firebar_flame`, `mm2_shot`, `smb2_tweeter`, `smb2_ninji_jumping`, `smb2_falling_log` (nine names; the firebar's covers five entities). The finders are general too: they read only labels, asset maps and traces.

| Hook | Pattern (`like`) | Why it stays |
|---|---|---|
| `smb_lift` | oscillate | The game's platform routines in its own byte arithmetic (`motion.rs`: speed and force bytes that wrap); `oscillate` in fixed point would move the slot comparisons. Its one record says `oscillate` for all nine routines, which the behaviour finder disputes for the horizontal lift |
| `smb_blooper`, `smb_lakitu` | pursue | Bursts by the LFSR and a distance table (LakituDiffAdj) in byte arithmetic; no `pursue` component yet |
| `smb_spiny` | projectile | The egg lands only at the game's foot point and y window (LandEnemyProperly), then becomes a walker facing the player on frames the frame counter picks |
| `smb_hammer`, `smb_bowser_flame` | projectile | Held over the thrower, then an arc, in byte arithmetic; the flame levels out at a random height with a hard-mode speed |
| `smb_hammer_bro` | throw | Shimmy, jumps between floors by the LFSR and hammers on a timer, in one routine |
| `smb_bowser`, `smb2_birdo` | boss | Mode machines of pace, hop, throw and facing whose timers interleave inside one routine |
| `mm2_batton` | pursue | Aims by the game's 16-bit division, quirk included, and its shield depends on its own state |
| `mm2_robbit` | hop | Hops toward the player after a wait, then throws on a timer, with its own wall test while airborne |
| `smb2_player` | player-physics | Gravity by the button every frame, a charged jump, moving by sign and magnitude and landing in a tile window (docs/research/smb2-schema-gaps.md, S2-16 to S2-18) |

The pattern most of these share is the NES's byte arithmetic for motion: a speed byte and a sub-pixel byte that wrap, signs and magnitudes, and a frame counter or LFSR picking when. Super Mario Bros.' `motion.rs` and Super Mario Bros. 2's player hook both carry it. Making that an integrator option of the components (a body that moves by bytes) is the next general primitive; it is filed as an M2.8 row in docs/open-questions.md, with `pursue`, a `throw` for enemies, and per-entity records for a hook that covers several routines.
