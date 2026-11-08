//! Mechanic records: what each mechanic the simulation runs is, as data
//! (docs/mechanics.md; the research is docs/research/mechanics.md).
//!
//! A mechanic is a named, versioned, parameterized composition of
//! [`Primitive`]s that an entity, a level or the game carries. Its
//! [`Record`] says where in the frame it runs, its parameters with types,
//! units and ranges, the state it reads and writes, the events it fires and
//! reacts to, what it assumes, what must hold for it (invariants), what
//! validation measures of it, and which primitives it's built from.
//!
//! Records live beside the code that runs them, so they can't drift from
//! it: [`COMPONENTS`] here for the components `gp-sim` runs, and
//! [`crate::Ext::describe`] in each hook crate for its hooks. `gp model
//! mechanics` reads both for a model (gp-interp's `mechanics` module).
//! Everything is `'static` data with no dependencies, so a hook crate can
//! declare its records as constants.

/// The twelve primitives mechanics are built from (docs/research/
/// mechanics.md, "Primitives under mechanics"). They aren't model syntax:
/// records name them in `built_from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Primitive {
    /// A typed fixed-point value held by a body or the game.
    Quantity,
    /// A rate added to a quantity each frame, with a cap, in a stated order.
    Integrator,
    /// Frames since something, firing at a count or every N.
    Timer,
    /// An input action read as held or newly pressed.
    InputBinding,
    /// A predicate over quantities, input, a region or another body.
    Condition,
    /// A state with its own parameter set, entered by transitions.
    Mode,
    /// A box resolved against the solid grid.
    TileCollision,
    /// Two boxes overlapping, resolved by their motion into an event.
    BodyContact,
    /// A body made or taken out of the level.
    SpawnRemove,
    /// A named signal with a subject, delivered in a fixed order.
    Event,
    /// A number from the world's seeded generator.
    RandomDraw,
    /// Reading another body's quantity: the player's position or facing.
    Sensing,
}

impl Primitive {
    pub const ALL: [Primitive; 12] = [
        Self::Quantity,
        Self::Integrator,
        Self::Timer,
        Self::InputBinding,
        Self::Condition,
        Self::Mode,
        Self::TileCollision,
        Self::BodyContact,
        Self::SpawnRemove,
        Self::Event,
        Self::RandomDraw,
        Self::Sensing,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Quantity => "quantity",
            Self::Integrator => "integrator",
            Self::Timer => "timer",
            Self::InputBinding => "input-binding",
            Self::Condition => "condition",
            Self::Mode => "mode",
            Self::TileCollision => "tile-collision",
            Self::BodyContact => "body-contact",
            Self::SpawnRemove => "spawn-and-remove",
            Self::Event => "event",
            Self::RandomDraw => "random-draw",
            Self::Sensing => "sensing",
        }
    }
}

/// What carries a mechanic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Carrier {
    Entity,
    Level,
    Game,
}

impl Carrier {
    pub fn name(self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Level => "level",
            Self::Game => "game",
        }
    }
}

/// A table a hook reads: part of how the mechanic works, which a formula
/// could replace (a firebar's circle), or choices, which are content and a
/// clean room replaces (Bowser's flame heights).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TableKind {
    Mechanic,
    Content,
}

impl TableKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mechanic => "mechanic",
            Self::Content => "content",
        }
    }
}

/// One parameter of a mechanic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    /// `fixed`, `frames`, `count`, `bool`, `action`, `choice`, `list` or
    /// `rect`.
    pub ty: &'static str,
    /// Such as `px/frame`, `px/frame^2`, `px` or `frames`; empty for none.
    pub unit: &'static str,
    /// The values allowed, such as `> 0`, `< 0` or `1..=12`; empty for any.
    pub range: &'static str,
    pub optional: bool,
}

impl Param {
    /// A required parameter.
    pub const fn new(name: &'static str, ty: &'static str, unit: &'static str) -> Self {
        Self {
            name,
            ty,
            unit,
            range: "",
            optional: false,
        }
    }

    pub const fn range(mut self, range: &'static str) -> Self {
        self.range = range;
        self
    }

    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// A table a hook reads, by its `ext_tables` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    pub name: &'static str,
    pub kind: TableKind,
}

/// One mechanic, as data. Build one from [`Record::EMPTY`] with struct
/// update syntax, naming only what it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    /// Such as `platformer.jump`, or for a hook `smb1.firebar`.
    pub name: &'static str,
    /// The version of what declares it: an extension's, or a hook crate's.
    pub version: &'static str,
    pub carrier: Carrier,
    /// The system it runs in (`model.systems`).
    pub system: &'static str,
    /// verb, mover, touch, collision, spawner, timer, economy, flow,
    /// camera, hazard or projectile: about the mechanic, where an entity's
    /// role is about the entity.
    pub class: &'static str,
    /// The code that runs it.
    pub implementation: &'static str,
    /// For a hook, the general mechanic it is an instance of, such as
    /// `orbit`, when a component could one day say it.
    pub like: &'static str,
    /// What it does, in a sentence.
    pub about: &'static str,
    pub params: &'static [Param],
    /// Tables a hook reads, by `ext_tables` name.
    pub tables: &'static [Table],
    /// What it keeps between frames.
    pub state: &'static [&'static str],
    /// What it reads beyond its own parameters and state: `action`,
    /// `tiles.solid`, `self.vx`, `player.x`.
    pub reads: &'static [&'static str],
    /// What it changes: `self.vy`, `counters`, `level`.
    pub writes: &'static [&'static str],
    /// Events it fires and reacts to.
    pub emits: &'static [&'static str],
    pub consumes: &'static [&'static str],
    /// Entities a hook makes, by the model's names, when it is fixed.
    pub spawns: &'static [&'static str],
    /// What must be true of the world or of other mechanics.
    pub assumes: &'static [&'static str],
    /// Mechanics it must run after, in the same system.
    pub after: &'static [&'static str],
    /// Mechanics an entity can't carry beside it.
    pub excludes: &'static [&'static str],
    pub built_from: &'static [Primitive],
    /// What must hold for any parameters in range.
    pub invariants: &'static [&'static str],
    /// What validation measures of it, by name, with the unit.
    pub measures: &'static [(&'static str, &'static str)],
    /// What it makes reachable, derived from its parameters by simulation.
    pub envelope: &'static [(&'static str, &'static str)],
}

impl Record {
    pub const EMPTY: Record = Record {
        name: "",
        version: "",
        carrier: Carrier::Entity,
        system: "",
        class: "",
        implementation: "",
        like: "",
        about: "",
        params: &[],
        tables: &[],
        state: &[],
        reads: &[],
        writes: &[],
        emits: &[],
        consumes: &[],
        spawns: &[],
        assumes: &[],
        after: &[],
        excludes: &[],
        built_from: &[],
        invariants: &[],
        measures: &[],
        envelope: &[],
    };

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    /// The table named `name`.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// The platformer extension's version its records are for. A test in
/// gp-interp holds it to `gp_model::platformer::VERSION`.
pub const PLATFORMER_VERSION: &str = "0.4";
/// The core schema's version its records are for, held to
/// `gp_model::CORE_VERSION` the same way.
pub const CORE_VERSION: &str = "0.4";

use Primitive as P;

const FIXED: &str = "fixed";
const PX_F: &str = "px/frame";
const PX_F2: &str = "px/frame^2";

/// The mechanics `gp-sim` runs from a model's components, fields and
/// tables. A component of the platformer extension is `platformer.<name>`,
/// as it is written in the model; the core's are `core.<name>`.
pub const COMPONENTS: &[Record] = &[
    Record {
        name: "platformer.run",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::run",
        about: "Moving left and right with the left and right actions, toward a top speed, faster while the run action is held",
        params: &[
            Param::new("walk_speed", FIXED, PX_F).range("> 0"),
            Param::new("run_speed", FIXED, PX_F).range("> 0").optional(),
            Param::new("run_action", "action", "").optional(),
            Param::new("acceleration", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("run_acceleration", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("deceleration", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("skid_deceleration", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("air_control", "bool", ""),
            Param::new("start", "table", "speed px/frame, frames").optional(),
            Param::new("air_speed", FIXED, PX_F).range("> 0").optional(),
            Param::new("wall_snap", "bool", "").optional(),
            Param::new("move_step", FIXED, "px").range("> 0").optional(),
            Param::new("cap_whole", "bool", "").optional(),
            Param::new("skid_by", "choice", "speed or last_motion").optional(),
            Param::new("skid_turn_below", FIXED, PX_F)
                .range("> 0")
                .optional(),
            Param::new("skid_turn_with", "list", "action names").optional(),
            Param::new("decel_bands", "list", "from px/frame, rate px/frame^2").optional(),
            Param::new("decel_hold_from", FIXED, PX_F)
                .range("> 0")
                .optional(),
            Param::new("run_hold_frames", "count", "frames").optional(),
            Param::new("run_with_motion", "bool", "").optional(),
            Param::new("takeoff_in_air", "bool", "").optional(),
            Param::new("face_on_ground", "bool", "").optional(),
            Param::new("air_run_from", FIXED, PX_F)
                .range("> 0")
                .optional(),
            Param::new("walls", "table", "left and right points px").optional(),
            Param::new("push_masks", "bool", "").optional(),
            Param::new("speed_before_walls", "bool", "").optional(),
            Param::new("face_both", "bool", "").optional(),
            Param::new("off", "bool", "").optional(),
            Param::new("step_cycle", "list", "px per walking frame, by the step counter").optional(),
            Param::new("air_step_cycle", "list", "px per frame in the air, by its own counter").optional(),
            Param::new("lone_direction", "bool", "").optional(),
            Param::new("face_way", "bool", "").optional(),
            Param::new("both_left", "bool", "left and right held together turn left and move nowhere").optional(),
        ],
        state: &[
            "vx",
            "vx_frac",
            "facing",
            "moved",
            "run_hold",
            "decel_hold",
            "seen_speed",
            "mechanic words 0 (step counter, with a step cycle) and 1 (air step counter)",
        ],
        reads: &[
            "action.left",
            "action.right",
            "action.run",
            "tiles.solid",
            "self.on_ground",
        ],
        writes: &[
            "self.x",
            "self.vx",
            "self.facing",
            "self.vx_frac",
            "self.moved",
            "self.run_hold",
            "self.decel_hold",
            "self.seen_speed",
        ],
        excludes: &[
            "platformer.walk",
            "platformer.patrol",
            "platformer.fly",
            "platformer.path",
        ],
        built_from: &[
            P::InputBinding,
            P::Integrator,
            P::Condition,
            P::TileCollision,
        ],
        invariants: &[
            "the horizontal speed never exceeds run_speed (or walk_speed without run), by less than move_step with cap_whole",
            "with move_step, vx is a whole number of steps and 0 <= vx_frac < move_step",
            "a blocked body stops at the last open pixel with speed 0",
        ],
        measures: &[
            ("walk_speed", PX_F),
            ("run_speed", PX_F),
            ("acceleration", PX_F2),
        ],
        envelope: &[("speed", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.jump",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::fall",
        about: "Jumping when the action is pressed while standing, with gravity, an optional held gravity and speed bands, landing on solid tiles; with a head point, a rise into a tile hits it by its kind, setting the speed and firing the hit's event",
        params: &[
            Param::new("action", "action", ""),
            Param::new("speed", FIXED, PX_F).range("< 0"),
            Param::new("gravity", FIXED, PX_F2).range("> 0"),
            Param::new("gravity_held", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("max_fall", FIXED, PX_F).range("> 0"),
            Param::new("max_fall_from_half", "bool", "").optional(),
            Param::new("no_gravity_on_ground", "bool", "").optional(),
            Param::new("land_keeps_fraction", "bool", "").optional(),
            Param::new("keep_fall_gravity", "bool", "").optional(),
            Param::new("start_fall_gravity", FIXED, PX_F2)
                .range("> 0")
                .optional(),
            Param::new("land_depth", "count", "px").optional(),
            Param::new("feet", "list", "px").optional(),
            Param::new("bands", "list", "jump-band").optional(),
            Param::new("move_first", "bool", "").optional(),
            Param::new("coyote_frames", "frames", "frames").optional(),
            Param::new("buffer_frames", "frames", "frames").optional(),
            Param::new("release_cap", FIXED, PX_F).optional(),
            Param::new(
                "head",
                "table",
                "point px, above px, row_from px, keep_fraction, hits by tile kind (speed px/frame, event, becomes, hold frames)",
            )
            .optional(),
            Param::new("collide_y", "list", "from and to px").optional(),
            Param::new("pickup", "table", "tile kind, points px, event, becomes").optional(),
            Param::new("takeoff_fraction", FIXED, "px").range(">= 0").optional(),
            Param::new("land_hold", "frames", "frames").optional(),
            Param::new("ledge_fall", "table", "speed px/frame, first px, kills").optional(),
            Param::new("fatal_drop", "px", "px").optional(),
            Param::new("fatal_drop_in_air", "bool", "").optional(),
        ],
        state: &["vy", "on_ground", "band", "air", "buffer", "held", "fall_gravity", "mechanic word 2 (landing hold)"],
        reads: &["action.jump", "tiles.solid", "tiles.blocks", "self.vx"],
        writes: &["self.y", "self.vy", "self.on_ground", "tiles.changed"],
        assumes: &["view = side"],
        after: &["platformer.run"],
        excludes: &["platformer.fall"],
        built_from: &[
            P::InputBinding,
            P::Condition,
            P::Mode,
            P::Integrator,
            P::TileCollision,
        ],
        invariants: &[
            "the rise is about speed^2 / (2 * gravity), within a frame of rounding",
            "the body never moves more than max_fall down in a frame (less than max_fall + 1 with max_fall_from_half)",
            "with head, a head hit changes only the vertical speed and the cell hit, never the position",
        ],
        measures: &[
            ("speed", PX_F),
            ("gravity", PX_F2),
            ("max_fall", PX_F),
            ("height", "px"),
            ("airtime", "frames"),
            ("rise_by_hold", "px per frames held"),
        ],
        envelope: &[
            ("rise", "px"),
            ("airtime", "frames"),
            ("carry", "px, walking and running"),
        ],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.walk",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::walk",
        about: "Walking at a steady speed from facing the player, turning at walls and optionally at ledges",
        params: &[
            Param::new("speed", FIXED, PX_F).range(">= 0"),
            Param::new("turn_at_ledges", "bool", "").optional(),
            Param::new("walls_at_top", "bool", "").optional(),
            Param::new("turn_after_move", "bool", "").optional(),
            Param::new("ledge_at", FIXED, "px").optional(),
            Param::new("bump", "table", "every and phase frames, view px").optional(),
        ],
        state: &["facing", "bumps"],
        reads: &["tiles.solid", "player.x", "others.hitbox", "camera.x"],
        writes: &["self.x", "self.facing", "others.facing"],
        excludes: &[
            "platformer.patrol",
            "platformer.fly",
            "platformer.path",
            "platformer.run",
        ],
        built_from: &[P::Sensing, P::Integrator, P::TileCollision, P::Condition],
        measures: &[("speed", PX_F), ("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.patrol",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::patrol",
        about: "Stepping back and forth between the spawn's x and its patrol_end, once every period frames",
        params: &[
            Param::new("step", FIXED, "px").range("> 0"),
            Param::new("period", "frames", "frames").range(">= 1"),
        ],
        state: &["facing"],
        reads: &["spawn.patrol_end"],
        writes: &["self.x", "self.facing"],
        assumes: &["each spawn gives patrol_end"],
        excludes: &[
            "platformer.walk",
            "platformer.fly",
            "platformer.path",
            "platformer.run",
        ],
        built_from: &[P::Timer, P::Integrator, P::Condition, P::Mode],
        invariants: &["the body stays between its spawn x and patrol_end"],
        measures: &[("speed", PX_F), ("step", "px"), ("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.fall",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::fall",
        about: "Gravity for an entity that doesn't jump, landing on solid tiles",
        params: &[
            Param::new("gravity", FIXED, PX_F2).range("> 0"),
            Param::new("max_fall", FIXED, PX_F).range("> 0"),
            Param::new("max_fall_from_half", "bool", "").optional(),
            Param::new("move_first", "bool", "").optional(),
            Param::new("feet", "list", "px").optional(),
            Param::new("stands_still", "bool", "").optional(),
        ],
        state: &["vy", "on_ground"],
        reads: &["tiles.solid"],
        writes: &["self.y", "self.vy", "self.on_ground"],
        excludes: &["platformer.jump"],
        built_from: &[P::Integrator, P::TileCollision],
        invariants: &[
            "the body never moves more than max_fall down in a frame (less than max_fall + 1 with max_fall_from_half)",
        ],
        measures: &[("gravity", PX_F2), ("max_fall", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.fly",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::fly",
        about: "Moving sideways at a steady speed with no gravity, through tiles",
        params: &[
            Param::new("speed", FIXED, PX_F).range(">= 0"),
            Param::new("heading", "choice", "player, left, right").optional(),
            Param::new("keep_speed", "bool", "").optional(),
        ],
        state: &["facing"],
        reads: &["player.x"],
        writes: &["self.x"],
        excludes: &[
            "platformer.walk",
            "platformer.patrol",
            "platformer.path",
            "platformer.run",
        ],
        built_from: &[P::Sensing, P::Integrator],
        measures: &[("speed", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.hop",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::fall",
        about: "Jumping again whenever the entity lands, optionally by a pattern of speeds in turn and only on a beat",
        params: &[
            Param::new("speed", FIXED, PX_F).range("< 0"),
            Param::new("pattern", "list", PX_F).optional(),
            Param::new("every", "frames", "frames")
                .range(">= 1")
                .optional(),
            Param::new("phase", "frames", "frames per body").optional(),
            Param::new("land_first", "bool", "").optional(),
            Param::new("land_while_stopped", "bool", "").optional(),
        ],
        state: &["mechanic word 0 (hops, with a pattern)"],
        reads: &["self.on_ground", "frame"],
        writes: &["self.vy"],
        assumes: &["platformer.fall on the same entity"],
        built_from: &[P::Condition, P::Integrator, P::Timer, P::Quantity],
        measures: &[("rise", "px"), ("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.path",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::path",
        about: "Moving out along one axis by a distance and back, pausing at each end, optionally waiting while the player is near",
        params: &[
            Param::new("axis", "choice", "x, y"),
            Param::new("distance", FIXED, "px"),
            Param::new("speed", FIXED, PX_F).range("> 0"),
            Param::new("pause", "frames", "frames").optional(),
            Param::new("wait_near", FIXED, "px")
                .range(">= 0")
                .optional(),
            Param::new("wait_near_wrap", "count", "px").optional(),
            Param::new("every", "frames", "frames").optional(),
            Param::new("phase", "frames", "frames").optional(),
        ],
        state: &["phase", "timer"],
        reads: &["player.x"],
        writes: &["self.x", "self.y"],
        excludes: &[
            "platformer.walk",
            "platformer.patrol",
            "platformer.fly",
            "platformer.run",
        ],
        built_from: &[P::Integrator, P::Timer, P::Condition, P::Sensing, P::Mode],
        measures: &[("speed", PX_F), ("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.contact",
        version: PLATFORMER_VERSION,
        system: "platformer.contact",
        class: "touch",
        implementation: "gp_sim::systems::contact",
        about: "What the player's touch does: a stomp from above, otherwise kill, hurt, collect, kick or nothing",
        params: &[
            Param::new("stomp_depth", FIXED, "px")
                .range("> 0")
                .optional(),
            Param::new("bounce", FIXED, PX_F).range("< 0").optional(),
            Param::new("keep_on_stomp", "bool", "").optional(),
            Param::new(
                "otherwise",
                "choice",
                "kill-player, hurt-player, collect, kick, nothing, defeat",
            ),
            Param::new("damage", "count", "hit points").optional(),
            Param::new("stand", "bool", "").optional(),
            Param::new("every", "frames", "frames").range(">= 1").optional(),
            Param::new("phase", "frames", "frames").optional(),
            Param::new(
                "invincible",
                "choice",
                "kill-player, hurt-player, collect, kick, nothing, defeat",
            )
            .optional(),
            Param::new("touch_edges", "bool", "").optional(),
            Param::new("in_turn", "bool", "").optional(),
            Param::new("after_move", "bool", "").optional(),
            Param::new("sides", "table", "left, right and top px").optional(),
            Param::new("ride", "table", "copy_below, wrap and above px").optional(),
            Param::new("near", "table", "within and right px").optional(),
        ],
        reads: &[
            "player.hitbox",
            "player.touch_points",
            "player.vy",
            "self.hitbox",
        ],
        writes: &[
            "player.vy",
            "player.x",
            "player.y",
            "player.vx",
            "self.removed",
        ],
        emits: &[
            "enemy.stomped",
            "enemy.kicked",
            "player.hurt",
            "player.died",
            "item.collected",
            "enemy.hurt_player",
            "enemy.stood_on",
            "enemy.defeated",
        ],
        assumes: &["the player falls with a mechanic that sets vy (platformer.jump)"],
        built_from: &[P::BodyContact, P::Condition, P::Event, P::SpawnRemove],
        invariants: &[
            "a stomp can be missed when the player falls more than stomp_depth in a frame",
        ],
        measures: &[("bounce", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.climb",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::climb",
        about: "Climbing ladder tiles: up or down grabs a ladder at the hitbox's middle and moves at a fixed speed with no gravity; release lets go, and a ladder's top can stand",
        params: &[
            Param::new("speed", FIXED, PX_F).range("> 0"),
            Param::new("up", "action", ""),
            Param::new("down", "action", ""),
            Param::new("release", "action", "").optional(),
            Param::new("stand_on_top", "bool", "").optional(),
            Param::new("grab_down", FIXED, "px").optional(),
            Param::new("hold_still", "action", "").optional(),
            Param::new("up_cycle", "list", "px per count").optional(),
            Param::new("down_cycle", "list", "px per count").optional(),
            Param::new("by_feet", "bool", "").optional(),
            Param::new("down_holds", "bool", "").optional(),
            Param::new("down_through", "px", "px").optional(),
            Param::new("grab", "px", "the box that must meet a ladder's for a climb to start").optional(),
            Param::new("grab_snap", "px", "the x a climb by a ladder box starts at, from the box's left").optional(),
            Param::new("down_through_step", "bool", "the climb down through the floor only after a step up").optional(),
            Param::new("top_test", "bool", "a climb from standing asks the level's ladder tops and stops").optional(),
            Param::new("lone_way", "bool", "up or down climbs only held alone; left or right alone turns the climber").optional(),
        ],
        state: &["climbing"],
        reads: &["action.up", "action.down", "tiles.ladder", "tiles.solid"],
        writes: &["self.x", "self.y", "self.vx", "self.vy", "self.on_ground"],
        built_from: &[P::InputBinding, P::Mode, P::TileCollision, P::Condition],
        measures: &[("climb_speed", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.health",
        version: PLATFORMER_VERSION,
        system: "platformer.contact",
        class: "economy",
        implementation: "gp_sim::systems::hurt_player, gp_sim::systems::hits",
        about: "Hit points: the player's in a counter that a hurt takes damage from, with invulnerable frames after; any other entity's its own, taken by hits",
        params: &[
            Param::new("counter", "counter", "").optional(),
            Param::new("points", "count", "hit points").optional(),
            Param::new("invulnerable", "frames", "frames").optional(),
            Param::new("shield", "bool", "").optional(),
            Param::new("damage_from", "table", "hit points by entity").optional(),
            Param::new("defeat", "table", "").optional(),
        ],
        state: &["hp", "invuln"],
        writes: &["counters", "self.removed"],
        emits: &["enemy.hit", "enemy.defeated"],
        built_from: &[P::Quantity, P::Timer, P::Event, P::SpawnRemove],
        measures: &[("invulnerable", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.knockback",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::knockback",
        about: "A hurt that counts pushes the player back against its facing for some frames, with a rise on the first",
        params: &[
            Param::new("speed", FIXED, PX_F).range(">= 0"),
            Param::new("rise", FIXED, PX_F),
            Param::new("frames", "frames", "frames"),
        ],
        state: &["knock"],
        consumes: &["player.hurt"],
        writes: &["self.x", "self.vx", "self.vy"],
        built_from: &[P::Timer, P::Integrator, P::Mode],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.hits",
        version: PLATFORMER_VERSION,
        system: "platformer.contact",
        class: "projectile",
        implementation: "gp_sim::systems::hits",
        about: "Touching an entity of a target role with its own health takes damage from it; the hitter leaves unless it keeps",
        params: &[
            Param::new("damage", "count", "hit points").optional(),
            Param::new("roles", "list", "entity roles").optional(),
            Param::new("keep", "bool", "").optional(),
            Param::new("every", "frames", "frames").range(">= 1").optional(),
            Param::new("phase", "frames", "frames").optional(),
            Param::new("move_first", "bool", "").optional(),
        ],
        reads: &["self.hitbox", "other.hitbox", "other.health"],
        writes: &["other.hp", "self.removed", "other.removed"],
        emits: &["enemy.hit", "enemy.defeated"],
        built_from: &[P::BodyContact, P::Condition, P::Event, P::SpawnRemove],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.shoot",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "spawner",
        implementation: "gp_sim::systems::shoot",
        about: "Spawns an entity ahead of this one on an action or every so many frames, while fewer than a maximum are in the level",
        params: &[
            Param::new("entity", "entity", ""),
            Param::new("action", "action", "").optional(),
            Param::new("every", "frames", "frames").optional(),
            Param::new("ahead", "pair", "px").optional(),
            Param::new("max", "count", ""),
            Param::new("unmirrored", "bool", "").optional(),
            Param::new("vy", FIXED, PX_F).optional(),
            Param::new("vx", FIXED, PX_F).optional(),
            Param::new("vx_both", FIXED, PX_F).optional(),
            Param::new("after_move", "bool", "").optional(),
            Param::new("whole", "bool", "").optional(),
            Param::new("in_slots", "bool", "").optional(),
        ],
        state: &["shot", "shots", "shot_turn"],
        reads: &["action", "self.x", "self.y", "self.facing"],
        built_from: &[P::InputBinding, P::Timer, P::SpawnRemove, P::Condition],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.carry",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::carry",
        about: "Lifting a carryable entity the player stands on, holding still while lifting, carrying it at an offset, and throwing it on the action again",
        params: &[
            Param::new("action", "action", ""),
            Param::new("lift_frames", "frames", "frames").optional(),
            Param::new("offset", "pair", "px"),
            Param::new("throw", "pair", PX_F),
            Param::new("throw_moving", "pair", PX_F).optional(),
            Param::new("moving_above", FIXED, PX_F).optional(),
        ],
        state: &["carrying", "lift"],
        reads: &["action", "self.riding", "self.vx"],
        writes: &["carried.x", "carried.y", "carried.vx", "carried.vy"],
        emits: &["item.lifted", "item.thrown"],
        built_from: &[P::InputBinding, P::Mode, P::Timer, P::Sensing, P::Event],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.carryable",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "verb",
        implementation: "gp_sim::systems::carry",
        about: "An entity a carry can lift; while carried it runs none of its components",
        params: &[Param::new("offset", "pair", "px").optional()],
        state: &["carrier"],
        consumes: &["item.lifted", "item.thrown"],
        built_from: &[P::Mode, P::Sensing],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.oscillate",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::oscillate",
        about: "Swinging around a center on one axis: a force toward the center each frame up to a top speed, through tiles",
        params: &[
            Param::new("axis", "choice", "x, y"),
            Param::new("center", FIXED, "px"),
            Param::new("force", FIXED, PX_F2).range("> 0"),
            Param::new("max_speed", FIXED, PX_F).range("> 0"),
            Param::new("low_at", FIXED, "px").optional(),
            Param::new("low_center", FIXED, "px").optional(),
            Param::new("starts_at_rest", "bool", "").optional(),
            Param::new("move_first", "bool", "").optional(),
            Param::new("rest_creep_every", "frames", "frames").optional(),
        ],
        state: &["vx or vy"],
        writes: &["self.x", "self.y", "self.vx", "self.vy"],
        excludes: &[
            "platformer.walk",
            "platformer.patrol",
            "platformer.fly",
            "platformer.path",
            "platformer.run",
        ],
        built_from: &[P::Integrator, P::Condition],
        invariants: &["the speed along the axis never passes max_speed"],
        measures: &[("speed", PX_F), ("period", "frames")],
        envelope: &[("swing", "px")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.leap",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "hazard",
        implementation: "gp_sim::systems::leap",
        about: "Leaping again and again from a start point: a launch, gravity back, and a random wait or a depth to start again from",
        params: &[
            Param::new("speed", FIXED, PX_F),
            Param::new("gravity", FIXED, PX_F2).range("> 0"),
            Param::new("max_fall", FIXED, PX_F).optional(),
            Param::new("every", "frames", "frames")
                .range(">= 1")
                .optional(),
            Param::new("from", FIXED, "px").optional(),
            Param::new("rise", "pair", "px/frame, px").optional(),
            Param::new("interval", "frames", "frames")
                .range(">= 1")
                .optional(),
            Param::new("waits", "list", "intervals").optional(),
            Param::new("below", FIXED, "px").optional(),
            Param::new("move_first", "bool", "").optional(),
            Param::new("max_fall_from_half", "bool", "").optional(),
            Param::new("beat", "choice", "a counter with an every").optional(),
            Param::new("keep_fraction", "bool", "").optional(),
            Param::new("place_on_appear", "bool", "").optional(),
            Param::new(
                "draw",
                "table",
                "counter with a shift, at bit, slot_step bits, fraction",
            )
            .optional(),
        ],
        state: &["mechanic words 0 (wait) and 1 (phase)"],
        writes: &["self.y", "self.vy"],
        excludes: &["platformer.fall", "platformer.jump"],
        built_from: &[P::Integrator, P::Timer, P::RandomDraw, P::Mode],
        measures: &[("rise", "px"), ("period", "frames")],
        envelope: &[("rise", "px")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.launcher",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "spawner",
        implementation: "gp_sim::systems::launchers",
        about: "Firing another entity on its own timer and a shift counter's bits into a free numbered slot, aimed at the player's side, and erasing a shot near the player or off the view on its first turn",
        params: &[
            Param::new("fires", "entity", "an entity with fly"),
            Param::new("counter", "counter", "a counter with a shift"),
            Param::new("at", "int", "bit"),
            Param::new("slot_step", "int", "bits").optional(),
            Param::new("mask", "int", "bits"),
            Param::new("hard_mask", "int", "bits").optional(),
            Param::new("slots", "count", "slots").range(">= 1"),
            Param::new("table", "count", "launchers").range(">= 1"),
            Param::new("next", "counter", "a counter"),
            Param::new("rearm", "count", "picks"),
            Param::new("offset", "pair", "px").optional(),
            Param::new("from_x", "int", "px").optional(),
            Param::new("near", "int", "px"),
            Param::new("near_wrap", "int", "px").optional(),
            Param::new("behind", "int", "px"),
        ],
        state: &[
            "mechanic words 0 (timer) and 1 (entry)",
            "the shot's mechanic word 0 (aimed)",
            "the counter next",
        ],
        reads: &["player.x", "camera.x", "counters"],
        writes: &["bodies", "counters"],
        spawns: &[],
        built_from: &[P::Timer, P::RandomDraw, P::SpawnRemove, P::Condition],
        measures: &[("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.stream",
        version: PLATFORMER_VERSION,
        system: "core.spawn_by_view",
        class: "spawner",
        implementation: "gp_sim::systems::stream",
        about: "A level marker that sends entities in from past the view's edge, in turn, at heights a random byte picks",
        params: &[
            Param::new("entity", "choice", "an entity"),
            Param::new("past_edge", FIXED, "px"),
            Param::new("wait", "frames", "frames").optional(),
            Param::new(
                "draw",
                "table",
                "counter with a shift, at bit, slot_step bits",
            ),
            Param::new("until", "choice", "an entity").optional(),
        ],
        tables: &[
            crate::mechanic::Table {
                name: "heights",
                kind: TableKind::Content,
            },
            crate::mechanic::Table {
                name: "times",
                kind: TableKind::Content,
            },
        ],
        state: &["mechanic words 0 (wait) and 1 (turn)"],
        built_from: &[P::Timer, P::RandomDraw, P::SpawnRemove],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.shuttle",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::shuttle",
        about: "Moving back and forth by a counter: a step every few frames, turning after a count of moves, with an optional sway across",
        params: &[
            Param::new("axis", "choice", "x, y"),
            Param::new("step", FIXED, "px"),
            Param::new("every", "frames", "frames")
                .range(">= 1")
                .optional(),
            Param::new("turn_after", "count", "moves"),
            Param::new("sway", "pair", "px, frames").optional(),
        ],
        state: &["mechanic words 0 (count) and 1 (way)"],
        reads: &["frame"],
        writes: &["self.x", "self.y", "self.facing"],
        excludes: &[
            "platformer.walk",
            "platformer.patrol",
            "platformer.fly",
            "platformer.path",
            "platformer.oscillate",
            "platformer.run",
        ],
        built_from: &[P::Timer, P::Quantity, P::Mode, P::Integrator],
        measures: &[("speed", PX_F), ("period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.springboard",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "touch",
        implementation: "gp_sim::springboard::step",
        about: "A body that holds a player standing on it for the frames of its steps, moving both a set amount each frame, then throws the player up, higher when an action is pressed during the press",
        params: &[
            Param::new("steps", "list", "frames, move px/frame, offset px"),
            Param::new("speed", FIXED, PX_F).range("< 0"),
            Param::new(
                "boost",
                "table",
                "speed px/frame, action, from_step steps, held",
            )
            .optional(),
            Param::new("throw_late", "bool", "").optional(),
        ],
        state: &["mechanic words 0 (frames pressed), 1 (the player held, plus one) and 2 (boost)"],
        reads: &[
            "player.x",
            "player.y",
            "player.on_ground",
            "player.riding",
            "action",
        ],
        writes: &[
            "self.y",
            "self.on_ground",
            "player.y",
            "player.vy",
            "player.on_ground",
        ],
        assumes: &[
            "the player stands on it through solid tiles at its top or through contact.stand",
            "while it holds a player, platformer.player doesn't move that player by its speed, jump or fall",
        ],
        excludes: &[
            "platformer.fall",
            "platformer.walk",
            "platformer.patrol",
            "platformer.fly",
            "platformer.path",
            "platformer.oscillate",
            "platformer.leap",
            "platformer.shuttle",
        ],
        built_from: &[
            P::Condition,
            P::Timer,
            P::InputBinding,
            P::Mode,
            P::Sensing,
            P::Quantity,
        ],
        invariants: &[
            "the player is thrown once a press's frames have run",
            "a thrown player isn't pressed again until it lands",
        ],
        measures: &[("hold", "frames"), ("throw", PX_F)],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.orbit",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "hazard",
        implementation: "gp_sim::systems::orbit",
        about: "A pivot that turns an angle by a spin each frame and carries parts placed around it",
        params: &[
            Param::new("spin", FIXED, "angle steps per frame"),
            Param::new("angles", "count", "steps per turn").optional(),
            Param::new("parts", "count", ""),
            Param::new("part", "entity", ""),
            Param::new("place", "pair", "px").optional(),
            Param::new("skip", "list", "angles").optional(),
            Param::new("in_view", "bool", "").optional(),
            Param::new("place_on_appear", "bool", "").optional(),
            Param::new("angle_in_slot", "bool", "").optional(),
        ],
        state: &["mechanic words 0 (angle) and 1 (placed)"],
        reads: &["camera"],
        writes: &["self.x", "self.y", "self.ext"],
        spawns: &["its part"],
        built_from: &[P::Quantity, P::Integrator, P::SpawnRemove, P::Condition],
        measures: &[("spin", "angle steps per frame")],
        envelope: &[("radius", "px"), ("sweep_period", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.orbit_part",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "hazard",
        implementation: "gp_sim::systems::orbit_part",
        about: "A part of an orbit, placed each frame from its pivot's angle by a quarter-turn table of offsets",
        params: &[Param::new("within", FIXED, "px")],
        tables: &[
            crate::mechanic::Table {
                name: "offsets",
                kind: crate::mechanic::TableKind::Mechanic,
            },
            crate::mechanic::Table {
                name: "rows",
                kind: crate::mechanic::TableKind::Mechanic,
            },
            crate::mechanic::Table {
                name: "signs",
                kind: crate::mechanic::TableKind::Mechanic,
            },
        ],
        state: &["mechanic words 0 (pivot) and 1 (which part)"],
        reads: &["pivot.angle", "pivot.x", "pivot.y"],
        writes: &["self.x", "self.y", "self.removed"],
        assumes: &["platformer.orbit on its pivot"],
        built_from: &[P::Sensing, P::Quantity],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.leave_view",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "spawner",
        implementation: "gp_sim::systems::leave_view",
        about: "Leaving the level once its position is within a margin of the view's edge or past it",
        params: &[
            Param::new("margin", FIXED, "px").range(">= 0"),
            Param::new("far_margin", FIXED, "px").range(">= 0").optional(),
            Param::new("bottom_margin", FIXED, "px").range(">= 0").optional(),
            Param::new("axis", "choice", "x, y").optional(),
        ],
        reads: &["camera"],
        writes: &["self.removed"],
        built_from: &[P::Condition, P::SpawnRemove, P::Sensing],
        measures: &[("lifetime", "frames")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.touch_points",
        version: PLATFORMER_VERSION,
        system: "platformer.contact",
        class: "touch",
        implementation: "gp_sim::systems::contact",
        about: "A player's points, in pixels from its position, that an entity's contact.near tests in place of the hitboxes",
        reads: &["self.x", "self.y"],
        built_from: &[P::BodyContact],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.locks",
        version: PLATFORMER_VERSION,
        system: "platformer.player",
        class: "input",
        implementation: "gp_sim::systems::climb, gp_sim::systems::fall",
        about: "A player's actions that do nothing while a counter holds (feature power.locks): Donkey Kong's hammer takes jumping and climbing away while its timer runs",
        params: &[
            Param::new("while", "condition", "counter"),
            Param::new("actions", "list", "action names"),
        ],
        reads: &["counter", "input"],
        built_from: &[P::Condition],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.invincible",
        version: PLATFORMER_VERSION,
        system: "platformer.contact",
        class: "touch",
        implementation: "gp_sim::systems::contact",
        about: "A player invincible while a counter holds: its touches do each entity's contact.invincible, by default defeating what would hurt it",
        params: &[Param::new("while", "condition", "counter")],
        reads: &["counter"],
        writes: &["other.removed"],
        emits: &["enemy.defeated"],
        built_from: &[P::BodyContact, P::Condition, P::Event, P::SpawnRemove],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.bounce",
        version: PLATFORMER_VERSION,
        system: "platformer.enemies",
        class: "mover",
        implementation: "gp_sim::systems::bounce",
        about: "Bouncing up off a tile at one point while falling, and leaving on one met while rising or on the frame after a bounce",
        params: &[
            Param::new("point", "pair", "px"),
            Param::new("speed", FIXED, PX_F).range("< 0"),
            Param::new("snap", "count", "px").optional(),
            Param::new("from_y", FIXED, "px").optional(),
        ],
        state: &["bounced"],
        reads: &["tiles"],
        writes: &["self.vy", "self.y", "self.removed"],
        built_from: &[P::Sensing, P::Condition, P::SpawnRemove],
        ..Record::EMPTY
    },
    Record {
        name: "core.tile_collision",
        version: CORE_VERSION,
        system: "platformer.player",
        class: "collision",
        implementation: "gp_sim::systems::run, gp_sim::systems::fall",
        about: "A body's tile box pushed out of solid tiles: landing, bumping its head, stopping at walls",
        params: &[
            Param::new("tile_collision", "bool", ""),
            Param::new("tile_hitbox", "rect", "px").optional(),
            Param::new("size", "rect", "px"),
        ],
        reads: &["tiles.solid"],
        writes: &["self.x", "self.y", "self.on_ground"],
        built_from: &[P::TileCollision],
        ..Record::EMPTY
    },
    Record {
        name: "core.modes",
        version: CORE_VERSION,
        system: "entity_states",
        class: "flow",
        implementation: "gp_sim::systems::entity_states",
        about: "An entity's state machine: each state sets some of its parameters, entered on events or after a number of frames",
        params: &[
            Param::new("initial", "choice", "a state"),
            Param::new("after", "frames", "frames").optional(),
            Param::new("set", "list", "parameter values").optional(),
            Param::new("freeze", "bool", "").optional(),
            Param::new("stops", "list", "systems").optional(),
            Param::new("stops_at_once", "list", "systems").optional(),
            Param::new("holds", "list", "counters").optional(),
            Param::new("keep_time", "bool", "").optional(),
            Param::new("align", "frames", "frames").optional(),
            Param::new("seen_every", "frames", "frames").optional(),
            Param::new("ground", "choice", "leave, land").optional(),
            Param::new("enter_speed", "list", "px/frame").optional(),
            Param::new("enter_keeps_fraction", "bool", "").optional(),
        ],
        state: &["state", "frames_in_state"],
        consumes: &["*"],
        writes: &["self.kind"],
        built_from: &[P::Mode, P::Timer, P::Event, P::Condition],
        invariants: &["a state's set changes parameters only"],
        measures: &[("state", "frames per state")],
        ..Record::EMPTY
    },
    Record {
        name: "core.flow",
        version: CORE_VERSION,
        carrier: Carrier::Game,
        system: "rules",
        class: "flow",
        implementation: "gp_sim::systems::rules",
        about: "The game's rules: dying, restarting, ending a level, game over, as states with timers, freezes and actions",
        params: &[
            Param::new("after", "frames", "frames").optional(),
            Param::new("freeze", "bool", "").optional(),
            Param::new("stops", "list", "systems").optional(),
            Param::new("stops_at_once", "list", "systems").optional(),
            Param::new("holds", "list", "counters").optional(),
            Param::new("keep_time", "bool", "").optional(),
            Param::new("align", "frames", "frames").optional(),
        ],
        state: &["state", "frames_in_state"],
        consumes: &["player.died", "goal.reached"],
        writes: &["level", "world"],
        built_from: &[P::Mode, P::Timer, P::Event, P::Condition],
        measures: &[("state", "frames per state")],
        ..Record::EMPTY
    },
    Record {
        name: "core.counters",
        version: CORE_VERSION,
        carrier: Carrier::Game,
        system: "counters",
        class: "economy",
        implementation: "gp_sim::systems::counters",
        about: "Numbers the game keeps (lives, coins, score, a timer), changed by events or every N frames, within bounds",
        params: &[
            Param::new("start", "count", ""),
            Param::new("by", "count", "").optional(),
            Param::new("every", "frames", "frames").optional(),
            Param::new("min", "count", "").optional(),
            Param::new("max", "count", "").optional(),
            Param::new("wrap", "bool", "").optional(),
            Param::new("runs_when_frozen", "bool", "").optional(),
            Param::new("of", "list", "entities").optional(),
            Param::new("after", "frames", "frames").optional(),
        ],
        state: &["value"],
        consumes: &["*"],
        writes: &["counters"],
        built_from: &[P::Quantity, P::Timer, P::Event],
        measures: &[("value", "count")],
        ..Record::EMPTY
    },
    Record {
        name: "core.camera",
        version: CORE_VERSION,
        carrier: Carrier::Game,
        system: "camera",
        class: "camera",
        implementation: "gp_sim::systems::camera",
        about: "A view following an entity with a lead, clamped to the level, optionally never scrolling back, or moving by the entity's move (by_move)",
        params: &[
            Param::new("follow", "choice", "an entity"),
            Param::new("axis", "choice", "x, y, both"),
            Param::new("size", "rect", "px"),
            Param::new("lead", FIXED, "px"),
            Param::new("clamp_to_level", "bool", "").optional(),
            Param::new("scroll_back", "bool", "").optional(),
            Param::new("by_move", "table", "from px, full px, short_by px, push_hold frames, keep_move, hold_with_counters").optional(),
            Param::new("keep_in_view", "rect", "px").optional(),
            Param::new("keep_in_view_touch", "bool", "").optional(),
            Param::new("keep_in_view_turn", "bool", "").optional(),
            Param::new("keep_in_view_rows", "table", "rows px, bounds px").optional(),
        ],
        state: &["x", "y", "seen_x", "seen_y", "move_x", "move_y"],
        reads: &["followed.x", "followed.y"],
        writes: &["camera"],
        built_from: &[P::Sensing, P::Quantity],
        invariants: &["spawn by view counts from the camera, so the camera is simulation"],
        measures: &[("lead", "px")],
        ..Record::EMPTY
    },
    Record {
        name: "core.spawn_by_view",
        version: CORE_VERSION,
        carrier: Carrier::Level,
        system: "spawn",
        class: "spawner",
        implementation: "gp_sim::systems::spawn",
        about: "A level's entities appear when the view comes within spawn_ahead of them and leave behind it by despawn_behind",
        params: &[
            Param::new("spawn_ahead", FIXED, "px").optional(),
            Param::new("despawn_behind", FIXED, "px").optional(),
            Param::new("spawn_slots", "count", "").optional(),
            Param::new("despawn_at", "choice", "").optional(),
            Param::new("despawn_after_move", "bool", "").optional(),
            Param::new("spawn_in_order", "bool", "").optional(),
            Param::new("appear_still", "bool", "").optional(),
            Param::new("slot_rest", "bool", "").optional(),
            Param::new("draw", "table", "").optional(),
            Param::new("scroll_stop", "table", "").optional(),
            Param::new("loop_back", "table", "").optional(),
        ],
        reads: &["camera"],
        writes: &["bodies"],
        assumes: &["core.camera"],
        built_from: &[P::Condition, P::SpawnRemove, P::Sensing],
        ..Record::EMPTY
    },
    Record {
        name: "core.exits",
        version: CORE_VERSION,
        carrier: Carrier::Level,
        system: "exits",
        class: "flow",
        implementation: "gp_sim::systems::exits",
        about: "Leaving a level by a box, a held action or standing, to another level's entry",
        params: &[
            Param::new("take", "rect", "px").optional(),
            Param::new("hold", "action", "").optional(),
            Param::new("standing", "bool", "").optional(),
        ],
        reads: &["player.x", "player.y", "action"],
        writes: &["level"],
        built_from: &[P::Condition, P::InputBinding, P::SpawnRemove],
        measures: &[("graph", "levels")],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.pit",
        version: PLATFORMER_VERSION,
        carrier: Carrier::Level,
        system: "platformer.player",
        class: "hazard",
        implementation: "gp_sim::systems::bounds",
        about: "A player whose top edge reaches kill_below dies, or fires the level's pit_event once and falls on",
        params: &[
            Param::new("kill_below", FIXED, "px"),
            Param::new("pit_event", "choice", "a model event").optional(),
        ],
        reads: &["player.y"],
        emits: &["player.died", "pit_event"],
        built_from: &[P::Condition, P::Event],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.goal",
        version: PLATFORMER_VERSION,
        carrier: Carrier::Level,
        system: "platformer.player",
        class: "flow",
        implementation: "gp_sim::systems::bounds",
        about: "A player whose left edge reaches the goal's x reaches the goal, unless its top edge is at or below under",
        params: &[
            Param::new("x", FIXED, "px"),
            Param::new("under", FIXED, "px").optional(),
        ],
        reads: &["player.x", "player.y"],
        emits: &["goal.reached"],
        built_from: &[P::Condition, P::Event],
        measures: &[("reachable", "bool")],
        envelope: &[(
            "reachable",
            "from the player's spawn, by the player's moves",
        )],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.hazard",
        version: PLATFORMER_VERSION,
        carrier: Carrier::Level,
        system: "platformer.player",
        class: "hazard",
        implementation: "gp_sim::systems::hazard",
        about: "A player whose hitbox touches a hazard tile is killed or hurt",
        params: &[
            Param::new("touch", "choice", "kill-player, hurt-player"),
            Param::new("damage", "count", "hit points").optional(),
        ],
        reads: &["player.hitbox", "tiles.hazard"],
        emits: &["player.died", "player.hurt"],
        built_from: &[P::TileCollision, P::Condition, P::Event],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.water",
        version: PLATFORMER_VERSION,
        carrier: Carrier::Level,
        system: "platformer.player",
        class: "mover",
        implementation: "gp_sim::systems::fall, gp_sim::systems::enemy",
        about: "Gravity and the fall limit while an entity's middle is in a water tile",
        params: &[
            Param::new("gravity", FIXED, PX_F2),
            Param::new("max_fall", FIXED, PX_F).range("> 0"),
        ],
        reads: &["tiles.water"],
        writes: &["self.vy"],
        built_from: &[P::Integrator, P::Condition],
        ..Record::EMPTY
    },
    Record {
        name: "platformer.ice",
        version: PLATFORMER_VERSION,
        carrier: Carrier::Level,
        system: "platformer.player",
        class: "mover",
        implementation: "gp_sim::systems::run",
        about: "Acceleration and deceleration while an entity stands on an ice tile",
        params: &[
            Param::new("acceleration", FIXED, PX_F2).range("> 0"),
            Param::new("deceleration", FIXED, PX_F2).range("> 0"),
        ],
        reads: &["tiles.ice"],
        writes: &["self.vx"],
        built_from: &[P::Integrator, P::Condition],
        ..Record::EMPTY
    },
];

/// The component record named `name`.
pub fn component(name: &str) -> Option<&'static Record> {
    COMPONENTS.iter().find(|r| r.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_parts_are_known() {
        let mut names: Vec<&str> = COMPONENTS.iter().map(|r| r.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "a record name repeats");
        for r in COMPONENTS {
            assert!(!r.built_from.is_empty(), "{} names no primitives", r.name);
            assert!(!r.system.is_empty(), "{}", r.name);
            for o in r.after.iter().chain(r.excludes) {
                assert!(component(o).is_some(), "{} names unknown {o}", r.name);
            }
            let mut ps: Vec<&str> = r.params.iter().map(|p| p.name).collect();
            ps.sort_unstable();
            ps.dedup();
            assert_eq!(ps.len(), r.params.len(), "{} repeats a parameter", r.name);
        }
        assert_eq!(Primitive::ALL.len(), 12);
    }
}
