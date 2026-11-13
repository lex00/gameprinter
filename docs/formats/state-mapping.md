# State mapping format

A state mapping says where the original game keeps each part of a model's state, so a run of the original and a run of the model can be compared frame by frame. The file is `mapping.toml` in a game's package folder (or `templates/mosshop/` for Moss Hop), next to `model.toml` and `fidelity.toml`, versioned with the model. `crates/gp-interp/src/mapping.rs` parses it, and `gp interp compare`, the corpus scorers and `gp validate` read through it. This is schema 22, and its optional parts are named features ([versions](#versions)). Its JSON Schema is `schema/mapping.json`, which `gp self schemas` writes from the Rust types. It came from game-printer's docs/formats/state-mapping.md; the format is the same, with an arcade platform added, and its examples from other games' mappings are replaced by Donkey Kong's and Moss Hop's.

Each field maps one model path in the interpreter's state to RAM bytes in the original, with a few simple transforms. The state mapping is data, not code: a new game needs a new file, written from its RAM map or its code, and nothing in the interpreter changes. `gp game new` writes an empty one, and the first work on a new game is mapping the player's place.

## Example

Moss Hop's, `templates/mosshop/mapping.toml`, in part:

```toml
schema = 22
model = "mosshop"
source = "mosshop"
align = "polled"

[[field]]
path = "entities.player[0].x"
ram = [0x10, 0x11]
note = "PLAYER_X, PLAYER_X_HI"

[[field]]
path = "entities.player[0].vy"
ram = [0x14]
frac = 0x15
signed = true
note = "PLAYER_VY, PLAYER_VYSUB"

[[field]]
path = "rules.game.state"
ram = [0x05]
values = ["playing", "dying", "clear", "game_over"]
note = "STATE"
```

## Fields

| Key | Meaning |
|---|---|
| `schema` | The format's version, 22 ([versions](#versions)) |
| `features` | Optional: the features the mapping uses, by name ([versions](#versions)) |
| `model` | The model's `model.name`; `gp interp compare` refuses a mapping of another model |
| `source` | The original's source ID, as its asset map and baselines name it |
| `platform` | Optional (feature `platform`): `nes` (the default) or `arcade` ([an arcade original](#an-arcade-original)) |
| `description` | Optional, for people |
| `align` | How the two runs' frames line up (below). `polled` is the only alignment so far |
| `read_at` | Optional (feature `align.read_at_nmi`): `nmi` reads the original's RAM as the CPU takes the NMI after the frame's end, instead of at the end (below) |
| `start` | Optional: a condition over the original's state, in the runtime protocol's condition grammar, that marks the original's frame the model's power-on state stands for: the first frame it holds (below) |
| `field` | The mapped values, a list |
| `slots` | Optional: the original's reused object slots (below) |

Each `[[field]]` takes these keys.

| Key | Meaning |
|---|---|
| `path` | A concrete model path in the interpreter's state, such as `entities.player[0].x`, a counter's count toward an `every` change, `ticks.<counter>[i]` ([below](#counts-toward-an-every-change)), or a body's animation, `present.<kind>[i].frame` or `.timer` ([below](#a-bodys-animation)). Each path is mapped once |
| `ram` | RAM addresses, low byte first: 1 to 5 bytes making one number, or up to 7 for a plain whole number (no `frac`, `scale`, `offset`, `bits`, `signed`, `flag`, `values` or `names`), or 8 with the feature `field.whole_64`, for a 64-bit random register |
| `nametable` | Optional (feature `field.nametable`): name table cells by PPU address, $2000 to $3EFF, low byte first, in place of `ram` ([below](#the-ppus-state)) |
| `oam` | Optional (feature `field.oam`): sprite memory bytes by index, 4 times the slot plus the byte (0 y, 1 tile, 2 attributes, 3 x), in place of `ram` ([below](#the-ppus-state)) |
| `oam_tile` | Optional (feature `field.oam_tile`): `{ tiles = [...], byte = B, none = N }`, byte B of the first sprite by slot whose tile is one of `tiles`, or the number N when none is, in place of `ram` ([below](#the-ppus-state)) |
| `machine` | Optional (feature `field.machine`): a selector of the original's runtime protocol whose number is the field's, read as 4 bytes low first so `bits` and `flag` apply, in place of `ram`: state the console keeps outside memory, such as gp-nes's `nmi.return_pc` (where the next NMI stops the code) or `nmi.cpu.cycles`. Donkey Kong's idle loop loads its place at a clip's start this way ([cycles](../cycles.md)) |
| `signed` | Optional: the bytes are a two's complement number |
| `bits` | Optional: only bits `[from, to)` of the number, for a value packed with others |
| `frac` | Optional: the address of a byte holding the fraction, in 256ths, as NES games keep subpixels |
| `scale`, `offset` | Optional: the model's value is the number times `scale`, plus `offset`, both fixed-point (an integer or an exact decimal string) |
| `flag` | Optional: the number is a flag, true unless zero; with `invert`, true only when zero |
| `digits` | Optional: the bytes are decimal digits, most significant first, as games keep scores and timers for display, rather than one number low byte first. Up to 14 |
| `lookup` | Optional (feature `field.lookup`): pairs `[number, value]`, the model's value for each number the bytes hold, for a byte a game's script steps unevenly; a number not listed is taken as it is, or with `lookup_else` reads as that value. Donkey Kong's death wait at $43 counts down one a divider wrap with extra steps at set values, so its pairs give the wraps left to the restart |
| `bcd` | Optional (feature `field.bcd`): each byte holds two decimal digits (packed BCD, the high nibble first), most significant byte first: 1 to 7 bytes, no `digits`, `signed`, `bits` or `frac`; `scale` and `offset` apply after. Donkey Kong keeps its score at $25-$27 this way and its bonus timer, in hundreds, at $2E |
| `whole` | Optional: compare whole parts only, the model's value rounded down, for an original that keeps no fraction the model has. With `frac` as well (feature `field.whole`), a translation loads the fraction and runs still compare whole parts: a fraction the game keeps that the screen doesn't show |
| `values` | Optional: the number names one of these, by position: 0 is the first |
| `names` | Optional (feature `field.names`): the number names one of these, by number, as `[[0, "walking"], [0x84, "sliding"]]`: for a byte whose values are sparse, such as a state byte with flags in its high bits. A number none has is unnamed |
| `case` | Optional: conditions that say more than the bytes do ([below](#cases)) |
| `presentation` | Optional (feature `field.presentation`): a presentation value. A translation loads it, but no run compares it: it is no measure, in no score and no split |
| `note` | Optional, for people: the original's names for the bytes, and how they were found |

A field is a number, a flag or a named value (`values` or `names`), never more than one: `frac`, `scale` and `offset` go with numbers only. Numbers compare by value whatever their kind, so the original's whole number 24 equals the model's fixed-point 24, and a named value the original holds no name for reads as `(unnamed value N)`, which never matches.

## The PPU's state

Some games keep a value only on the screen. Donkey Kong draws Kong as background tiles: $C815 queues the pose $EB54 picks for the name table at $2084, and no RAM byte keeps which pose that was. A field can read the original's PPU state in place of RAM, from the console state the runtime holds: its name tables, the cartridge's mirroring and OAM. The keys are these.

| Key | Reads |
|---|---|
| `nametable = [0x20C5]` | Name table cells by PPU address. The address goes through the cartridge's mirroring as it stands (0 vertical, 1 horizontal, 2 one-screen lower, 3 one-screen upper, 4 four-screen) to its byte of the name tables (`gp_interp::mapping::nametable_index`) |
| `oam = [0xF1]` | Sprite memory bytes by index: slot 60's tile is 0xF1 |
| `oam_tile = { tiles = [0x50, 0x51], byte = 3, none = 0 }` | The first sprite, by slot, whose tile is one of `tiles`: its byte `byte`, or `none` when no sprite has one, for a sprite a game puts in no fixed slot |

The rest of the field (`lookup`, `offset`, `bits` and the cases) applies to the bytes read as to RAM's. A case's `ram` reads RAM in place of the PPU while it holds: a value on its way to the screen, such as a write a game keeps in a buffer for the next NMI. The PPU is read as the frame ends, so the name tables and OAM hold what the frame's vblank writes and DMA sent: OAM is the sprite table the game wrote the frame before. A reader that holds a frame's RAM alone reads such a field's bytes as 0. Slot fields read RAM only.

Donkey Kong's `counters.kong_pose` reads $20C5, whose tile is another in each of Kong's six poses, with a `lookup` from the tile to the pose; `counters.kong_drawn` reads the same, or the buffer at $0331 while it holds Kong's columns; `counters.pauline_shown` reads `oam = [0xF1]`, the $02F1 the DMA sent.

## Cases

A field's bytes don't always say all of a value. Donkey Kong reads Mario's x from his sprite at $0203, which holds something else at times: the title's cursor, the GAME OVER text, or nothing while the game clears the sprites in a death's wait. Each `[[field.case]]` has a condition over the original's state, in the runtime protocol's condition grammar. The first whose condition holds applies.

| Key | Meaning |
|---|---|
| `when` | The condition, such as `ram[0x0096] == 255 && ram[0x0200] == 255` |
| `value` | Optional: the field's value while `when` holds, whatever the bytes say. A name, so only for a field with `values`; it can be a name the bytes alone never give |
| `approximated` | Optional: why a model state taken from the original while `when` holds is only near the original's. A translation notes the field `approximated` with this reason after its own |
| `number` | Optional (feature `field.case.number`): a number field's value while `when` holds, whatever its bytes say. Donkey Kong keeps a jump's takeoff height in $95 and puts $FF there once the fall is too far; its mapping loads the takeoff word with `number = -1000` then, a takeoff far above, so the landing kills as the game's does |
| `ram` | Optional (feature `field.case.ram`): addresses the field reads in place of its `ram` while `when` holds, as many as it has. Donkey Kong keeps each player's score at $25 + 4 times the player in $52, so the current player's score reads $29-$2B on player 2's turn, and player 2's pad is $17 where player 1's is $15 |
| `oam` | Optional (feature `field.case.oam`): for a field that reads OAM, the sprite table bytes it reads in place of its own while `when` holds, as many. Donkey Kong: Original Edition's 50m sends its sprites in turns from page $07 ($895F), so Pauline's body tile is OAM's $41 there, not $F1 |
| `nametable` | Optional (feature `field.case.nametable`): for a field that reads the name tables, other PPU addresses while `when` holds, for a game that draws a picture at another place on one stage |
| `unmeasured` | Optional (feature `field.case.unmeasured`): why the field means nothing to compare while `when` holds. A run doesn't compare it on those frames, which count as agreeing; a translation still loads it. Donkey Kong counts Kong's throw timer and the 11-frame divider on through a death and reuses $43 as the death's timer, while the model holds them and restart-level sets them again, so its mapping leaves them unmeasured while $96 is 255. A field unmeasured on a frame whose path names nothing in the model's state then (a body of a kind the level has none of: Donkey Kong's hammers on 75m) isn't read |
| `note` | Optional, for people |

A case gives a `value` or a `number`. It may give an `approximated` reason or an `unmeasured` reason instead, or more than one of these. A flag's case (feature `field.case_flag`, found only by being listed) gives `true` or `false`, for a flag its bytes don't always say: Donkey Kong's climbing flag is bit 1 of $96, which the hammer's state 10 sets too. The comparison and the lockstep read the field with its cases, and so do translate and the inspector.

```toml
[[field]]
path = "entities.player[0].x"
ram = [0x0203]
offset = 5
whole = true
note = "Mario's x: the sprite buffer's $0203 + 5"

[[field.case]]
unmeasured = "Late in a death's wait the game clears the sprites to redraw the screen, Mario's too ($0200 at 255)"
when = "ram[0x0096] == 255 && ram[0x0200] == 255"
```

## Counts toward an `every` change

A counter that changes every so many frames (`every = 24` in the model) has a value and a count of the frames since its last change. The value alone doesn't say when the next change comes, so a model started from the original's state would change on its own count. `ticks.<counter>[i]` (feature `ticks`, found only by being listed) names the count toward the counter's `i`-th `every` change, in the order the model lists its changes: 0 on the frame it changes, then one more each frame up to one less than `every`. It is a number, so `scale`, `offset` and the byte keys apply, and `flag` and `values` don't.

A game usually counts down instead: it sets a byte to 24 on the frame its timer ticks and takes 1 off it each frame. Its mapping reads the byte with `scale = -1` and `offset = 24`:

```toml
[[field]]
path = "ticks.time[0]"
ram = [0x0787]
scale = -1
offset = 24
```

Read in reverse ([below](#read-in-reverse)), a count past `every - 1` loads as `every - 1`, since either changes the counter on the next frame (a stopped timer's 0 loads as 23), and one below 0 loads as 0, noted as approximated. The lockstep and `compare` compare it as any number, and its owner is `core.counters`.

## Alignment

`polled`: the model runs one frame for each frame the original reads its controller, with that frame's input. The model's power-on state is compared with the original's frame just before the first frame it reads the controller, and the original's lag frames after that are skipped, neither run in the model nor compared. Moss Hop reads its controller from frame 6, once every frame after, so its model's power-on state is the game's frame 5.

A game with a title screen reads its controller long before play starts, so its mapping can give `start`, a condition that holds once play has begun: in a game, in its main task, with the player in control. The model's frame 0 stands for the first frame that holds, and polling takes over after it.

A game that runs its frame in the NMI handler can take longer than a frame on a busy one. At the frame's end the handler is still running, and its RAM is a frame half done; the next NMI is taken only once the handler lets it. `read_at = "nmi"` reads each frame's RAM there: gp-nes runs a copy of the console on to that NMI (`Nes::wram_at_next_nmi`), and a frame whose NMI is off reads at its end. Donkey Kong's ladder grab with seven barrels and three fires out runs past the end, before its timer routine $F4AC: read at the end, its divider and frame timers are a step behind for one frame. The original's runtime must know `read_at_nmi`; a lockstep with another refuses the mapping.

## Read in reverse

The same fields turn the original's state into a model state: `gp_interp::translate` writes each mapped value into a model world, and notes what the mapping doesn't hold. The lockstep's resync and `gp interp translate` use it. The level is the `level` field's when there is one, and otherwise the first `[[slots.level]]` whose `when` holds. A field that reads well reads back as well, since its value is already in the model's units. Because a `ticks` path carries the count, a translated counter changes on the same frame as the original's.

An `input.<action>` field (feature `input.held`) loads whether the action is held on the original's frame, so the model's first frame takes it as held the frame before and a press the original doesn't see as new isn't new in the model either. Donkey Kong keeps the pad at $15 after its direction latch, and its mapping reads `input.up`, `input.down`, `input.left` and `input.right` from its bits.

A machine's `state` field (`entities.<kind>[i].state` or `rules.<name>.state`) enters the state the original's value names. An entity's mechanic words (`entities.<kind>[i].ext[0]` to `ext[3]`) and a path's state (`home_x`, `home_y`, `path_back`, `wait`) load too (feature `entity.mechanic_paths`); a `wait` below zero loads as 0. `entities.<kind>[i].time` (feature `entity.time`) and `.jumping` (feature `entity.jumping`, for a kind with a jump, so a start inside a jump goes on along its arc) load when listed.

A `[[pending]]` entry names changes to a counter the original holds back, so a start just after one still makes it. Each of `count` slots has a byte at `amount` plus the slot and a timer at `timer` plus the slot. A slot with a nonzero amount and a timer at or above `at` loads as one of the model's waiting changes for `counter`, by the amount's entry in `by` (its last for an amount past the end), with `timer - at + 1` frames left, in place of those the base world has for that counter. Points a game shows floating up before it adds them are the case it was made for. No run compares these bytes.

A mapping with `[slots]` carries the original's slot objects into the translated world (`translate::from_original`). The model's own entities but the player go first, except a level's fixtures: a body the level's spawns placed whose entity takes no slot (`no_slot`) and that no slot kind names, such as Donkey Kong's oil drum. Then each living one of a kind the slots name becomes a body at the slot's `x` and `y`, and every other slot field it maps loads into that body as the same field of a mapped entity does. A body at an end of its path heads for the other end, as the path has it: at the far end it is on its way back (after its `wait`), at the spawn end on its way out; between the ends `path_back` says the way. The model's frame count moves forward to the next count whose low byte is the slots' `clock`, as `gp interp slots` runs its bodies, so a component that moves on some frames (`every`) moves on the original's. An entity's state carries what its `set` changes (a hitbox, a sprite, a walk), so entering it applies them. The state is noted `loaded`. When it is a different state from the base world's, the machine's time is 0, noted `approximated` at `<machine>.time`, unless the mapping maps the time too (`entities.<kind>[i].time` or `rules.<name>.time`), which then sets it.

An entity the mapping's own fields name, such as Donkey Kong's `entities.kong[0].ext[0]`, is no slot object: a start that carries the slots' objects keeps it, and its fields load as any mapped entity's.

## Slots

A game that keeps its enemies in a few slots it reuses can't map an enemy to an address, so `[slots]` says how the slots are laid out, and `gp interp slots` (`gp_interp::compare::compare_slots`) compares each slot's occupant with a body of its model entity. Donkey Kong's mapping has one for its barrels.

| Key | Meaning |
|---|---|
| `slots.count` | How many slots. Slot `i`'s value is at each address below plus `i`, or for a field with a `stride`, plus `i` times it |
| `slots.active` | Slot 0's flag: the slot holds an object while it isn't zero |
| `slots.active_stride` | Optional (feature `slots.active_stride`): the flag's bytes from one slot to the next, as a field's `stride`. Without it, 1 |
| `slots.free` | Optional (feature `slots.free`): the flag's value for an empty slot, instead of 0. Any other value holds an object and reads as the flag 1. Donkey Kong's barrels are free while their sprite's y is $FF: `active = 0x230`, `active_stride = 16`, `free = 0xFF` |
| `slots.id` | Slot 0's object ID. Optional (feature `slots.no_id`): without it every slot's ID is 0, for slots that hold one kind of object, and the kind lists `ids = [0]` |
| `slots.clock` | The original's frame counter, one byte. Optional (feature `slots.no_clock`): without it the bodies step once a frame and a start leaves the model's frame count as it is, for a game that keeps no frame counter (Donkey Kong) |
| `slots.resync` | Optional (feature `slots.resync`): a resync carries the original's slot objects into the model as a start does, in place of the model's own, and a body's split starts a resync too, so the whole world goes back in step; each body still held pairs again with the one carried for it and keeps its sides' motion. Without it a resync keeps the model's enemies. Donkey Kong's barrels |
| `slots.moving` | Optional: a condition (the runtime protocol's grammar) that holds on frames the original moves its objects |
| `slots.vacant` | Optional (feature `slots.vacant`): a condition (the same grammar) under which the slots hold none of their objects, so every slot reads free, for a scene the game draws in the bytes the slots are read from. A group without one takes its slots'. Donkey Kong: Original Edition's stage-clear scene draws Kong carrying Pauline up in the barrels' and the fires' sprites from its step 3 ($8B03): `ram[0x0450] >= 3` is part of its condition |
| `slots.vacant_from` | Optional (feature `slots.vacant_from`): a condition under which the slots from a given one on read free, for a stage that uses fewer of a group's slots and the rest's bytes for something else |
| `slots.state`, `slots.ends` | Optional: slot 0's state byte, and the bits of it that end a life (`ends` and a kind's `ends` need `state`). A slot whose state has one of them set isn't compared: a defeated enemy the game moves as defeated and the hook doesn't |
| `[[slots.level]]` | `level`, a model level, and `when`, a condition that holds while the original is in it. The first that holds is a life's level |
| `[[slots.field]]` | Slot 0's values, as fields above with `path` one of `x`, `y`, `vx`, `vy` and `ext[0]` to `ext[3]` (a body's own fields and its hook's words), numbers. Also `facing` and `state` (named values), `home_x`, `home_y` and `wait` (numbers) and `path_back` (a flag), which only a translation reads (feature `slot.body_fields`): no run compares them |
| `slots.part_flag` | Optional (feature `slots.part_flag`): the flag bit of a slot that is only a second part of another's object. The flag's other bits name that object's slot and the slot's ID means nothing, so it is no object unless a kind's `flags` lists the flag. A start gives the slot to that object's body when its entity has `extra_slots` |
| `[[slots.kind]]` | `entity`, a model entity, and `ids`, the object IDs that are it, with its own `[[slots.kind.field]]` list that adds values or replaces the slots' of the same path. Optional `flags`: the flag values its lives have, when the original also keeps a second part of it in a slot under the same ID with another flag. Optional `ends`: the state bits that end its lives, instead of `slots.ends`. Optional `holds_slot` (feature `slot_kind.holds_slot`): the object only holds its slot, so a start carries it as a body of this entity and no run pairs or measures it, for an object that stays in its slot after it is defeated until the game erases it |
| `[[slots.rest]]` | Optional (feature `slots.rest`): what a slot the model doesn't carry a body into holds, read at each slot's address as the fields are, with `path` `x_frac` (the fraction of its place across), `vy` (its speed up and down), `y_frac` (the fraction of its height, feature `rest.y_frac`), numbers, or `word` (feature `slot.rest_word`), a whole number that loads into the slot's word for a slot with a body too, for a byte that belongs to the slot and that hooks keep. A translation loads them into a model level with `slot_rest`, so the next body there starts with them, as a game's init routine leaves them as the slot had them. An empty slot holds what its last object left; one holding an object the model doesn't carry holds that object's, which it leaves much the same |
| `slots.held` | Optional (feature `slots.held`): an object past the end of its life that keeps its slot. `entity`, a model entity; `ids`, the object IDs it can have; `state`, the state bits that mark it (needs `slots.state`); and `[[slots.held.field]]`, as a kind's. A translation carries such an object into a start as a body in its slot, and no run compares it, so the next object takes the slot the original gives it |
| `[[slots.group]]` | Optional (feature `slots.group`): more object slots of their own, each with the keys above (`count`, `active`, `id`, `clock`, `moving`, `state`, `ends`, `[[slots.group.field]]` and `[[slots.group.kind]]`) but no levels (it takes these slots') and no groups. A group can have a `rest`, by the paths `x_frac`, `y_frac` and `vy_frac`, which a translation loads into an empty shot slot of a `shoot.in_slots`, and a `turn`, the byte whose value is the model's `shot_turn` (feature `group.rest`). Its lives are paired and measured as bodies and carried into a start as the first slots' are, but they take none of a level's numbered slots. Donkey Kong's oil-drum fires and 75m's springs are groups. `gp interp slots` measures the first slots only |
| `name` | Optional (feature `slots.group.name`): a group's name, for people (`fires`, `springs`). No run reads it |
| `note` | Optional, for people |

A slot field can also be `time` (the time in the body's machine state) or `hp` (feature `slot.time_hp`), and a slot kind can have `refuse` or `leave` (features `slot_kind.refuse` and `slot_kind.leave`), a reason in words. A `refuse` kind is an object the model can't hold as the original has it: a start with one loaded is refused, saying why. A `leave` kind is one the model keeps no body for and needs none (a coin jumping out of a block, only drawn): a start leaves it out. Neither is paired or measured, and neither needs its `entity` in the model.

A slot field with `shared = true` (feature `field.shared`) reads its bytes at the same address for every slot: a value the game keeps once, not per object, that a carried body's hook keeps a copy of.

A slot field with `index = true` and `ram = []` (feature `slots.field.index`) reads the slot's own number, plus its `offset`, for a hook that keeps which slot it is in a word. Donkey Kong's oil-drum fires keep theirs in `ext[3]` as the slot plus one: slot 0 reads the random byte $19 for its choices, slot 1 reads $1A.

A slot field's `stride` (feature `slots.field.stride`) is its bytes from one slot to the next: slot `i`'s bytes are `i * stride` past slot 0's. Without it, 1. A stride is at least 1, a shared field has none, and a field outside `[slots]` is refused with one. Donkey Kong keeps no copy of a barrel's place outside the sprite buffer, four sprites (16 bytes) a barrel, so its mapping reads slot `k`'s y at $0230 + 16k and x at $0233 + 16k, its state at $5E + k, and has no `id` and no `clock`:

```toml
[slots]
count = 9
active = 0x230
active_stride = 16
free = 0xFF

[[slots.field]]
path = "x"
ram = [0x233]
stride = 16

[[slots.kind]]
entity = "barrel"
ids = [0]
```

A slot kind's `up_down` (feature `slot_kind.up_down`) makes the body's direction up and down a measure too, `bodies.<kind>[n].moving_y` (`up`, `down`, or `none` before it has moved), with a change as a turn event ("turns up"): a fireball's bounce off the ground. A slot field with `presentation = true` loads into a carried body and is no measure: speeds a start needs that the directions already measure.

A slot life starts when a slot's flag is set with an ID a kind lists (and a flag value it lists, and no state bit that ends it), and ends when that stops holding or the ID changes. Each life is matched to exactly one body in a world of its own. That world is the model's, in the life's level, and holds the player and one body of the life's entity, placed where the slot is when the life starts. The body steps once there, so its hook sets itself up as the game's init routine has. Then it takes the slot's values: its position, and any `ext` words the kind maps, which can start where the slot's last occupant left them. After that:

- The player is put where the mapping's `entities.<player>[0].x` and `.y` fields read, with the speed `.vx` reads, every time the original steps.
- The body steps through its components and its `ext` hook, as a frame of the enemies' system runs them (`gp_sim::systems::enemy`), once for each tick of the clock while `moving` holds. An entity with no hook is compared on its components alone. The clock is the low byte of the world's frame count, so a hook that reads the frame counter sees the original's.
- Bodies the hook spawns step too, but aren't compared.
- Each value is compared after every step but the first. Within a pixel agrees; an `ext` word must be equal. Both sides' speed (the mean change a step) and period (the steps between every second turn back) are measured over all the entity's lives, leaving out jumps of more than 128 pixels, which are wraps.

`gp interp slots` prints each entity's lives and agreement, and both sides' speed and period with how far apart they are. `--json` gives the same for the validation engine. `SlotReport::measures` gives them as `speed` and `period`, the names a contract's tolerances use (`speed = "3%"`).

The lockstep measures the same slots side by side: each slot life is paired with a living model body of its entity, the nearest within a view's width, for the run, and its place, direction, phase and being alive are measures of their own, named `bodies.<kind>[n]`. A slot value without `frac` is compared on whole pixels there, since the original keeps no fraction.

This proves an entity's motion given the original's player: the path its components or its hook move it along and its speed and rhythm, with everything they read from the player held to the original's. The body is alone with the player, so it meets no other enemy: a walker that turns where the original's bumps into another doesn't. `gp validate` holds the speed to the contract and leaves a walker's turns out ([validation](../validation.md)). It doesn't prove anything that depends on the model's own player. Being stomped, hurting the player and a lift carrying the player are left to the side-by-side comparison and the playthroughs. So is the model's own spawning: which slot an enemy would take, and when and where the model places it. A hook's init offsets to its placement aren't compared either, since the body goes back to the slot's position.

A launcher table (the cannons a game has drawn, from which it picks one to fire) is a `[[slots.group]]` of a launcher entity, where each entry is a slot with its flag and ID as the entry's page and its fields as the launcher's place and its timer (`ext[0]`). A translation gives each carried launcher its slot as its entry. The next entry is a counter field. Schema 19's `[cannons]` is refused.

## Random draw sites

`[[draws]]` (feature `draws`) names the original's random draw sites. Each has a `site` name and the `pc` of the instruction that reads a random byte, and gives that byte as `ram`. A site with `soft = true` (feature `draws.soft`) is a byte read only for presentation, such as a sound driver's random pitch: the rebuild takes its draws when it asks on the same frame, and they never desync the script. `gp interp draws` records a clip's draws at them, which `gp interp compare --draws` gives the rebuild ([random draws script](draws.md)). With `score_draws = true` (feature `draws.score`) every score of a clip takes them with no script file. `gp game new` proposes a new game's sites from a minute of no input.

## Stretches

`[stretches]` says what a long run's exact stretches read from the original. `gp interp compare` and `gp corpus score` report the stretches of any run; without it every frame is play and the splits aren't listed by state.

| Key | Feature | Meaning |
|---|---|---|
| `play` | `stretches.play` | A condition over the original's state (the runtime protocol's grammar) outside which a frame isn't play: the player dying or waiting, a clear scene, a pause. Such a frame ends a stretch and isn't counted |
| `[[stretches.state]]` | `stretches.state` | The original's states, each a `name` and a `when`: a split frame is listed under the first that holds |

Part of Donkey Kong's:

```toml
[stretches]
play = "ram[0x0096] != 255 && (ram[0x004f] != 0 && ram[0x009a] == 0) && ram[0x0516] == 0"

[[stretches.state]]
name = "walk"
when = "ram[0x0096] == 1"
```

## Scope

`[scope]` says where a run leaves what the game's model holds, so a clip that goes on past its stage (a 25m clear that runs into 75m) is scored up to there and no further. A lockstep stops at the first of the original's frames out of scope, after its comparison has started; that frame isn't compared, and it and the input's frames after it are counted as out of scope. `gp interp compare` prints the frame and the count (`lockstep.scope` in JSON), and the corpus scorers keep them in each clip's score record (`out_of_scope`). The frames together, the stretches and the first split are those of the frames in scope. The comparison from power-on that `gp interp compare` prints first doesn't use it.

| Key | Feature | Meaning |
|---|---|---|
| `while` | `scope.while` | A condition over the original's state (the runtime protocol's grammar) that holds while the run is in scope |
| `leaves` | `scope.leaves` | Selectors over the original's state, such as a stage byte: the run is out of scope from the first frame one of them leaves the value it had on the first compared frame |
| `note` | | Why, in words |

With both, the run is out of scope from the first frame either says. Donkey Kong's `while` starts `ram[0x0053] == 1 || ...`: 25m is in scope, and so is 25m's clear scene, which runs until the scene's script on $43 steps from 116 to 114 and the stage setup draws the next stage's background. `leaves = ["ram[0x0053]"]` alone would end 25m at the stage clear, since the game sets the stage byte to the next stage when the clear scene starts.

## A body's animation

A model's presentation keeps each body's animation beside the world: the clip it shows, the picture the clip is at and a timer. None of it is in the model's state, so a start from the original's state would begin every animation at its first picture, out of step with the game's. Two paths load it (feature `present.animation`), both presentation (`presentation = true` is required):

| Path | What the original's value is |
|---|---|
| `present.<kind>[i].frame` | The picture the `i`th body of that kind is at in its clip, from 0 in the clip's order |
| `present.<kind>[i].timer` | The frames left on the body's animation timer, for a clip with `clock = "body"` |

A translation notes them `loaded` and leaves the world as it is. After each load the lockstep reads them from the original and hands them to the screen measure (`Screens::animation`), which starts the body's animation there (`Animator::seed`): the clip the body's state picks, at that picture, with that much time on the timer. A game that draws before it steps has a timer just set again on the frame it stepped, so a timer equal to the clip's time shows the picture before `frame`.

## The level grid

A game keeps its level's cells in RAM, and some of them change before a clip starts: a coin taken, a block bumped. The model's level has every coin, so a start past one would have it back. `[cells]` (feature `cells`) says where the original keeps the cells around the view. A translation reads the byte of each cell in view whose kind in the model is one of `kinds`, and empties the cell where the byte is 0; the note on `levels.<name>.tiles` says how many.

| Key | What it is |
|---|---|
| `pages` | Each page's address; columns take them in turn |
| `page_columns` | The columns in a page |
| `rows` | The rows the grid holds |
| `top` | The level row of the grid's first row |
| `view` | The columns from the camera's that the grid always holds |
| `kinds` | The tile kinds read, as the model names them |
| `becomes` | Optional (feature `cells.becomes`): the kind a cell of one of `kinds` takes for each byte, `[[0, "empty"], [7, "solid"]]`. Without it a byte of 0 empties the cell and any other leaves it. Moss Hop's ? block reads 7 once bumped, its used block |

A cell's byte is its page's address plus its column in the page, plus its row less `top` times `page_columns`. Moss Hop's mapping reads `LEVEL_LEFT` and `LEVEL_RIGHT`:

```toml
[cells]
pages = [0x300, 0x3F0]
page_columns = 16
rows = 15
top = 0
view = 17
kinds = ["coin", "coin_block"]
becomes = [[0, "empty"], [7, "solid"]]
```

## The ignore list

A mapping can list the RAM bytes the model needs no field for (feature `ignore`), each with its role and why, so a count of the memory map can leave them out.

Moss Hop's, in part:

```toml
[[ignore]]
ram = 0x0002  # PAD, PAD_PREV, PAD_NEW
len = 3
role = "input"
why = "written by the routine that reads the controller"
```

| Key | What |
|---|---|
| `ram` | The first byte: $0000-$07FF on the NES, any of the main CPU's addresses on an arcade board |
| `len` | Optional, 1 by default: bytes from `ram` |
| `role` | `input`, `counter`, `scratch`, `sound` or `pointer` |
| `why` | The rule that put it there, in words |
| `review` | Optional: the rule held on nearly every frame but not all, so a person checks it |

A byte the mapping names is never in it. No translation or comparison reads the list. Moss Hop's was written by game-printer's `gp progress --write-ignore` from a trace of the original. `gp progress` counts the memory map against the list, and `gp progress --write-ignore MAPPING` writes it.

## An arcade original

`platform = "arcade"` (feature `platform`) marks a mapping of a MAME romset's game. Its `ram[...]` reads the main CPU's addresses: Tapper's work RAM is at `0xE000`, and its mapping reads the bartender's place from his object slot at `0xE2E6`. The NES's own parts are refused in it: `nametable`, `oam`, `oam_tile` and `read_at`. Its `ignore` may name any of the main CPU's addresses. Its `[[draws]]` name the main Z80's sites: a site that is a call draws the value the routine returns in A, any other the byte it reads.

```toml
platform = "arcade"
schema = 22
model = "tapper"
source = "tapper"
align = "polled"

[[field]]
path = "entities.player[0].x"
ram = [0xE2E9]
whole = true

[[field.case]]
unmeasured = "Out of a game (E3BD 1 in attract mode, 2 at game over) the slot holds the attract mode's objects"
when = "ram[0xE3BD] != 0"
```

## Carrying any loaded object

A start from the original's state carries every loaded slot object, by one rule (`gp_interp::carry`). A body of an entity keeps some state between frames, by its components: its place, with the fraction when it moves by fractions; the facing for a walk or a flight that moves; the speed up and down for a fall, with the place's fraction when gravity has one; a path's spawn end, way and wait; an oscillation's center and speed; the mechanic words of a hop's pattern or a leap; of a shuttle or a springboard; of an orbit's angle or a hook (all four); its machine's state when it has more than one, and the time in it while it is in a state that ends by time; its damage taken when it has more than one hit point. Each must be a slot field of its kind, and each value the original holds must load. An object a start can't carry is listed with one of four causes:

| Cause | When |
|---|---|
| An unmapped kind | No slot kind names its ID, or the model has no entity for the one that does |
| A missing field | A field of its place, speed, facing, path or oscillation isn't read |
| A timer or phase not read | A mechanic word, a wait, a path's way, its time in a state that ends by time or its damage isn't read |
| A state not named | Its machine state isn't read, or the original holds a value the field names nothing for |

A kind with `refuse` is refused for its reason. An object whose state has an ending bit (`ends`, an enemy falling defeated) or a group's (`ends`, a fireball exploding) is gone in the model, as the model's own body is once defeated: its slot is free with what it left, and that is no gap. A slot with an ID a kind lists under another flag is that kind's second part. A kind is carried by adding its rows; nothing in the rule names an entity. `gp capture carry GAME` lists what the game's clip starts can't carry, grouped by cause, with the starts each gap alone holds back.

## Versions

`schema` is the format's version, and it bumps only for a breaking change, once per release: 22 is the current one, the last number counted before features (20 dropped `[cannons]`). A part added since is a named feature, not a step. A mapping may list the features it uses:

```toml
schema = 22
features = ["field.case", "slots.group", "cells"]
```

Without the list they're derived from the fields the mapping sets. With it, each one used must be listed. A reader refuses a mapping that lists a feature it doesn't know, by name, before it reads the rest: "the state mapping uses the feature `rope.swing`, which this build doesn't know; update gp to read it". Each step from 2 to 22, other than 20, is now a feature, with its schema kept as a note (`crates/gp-interp/src/mapping_features.rs` lists them all). A mapping of an earlier schema reads as before, except one with `[cannons]`, which is refused.

| Schema | Added (the feature now) |
|---|---|
| 1 | The first version |
| 2 | `ticks.<counter>[i]` paths (`ticks`) |
| 3 | A field's cases (`field.case`), and `entities.<kind>[i].time` as a loadable path (`entity.time`) |
| 4 | A field's `presentation` (`field.presentation`) |
| 5 | The slot fields a translation loads (`slot.body_fields`), `whole` with `frac` (`field.whole`), and an entity's mechanic words and path state as loadable paths (`entity.mechanic_paths`) |
| 6 | `present.<kind>[i].frame` and `.timer` (`present.animation`) |
| 7 | `[[slots.rest]]` (`slots.rest`) |
| 8 | `cameras.<name>.move_x` and `.move_y`, a camera's last move of its entity (`camera.last_move`) |
| 9 | `[[slots.group]]` (`slots.group`), a slot kind's `up_down` (`slot_kind.up_down`) and `input.<action>` (`input.held`) |
| 10 | `entities.<kind>[i].seen_speed`, the speed as the run last left it before a wall stopped it (`entity.seen_speed`) |
| 11 | A slot group's `rest` and `turn` (`group.rest`) |
| 12 | The value `both` of an entity's `facing` and `moved` (`facing.both`) |
| 13 | A slot field's `shared` (`field.shared`) |
| 14 | `[cells]` (`cells`) |
| 15 | A slot rest's `y_frac` (`rest.y_frac`) and `slots.held` (`slots.held`) |
| 16 | `[[pending]]` (`pending`) |
| 17 | A slot field's `time` and `hp` (`slot.time_hp`), a field's `names` (`field.names`), and a slot kind's `refuse` and `leave` (`slot_kind.refuse`, `slot_kind.leave`) |
| 18 | A plain whole number of 6 or 7 addresses (`field.wide_whole`) |
| 19 | `[cannons]`, a slot kind's `holds_slot` (`slot_kind.holds_slot`) and `slots.part_flag` (`slots.part_flag`) |
| 20 | Drops `[cannons]`: a launcher table is a slot group of a launcher entity |
| 21 | `[[ignore]]` (`ignore`) |
| 22 | A slot rest's `word` (`slot.rest_word`) |

Each step read every earlier mapping as before and refused one that used what it added, and a build that read only up to the step before refused it. The features added after 22 have no schema note: `cells.becomes`, `slots.field.stride`, `slots.no_id`, `slots.no_clock`, `slots.active_stride`, `slots.field.index`, `slots.free`, `slots.vacant`, `slots.vacant_from`, `draws`, `draws.soft`, `draws.score`, `stretches.play`, `stretches.state`, `scope.while`, `scope.leaves`, `field.bcd`, `field.case.unmeasured`, `slots.resync`, `field.lookup`, `field.case.ram`, `field.case.oam`, `field.case.nametable`, `field.case.number`, `field.case_flag`, `entity.jumping`, `field.whole_64`, `align.read_at_nmi`, `field.nametable`, `field.oam`, `field.oam_tile`, `field.machine`, `slots.group.name` and `platform`.

## Decisions

| Question | Decision |
|---|---|
| Where the mapping lives | In the game's folder, versioned with the model, rather than in the source adapter or the interpreter. It describes the original, not the game the model makes |
| What it addresses | RAM bytes by address, as the original runtime's selectors (`ram[0x0010]`) read them, so it needs nothing from the trace format and works on any runtime the protocol drives, the NES's or an arcade board's |
| Enemies in reused slots | Each slot life gets one body in a world of its own, with the player held to the original's, rather than running the model side by side and pairing slots with the model's bodies by distance. A model's player leaves the original's path within seconds of a long recording, after which a side-by-side run never reaches most enemies |
| Slot lives in a side-by-side run | Paired with the model's own bodies, nearest of the same kind within a view's width, when either side appears, and kept for the run. A clip is a few seconds from the original's state, where the start carries the slot objects in, so the model's bodies are still near the original's. A body only one game has is a measure of its own. This adds to the slot lives' own worlds above, which still prove the motion over a whole recording |
| Transforms | Byte order, sign, bits, a 256ths fraction byte, scale and offset, flags, named values, lookups and BCD. Anything more is code, which belongs in a later `ext` for mappings, if a game needs one |
| A value the bytes don't say alone | A field's cases, conditions in the protocol's grammar checked in order, rather than a table keyed by several bytes or a field with code. The conditions are the ones `start` and `[[slots.level]]` already use |
| An `every` change's count | A path of its own, `ticks.<counter>[i]`, rather than a key on the counter's field: the state view has `counters.<name>` as a number, a counter can have more than one `every` change, and a path is compared and owned like any other |
