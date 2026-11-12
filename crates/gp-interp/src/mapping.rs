//! A state mapping: how the original game's RAM holds the model's state,
//! as data kept with the model (`mapping.toml` in a template directory,
//! docs/formats/state-mapping.md). Each field names a model path in the
//! interpreter's state and the RAM bytes that hold the same value in the
//! original, with the simple transforms that turn one into the other.

use gp_model::Fixed;
use gp_runtime::Value;
use serde::Deserialize;

/// The file a model directory keeps its state mapping in.
pub const MAPPING_FILE: &str = "mapping.toml";

/// The mapping format's version. It bumps only for a breaking change,
/// once per release. 22 is the last number counted before features: 20
/// dropped `[cannons]`, and each other step from 2 to 22 added an
/// optional part, now a named feature in [`crate::mapping_features`] with
/// its schema kept as a note. A part added since is a feature too, not a
/// bump. A mapping of an earlier schema reads the same as before, except
/// one with `[cannons]`, which is refused.
pub const SCHEMA: u32 = 22;

/// The mapping format's JSON Schema, as `schema/mapping.json` holds it,
/// generated from [`Mapping`]. The parser is stricter: it also checks a
/// field's path and the parts each schema needs.
pub fn schema() -> String {
    let mut schema = schemars::schema_for!(Mapping);
    schema.insert(
        "$id".into(),
        format!("game-printer:mapping@{SCHEMA}").into(),
    );
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}

/// The versions this build reads.
pub const READS: [u32; 22] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
];

/// The paths of a slot's `rest` (schema 7, `y_frac` from schema 15): the
/// fraction across, the speed up and down and the fraction of the height
/// its last object left, which a model level with `slot_rest` gives the
/// next body in that slot. `word` is the slot's own word
/// (`gp_sim::world::Slot::word`), read for a taken slot too.
pub const REST_PATHS: [&str; 4] = ["x_frac", "vy", "y_frac", "word"];

/// The paths of a group's `rest` (schema 11): the fractions a slot's last
/// shot left, which a `shoot.in_slots` gives the next shot in that slot.
pub const SHOT_REST_PATHS: [&str; 3] = ["x_frac", "y_frac", "vy_frac"];

/// The parts of a body's animation a `present.<kind>[i]` path names: the
/// picture its clip is at, and the frames left on its animation timer.
pub const PRESENT_PARTS: [&str; 2] = ["frame", "timer"];

#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub schema: u32,
    /// The optional features the mapping uses, by name
    /// ([`crate::mapping_features`]). A reader refuses a mapping listing one
    /// it doesn't know. Without the list they're derived from the fields
    /// it sets.
    #[serde(default)]
    pub features: Vec<String>,
    /// The model's name (`model.name`).
    pub model: String,
    /// The original's source ID, as in its asset map and baselines.
    pub source: String,
    /// Feature `platform`: the original's machine, `nes` (the default) or
    /// `arcade`. An arcade mapping's `ignore` may name any of the main
    /// CPU's addresses, and the NES's own parts (`nametable`, `oam`,
    /// `oam_tile`, `read_at`) are refused in it. Its `[[draws]]` name main
    /// Z80 sites (port wave 6): a site that is a call draws the value the
    /// routine returns in A, any other the byte it reads.
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// How the model's frames line up with the original's.
    pub align: Align,
    /// Feature `align.read_at_nmi`: when in its frame the original's RAM
    /// is read. Without it, at the frame's end. `nmi`: as the CPU takes the
    /// next NMI, for a game that runs its frame in the NMI handler and
    /// sometimes takes longer than a frame, so the handler's last routines
    /// (Donkey Kong's timers at $F4AC) have run when the frame is read.
    /// The original's runtime must know `read_at_nmi` (gp-nes does).
    #[serde(default)]
    pub read_at: Option<ReadAt>,
    /// A condition over the original's state (the runtime protocol's
    /// condition grammar, docs/runtime-protocol.md) that marks the
    /// original's frame the model's power-on state stands for: the first
    /// frame it holds. Without it, the frame before the first one the
    /// original reads its controller.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(rename = "field")]
    pub fields: Vec<Field>,
    /// The original's reused object slots, if it keeps its enemies that
    /// way, compared entity by entity ([`crate::compare::compare_slots`]).
    #[serde(default)]
    pub slots: Option<Slots>,
    /// Where the original keeps its level grid, so a translation knows
    /// which cells have been taken (schema 14).
    #[serde(default)]
    pub cells: Option<Cells>,
    /// Changes to a counter the original holds back, which a translation
    /// loads as the model's own waiting changes (`after`).
    #[serde(default)]
    pub pending: Vec<Pending>,
    /// Schema 19's cannon table, gone in schema 20: a game's cannons are a
    /// slot group of launchers (platformer 0.4.28), so a mapping with one
    /// is refused.
    #[serde(default)]
    #[schemars(skip)]
    pub cannons: Option<toml::Value>,
    /// RAM bytes the model needs no field for, each with its role and why
    /// (schema 21): the memory map's progress leaves them out
    /// (docs/progress.md).
    #[serde(default, rename = "ignore")]
    pub ignore: Vec<Ignore>,
    /// The original's random draw sites (feature `draws`): where its code
    /// reads a random byte, so `gp interp draws` can record a clip's draws
    /// for a rebuild to take ([`gp_sim::draws`]).
    #[serde(default, rename = "draws")]
    pub draws: Vec<DrawSite>,
    /// Feature `draws.score`: a clip's score (`gp corpus score`, the
    /// queue, `gp loop`) gives the rebuild the original's draws over the
    /// clip, recorded at the `[[draws]]` sites, as `gp interp compare
    /// --draws` does. Needs `[[draws]]`.
    #[serde(default)]
    pub score_draws: bool,
    /// Features `stretches.play` and `stretches.state`: which of the
    /// original's frames are play for a long run's exact stretches, and
    /// the original's states its splits are listed by
    /// (docs/benchmarks.md, "Long runs").
    #[serde(default)]
    pub stretches: Option<Stretches>,
    /// Features `scope.while` and `scope.leaves`: where the template's
    /// scope ends in a run (docs/formats/state-mapping.md, "Scope"). A
    /// lockstep stops at the first frame out of scope and counts the
    /// frames from there as out of scope.
    #[serde(default)]
    pub scope: Option<Scope>,
}

/// Where a run leaves what the template holds: the first of the
/// original's frames after the comparison starts on which `while` no
/// longer holds, or on which a `leaves` selector's value differs from its
/// value on the first compared frame (Donkey Kong's stage byte when 25m
/// gives way to 75m).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// A condition over the original's state that holds while the run is
    /// in scope.
    #[serde(default, rename = "while")]
    pub while_: Option<String>,
    /// Selectors over the original's state (`ram[0x0053]`): the run is
    /// out of scope from the first frame one of them leaves the value it
    /// had on the first compared frame.
    #[serde(default)]
    pub leaves: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// What a long run's exact stretches read from the original
/// (docs/benchmarks.md): `play`, a condition over the original's state
/// outside which a frame isn't play (the player dying or waiting, a clear
/// scene, a pause), so it ends a stretch and isn't counted; and named
/// states, the first whose `when` holds naming a split frame's state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stretches {
    #[serde(default)]
    pub play: Option<String>,
    #[serde(default)]
    pub state: Vec<NamedState>,
}

/// One of the original's states, by a condition over its state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedState {
    pub name: String,
    pub when: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// One of the original's random draw sites: the instruction at `pc` reads
/// a random byte, `ram` or, indexed, one past it (Donkey Kong's fires read
/// `$19,X`); a recording takes the byte the instruction reads. A hook
/// draws for it by `site`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrawSite {
    pub site: String,
    pub pc: u16,
    pub ram: u16,
    /// Feature `draws.soft`: the byte is read only for presentation (a
    /// sound driver's random pitch), so the rebuild takes the draw when it
    /// asks on the same frame and is never out of step for drawing it on
    /// another or not at all ([`gp_sim::DrawScript::soft`]).
    #[serde(default)]
    pub soft: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// RAM bytes the model needs no field for (schema 21): `len` bytes from
/// `ram`, holding the controller's bits, a frame counter, a scratch value,
/// a pointer or the sound driver's state (`role`), and why. `gp progress
/// --write-ignore` writes the list from a trace of the original; `review`
/// marks a byte whose rule held on nearly every frame but not all.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    pub ram: u16,
    #[serde(default = "one")]
    pub len: u16,
    pub role: String,
    pub why: String,
    #[serde(default)]
    pub review: bool,
}

fn one() -> u16 {
    1
}

/// The roles an ignored byte can have.
pub const IGNORE_ROLES: [&str; 5] = ["input", "counter", "scratch", "sound", "pointer"];

impl Ignore {
    /// The RAM addresses it covers, on the NES (whose $0800 to $1FFF
    /// mirror the 2 KB of RAM).
    pub fn addrs(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.len).map(move |i| self.ram.wrapping_add(i) & 0x7FF)
    }
}

/// Changes to a counter the original holds back in `count` slots of its
/// own, each with a byte that says how much and a timer. A slot changes
/// the counter on the frame it reads its timer as `at`, so a timer above
/// that is a change still to come. Super Mario Bros.' floatey numbers hold
/// an award this way.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    /// The model's counter.
    pub counter: String,
    pub count: u16,
    /// Slot 0's byte that says how much: 0 is an empty slot, and any other
    /// value picks from `by`, the last entry for one past its end.
    pub amount: u16,
    /// Slot 0's timer, counted down once a frame.
    pub timer: u16,
    /// What the timer reads on the frame the change is made.
    pub at: u8,
    /// How much the counter changes by, in the counter's units.
    pub by: Vec<i64>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The original's level grid in RAM (schema 14). It holds the columns
/// around the view in pages of `page_columns` columns: the byte of a cell
/// is its page's address plus its column in the page, plus its row less
/// `top` times `page_columns`. A cell in view of a kind in `kinds` whose
/// byte is 0 has been taken, and a translation empties it; with `becomes`
/// (the feature `cells.becomes`), a byte listed there gives the cell that
/// kind instead, as Moss Hop's used block, metatile 7, is a ? block that
/// is now solid.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cells {
    /// Each page's address; columns take them in turn.
    pub pages: Vec<u16>,
    pub page_columns: i64,
    /// Rows the grid holds.
    pub rows: i64,
    /// The level row of the grid's first.
    pub top: i64,
    /// Columns from the camera's that the grid always holds.
    pub view: i64,
    pub kinds: Vec<gp_model::model::TileKind>,
    /// The kind a cell of one of `kinds` takes for each byte (the feature
    /// `cells.becomes`):
    /// `[[0, "empty"], [7, "solid"]]`. Without it, 0 is empty and any other
    /// byte leaves the model's cell as it is.
    #[serde(default)]
    pub becomes: Vec<(u8, gp_model::model::TileKind)>,
}

/// Feature `slots.vacant_from`: slots from `slot` on read free while
/// `when` holds.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VacantFrom {
    pub slot: u16,
    pub when: String,
}

/// Object slots the original reuses: `count` of them, each value at a base
/// address plus the slot's index. A slot holds one model entity from the
/// frame its flag is set with an ID a [`SlotKind`] names until the flag
/// clears or the ID changes: one life.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slots {
    pub count: u16,
    /// Slot 0's flag: the slot holds something while it isn't zero (or,
    /// with `free`, while it isn't that value).
    pub active: u16,
    /// The flag's bytes from one slot to the next (feature
    /// `slots.active_stride`). Without it, 1. Donkey Kong's oil-drum fires
    /// are free while their sprite's y in the sprite buffer, 16 bytes a
    /// fire, is $FF.
    #[serde(default)]
    pub active_stride: Option<u16>,
    /// The flag's value for an empty slot, instead of 0 (feature
    /// `slots.free`). With it, every other value reads as the flag 1.
    #[serde(default)]
    pub free: Option<u8>,
    /// Slot 0's object ID. Without it (feature `slots.no_id`), every
    /// slot's ID is 0, for a game whose slots hold one kind of object and
    /// keep no ID: Donkey Kong's barrels.
    #[serde(default)]
    pub id: Option<u16>,
    /// The original's frame counter, one byte. A life's body steps once
    /// each time it advances, with its value as the frame's low byte.
    /// Without it (feature `slots.no_clock`), the bodies step once a frame
    /// and a start leaves the model's frame count as it is: Donkey Kong
    /// keeps no frame counter.
    #[serde(default)]
    pub clock: Option<u16>,
    /// A condition over the original's state (the runtime protocol's
    /// condition grammar) that holds on frames it moves its objects. A
    /// clock tick on any other frame doesn't step the body.
    #[serde(default)]
    pub moving: Option<String>,
    /// A condition over the original's state under which its slots hold
    /// none of these objects, so every slot reads free (feature
    /// `slots.vacant`): the game has drawn something else in the bytes the
    /// slots are read from. A group without one takes its slots'. Donkey
    /// Kong: Original Edition's stage-clear scene draws Kong carrying
    /// Pauline in the barrels' and the fires' sprites ($8B03, while its
    /// scene step $0450 is 3 or more).
    #[serde(default)]
    pub vacant: Option<String>,
    /// Feature `slots.vacant_from`: the slots from `slot` on read free
    /// while `when` holds, for a stage that uses fewer of them and the
    /// bytes of the rest for something else. Donkey Kong's fires take 4
    /// slots on 100m (C1F6[3]) and 2 elsewhere, where slots 2 and 3's
    /// sprites at $0230 and $0240 are barrels'.
    #[serde(default)]
    pub vacant_from: Option<VacantFrom>,
    /// Slot 0's state byte, with `ends`: a slot whose state has any of
    /// `ends`'s bits set ends its life and starts none, as for an object
    /// the original moves as defeated, which a hook doesn't.
    #[serde(default)]
    pub state: Option<u16>,
    #[serde(default)]
    pub ends: Option<u8>,
    /// The flag bit of a slot that is only a second part of another's
    /// object: its other bits name that object's slot, and its ID means
    /// nothing, so it is no object unless a kind's `flags` lists the flag.
    /// A start gives it to that object's body, for a model entity with
    /// `extra_slots`. Super Mario Bros.' DuplicateEnemyObj.
    #[serde(default)]
    pub part_flag: Option<u8>,
    /// Feature `slots.resync`: a resync carries the original's slot
    /// objects into the model as a start does, in place of the model's
    /// own, so after a split the whole world is back in step. Without it a
    /// resync keeps the model's enemies. Donkey Kong's barrels: a barrel
    /// that has split stays split for the rest of a clip otherwise.
    #[serde(default)]
    pub resync: bool,
    /// The model's level a life's world is in: the first whose `when`
    /// holds on the life's first frame. A slot that comes alive while none
    /// holds isn't compared.
    #[serde(default, rename = "level")]
    pub levels: Vec<SlotLevel>,
    /// Slot 0's values, by body field: `x`, `y`, `vx`, `vy` or `ext[0]` to
    /// `ext[3]`, and since schema 5 `facing`, `state`, `home_x`, `home_y`,
    /// `path_back` and `wait`, which only a translation reads. A life's body
    /// starts at the first frame's `x` and `y`.
    #[serde(rename = "field")]
    pub fields: Vec<Field>,
    #[serde(rename = "kind")]
    pub kinds: Vec<SlotKind>,
    /// What an empty slot's last object left, read at each slot's address
    /// as the fields are (schema 7): `x_frac`, the fraction of its place
    /// across, and `vy`, its speed up and down. A translation loads them
    /// into a model level with `slot_rest`.
    #[serde(default)]
    pub rest: Vec<Field>,
    /// Since schema 11: an object past the end of its life that still has
    /// its slot, which a translation carries as a body so the slot stays
    /// taken until it leaves. No run compares it.
    #[serde(default)]
    pub held: Option<SlotHeld>,
    /// A presentation value (schema 4): a translation loads it, but no
    /// run compares it, so it is no measure and counts for nothing in a
    /// score. Super Mario Bros.' color rotation is one.
    #[serde(default)]
    pub presentation: bool,
    /// A group's (schema 11): the byte whose value is the model's
    /// `shot_turn`, which a translation loads.
    #[serde(default)]
    pub turn: Option<u16>,
    /// Since schema 9: more object slots of their own, each with its own
    /// count, flag, ID and kinds, read as these are: Super Mario Bros.'
    /// fireballs and hammers, which the game keeps apart from its enemies.
    /// A group's lives are paired and measured as bodies and carried into
    /// a start as these are; it takes the levels of these slots and has
    /// none of its own and no groups. Its `rest` (schema 11) is by
    /// [`SHOT_REST_PATHS`].
    #[serde(default, rename = "group")]
    pub groups: Vec<Slots>,
    /// A group's name (feature `slots.group.name`), for people and for a
    /// derived template: `gp template derive` merges a delta's groups over
    /// its base's by name when every group has one, so a derived game adds
    /// a group of its own and keeps its base's (until 2026-11-06, Donkey
    /// Kong: Original Edition's 50m pies and moving ladders beside the
    /// fires, springs and elevators of the template it was derived from).
    /// No run reads it.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// A model level and the condition over the original's state that holds
/// while the original is in it.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotLevel {
    pub level: String,
    pub when: String,
}

/// An object that keeps its slot after its life ends (schema 11): one with
/// an ID of `ids` and a state bit of `state`, as a defeated enemy that
/// falls until it leaves the screen. A start carries it as an `entity`.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotHeld {
    pub entity: String,
    pub ids: Vec<u8>,
    pub state: u8,
    /// Values a start loads besides the slots' own; one with the same
    /// `path` replaces the slots'.
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The slots' IDs that are one model entity, and values only it has.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotKind {
    pub entity: String,
    pub ids: Vec<u8>,
    /// Optional: the flag values (at `slots.active`) the entity's lives
    /// have, when an object with its IDs also takes other slots, as a
    /// second half does. Without it, any flag but 0.
    #[serde(default)]
    pub flags: Option<Vec<u8>>,
    /// Optional: the state bits that end this entity's lives, instead of
    /// the slots' `ends`.
    #[serde(default)]
    pub ends: Option<u8>,
    /// Optional: the object only holds its slot. A start carries it as a
    /// body of this entity, so a new object takes the slot the original's
    /// does, and no run pairs or measures it: Super Mario Bros.' defeated
    /// piranha plant, which stays in its pipe until the view leaves it.
    #[serde(default)]
    pub holds_slot: bool,
    /// Values compared for this entity besides the slots' own; one with
    /// the same `path` replaces the slots'.
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
    /// Since schema 9: its direction up and down is a measure too
    /// (`moving_y`), with its turns as events: a fireball's bounce.
    #[serde(default)]
    pub up_down: bool,
    /// Since schema 17: why the model can't hold this object as the
    /// original has it, so a start with one loaded is refused rather than
    /// carried wrong (`gp_interp::carry`). Its lives aren't paired or
    /// measured.
    #[serde(default)]
    pub refuse: Option<String>,
    /// Since schema 17: why a start leaves this object out on purpose: the
    /// model keeps no body for it and needs none (a coin jumping out of a
    /// block, which is only drawn). Its lives aren't paired or measured.
    #[serde(default)]
    pub leave: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl SlotKind {
    /// A kind a start carries: neither refused nor left out (schema 17).
    pub fn carried(&self) -> bool {
        self.refuse.is_none() && self.leave.is_none()
    }

    /// A kind the model carries and measures as a body: more than a
    /// slot's holder.
    pub fn bodied(&self) -> bool {
        self.carried() && !self.holds_slot
    }
}

/// The body fields a slot value can name.
pub const SLOT_PATHS: [&str; 8] = ["x", "y", "vx", "vy", "ext[0]", "ext[1]", "ext[2]", "ext[3]"];

/// The slot fields a translation loads into a carried body but no run
/// compares (schema 5): its facing and machine state, by name, and a
/// path's spawn end, way and wait; since schema 17 its time in its state
/// and its damage taken ([`SLOT_LOADS_17`]).
pub const SLOT_LOADS: [&str; 8] = [
    "facing",
    "state",
    "home_x",
    "home_y",
    "path_back",
    "wait",
    "time",
    "hp",
];

/// The slot fields schema 17 added to [`SLOT_LOADS`].
pub const SLOT_LOADS_17: [&str; 2] = ["time", "hp"];

/// An entity's fields schema 5 added to the loadable paths: its mechanic
/// words and a path's state.
const ENTITY_LOADS: [&str; 5] = ["ext", "home_x", "home_y", "path_back", "wait"];

/// When in its frame the original's RAM is read (feature
/// `align.read_at_nmi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReadAt {
    /// As the CPU takes the NMI after the frame's end: once a handler that
    /// ran past the end has let it be taken.
    Nmi,
}

impl Mapping {
    /// Sets the original's runtime to read the RAM where `read_at` says.
    pub fn read_original_at<O: gp_runtime::Runtime>(&self, original: &mut O) -> Result<(), String> {
        match self.read_at {
            None => Ok(()),
            Some(ReadAt::Nmi) => original
                .set("read_at_nmi", gp_runtime::Value::Flag(true))
                .map_err(|e| format!("the mapping's read_at = \"nmi\": {e}")),
        }
    }
}

/// How frames line up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    /// The model runs one frame for each frame the original reads its
    /// controller, with that frame's input. Its power-on state is the
    /// original's frame just before the first such frame, and the frames
    /// in which the original doesn't read the controller (lag frames) are
    /// skipped.
    Polled,
}

#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Field {
    /// A concrete model path in the interpreter's state.
    pub path: String,
    /// RAM addresses, low byte first. Empty for a field that reads the
    /// PPU's state (`nametable`, `oam`, `oam_tile`) or is an `index`.
    #[serde(default)]
    pub ram: Vec<u16>,
    /// Feature `field.nametable`: name table cells by PPU address ($2000
    /// to $2FFF), low byte first, in place of `ram`, read through the
    /// cartridge's mirroring (the canonical state's `mapper.mirroring`)
    /// from its `nametables`. For a value a game keeps only on the
    /// screen: Donkey Kong draws Kong as background tiles.
    #[serde(default)]
    pub nametable: Vec<u16>,
    /// Feature `field.oam`: sprite memory bytes by index (a slot's 4
    /// bytes start at 4 times the slot: y, tile, attributes, x), low byte
    /// first, in place of `ram`. OAM holds the sprite table the last DMA
    /// sent, the one on the screen this frame.
    #[serde(default)]
    pub oam: Vec<u16>,
    /// Feature `field.oam_tile`: one byte of the first sprite, by slot,
    /// whose tile is one of `tiles`, in place of `ram`.
    #[serde(default)]
    pub oam_tile: Option<OamTile>,
    /// Feature `field.machine`: a selector of the original's runtime
    /// protocol whose number is the field's, in place of `ram`, for state
    /// the console keeps outside memory: gp-nes's `nmi.` selectors, the
    /// console as the CPU takes the next NMI (where the code it stopped
    /// goes on, the CPU cycle and the PPU dot of its entry), which Donkey
    /// Kong's idle loop needs at a clip's start (docs/cycles.md). Read
    /// as 4 bytes, low first, so `bits` and `flag` apply.
    #[serde(default)]
    pub machine: Option<String>,
    /// The address of a byte holding the fraction, in 256ths.
    #[serde(default)]
    pub frac: Option<u16>,
    /// The bytes are a two's complement number.
    #[serde(default)]
    pub signed: bool,
    /// Only these bits, `[from, to)`, of the number the bytes make.
    #[serde(default)]
    pub bits: Option<[u32; 2]>,
    /// The number is a flag: any value but zero is true, or with `invert`,
    /// only zero is.
    #[serde(default)]
    pub flag: bool,
    #[serde(default)]
    pub invert: bool,
    /// The bytes are decimal digits, most significant first, as games keep
    /// scores and timers for display. Instead of low byte first.
    #[serde(default)]
    pub digits: bool,
    /// Feature `field.bcd`: each byte holds two decimal digits (packed
    /// BCD), most significant byte first, as Donkey Kong keeps its score
    /// ($25-$27) and its bonus timer ($2E).
    #[serde(default)]
    pub bcd: bool,
    /// Compare whole parts only: the model's value rounded down against the
    /// original's, for an original that keeps no fraction the model has.
    /// With `frac` as well, a translation loads the fraction and a run
    /// still compares whole parts: a fraction the game keeps but doesn't
    /// draw (schema 5).
    #[serde(default)]
    pub whole: bool,
    /// The number names one of these, by position.
    #[serde(default)]
    pub values: Vec<String>,
    /// The number names one of these, by number (schema 17): for a byte
    /// whose values are sparse, such as a state byte with flags in its high
    /// bits (schema 17). A number none of them has is unnamed.
    #[serde(default)]
    pub names: Vec<(u32, String)>,
    /// Feature `field.lookup`: pairs of the number the bytes hold and the
    /// number the model has for it, for a byte that a game's script steps
    /// unevenly (Donkey Kong's death wait at $43). A number not listed is
    /// taken as it is.
    #[serde(default)]
    pub lookup: Vec<(i64, i64)>,
    /// With `lookup`, the value of a number not listed, instead of the
    /// number itself.
    #[serde(default)]
    pub lookup_else: Option<i64>,
    /// The model's value is the number times `scale`, plus `offset`.
    #[serde(default)]
    pub scale: Option<Fixed>,
    #[serde(default)]
    pub offset: Option<Fixed>,
    /// Conditions over the original's state that say more than the bytes
    /// do: the first that holds gives the value instead, or notes that a
    /// start from the original approximates it (schema 3).
    #[serde(default, rename = "case")]
    pub cases: Vec<Case>,
    /// A presentation value (schema 4): a translation loads it, but no
    /// run compares it, so it is no measure and counts for nothing in a
    /// score. Super Mario Bros.' color rotation is one.
    #[serde(default)]
    pub presentation: bool,
    /// A slot field whose bytes are at one address for every slot (schema
    /// 12), a value the game keeps once and not per object: Super Mario
    /// Bros.' FrenzyEnemyTimer.
    #[serde(default)]
    pub shared: bool,
    /// A slot field's bytes from one slot to the next (feature
    /// `slots.field.stride`): slot `i`'s are `i * stride` past slot 0's.
    /// Without it, 1. Donkey Kong's barrels are drawn from a sprite buffer
    /// with 16 bytes a barrel, and the game keeps no other copy of their
    /// places.
    #[serde(default)]
    pub stride: Option<u16>,
    /// A slot field whose value is the slot's own number, plus `offset`
    /// (feature `slots.field.index`), with `ram = []`: for a hook that
    /// keeps which slot it is in a word, as Donkey Kong's oil-drum fires,
    /// whose slot picks their random byte and their climb.
    #[serde(default)]
    pub index: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// Feature `field.oam_tile`: a sprite found in OAM by its tile.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OamTile {
    /// The tiles to look for, as OAM's byte 1 holds them.
    pub tiles: Vec<u8>,
    /// The byte of the sprite found that the field reads: 0 its y, 1 its
    /// tile, 2 its attributes, 3 its x.
    pub byte: u8,
    /// The field's number when no sprite has one of the tiles.
    #[serde(default)]
    pub none: i64,
}

/// The index into a canonical state's `nametables` (the console's CIRAM,
/// 2KB, or 4KB on a four-screen board) of PPU address `addr` ($2000 to
/// $2FFF and its mirror up to $3EFF) under `mirroring` as the canonical
/// state numbers it (docs/formats/recording.md): 0 vertical, 1
/// horizontal, 2 one-screen lower, 3 one-screen upper, 4 four-screen.
pub fn nametable_index(addr: u16, mirroring: i64) -> u16 {
    let a = addr & 0x0FFF;
    let (table, off) = (a / 0x400, a % 0x400);
    let bank = match mirroring {
        0 => table & 1,
        1 => table >> 1,
        2 => 0,
        3 => 1,
        _ => table,
    };
    bank * 0x400 + off
}

/// One of a field's cases: while `when` holds, the field is `value`
/// whatever its bytes say, and a translation of the original's state notes
/// it `approximated` for the reason given.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// A condition over the original's state, in the runtime protocol's
    /// condition grammar.
    pub when: String,
    /// A name, for a field of named values: one of its `values` or a name
    /// the bytes alone don't give.
    #[serde(default)]
    pub value: Option<String>,
    /// Why a model state taken from the original while `when` holds is
    /// only approximately the original's.
    #[serde(default)]
    pub approximated: Option<String>,
    /// Feature `field.case.unmeasured`: why the field means nothing to
    /// compare while `when` holds. A run doesn't compare it on those
    /// frames (they count as agreeing); a translation still loads it.
    /// Donkey Kong counts Kong's throw timer and the 11-frame divider on
    /// through a death and reuses $43 as the death's own timer, while
    /// the model holds them still and restart-level sets them again.
    #[serde(default)]
    pub unmeasured: Option<String>,
    /// Feature `field.case.number`: a number field's value while `when`
    /// holds, whatever its bytes say. Donkey Kong keeps a jump's takeoff
    /// height in $95 and puts $FF there once the fall is too far; a case
    /// gives that the takeoff far above, so the landing kills.
    #[serde(default)]
    pub number: Option<i64>,
    /// Feature `field.case.ram`: while `when` holds, the field reads these
    /// addresses in place of its `ram`, as many of them. A value a game
    /// keeps once per player at an address picked by whose turn it is:
    /// Donkey Kong's scores at $25 + 4 times the player ($52).
    #[serde(default)]
    pub ram: Option<Vec<u16>>,
    /// Feature `field.case.oam`: while `when` holds, a field that reads
    /// OAM (its `oam`) reads these sprite table bytes in place of its own,
    /// as many of them. A game that sends its sprites to OAM in another
    /// order on one stage: Donkey Kong: Original Edition's 50m copies its
    /// sprite buffer into page $07 in turns ($895F), so Pauline's body is
    /// sprite 16 there, not 60.
    #[serde(default)]
    pub oam: Option<Vec<u16>>,
    /// Feature `field.case.nametable`: while `when` holds, a field that
    /// reads the name tables (its `nametable`) reads these PPU addresses
    /// in place of its own, as many of them. A picture a game draws at
    /// another place on one stage: Donkey Kong draws Kong at $2084 on 25m
    /// and 75m and at $208D on 100m (the table at $C608 by $53), so the
    /// cell 25m reads at $20C5 is at $20CE there.
    #[serde(default)]
    pub nametable: Option<Vec<u16>>,
    #[serde(default)]
    pub note: Option<String>,
}

impl Mapping {
    /// The mapping without its presentation fields: what a run compares.
    pub fn measured(&self) -> Mapping {
        let mut m = self.clone();
        m.fields.retain(|f| !f.presentation);
        m
    }

    /// Only the fields that name a body's animation (`present.` paths,
    /// schema 5): what a screen measure loads into the model's
    /// presentation after a translation.
    pub fn animation(&self) -> Mapping {
        let mut m = self.clone();
        m.fields.retain(|f| f.path.starts_with("present."));
        m
    }

    /// An arcade mapping has none of the NES's own parts.
    fn check_arcade(&self) -> Result<(), String> {
        let refuse = |what: &str| {
            Err(format!(
                "{what} is the NES's: an arcade mapping (platform = \"arcade\") reads ram[...] and the runtime's own selectors"
            ))
        };
        if self.read_at.is_some() {
            return refuse("`read_at`");
        }
        fn of_slots(s: &Slots) -> Vec<&Field> {
            let mut v: Vec<&Field> = s.fields.iter().chain(&s.rest).collect();
            v.extend(s.groups.iter().flat_map(of_slots));
            v
        }
        let mut fields: Vec<&Field> = self.fields.iter().collect();
        fields.extend(self.slots.iter().flat_map(of_slots));
        for f in fields {
            if !f.nametable.is_empty() || !f.oam.is_empty() || f.oam_tile.is_some() {
                return refuse(&format!("{}: `nametable`, `oam` or `oam_tile`", f.path));
            }
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let listed = gp_model::features::listed_in(text, None);
        let unknown = gp_model::features::unknown(&listed, crate::mapping_features::FEATURES);
        if !unknown.is_empty() {
            return Err(gp_model::features::refusal("the state mapping", &unknown));
        }
        let mut m: Mapping = toml::from_str(text).map_err(|e| e.to_string())?;
        if let Some(s) = m.slots.as_mut() {
            let vacant = s.vacant.clone();
            for g in s.groups.iter_mut().filter(|g| g.vacant.is_none()) {
                g.vacant.clone_from(&vacant);
            }
        }
        if !m.features.is_empty() {
            let tree =
                toml::Value::Table(text.parse().map_err(|e: toml::de::Error| e.to_string())?);
            let missing: Vec<&str> =
                gp_model::features::used(&tree, crate::mapping_features::FEATURES)
                    .into_iter()
                    .filter(|f| !m.features.iter().any(|n| n == f))
                    .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "the state mapping uses {} and its `features` doesn't list {}",
                    missing
                        .iter()
                        .map(|f| format!("`{f}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if missing.len() == 1 { "it" } else { "them" }
                ));
            }
        }
        if !READS.contains(&m.schema) {
            return Err(format!(
                "state mapping schema {}; this build reads 1 to {SCHEMA}",
                m.schema
            ));
        }
        if m.cannons.is_some() {
            return Err(
                "`cannons` (schema 19) is gone since schema 20: map the cannons as a `[[slots.group]]` of a launcher entity".into(),
            );
        }
        if !m.ignore.is_empty() && m.schema < 21 {
            return Err("`ignore` needs schema 21".into());
        }
        if m.score_draws && m.draws.is_empty() {
            return Err("`score_draws` needs `[[draws]]` sites".into());
        }
        if let Some(st) = &m.stretches {
            if let Some(p) = &st.play {
                gp_runtime::select::Condition::parse(p)
                    .map_err(|e| format!("stretches.play `{p}`: {e}"))?;
            }
            for n in &st.state {
                gp_runtime::select::Condition::parse(&n.when)
                    .map_err(|e| format!("stretches.state {}: `{}`: {e}", n.name, n.when))?;
            }
        }
        if let Some(sc) = &m.scope {
            if sc.while_.is_none() && sc.leaves.is_empty() {
                return Err("`[scope]` needs `while` or `leaves`".into());
            }
            if let Some(w) = &sc.while_ {
                gp_runtime::select::Condition::parse(w)
                    .map_err(|e| format!("scope.while `{w}`: {e}"))?;
            }
            for l in &sc.leaves {
                gp_runtime::select::Selector::parse(l)
                    .map_err(|e| format!("scope.leaves `{l}`: {e}"))?;
            }
        }
        for (i, d) in m.draws.iter().enumerate() {
            if d.site.is_empty() || d.site.contains(char::is_whitespace) {
                return Err(format!("draws: `{}` isn't a site name", d.site));
            }
            if m.draws[..i].iter().any(|e| e.pc == d.pc) {
                return Err(format!("draws: two sites at pc {:#06x}", d.pc));
            }
        }
        let arcade = match m.platform.as_deref() {
            None | Some("nes") => false,
            Some("arcade") => true,
            Some(p) => return Err(format!("platform {p:?}: nes or arcade")),
        };
        if arcade {
            m.check_arcade()?;
        }
        for i in &m.ignore {
            if !IGNORE_ROLES.contains(&i.role.as_str()) {
                return Err(format!(
                    "ignore ${:04X}: role {:?} is none of {}",
                    i.ram,
                    i.role,
                    IGNORE_ROLES.join(", ")
                ));
            }
            if arcade {
                if i.len == 0 || u32::from(i.ram) + u32::from(i.len) > 0x1_0000 {
                    return Err(format!(
                        "ignore ${:04X}: {} bytes from there aren't all in the main CPU's addresses",
                        i.ram, i.len
                    ));
                }
            } else if i.len == 0 || u32::from(i.ram) + u32::from(i.len) > 0x800 {
                return Err(format!(
                    "ignore ${:04X}: {} bytes from there aren't all in RAM ($0000-$07FF)",
                    i.ram, i.len
                ));
            }
        }
        if let Some(c) = &m.cells {
            if m.schema < 14 {
                return Err("`cells` needs schema 14".into());
            }
            if c.pages.is_empty() || c.page_columns < 1 {
                return Err("`cells` needs a page and at least 1 column a page".into());
            }
        }
        for f in &m.fields {
            if f.presentation && m.schema < 4 {
                return Err(format!("{}: `presentation` needs schema 4", f.path));
            }
            if f.shared {
                return Err(format!("{}: `shared` is for a slot field", f.path));
            }
            if f.stride.is_some() {
                return Err(format!("{}: `stride` is for a slot field", f.path));
            }
            if f.index {
                return Err(format!("{}: `index` is for a slot field", f.path));
            }
            let p = gp_model::Path::parse(&f.path).map_err(|e| format!("{}: {e}", f.path))?;
            if !p.is_concrete() {
                return Err(format!("{}: a mapped path names one value", f.path));
            }
            if p.segments[0].name == "ticks" {
                if m.schema < 2 {
                    return Err(format!("{}: a ticks path needs schema 2", f.path));
                }
                let one = p.segments.len() == 2
                    && p.segments[0].index.is_none()
                    && p.segments[1].index.is_some();
                if !one {
                    return Err(format!(
                        "{}: a ticks path is ticks.<counter>[i], the counter's i-th `every` change",
                        f.path
                    ));
                }
                if f.flag || f.named() {
                    return Err(format!("{}: a count of frames is a number", f.path));
                }
            }
            if !f.names.is_empty() && m.schema < 17 {
                return Err(format!("{}: `names` needs schema 17", f.path));
            }
            if f.ram.len() > 5 && !f.digits && m.schema < 18 {
                return Err(format!(
                    "{}: more than 5 addresses for a whole number needs schema 18",
                    f.path
                ));
            }
            if f.whole && f.frac.is_some() && m.schema < 5 {
                return Err(format!("{}: `whole` with `frac` needs schema 5", f.path));
            }
            if p.segments[0].name == "present" {
                if m.schema < 6 {
                    return Err(format!("{}: a present path needs schema 6", f.path));
                }
                let one = p.segments.len() == 3
                    && p.segments[1].index.is_some()
                    && PRESENT_PARTS.contains(&p.segments[2].name.as_str());
                if !one {
                    return Err(format!(
                        "{}: a present path is present.<kind>[i].frame or .timer",
                        f.path
                    ));
                }
                if !f.presentation {
                    return Err(format!(
                        "{}: a present path is presentation: give it `presentation = true`",
                        f.path
                    ));
                }
                if f.flag || f.named() {
                    return Err(format!("{}: an animation's place is a number", f.path));
                }
            }
            if !f.cases.is_empty() && m.schema < 3 {
                return Err(format!("{}: a field's cases need schema 3", f.path));
            }
            f.check()?;
            f.check_cases()?;
        }
        for f in &m.fields {
            let p = gp_model::Path::parse(&f.path).map_err(|e| format!("{}: {e}", f.path))?;
            let last = p.segments.last().map(|s| s.name.as_str()).unwrap_or("");
            if m.schema < 5 && p.segments[0].name == "entities" && ENTITY_LOADS.contains(&last) {
                return Err(format!("{}: an entity's `{last}` needs schema 5", f.path));
            }
            if m.schema < 9 && p.segments[0].name == "input" {
                return Err(format!("{}: an input path needs schema 9", f.path));
            }
            if m.schema < 8
                && p.segments[0].name == "cameras"
                && matches!(last, "move_x" | "move_y")
            {
                return Err(format!("{}: a camera's `{last}` needs schema 8", f.path));
            }
            if m.schema < 10 && p.segments[0].name == "entities" && last == "seen_speed" {
                return Err(format!(
                    "{}: an entity's `seen_speed` needs schema 10",
                    f.path
                ));
            }
            if m.schema < 12
                && p.segments[0].name == "entities"
                && matches!(last, "facing" | "moved")
                && f.values.iter().any(|v| v == "both")
            {
                return Err(format!("{}: the value `both` needs schema 12", f.path));
            }
        }
        if let Some(slots) = &m.slots {
            slots.check(m.schema, &REST_PATHS)?;
            if m.schema < 15 && slots.rest.iter().any(|f| f.path == "y_frac") {
                return Err("slots: rest y_frac needs schema 15".into());
            }
            if m.schema < 22 && slots.rest.iter().any(|f| f.path == "word") {
                return Err("slots: rest word needs schema 22".into());
            }
            if slots.turn.is_some() {
                return Err("slots: only a group has a turn".into());
            }
            if !slots.groups.is_empty() && m.schema < 9 {
                return Err("slots: a group needs schema 9".into());
            }
            if slots.kinds.iter().any(|k| k.up_down) && m.schema < 9 {
                return Err("slots: up_down needs schema 9".into());
            }
            let mut names = std::collections::BTreeSet::new();
            for k in &slots.kinds {
                names.insert(k.entity.as_str());
            }
            for g in &slots.groups {
                g.check(m.schema, &SHOT_REST_PATHS)?;
                if !g.groups.is_empty() || !g.levels.is_empty() || g.held.is_some() {
                    return Err("slots: a group has no groups, levels or held of its own".into());
                }
                if (!g.rest.is_empty() || g.turn.is_some()) && m.schema < 11 {
                    return Err("slots: a group's rest and turn need schema 11".into());
                }
                for k in &g.kinds {
                    if !names.insert(k.entity.as_str()) {
                        return Err(format!("slots: {} is in two groups", k.entity));
                    }
                }
            }
        }
        for p in &m.pending {
            if p.count == 0 || p.by.is_empty() {
                return Err(format!(
                    "pending {}: give a `count` and a `by` list",
                    p.counter
                ));
            }
        }
        let mut paths: Vec<&str> = m.fields.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        if let Some(w) = paths.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("{} is mapped twice", w[0]));
        }
        Ok(m)
    }
}

impl Mapping {
    /// The slot groups: `[slots]`, then each of its `group`s (schema 9).
    pub fn slot_groups(&self) -> Vec<&Slots> {
        match &self.slots {
            None => Vec::new(),
            Some(s) => std::iter::once(s).chain(&s.groups).collect(),
        }
    }
}

impl Slots {
    fn check(&self, schema: u32, rest_paths: &[&str]) -> Result<(), String> {
        if self.active_stride == Some(0) {
            return Err("slots: active_stride is at least 1".into());
        }
        if self.count == 0 {
            return Err("slots: count is at least 1".into());
        }
        for f in &self.rest {
            if schema < 7 {
                return Err("slots: rest needs schema 7".into());
            }
            if !rest_paths.contains(&f.path.as_str()) {
                return Err(format!(
                    "slots: rest {} isn't one of {}",
                    f.path,
                    rest_paths.join(", ")
                ));
            }
            if f.flag || f.named() || f.digits || !f.cases.is_empty() {
                return Err(format!("slots: rest {} is a number", f.path));
            }
            f.check()?;
            if f.reads_ppu() {
                return Err(format!("slots: {} reads RAM, not the PPU", f.path));
            }
        }
        if self.part_flag.is_some() && schema < 19 {
            return Err("slots: part_flag needs schema 19".into());
        }
        if self.kinds.iter().any(|k| k.holds_slot) && schema < 19 {
            return Err("slots: a kind's holds_slot needs schema 19".into());
        }
        if self.held.is_some() {
            if schema < 15 {
                return Err("slots: held needs schema 15".into());
            }
            if self.state.is_none() {
                return Err("slots: held needs state".into());
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for f in self
            .fields
            .iter()
            .chain(self.kinds.iter().flat_map(|k| &k.fields))
            .chain(self.held.iter().flat_map(|h| &h.fields))
        {
            let p = f.path.as_str();
            let loads = SLOT_LOADS.contains(&p);
            if !SLOT_PATHS.contains(&p) && !loads {
                return Err(format!(
                    "slots: {} isn't one of {}, {}",
                    f.path,
                    SLOT_PATHS.join(", "),
                    SLOT_LOADS.join(", ")
                ));
            }
            if loads && schema < 5 {
                return Err(format!("slots: {} needs schema 5", f.path));
            }
            if SLOT_LOADS_17.contains(&p) && schema < 17 {
                return Err(format!("slots: {} needs schema 17", f.path));
            }
            if !f.names.is_empty() && schema < 17 {
                return Err(format!("slots: {}: `names` needs schema 17", f.path));
            }
            if f.shared && schema < 13 {
                return Err(format!("slots: {}: `shared` needs schema 13", f.path));
            }
            if f.stride == Some(0) || (f.shared && f.stride.is_some()) {
                return Err(format!(
                    "slots: {}: a stride is at least 1, and a shared field has none",
                    f.path
                ));
            }
            let named = matches!(p, "facing" | "state");
            if f.digits
                || (f.flag && p != "path_back")
                || (f.named() && !named)
                || (p == "path_back" && !f.flag)
                || (named && !f.named())
            {
                let what = match p {
                    "path_back" => "a flag",
                    "facing" | "state" => "named values",
                    _ => "a number",
                };
                return Err(format!("slots: {} is {what}", f.path));
            }
            if !f.cases.is_empty() {
                return Err(format!("slots: {} takes no cases", f.path));
            }
            f.check()?;
            if f.reads_ppu() {
                return Err(format!("slots: {} reads RAM, not the PPU", f.path));
            }
        }
        for k in &self.kinds {
            if (k.refuse.is_some() || k.leave.is_some()) && schema < 17 {
                return Err(format!(
                    "slots: {}: `refuse` and `leave` need schema 17",
                    k.entity
                ));
            }
            if k.refuse.is_some() && k.leave.is_some() {
                return Err(format!(
                    "slots: {}: `refuse` or `leave`, not both",
                    k.entity
                ));
            }
            // A kind that only holds a slot takes what its ID's kind leaves.
            if k.holds_slot {
                continue;
            }
            for id in &k.ids {
                if !ids.insert(*id) {
                    return Err(format!("slots: ID {id:#04x} names two entities"));
                }
            }
        }
        if self.state.is_none()
            && (self.ends.is_some() || self.kinds.iter().any(|k| k.ends.is_some()))
        {
            return Err("slots: ends needs state".into());
        }
        if let Some(c) = &self.moving {
            gp_runtime::select::Condition::parse(c).map_err(|e| format!("slots.moving: {e}"))?;
        }
        if let Some(c) = &self.vacant {
            gp_runtime::select::Condition::parse(c).map_err(|e| format!("slots.vacant: {e}"))?;
        }
        if let Some(v) = &self.vacant_from {
            gp_runtime::select::Condition::parse(&v.when)
                .map_err(|e| format!("slots.vacant_from: {e}"))?;
            if v.slot >= self.count {
                return Err(format!(
                    "slots.vacant_from: slot {} isn't below the count {}",
                    v.slot, self.count
                ));
            }
        }
        for l in &self.levels {
            gp_runtime::select::Condition::parse(&l.when)
                .map_err(|e| format!("slots.level {}: {e}", l.level))?;
        }
        Ok(())
    }

    /// Slot `slot`'s ID, read by `byte` from its address, or 0 for slots
    /// with no ID (`slots.no_id`).
    pub fn id_of(
        &self,
        slot: u16,
        byte: impl FnOnce(u16) -> Result<u8, String>,
    ) -> Result<u8, String> {
        match self.id {
            Some(at) => byte(at + slot),
            None => Ok(0),
        }
    }

    /// The address of slot `slot`'s flag (`active`, by `active_stride`).
    pub fn active_at(&self, slot: u16) -> u16 {
        self.active + slot * self.active_stride.unwrap_or(1)
    }

    /// Slot `slot`'s flag, read by `byte` from its address: the byte, or
    /// with `free`, 0 for that value and 1 for any other.
    pub fn flag_of(
        &self,
        slot: u16,
        byte: impl FnOnce(u16) -> Result<u8, String>,
    ) -> Result<u8, String> {
        let raw = byte(self.active_at(slot))?;
        Ok(match self.free {
            Some(f) => u8::from(raw != f),
            None => raw,
        })
    }

    /// Whether `flag` marks a second part (`part_flag`).
    pub fn is_part(&self, flag: u8) -> bool {
        self.part_flag.is_some_and(|b| flag & b != 0)
    }

    /// Whether a slot with `flag` can hold `kind`'s object.
    pub fn flag_fits(&self, kind: &SlotKind, flag: u8) -> bool {
        match &kind.flags {
            Some(f) => f.contains(&flag),
            None => !self.is_part(flag),
        }
    }

    /// The entity an ID names, if any.
    pub fn kind_of(&self, id: u8) -> Option<&SlotKind> {
        self.kinds.iter().find(|k| k.ids.contains(&id))
    }

    /// The values compared for an entity: the slots', with its own
    /// replacing those of the same path, in the slots' order then its own.
    pub fn fields_of<'a>(&'a self, kind: &'a SlotKind) -> Vec<&'a Field> {
        self.fields_with(&kind.fields)
    }

    /// The slots' values, with `own` replacing those of the same path, in
    /// the slots' order then `own`'s.
    pub fn fields_with<'a>(&'a self, own: &'a [Field]) -> Vec<&'a Field> {
        let mut out: Vec<&Field> = self
            .fields
            .iter()
            .filter(|f| !own.iter().any(|k| k.path == f.path))
            .collect();
        out.extend(own);
        out
    }

    /// The values `gp interp slots` compares for an entity: those of
    /// [`fields_of`](Self::fields_of) that are numbers of the body's
    /// (`SLOT_PATHS`). The rest only load into a translated body.
    pub fn measured_of<'a>(&'a self, kind: &'a SlotKind) -> Vec<&'a Field> {
        self.fields_of(kind)
            .into_iter()
            .filter(|f| SLOT_PATHS.contains(&f.path.as_str()))
            .collect()
    }
}

impl Field {
    /// The checks every field takes, whatever its path names.
    fn check(&self) -> Result<(), String> {
        let f = self;
        // The value is shifted 16 bits for its fraction, so it has 47
        // bits: 5 bytes, or 14 digits. A plain whole number takes no shift:
        // up to 8 bytes, the eighth's top bit the number's sign bit
        // (feature field.whole_64, Donkey Kong's 64-bit register $18-$1F).
        let plain = f.frac.is_none()
            && f.scale.is_none()
            && f.offset.is_none()
            && f.bits.is_none()
            && !f.signed
            && !f.flag
            && f.values.is_empty()
            && f.names.is_empty();
        if f.reads_ppu() {
            let sources = usize::from(!f.nametable.is_empty())
                + usize::from(!f.oam.is_empty())
                + usize::from(f.oam_tile.is_some())
                + usize::from(f.machine.is_some());
            if sources > 1 || !f.ram.is_empty() || f.frac.is_some() || f.index {
                return Err(format!(
                    "{}: a field reads one of ram, nametable, oam, oam_tile and machine, with no frac or index",
                    f.path
                ));
            }
            if f.nametable.iter().any(|a| !(0x2000..0x3F00).contains(a)) {
                return Err(format!(
                    "{}: nametable takes PPU addresses $2000 to $3EFF",
                    f.path
                ));
            }
            if f.oam.iter().any(|i| *i > 255) {
                return Err(format!("{}: oam takes indexes 0 to 255", f.path));
            }
            if let Some(t) = &f.oam_tile
                && (t.tiles.is_empty() || t.byte > 3)
            {
                return Err(format!(
                    "{}: oam_tile takes one tile or more and a byte 0 to 3",
                    f.path
                ));
            }
        }
        if f.index {
            if !f.ram.is_empty() || f.frac.is_some() || f.scale.is_some() || f.named() || f.flag {
                return Err(format!(
                    "{}: an index field has `ram = []` and at most an offset",
                    f.path
                ));
            }
            return Ok(());
        }
        let n = f.width();
        if n == 0 || n > 5 && !f.digits && !(plain && n <= 8) || n > 14 {
            return Err(format!(
                "{}: ram lists 1 to 5 addresses, 8 for a plain whole number, or up to 14 digits",
                f.path
            ));
        }
        if f.invert && !f.flag {
            return Err(format!("{}: invert goes with flag", f.path));
        }
        if f.lookup_else.is_some() && f.lookup.is_empty() {
            return Err(format!("{}: lookup_else goes with lookup", f.path));
        }
        if !f.lookup.is_empty() && (f.flag || f.named()) {
            return Err(format!("{}: lookup goes with a number", f.path));
        }
        if f.bcd && (f.digits || f.signed || f.bits.is_some() || f.frac.is_some() || n > 7) {
            return Err(format!(
                "{}: bcd takes 1 to 7 bytes and no digits, signed, bits or frac",
                f.path
            ));
        }
        if f.digits && (f.signed || f.bits.is_some() || f.frac.is_some()) {
            return Err(format!(
                "{}: digits don't take signed, bits or frac",
                f.path
            ));
        }
        if let Some([a, b]) = f.bits
            && (a >= b || b > 8 * n as u32)
        {
            return Err(format!(
                "{}: bits [{a}, {b}) aren't within the {n} bytes",
                f.path
            ));
        }
        let kinds = usize::from(f.flag)
            + usize::from(!f.values.is_empty())
            + usize::from(!f.names.is_empty())
            + usize::from(f.frac.is_some() || f.scale.is_some() || f.offset.is_some());
        if kinds > 1 {
            return Err(format!(
                "{}: a field is a flag, a named value or a number, not more than one",
                f.path
            ));
        }
        Ok(())
    }
    fn check_cases(&self) -> Result<(), String> {
        for c in &self.cases {
            gp_runtime::select::Condition::parse(&c.when)
                .map_err(|e| format!("{}: case `{}`: {e}", self.path, c.when))?;
            if c.number.is_some() && (self.flag || self.named()) {
                return Err(format!(
                    "{}: case `{}` gives a number, so the field is a number",
                    self.path, c.when
                ));
            }
            if c.ram.as_ref().is_some_and(|r| r.len() != self.width()) {
                return Err(format!(
                    "{}: case `{}`: its `ram` names as many addresses as the field's",
                    self.path, c.when
                ));
            }
            if let Some(o) = &c.oam {
                if self.oam.is_empty() || c.ram.is_some() {
                    return Err(format!(
                        "{}: case `{}`: an `oam` is for a field that reads OAM, without a case `ram`",
                        self.path, c.when
                    ));
                }
                if o.len() != self.oam.len() || o.iter().any(|i| *i > 0xFF) {
                    return Err(format!(
                        "{}: case `{}`: its `oam` names as many sprite table bytes as the field's, each below 256",
                        self.path, c.when
                    ));
                }
            }
            if let Some(n) = &c.nametable {
                if self.nametable.is_empty() || c.ram.is_some() {
                    return Err(format!(
                        "{}: case `{}`: a `nametable` is for a field that reads the name tables, without a case `ram`",
                        self.path, c.when
                    ));
                }
                if n.len() != self.nametable.len() {
                    return Err(format!(
                        "{}: case `{}`: its `nametable` names as many PPU addresses as the field's",
                        self.path, c.when
                    ));
                }
            }
            if c.value.is_none()
                && c.approximated.is_none()
                && c.unmeasured.is_none()
                && c.number.is_none()
                && c.ram.is_none()
                && c.oam.is_none()
                && c.nametable.is_none()
            {
                return Err(format!(
                    "{}: case `{}` gives a value, an approximated or an unmeasured reason",
                    self.path, c.when
                ));
            }
            if self.flag {
                if c.value
                    .as_deref()
                    .is_some_and(|v| v != "true" && v != "false")
                {
                    return Err(format!(
                        "{}: case `{}`: a flag's value is `true` or `false`",
                        self.path, c.when
                    ));
                }
            } else if c.value.is_some() && !self.named() {
                return Err(format!(
                    "{}: case `{}` gives a name, so the field is a named value (`values`)",
                    self.path, c.when
                ));
            }
        }
        Ok(())
    }

    /// The first case whose condition holds, reading the original's state
    /// through `read` (a selector's number). `None` for a field with no
    /// case that holds.
    pub fn case_held(
        &self,
        read: &mut impl FnMut(&gp_runtime::select::Selector) -> Result<i64, String>,
    ) -> Result<Option<&Case>, String> {
        for c in &self.cases {
            let cond = gp_runtime::select::Condition::parse(&c.when)?;
            if cond.eval(read)? {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// The model value the original's state stands for, as
    /// [`Field::value`] reads its bytes, with the case that holds, if any,
    /// giving the value instead. `read` reads a selector's number.
    pub fn read(
        &self,
        read: &mut impl FnMut(&str) -> Result<i64, String>,
    ) -> Result<(Value, Option<&Case>), String> {
        let case = self.case_held(&mut |s| read(&s.to_string()))?;
        let bytes = self.read_bytes(case, read)?;
        let v = match case {
            Some(Case {
                number: Some(n), ..
            }) => Value::Fixed(n << 16),
            _ => match (case.and_then(|c| c.value.as_ref()), bytes) {
                (Some(name), _) => self.case_value(name),
                (None, Some(bytes)) => self.value(&bytes),
                (None, None) => self.value_of(self.oam_tile.as_ref().map_or(0, |t| t.none)),
            },
        };
        Ok((v, case))
    }

    /// Whether the field reads the PPU's state or the machine's rather
    /// than RAM (features `field.nametable`, `field.oam`, `field.oam_tile`,
    /// `field.machine`).
    pub fn reads_ppu(&self) -> bool {
        !self.nametable.is_empty()
            || !self.oam.is_empty()
            || self.oam_tile.is_some()
            || self.machine.is_some()
    }

    /// The number of bytes the field's value is made of.
    fn width(&self) -> usize {
        if self.oam_tile.is_some() {
            1
        } else if self.machine.is_some() {
            4
        } else if !self.nametable.is_empty() {
            self.nametable.len()
        } else if !self.oam.is_empty() {
            self.oam.len()
        } else {
            self.ram.len()
        }
    }

    /// The bytes the field reads while `case` holds, through `read`:
    /// RAM, or the PPU's name tables or OAM. `None` for an `oam_tile`
    /// that no sprite has.
    fn read_bytes(
        &self,
        case: Option<&Case>,
        read: &mut impl FnMut(&str) -> Result<i64, String>,
    ) -> Result<Option<Vec<u8>>, String> {
        let byte = |read: &mut dyn FnMut(&str) -> Result<i64, String>, sel: String| {
            read(&sel).map(|v| v as u8)
        };
        if let Some(t) = self
            .oam_tile
            .as_ref()
            .filter(|_| case.and_then(|c| c.ram.as_ref()).is_none())
        {
            for slot in 0..64u16 {
                let tile = byte(read, format!("oam[{:#06x}]", slot * 4 + 1))?;
                if t.tiles.contains(&tile) {
                    let at = slot * 4 + u16::from(t.byte);
                    return Ok(Some(vec![byte(read, format!("oam[{at:#06x}]"))?]));
                }
            }
            return Ok(None);
        }
        if let Some(m) = self
            .machine
            .as_ref()
            .filter(|_| case.and_then(|c| c.ram.as_ref()).is_none())
        {
            let n = read(m)?;
            return Ok(Some((n as u32).to_le_bytes().to_vec()));
        }
        let sels: Vec<String> = if !self.nametable.is_empty() {
            let mirroring = read("mapper.mirroring")?;
            case.and_then(|c| c.nametable.as_ref())
                .unwrap_or(&self.nametable)
                .iter()
                .map(|a| format!("nametables[{:#06x}]", nametable_index(*a, mirroring)))
                .collect()
        } else if !self.oam.is_empty() {
            case.and_then(|c| c.oam.as_ref())
                .unwrap_or(&self.oam)
                .iter()
                .map(|i| format!("oam[{i:#06x}]"))
                .collect()
        } else {
            self.selectors_in(case)
        };
        // A case's `ram` (feature `field.case.ram`) reads RAM in place of
        // the PPU while it holds: a value on its way to the screen, such
        // as a name table write a game keeps in a buffer for the next NMI.
        let sels = match case.and_then(|c| c.ram.as_ref()) {
            Some(_) if self.reads_ppu() => self.selectors_in(case),
            _ => sels,
        };
        let mut bytes = Vec::with_capacity(sels.len());
        for sel in sels {
            bytes.push(byte(read, sel)?);
        }
        Ok(Some(bytes))
    }

    /// The model value of a number the field's bytes would make, as
    /// [`Self::value`] gives it after the bytes: scale, offset and the rest.
    fn value_of(&self, n: i64) -> Value {
        let bytes: Vec<u8> = (0..self.width().max(1))
            .map(|i| (n >> (8 * i)) as u8)
            .collect();
        self.value(&bytes)
    }

    /// [`Field::read`] from a frame's RAM, for a reader that holds RAM
    /// rather than a runtime. An address past its end reads 0, and a case
    /// that reads anything but RAM doesn't hold. A field that reads the
    /// PPU's state ([`Self::reads_ppu`]) reads zero bytes here.
    pub fn read_ram(&self, ram: &[u8]) -> Value {
        let byte = |a: i64| {
            usize::try_from(a)
                .ok()
                .and_then(|a| ram.get(a))
                .map_or(0, |b| *b)
        };
        let case = self
            .case_held(&mut |s| match s.segments.as_slice() {
                [seg] if seg.name == "ram" => match seg.index {
                    Some(gp_runtime::select::Index::At(a)) => Ok(i64::from(byte(a))),
                    _ => Err(format!("{s} names no one byte")),
                },
                _ => Err(format!("{s} isn't RAM")),
            })
            .ok()
            .flatten();
        let bytes: Vec<u8> = if self.reads_ppu() && case.and_then(|c| c.ram.as_ref()).is_none() {
            vec![0; self.width()]
        } else {
            case.and_then(|c| c.ram.as_ref())
                .unwrap_or(&self.ram)
                .iter()
                .chain(&self.frac)
                .map(|a| byte(i64::from(*a)))
                .collect()
        };
        match case {
            Some(Case {
                number: Some(n), ..
            }) => Value::Fixed(n << 16),
            _ => match case.and_then(|c| c.value.as_ref()) {
                Some(name) => self.case_value(name),
                None => self.value(&bytes),
            },
        }
    }

    /// A case's value: a name, or for a flag (feature `field.case_flag`)
    /// `true` or `false`.
    fn case_value(&self, name: &str) -> Value {
        if self.flag {
            Value::Flag(name == "true")
        } else {
            Value::Text(name.to_string())
        }
    }

    /// A named value: `values` by position or `names` by number.
    pub fn named(&self) -> bool {
        !self.values.is_empty() || !self.names.is_empty()
    }

    /// The RAM addresses the field reads while `case` holds (its `ram`,
    /// feature `field.case.ram`), else its own, as runtime selectors.
    fn selectors_in(&self, case: Option<&Case>) -> Vec<String> {
        case.and_then(|c| c.ram.as_ref())
            .unwrap_or(&self.ram)
            .iter()
            .chain(&self.frac)
            .map(|a| format!("ram[{a:#06x}]"))
            .collect()
    }

    /// Every RAM address the field reads, as runtime selectors.
    pub fn selectors(&self) -> Vec<String> {
        self.ram
            .iter()
            .chain(&self.frac)
            .map(|a| format!("ram[{a:#06x}]"))
            .collect()
    }

    /// The RAM addresses for slot `slot`: each one `slot` strides past the
    /// field's, or the field's own when it is `shared`.
    pub fn slot_addresses(&self, slot: u16) -> Vec<u16> {
        let step = if self.shared {
            0
        } else {
            slot * self.stride.unwrap_or(1)
        };
        self.ram
            .iter()
            .chain(&self.frac)
            .map(|a| a + step)
            .collect()
    }

    /// Slot `slot`'s value from the bytes its selectors read: the slot's
    /// number plus `offset` for an `index` field, else [`Self::value`].
    pub fn slot_value(&self, slot: u16, bytes: &[u8]) -> Value {
        if self.index {
            let offset = self.offset.map_or(0, |o| o.raw());
            return Value::Fixed((i64::from(slot) << 16) + offset);
        }
        self.value(bytes)
    }

    /// [`Self::slot_addresses`] as runtime selectors.
    pub fn slot_selectors(&self, slot: u16) -> Vec<String> {
        self.slot_addresses(slot)
            .into_iter()
            .map(|a| format!("ram[{a:#06x}]"))
            .collect()
    }

    /// The model value the original's bytes stand for, given the bytes the
    /// selectors read, in order.
    pub fn value(&self, bytes: &[u8]) -> Value {
        let n = self.width();
        let mut v: i64 = 0;
        if self.digits {
            for b in &bytes[..n] {
                v = v * 10 + i64::from(*b);
            }
        } else if self.bcd {
            for b in &bytes[..n] {
                v = v * 100 + i64::from(b >> 4) * 10 + i64::from(b & 0x0f);
            }
        } else {
            for (i, b) in bytes[..n].iter().enumerate() {
                v |= i64::from(*b) << (8 * i);
            }
            if self.signed && bytes[n - 1] & 0x80 != 0 {
                v -= 1 << (8 * n);
            }
        }
        if let Some([a, b]) = self.bits {
            v = (v >> a) & ((1 << (b - a)) - 1);
        }
        if let Some((_, to)) = self.lookup.iter().find(|(from, _)| *from == v) {
            v = *to;
        } else if let Some(other) = self.lookup_else.filter(|_| !self.lookup.is_empty()) {
            v = other;
        }
        if self.flag {
            return Value::Flag((v != 0) != self.invert);
        }
        if !self.values.is_empty() {
            return match usize::try_from(v).ok().and_then(|i| self.values.get(i)) {
                Some(name) => Value::Text(name.clone()),
                None => Value::Text(format!("(unnamed value {v})")),
            };
        }
        if !self.names.is_empty() {
            return match self.names.iter().find(|(n, _)| i64::from(*n) == v) {
                Some((_, name)) => Value::Text(name.clone()),
                None => Value::Text(format!("(unnamed value {v})")),
            };
        }
        if n > 5 && !self.digits && !self.bcd {
            return Value::Int(v);
        }
        let mut raw = v << 16;
        if self.frac.is_some() {
            raw += i64::from(bytes[n]) << 8;
        }
        if let Some(s) = self.scale {
            raw = ((i128::from(raw) * i128::from(s.raw())) >> 16) as i64;
        }
        if let Some(o) = self.offset {
            raw += o.raw();
        }
        Value::Fixed(raw)
    }
}

/// Whether the original's value `a` and the model's `b` agree for this
/// field: [`same`], or with `whole`, the same whole part.
pub fn agrees(f: &Field, a: &Value, b: &Value) -> bool {
    if f.whole {
        let floor = |v: &Value| match v {
            Value::Fixed(raw) => Some(raw >> 16),
            Value::Int(n) => Some(*n),
            _ => None,
        };
        if let (Some(x), Some(y)) = (floor(a), floor(b)) {
            return x == y;
        }
    }
    same(a, b)
}

/// Whether a model value and a mapped value are the same: numbers by
/// value whatever their kind, flags and names as they are.
pub fn same(a: &Value, b: &Value) -> bool {
    let num = |v: &Value| match v {
        Value::Fixed(raw) => Some(i128::from(*raw)),
        Value::Int(n) => Some(i128::from(*n) << 16),
        Value::Entity(n) => Some(i128::from(*n) << 16),
        _ => None,
    };
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// A value as a report prints it: fixed-point as an exact decimal.
pub fn show(v: &Value) -> String {
    match v {
        Value::Fixed(raw) => gp_sim::Fx::from_raw(*raw).to_string(),
        Value::Text(t) => t.clone(),
        other => other.to_string(),
    }
}
