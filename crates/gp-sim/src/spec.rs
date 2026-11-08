//! A game as the simulation runs it: every number and table it reads,
//! already resolved, so stepping never parses anything or looks anything up
//! by name. The interpreter compiles a model and its lifted assets into a
//! [`Spec`] (`gp_interp::compile`); a generated build writes the same
//! values out as code. Two runtimes with equal specs step identically.
//!
//! Indexes into the spec's lists stand for names: an entity's `kind` is an
//! index into [`Spec::kinds`], an action an index into [`Spec::actions`],
//! and so on. Names are kept for queries and reports, never for logic.

use crate::fx::Fx;
pub use crate::reads::{Element, List, Parts};

/// The systems a frame runs, in the order [`Spec::order`] gives, which is
/// the model's `model.systems`. Each is a function in [`crate::systems`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum System {
    /// The game's state machines count time and take `after` transitions.
    Rules,
    /// Each entity's own machine does the same.
    EntityStates,
    /// Counters with an `every` change count frames and change.
    Counters,
    /// Entities near the view appear and those far behind it go.
    Spawn,
    /// Cameras follow their entity.
    Camera,
    /// The player runs and jumps against tiles, then falls into a pit or
    /// reaches the goal.
    Player,
    /// Other entities patrol.
    Enemies,
    /// The player's touches with other entities: stomps and the rest.
    Contact,
    /// The player takes an exit whose `take` it meets.
    Exits,
    /// Top-down 0.1: each player with `move8` moves across, then down,
    /// against solid tiles.
    TopdownPlayer,
}

impl System {
    /// The core's systems, which every model orders.
    pub const CORE: [System; 6] = [
        System::Rules,
        System::EntityStates,
        System::Counters,
        System::Spawn,
        System::Camera,
        System::Exits,
    ];
    /// The platformer extension's systems.
    pub const PLATFORMER: [System; 3] = [System::Player, System::Enemies, System::Contact];
    /// The top-down extension's systems.
    pub const TOPDOWN: [System; 1] = [System::TopdownPlayer];

    pub const ALL: [System; 10] = [
        System::Rules,
        System::EntityStates,
        System::Counters,
        System::Spawn,
        System::Camera,
        System::Player,
        System::Enemies,
        System::Contact,
        System::Exits,
        System::TopdownPlayer,
    ];

    /// The name `model.systems` uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::EntityStates => "entity_states",
            Self::Counters => "counters",
            Self::Spawn => "spawn",
            Self::Camera => "camera",
            Self::Player => "platformer.player",
            Self::Enemies => "platformer.enemies",
            Self::Contact => "platformer.contact",
            Self::Exits => "exits",
            Self::TopdownPlayer => "topdown.player",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Whether a frozen world stops it. State machines keep counting.
    pub fn moves(self) -> bool {
        !matches!(self, Self::Rules | Self::EntityStates)
    }
}

/// An event, by index into [`Spec::events`]. The first ones are the
/// events every spec has, at these indexes; counters' own events follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventKind(pub usize);

impl EventKind {
    pub const LEVEL_STARTED: EventKind = EventKind(0);
    pub const PLAYER_DIED: EventKind = EventKind(1);
    pub const PLAYER_HURT: EventKind = EventKind(2);
    pub const ENEMY_STOMPED: EventKind = EventKind(3);
    pub const ENEMY_KICKED: EventKind = EventKind(4);
    pub const ITEM_COLLECTED: EventKind = EventKind(5);
    pub const GOAL_REACHED: EventKind = EventKind(6);
    pub const ENEMY_HURT_PLAYER: EventKind = EventKind(7);
    pub const ENEMY_HIT: EventKind = EventKind(8);
    pub const ENEMY_DEFEATED: EventKind = EventKind(9);
    pub const ENEMY_STOOD_ON: EventKind = EventKind(10);
    pub const ITEM_LIFTED: EventKind = EventKind(11);
    pub const ITEM_THROWN: EventKind = EventKind(12);
}

/// The names of the events at the start of every [`Spec::events`], in
/// [`EventKind`]'s order.
pub const BUILTIN_EVENTS: [&str; 13] = [
    "level.started",
    "player.died",
    "player.hurt",
    "enemy.stomped",
    "enemy.kicked",
    "item.collected",
    "goal.reached",
    "enemy.hurt_player",
    "enemy.hit",
    "enemy.defeated",
    "enemy.stood_on",
    "item.lifted",
    "item.thrown",
];

/// What a transition does on the way to its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Do {
    /// Every entity back to its spawn and every camera to its start;
    /// counters are kept but for the level's own values.
    RestartLevel,
    /// The level's `next` starts.
    NextLevel,
    /// `next-level` with every counter and its clocks as they were, and
    /// each player keeping its mechanic words (`enter-level`, feature
    /// `action.enter_level`): the level and its bodies, for a game that
    /// draws the next stage and its objects before it sets the stage's
    /// state up, which a later `next-level` does.
    EnterLevel,
    /// Every counter back to its start, and the first level starts.
    ResetGame,
    /// `player.died` fires.
    KillPlayer,
    /// An entity's machine only: that instance leaves the level.
    Remove,
    /// An entity's machine only: that instance is replaced by one of this
    /// kind (`become:<entity>`).
    Become(usize),
    /// A counter, by index, changes by this much (`change:<counter>:<by>`).
    Change(usize, i64),
    /// The event fires (`emit:<event>`, feature `action.emit`).
    Emit(EventKind),
    /// The first counter changes by the second's value, as a change would
    /// (`add:<to>:<from>`, feature `action.add`).
    Add(usize, usize),
    /// The counter is put at this value, within its bounds, firing no event
    /// (`set:<counter>:<value>`, feature `action.set`).
    Set(usize, i64),
    /// Every active body of this entity type, by index, leaves the level at
    /// once, with no event (`clear:<entity>`, feature `action.clear`).
    Clear(usize),
    /// Every active body of this entity type, by index, that a spawn of the
    /// current level placed goes back to that spawn's place at rest, its
    /// machine's state kept (`respawn:<entity>`, feature `action.respawn`).
    Respawn(usize),
    /// The body whose ID plus one the counter (by index) holds leaves the
    /// level, if there is one, and the counter goes to 0
    /// (`remove-body:<counter>`, feature `action.remove_body`).
    RemoveBody(usize),
}

impl Do {
    pub const ALL: [Do; 6] = [
        Do::RestartLevel,
        Do::NextLevel,
        Do::EnterLevel,
        Do::ResetGame,
        Do::KillPlayer,
        Do::Remove,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::RestartLevel => "restart-level",
            Self::NextLevel => "next-level",
            Self::EnterLevel => "enter-level",
            Self::ResetGame => "reset-game",
            Self::KillPlayer => "kill-player",
            Self::Remove => "remove",
            Self::Become(_) => "become",
            Self::Change(..) => "change",
            Self::Emit(_) => "emit",
            Self::Add(..) => "add",
            Self::Set(..) => "set",
            Self::Clear(_) => "clear",
            Self::Respawn(_) => "respawn",
            Self::RemoveBody(_) => "remove-body",
        }
    }

    /// The action of this name; `become:<entity>` is read by the compiler,
    /// which knows the kinds.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The model's name.
    pub name: String,
    /// Input actions, each the controller bits that trigger it.
    pub actions: Vec<Action>,
    pub kinds: Parts<Kind>,
    pub levels: Parts<Level>,
    /// The level the game starts in.
    pub start_level: usize,
    pub cameras: Parts<Camera>,
    pub machines: Parts<Machine>,
    pub counters: Parts<Counter>,
    /// Event names by [`EventKind`]: [`BUILTIN_EVENTS`], then each
    /// counter's `<name>.reached_min` and `<name>.wrapped`, then the
    /// model's own events (`model.events`) from [`Spec::own_events`].
    pub events: Vec<String>,
    /// The index in [`Spec::events`] the model's own events start at. One
    /// of them fired with an entity concerns only that instance.
    pub own_events: usize,
    /// The systems each frame runs, in order.
    pub order: Vec<System>,
    /// The random number generator's seed.
    pub seed: u64,
    /// The model's profile (`model.profile`, core 0.4.5), such as
    /// "normal"; none for the base profile, "retro". The state hash
    /// includes it ([`crate::World::hash`]).
    pub profile: Option<String>,
    /// Core `input.keep_direction`: the controller's direction bits (the
    /// upper four in the runtime protocol's order) are latched each frame:
    /// those held the frame before and still held stay, alone.
    pub keep_direction: bool,
    /// Core `input.turn`: the counter that says whose turn it is and the
    /// controller bits of the actions that read that player's controller
    /// ([`crate::systems::begin_frame_pads`]).
    pub turn: Option<(usize, u8)>,
    /// Core `stages`: the counter that says which of the original's stages
    /// is being played, and the level that plays each, in order (a
    /// stand-in for a stage the model has no level for, `None` for a
    /// stage the game passes over: the counter steps on past it).
    /// `next-level` and `restart-level` start the counter's stage's level.
    pub stages: Option<(usize, Vec<Option<usize>>)>,
    /// Core `cost.frame`: the original's CPU cycles for a frame's work
    /// ([`Cost`]).
    pub cost: Option<Cost>,
}

/// Core `cost.frame`: `base` plus each term whose tests hold, summed at
/// the end of each frame before the `hook_last` hooks
/// ([`crate::systems::frame_cost`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cost {
    pub base: i64,
    pub terms: Vec<CostTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CostTerm {
    pub cycles: i64,
    pub tests: Vec<CostTest>,
    /// Counted once for each active body of this kind that passes
    /// `per_tests`.
    pub per: Option<usize>,
    pub per_tests: Vec<CostTest>,
}

/// What a [`CostTest`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostOn {
    /// A counter by index, as the frame started (true) or as it ends.
    Counter(usize, bool),
    /// The controller bits of an action, held this frame (true) or the
    /// frame before: 1 or 0.
    Held(u8, bool),
    /// The controller bits of an action on controller 1 as pressed this
    /// frame (core `cost.frame_pad`): 1 or 0.
    Pad(u8),
    /// A game machine in a state as the frame started: 1 or 0.
    State(usize, usize),
    /// A body's `ext` word.
    Word(usize),
}

/// A value in `lo..hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostTest {
    pub on: CostOn,
    pub lo: i64,
    pub hi: i64,
}

macro_rules! element {
    ($t:ty, $p:ident) => {
        impl Element for $t {
            const PART: List = List::$p;
            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}
element!(Kind, Kind);
element!(Level, Level);
element!(Machine, Machine);
element!(Counter, Counter);
element!(Camera, Camera);

impl Parts<Kind> {
    /// A kind's role, without noting a read of the kind: a scorer reads
    /// every kind's role whole (it is in its fingerprint's spec part).
    pub fn role_of(&self, k: usize) -> Option<Role> {
        self.unnoted().get(k).map(|x| x.role)
    }
}

/// What the simulation reads across every kind on each frame, worked out
/// without noting a read of each ([`crate::reads`]): a scorer counts it
/// among what every clip reads (`Spec::across` is in its fingerprint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Across {
    /// Some kind, or a state's kind, walks with `walk.bump`.
    pub bumping: bool,
    /// Each launcher: its kind, the counter it fills its entries from,
    /// the kind it fires and what that kind becomes when defeated.
    pub launchers: Vec<(usize, usize, usize, Option<usize>)>,
}

impl Spec {
    /// [`Across`] for this spec.
    pub fn across(&self) -> Across {
        let kinds = self.kinds.unnoted();
        let bump = |k: &Kind| k.walk.as_ref().is_some_and(|w| w.bump.is_some());
        Across {
            bumping: kinds.iter().any(bump)
                || kinds
                    .iter()
                    .filter_map(|k| k.machine.as_ref())
                    .any(|m| m.states.iter().any(|s| bump(&s.kind))),
            launchers: kinds
                .iter()
                .enumerate()
                .filter_map(|(i, k)| k.launcher.map(|l| (i, l)))
                .map(|(i, l)| {
                    let becomes = kinds
                        .get(l.fires)
                        .and_then(|f| f.health.as_ref())
                        .and_then(|h| h.defeat)
                        .map(|d| d.becomes);
                    (i, l.next, l.fires, becomes)
                })
                .collect(),
        }
    }

    /// The entity type a body is, as its machine's current state makes it.
    pub fn kind(&self, b: &crate::world::Body) -> &Kind {
        let k = &self.kinds[b.kind];
        match &k.machine {
            Some(m) => &m.states[b.state].kind,
            None => k,
        }
    }

    /// A counter's `reached_min` and `wrapped` events.
    pub fn counter_events(counter: usize) -> (EventKind, EventKind) {
        let base = BUILTIN_EVENTS.len() + 2 * counter;
        (EventKind(base), EventKind(base + 1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    /// Controller bits, NES order from bit 0: A, B, Select, Start, Up, Down,
    /// Left, Right (the runtime protocol's `input-frame`).
    pub mask: u8,
}

/// An entity's role, as far as the simulation cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Player,
    Other,
}

/// A box from an entity's position, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: Fx,
    pub y: Fx,
    pub w: Fx,
    pub h: Fx,
}

/// An entity type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kind {
    pub name: String,
    pub role: Role,
    pub size: [Fx; 2],
    /// The box that touches other entities.
    pub hitbox: Rect,
    /// The box that touches tiles.
    pub tile_hitbox: Rect,
    pub tile_collision: bool,
    pub run: Option<Run>,
    pub jump: Option<Jump>,
    pub patrol: Option<Patrol>,
    pub contact: Option<Contact>,
    pub walk: Option<Walk>,
    pub fall: Option<Fall>,
    pub fly: Option<Fly>,
    /// Hopping: jumping again when it lands.
    pub hop: Option<Hop>,
    pub path: Option<Path>,
    pub oscillate: Option<Oscillate>,
    pub leave_view: Option<LeaveView>,
    pub leap: Option<Leap>,
    /// Firing another entity on its own timer (platformer 0.4.28).
    pub launcher: Option<Launcher>,
    /// A launcher fires it: it takes its turns with the launchers, not with
    /// the other enemies, and keeps "aimed" in mechanic word 0.
    pub launched: bool,
    pub shuttle: Option<Shuttle>,
    pub orbit: Option<Orbit>,
    /// An orbit's part: how near its pivot must be placed.
    pub orbit_part: Option<Fx>,
    /// A springboard: it holds a player that stands on it, then throws it.
    pub springboard: Option<Springboard>,
    /// A level marker that sends entities in past the view's edge.
    pub stream: Option<Stream>,
    pub climb: Option<Climb>,
    pub health: Option<Health>,
    pub knockback: Option<Knockback>,
    pub hits: Option<Hits>,
    pub shoot: Option<Shoot>,
    pub carry: Option<Carry>,
    /// It can be lifted, and rides at this offset from its carrier (or the
    /// carry's own when `None`).
    pub carryable: Option<Option<[Fx; 2]>>,
    /// Top-down 0.1: moving in eight directions with no gravity.
    pub move8: Option<Move8>,
    /// The entity's own state machine. Each state holds the entity as it
    /// is in that state (`set` merged in), with no machine of its own.
    pub machine: Option<EntityMachine>,
    /// The hand-written behavior behind `ext`, by name. The simulation runs
    /// no `ext` code: a runtime that has it calls it, and the interpreter
    /// runs only the components.
    pub ext: Option<String>,
    /// The numbers the `ext` hook reads, by name, from the model's
    /// `ext_params` ([`crate::ext::ExtCx::param`]).
    pub ext_params: std::collections::BTreeMap<String, Fx>,
    /// The tables the hook reads, by name, from the entity's `ext_tables`
    /// ([`crate::ext::ExtCx::table`]).
    pub ext_tables: std::collections::BTreeMap<String, Vec<i64>>,
    /// It takes none of a level's `spawn_slots`.
    pub no_slot: bool,
    /// Core `entity.hook_through_stops`: its hook runs on a frame a state's
    /// `stops` lists its system ([`crate::systems::run_stopped_ext`]).
    pub hook_through_stops: bool,
    /// Core `entity.hook_frame_start`: its hook's [`crate::Ext::frame_start`]
    /// runs at the start of every frame ([`crate::systems::begin_frame_ext`]).
    pub hook_frame_start: bool,
    /// Core `entity.hook_first`: its hook runs at the start of the
    /// `platformer.player` system, before the player moves, and not in
    /// `platformer.enemies`.
    pub hook_first: bool,
    /// Core `entity.hook_last`: its hook runs once at the end of every
    /// frame, after every system, frozen, stopped or a level start alike
    /// ([`crate::systems::end_frame_ext`]), and not in
    /// `platformer.enemies`.
    pub hook_last: bool,
    /// How many more of a level's `spawn_slots` it takes besides its own.
    pub extra_slots: u32,
    /// Its own `despawn_behind`, in place of the level's.
    pub despawn_behind: Option<Fx>,
    /// `[from, behind]`: `behind` stands in for `despawn_behind` while the
    /// view's left edge is `from` or more into its 256-pixel page.
    pub despawn_behind_wrap: Option<[u32; 2]>,
    /// Its own `despawn_ahead`, in place of the level's.
    pub despawn_ahead: Option<Fx>,
    /// A player that is invincible while this holds (platformer 0.4.10).
    pub invincible: Option<Guard>,
    /// Actions (by index) that count as not held while the guard holds
    /// (feature `power.locks`).
    pub locks: Option<(Guard, Vec<usize>)>,
    /// Bouncing off the ground at a point, and leaving (platformer 0.4.10).
    pub bounce: Option<Bounce>,
    /// A player's points that a `contact.near` tests, in pixels from its
    /// position.
    pub touch_points: Vec<[i64; 2]>,
}

/// Bouncing off the ground at a point (platformer 0.4.10): see
/// `gp_model::platformer::Bounce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounce {
    pub point: [Fx; 2],
    pub speed: Fx,
    /// The whole height is rounded down to a multiple of this; 0 or 1 for
    /// none.
    pub snap: u32,
    /// Nothing is looked at above this y.
    pub from_y: Option<Fx>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMachine {
    pub initial: usize,
    pub states: Vec<EntityState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityState {
    pub name: String,
    pub freeze: bool,
    pub part: Part,
    pub kind: Kind,
    pub transitions: Vec<Transition>,
}

/// Climbing ladder tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Climb {
    pub speed: Fx,
    pub up: usize,
    pub down: usize,
    pub release: Option<usize>,
    /// A ladder's top cell stands like a `top` tile while not climbing.
    pub stand_on_top: bool,
    /// Pixels moved down into the ladder on starting to climb down.
    pub grab_down: Fx,
    pub hold_still: Option<usize>,
    /// Platformer `climb.up_cycle`: whole pixels up on each count of the
    /// step counter (mechanic word 0), in place of `speed`. Empty for none.
    pub up_cycle: Vec<i64>,
    /// Platformer `climb.down_cycle`: the same, down.
    pub down_cycle: Vec<i64>,
    /// Platformer `climb.by_feet`: held by the ladder under its feet.
    pub by_feet: bool,
    /// Platformer `climb.down_holds`: down held while standing keeps it
    /// from walking.
    pub down_holds: bool,
    /// Platformer `climb.down_through`: the height a down climb wraps at;
    /// 0 for none.
    pub down_through: i64,
    /// Platformer `climb.grab`: the box from the body's place, edges
    /// included, that must meet one of the level's ladder boxes for a
    /// climb to start, in a level with ladders.
    pub grab: Option<[i64; 4]>,
    /// With `grab`: a climb up by a ladder box starts with the body's x at
    /// the box's left plus this.
    pub grab_snap: Option<i64>,
    /// Platformer `climb.down_through_step`: `down_through` only while the
    /// body's last move across stepped it up a surface.
    pub down_through_step: bool,
    /// Platformer `climb.top_test`: a climb from standing asks the level's
    /// `ladder_tops` and `ladder_stops` by the grab box, and a climb up
    /// doesn't move where the box meets a stop.
    pub top_test: bool,
    /// Platformer `climb.lone_way`: up or down climbs only held alone, and
    /// on a ladder a lone left or right turns the climber.
    pub lone_way: bool,
}

/// Hit points: the player's in a counter, any other entity's its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub counter: Option<usize>,
    /// An entity's own hit points; a body counts the damage it has taken.
    pub points: i64,
    pub invulnerable: u32,
    pub shield: bool,
    /// Damage taken from a hitter of each kind, instead of its own.
    pub damage_from: Vec<(usize, i64)>,
    /// What a defeat leaves in its place (platformer 0.4.12).
    pub defeat: Option<Defeat>,
}

/// A defeated body's successor: a body of `becomes` at its place, `rise`
/// pixels higher, with `speed`'s whole part and the old speed's fraction.
/// It keeps the numbered slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defeat {
    /// An index into [`Spec::kinds`].
    pub becomes: usize,
    pub rise: Fx,
    pub speed: Fx,
}

/// Knocked back for `frames` frames by a hurt that counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Knockback {
    pub speed: Fx,
    pub rise: Fx,
    pub frames: u32,
}

/// Hitting entities with health: the kinds it hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hits {
    pub damage: i64,
    /// Entity kinds it hits, by index into [`Spec::kinds`].
    pub targets: Vec<usize>,
    pub keep: bool,
    /// Tested only on frames whose number less `phase` is a multiple of
    /// this; 1 is every frame (platformer 0.4.10).
    pub every: u32,
    pub phase: u32,
    /// The hitter moves in the contact system, just before its hits are
    /// tested, instead of in the enemies system (platformer 0.4.10).
    pub move_first: bool,
}

/// Spawning another entity on an action or a timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shoot {
    pub entity: usize,
    pub action: Option<usize>,
    /// Frames between spawns, without an action; 0 with one.
    pub every: u32,
    pub ahead: [Fx; 2],
    pub max: u32,
    /// `ahead` across isn't mirrored for a shooter facing left (platformer
    /// 0.4.10).
    pub unmirrored: bool,
    /// The new body's speed up and down (platformer 0.4.10).
    pub vy: Fx,
    /// The new body's speed across, mirrored for a shooter facing left
    /// (platformer 0.4.10).
    pub vx: Fx,
    /// The new body's speed across, signed and not mirrored, for a shooter
    /// facing both ways.
    pub vx_both: Option<Fx>,
    /// A player shoots after it moves this frame (platformer 0.4.10).
    pub after_move: bool,
    /// The new body starts from the shooter's whole-pixel place
    /// (platformer 0.4.10).
    pub whole: bool,
    /// Its `max` shots take numbered slots in turn, nothing is shot while
    /// the slot whose turn it is holds one, and a new shot keeps the
    /// fractions its slot's last shot left (platformer 0.4.12).
    pub in_slots: bool,
}

/// Lifting, carrying and throwing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carry {
    pub action: usize,
    pub lift_frames: u32,
    pub offset: [Fx; 2],
    pub throw: [Fx; 2],
    /// The throw while moving at the speed or more.
    pub throw_moving: Option<([Fx; 2], Fx)>,
}

/// Walking at a steady speed, turning at walls and, with
/// `turn_at_ledges`, at ledges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    pub speed: Fx,
    pub turn_at_ledges: bool,
    /// Walls are met at the tile hitbox's top row only (platformer 0.4.8).
    pub walls_at_top: bool,
    /// A wall doesn't stop the move: it turns the walker where it stands
    /// after it.
    pub turn_after_move: bool,
    /// The ledge is looked for after the move, this far across the entity,
    /// in place of the leading corner before it.
    pub ledge_at: Option<Fx>,
    /// Walkers with this turn each other when they touch.
    pub bump: Option<Bump>,
}

/// When walkers test their touch ([`crate::systems::bump`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bump {
    /// Tested on frames where the frame less `phase` is a multiple of this;
    /// 0 never.
    pub every: u32,
    pub phase: u32,
    /// How far from the view's left edge a body's x may be, first to last.
    pub view: Option<[i64; 2]>,
}

/// Moving sideways at a steady speed, with no gravity, through tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fly {
    pub speed: Fx,
    pub heading: Heading,
    /// It keeps the speed across it has, taking `speed` only while that is
    /// 0 (platformer 0.4.10).
    pub keep_speed: bool,
}

/// Which way a flying entity goes, chosen when it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    Player,
    Left,
    Right,
}

/// Back and forth along one axis from the spawn, with no gravity, through
/// tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// Along y rather than x.
    pub vertical: bool,
    /// From the spawn to the far end; negative is left or up.
    pub distance: Fx,
    pub speed: Fx,
    /// Frames it waits at each end.
    pub pause: u32,
    /// It doesn't leave the spawn end while the player's x is within this
    /// many pixels of its own.
    pub wait_near: Option<Fx>,
    /// With `wait_near`, the whole-pixel distance is taken modulo this; 0
    /// for none (platformer 0.4.10).
    pub wait_near_wrap: u32,
    /// It moves only on frames whose number plus `phase` is a multiple of
    /// `every`; 0 or 1 for every frame.
    pub every: u32,
    pub phase: u32,
}

/// Hopping again on landing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    /// The vertical speed it jumps at (negative, up).
    pub speed: Fx,
    /// Its hops' speeds in turn, instead of `speed`; the count of hops is
    /// the body's mechanic word 0.
    pub pattern: Vec<Fx>,
    /// It hops only on frames where frame + `phase` * its index is a
    /// multiple of `every`; 0 for every frame.
    pub every: u32,
    pub phase: u32,
    /// The landing is checked before the move (platformer 0.4.8).
    pub land_first: bool,
    /// With `land_first`, it lands on frames the enemies don't move too.
    pub land_while_stopped: bool,
}

/// Swinging around a center on one axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oscillate {
    pub vertical: bool,
    /// From where it's placed to the center.
    pub center: Fx,
    pub force: Fx,
    pub max_speed: Fx,
    /// Placed at or past the first, the center is the second from where
    /// it's placed instead.
    pub low: Option<(Fx, Fx)>,
    /// It takes no speed from its numbered slot (platformer 0.4.23).
    pub starts_at_rest: bool,
    /// It moves by the speed it had before the force (platformer 0.4.23).
    pub move_first: bool,
    /// At no speed its fraction is cleared, and short of its place it
    /// creeps a pixel on frames that are multiples of this.
    pub rest_creep_every: Option<u32>,
}

/// A pivot that turns and carries parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orbit {
    pub spin: Fx,
    pub angles: u32,
    pub parts: u32,
    /// The part's entity, an index into the kinds.
    pub part: usize,
    pub place: [Fx; 2],
    pub skip: Vec<u32>,
    pub in_view: bool,
    /// Placed on the frame it appears (platformer 0.4.20).
    pub place_on_appear: bool,
    /// A new pivot takes its whole angle from its slot's speed up and down,
    /// and one that leaves puts it there.
    pub angle_in_slot: bool,
}

/// Back and forth by a counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shuttle {
    pub vertical: bool,
    pub step: Fx,
    pub every: u32,
    pub turn_after: u32,
    /// The sway across: its step and its half period in frames.
    pub sway: Option<(Fx, u32)>,
}

/// Leaping again and again from a start point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leap {
    pub speed: Fx,
    pub gravity: Fx,
    pub max_fall: Option<Fx>,
    /// Gravity applies on frames whose number is a multiple of this.
    pub every: u32,
    /// The start's y; where it's placed when `None`.
    pub from: Option<Fx>,
    /// A slow rise first: its speed and height.
    pub rise: Option<(Fx, Fx)>,
    /// Frames a wait counts by, and the counts drawn from.
    pub interval: u32,
    pub waits: Vec<u32>,
    /// It starts again once this far under the start.
    pub below: Option<Fx>,
    /// The old speed moves it and gravity comes after.
    pub move_first: bool,
    /// The fall limit acts from one half past it ([`crate::systems::fall_limit`]).
    pub max_fall_from_half: bool,
    /// The counter whose `every` ticks the wait, instead of frames.
    pub beat: Option<usize>,
    /// A new leap keeps the fraction of the height.
    pub keep_fraction: bool,
    /// A body is at its start point on the frame it appears.
    pub place_on_appear: bool,
    /// A byte of a shift counter picks the wait and the speed's fraction,
    /// instead of the generator.
    pub draw: Option<LeapDraw>,
}

/// Where a leap reads its random byte: counter `counter`, a [`Shift`], from
/// bit `at`, `slot_step` bits lower for each numbered slot after the first.
/// The byte modulo the list picks the wait, and the byte or'd with `fraction`
/// is the launch speed's fraction, in 256ths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeapDraw {
    pub counter: usize,
    pub at: u32,
    pub slot_step: u32,
    pub fraction: u8,
}

/// A launcher (platformer 0.4.28): see `gp_model::platformer::Launcher`.
/// Run by [`crate::systems::launchers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launcher {
    /// The kind it fires.
    pub fires: usize,
    /// The shift counter whose bits pick a launcher, from bit `at`,
    /// `slot_step` lower for each slot after the first, and'ed with `mask`
    /// (`hard_mask` in a level with `hard_spawns`).
    pub counter: usize,
    pub at: u32,
    pub slot_step: u32,
    pub mask: u32,
    pub hard_mask: Option<u32>,
    /// The first numbered slots it fires into.
    pub slots: u32,
    /// The entries kept, and the counter naming the next one taken.
    pub table: u32,
    pub next: usize,
    /// A fired launcher's timer.
    pub rearm: u32,
    /// The shot's place from the launcher's.
    pub offset: [Fx; 2],
    /// A launcher placed left of this fires nothing.
    pub from_x: Fx,
    /// A shot this near the player on its first turn goes, by the
    /// difference modulo `near_wrap` (none when 0).
    pub near: u32,
    pub near_wrap: u32,
    /// A shot this far or more left of the view on its first turn goes.
    pub behind: u32,
}

/// A level marker that sends entities in: see
/// `gp_model::platformer::Stream`. Run by the spawn system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub entity: usize,
    pub past_edge: Fx,
    pub wait: u32,
    pub draw: LeapDraw,
    pub until: Option<usize>,
}

/// A springboard (platformer 0.4.4): a player that stands on it is held
/// for its steps' frames and then thrown up. Run by
/// [`crate::springboard`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Springboard {
    /// The press, in order: each step's frames, the player's move a frame
    /// (positive down) and the springboard's offset from where it was
    /// placed (positive down) while the step lasts.
    pub steps: Vec<SpringStep>,
    /// The vertical speed of a plain throw (negative, up).
    pub speed: Fx,
    /// A higher throw: the speed, the action, the first step it counts
    /// in, and whether holding the action counts (else only a new press).
    pub boost: Option<SpringBoost>,
    /// The thrown player stays standing for one more frame, moving across
    /// but not up, and rises from the frame after.
    pub throw_late: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpringStep {
    pub frames: u32,
    /// The player's move each frame of the step.
    pub moves: Fx,
    pub offset: Fx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpringBoost {
    pub speed: Fx,
    pub action: usize,
    pub from_step: u32,
    pub held: bool,
}

/// Leaving the level once off the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveView {
    pub vertical: bool,
    pub margin: Fx,
    /// The margin at the view's far edge, when it isn't `margin`
    /// (platformer 0.4.10).
    pub far_margin: Option<Fx>,
    /// Also leaving at or past the view's height less this margin
    /// (platformer 0.4.24).
    pub bottom_margin: Option<Fx>,
}

/// Falling, for an entity that doesn't jump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fall {
    pub gravity: Fx,
    pub max_fall: Fx,
    /// The fall limit acts only from one half past it.
    pub max_fall_from_half: bool,
    /// Move by the old vertical speed, then add gravity.
    pub move_first: bool,
    /// The columns where the ground is looked for, from the body's x; `None`
    /// is every column of the tile hitbox (platformer 0.4.19).
    pub feet: Option<Vec<Fx>>,
    /// A body on the ground only looks for it, and falls from the frame
    /// after it has gone (platformer 0.4.19).
    pub stands_still: bool,
}

/// Top-down 0.1's `move8`: `speed` pixels a frame on each axis while its
/// actions are held (indices into the spec's actions: left, right, up,
/// down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move8 {
    pub speed: Fx,
    pub left: usize,
    pub right: usize,
    pub up: usize,
    pub down: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub left: usize,
    pub right: usize,
    pub walk_speed: Fx,
    /// The run speed and the action that selects it.
    pub run: Option<(Fx, usize)>,
    pub acceleration: Option<Fx>,
    /// `acceleration` instead while the run action is held.
    pub run_acceleration: Option<Fx>,
    pub deceleration: Option<Fx>,
    /// Slowing while the direction held is against the motion, on the
    /// ground.
    pub skid_deceleration: Option<Fx>,
    pub air_control: bool,
    /// Slow first steps from standing: the speed and how many frames.
    pub start: Option<(Fx, u32)>,
    /// The top speed in the air with a direction held.
    pub air_speed: Option<Fx>,
    /// Holding a direction into a wall it touches clears the fraction.
    pub wall_snap: bool,
    /// The body moves by its speed rounded down to this step; the rest is
    /// kept in `Body::vx_frac`.
    pub move_step: Option<Fx>,
    /// The top speed caps the speed's whole steps and keeps its fraction.
    pub cap_whole: bool,
    /// On the ground, the rate in use doubles while the body faces away
    /// from the way it last moved, instead of a skid by the speed's sign.
    pub skid_last_motion: bool,
    /// A direction held against the way it last moved, on the ground,
    /// slower than this, sets the speed to 0 and turns at once.
    pub skid_turn_below: Option<Fx>,
    /// Actions that steer for the quick turn as a direction does: one held
    /// with no direction, or a direction not the way it last moved, turns
    /// a slow skid.
    pub skid_turn_with: Vec<usize>,
    /// While the run doesn't count: at or above each speed, this rate
    /// replaces the walking rates. In order of speed.
    pub decel_bands: Vec<(Fx, Fx)>,
    /// From this speed the fastest band holds until a direction is held
    /// the way it last moved below it.
    pub decel_hold_from: Option<Fx>,
    /// The run action still counts for this many frames, less one, after
    /// it's let go.
    pub run_hold_frames: u32,
    /// The run action counts only while the direction held is the way it
    /// last moved.
    pub run_with_motion: bool,
    /// The frame a jump starts from the ground is a frame in the air.
    pub takeoff_in_air: bool,
    /// The facing changes only on the ground.
    pub face_on_ground: bool,
    /// In the air the run counts from this speed up, and not below it,
    /// whatever the run action.
    pub air_run_from: Option<Fx>,
    /// Wall points, left then right, found after the vertical move instead
    /// of the tile hitbox stopping the move across.
    pub walls: Option<Walls>,
    /// The frame after a push out of a wall (or by the feet), a direction
    /// held toward that side doesn't change the speed
    /// ([`crate::world::Body::pushed`]).
    pub push_masks: bool,
    /// The run's speed tests and the jump's bands read the speed the run
    /// last left, before a wall stopped it
    /// ([`crate::world::Body::seen_speed`]).
    pub speed_before_walls: bool,
    /// Both directions held where the facing turns face neither way
    /// ([`crate::world::Body::faced_both`]), and the way held is both.
    pub face_both: bool,
    /// The run does nothing: the speed across stays as it is and no input
    /// reaches it (platformer 0.4.16).
    pub off: bool,
    /// Feature `run.step_cycle`: whole pixels moved on each walking frame
    /// on the ground, by the body's step counter (mechanic word 0), which
    /// a frame advances first. The counter also advances on every frame in
    /// the air and of a landing hold. Empty for none.
    pub step_cycle: Vec<i64>,
    /// Feature `run.air_step_cycle`: whole pixels moved on each frame in
    /// the air while the body keeps a way across, by its own counter
    /// (mechanic word 1). Empty for none.
    pub air_step_cycle: Vec<i64>,
    /// Feature `run.lone_direction`: a direction moves the body only with
    /// neither of its climb's up and down actions held.
    pub lone_direction: bool,
    /// Feature `run.face_way`: a takeoff turns the body to a lone
    /// direction held, a landing hold and a restart keep the facing, and an
    /// edge turn in the air turns it.
    pub face_way: bool,
    /// Feature `run.both_left`: left and right held together turn the
    /// body left and move it nowhere.
    pub both_left: bool,
}

/// Wall points from the body's position, in whole pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walls {
    pub left: Vec<[Fx; 2]>,
    pub right: Vec<[Fx; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
    pub action: usize,
    pub speed: Fx,
    pub gravity: Fx,
    pub gravity_held: Option<Fx>,
    pub max_fall: Fx,
    /// The fall limit acts only once the speed's whole part is at least
    /// `max_fall` and its fraction at least one half.
    pub max_fall_from_half: bool,
    /// A frame that starts on the ground without a jump adds no gravity
    /// and doesn't move the body up or down.
    pub no_gravity_on_ground: bool,
    /// Landing keeps the height's fraction, and a takeoff clears it.
    pub land_keeps_fraction: bool,
    /// The body's fall gravity is kept from its last jump (the band's
    /// gravity, or `gravity`), after landing too, and a fall that isn't a
    /// jump uses it ([`crate::world::Body::fall_gravity`]).
    pub keep_fall_gravity: bool,
    /// With `keep_fall_gravity`, the fall gravity a body has before its
    /// first jump, from its placing at a level start or through an exit;
    /// `gravity` without it.
    pub start_fall_gravity: Option<Fx>,
    /// A landing only while the bottom row is less than this many pixels
    /// into its tile; deeper, the feet are a wall.
    pub land_depth: Option<i64>,
    /// Ground points: the columns, from the body's x, where the tile
    /// hitbox's bottom looks for ground, instead of across its width.
    pub feet: Option<Vec<Fx>>,
    /// Jumps that depend on the horizontal speed at takeoff, in order of
    /// `below`.
    pub bands: Vec<JumpBand>,
    pub coyote_frames: u32,
    pub buffer_frames: u32,
    /// Move by the old vertical speed, then add gravity.
    pub move_first: bool,
    /// The fastest rise once the action is let go.
    pub release_cap: Option<Fx>,
    /// One point over the head tested against tiles while rising, instead
    /// of the tile hitbox's top.
    pub head: Option<Head>,
    /// Tiles are met (landing, the head, the feet's depth, the wall
    /// points) only while the body's whole y is at least the first and
    /// below the second.
    pub collide_y: Option<[i64; 2]>,
    /// Points that take a tile by touching it.
    pub pickup: Option<Pickup>,
    /// Feature `jump.takeoff_fraction`: the height's fraction a takeoff
    /// sets.
    pub takeoff_fraction: Option<Fx>,
    /// Feature `jump.land_hold`: frames after a jump's landing in which the
    /// body neither walks nor jumps (mechanic word 2 counts them).
    pub land_hold: u32,
    /// Platformer `jump.ledge_fall`: (speed, first-frame drop, kills).
    pub ledge_fall: Option<(i64, i64, bool)>,
    /// Platformer `jump.fatal_drop`: pixels below the takeoff that kill.
    pub fatal_drop: Option<i64>,
    /// Platformer `jump.fatal_drop_in_air`: the takeoff word is the height
    /// after the takeoff frame's move, and the drop is tested in the air.
    pub fatal_drop_in_air: bool,
}

/// Pickup points: after the frame's moves, the first point in a cell of
/// this kind takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pickup {
    pub tile: Tile,
    /// The points, from the body's position, in whole pixels, in order.
    pub points: Vec<[Fx; 2]>,
    /// Fired for the body.
    pub event: Option<EventKind>,
    /// The kind the cell becomes.
    pub becomes: Tile,
}

/// A head point: while the body rises, the cell this point is in decides
/// what the rise does, instead of the tile hitbox's top edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The point, from the body's position, in whole pixels.
    pub point: [Fx; 2],
    /// No head test while the body's y is above (less than) this.
    pub above: Option<Fx>,
    /// A head test only when the body's y, in the tile it's in, is at
    /// least this many pixels down (y mod the tile size).
    pub row_from: i64,
    /// A hit sets the speed's whole part and keeps its fraction.
    pub keep_fraction: bool,
    /// What a hit on each kind of cell does, first match; a cell that
    /// stops movement and isn't listed sets the speed's whole part to 0.
    pub hits: Vec<HeadHit>,
    /// While this holds, every cell that stops movement is hit as a
    /// `solid` one, with no event and no change (feature `head.busy_while`).
    pub busy_while: Option<Guard>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadHit {
    pub tile: Tile,
    /// The vertical speed's whole part after the hit.
    pub speed: Fx,
    /// Fired for the body.
    pub event: Option<EventKind>,
    /// The kind the cell becomes.
    pub becomes: Option<Tile>,
    /// Frames after the hit's frame the cell stays solid, before it takes
    /// `becomes` (or its kind before the hit, without one).
    pub hold: u32,
    /// What the hit does to the cell above (feature `head.above`).
    pub above: Option<HeadAbove>,
}

/// A head hit's change to the cell above the one hit: when it is `tile`,
/// it becomes `becomes` and `event` fires for the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadAbove {
    pub tile: Tile,
    pub becomes: Tile,
    pub event: Option<EventKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpBand {
    /// The band holds for a takeoff speed below this.
    pub below: Fx,
    pub speed: Fx,
    pub gravity: Fx,
    pub gravity_held: Option<Fx>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patrol {
    pub step: Fx,
    pub period: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// The stomp depth and the bounce speed.
    pub stomp: Option<(Fx, Fx)>,
    /// A stomped entity stays, for its machine, instead of dying.
    pub keep_on_stomp: bool,
    pub otherwise: Touch,
    /// What a hurt takes from the player's health.
    pub damage: i64,
    /// A falling player lands on it within this depth of its top, and
    /// rides it, instead of stomping it.
    pub stand: Option<Fx>,
    /// The touch is tested only on frames whose number is a multiple of
    /// this, less `phase` (platformer 0.4.9); 1 is every frame.
    pub every: u32,
    pub phase: u32,
    /// What a touch does while the player is invincible (platformer
    /// 0.4.10), its default already resolved.
    pub invincible: Touch,
    /// The touch's test takes each hitbox a pixel wider and taller, so
    /// meeting at the edges is a touch (platformer 0.4.14).
    pub touch_edges: bool,
    /// A freeze this touch sets off leaves the bodies in earlier numbered
    /// slots to move this frame (platformer 0.4.25).
    pub in_turn: bool,
    /// The touch is tested in the entity's own turn, after its move.
    pub after_move: bool,
    /// The entity's sides stop the player (platformer 0.4.22).
    pub sides: Option<Sides>,
    /// With `stand`: the player is placed on it after its move.
    pub ride: Option<Ride>,
    /// The touch tests the player's `touch_points`, in the entity's turn.
    pub near: Option<Near>,
}

/// How a touch tests points of the player, in pixels (`contact.near`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Near {
    /// A point touches while it's less than this from the entity's position
    /// on both axes.
    pub within: i64,
    /// An entity this far or more from the view's left edge touches nothing.
    pub right: Option<i64>,
}

/// How the player rides an entity, in pixels (`contact.ride`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ride {
    /// A second box this far below the first; 0 is one box.
    pub copy_below: i64,
    /// The boxes' heights wrap here; 0 is no wrap.
    pub wrap: i64,
    /// A box whose top is above this y isn't tested.
    pub above: i64,
}

/// How a touch finds a side of the entity that stops the player, in
/// pixels, measured with the touch's edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides {
    /// The player is on the left side while its right edge is less than
    /// this past the entity's left edge.
    pub left: i64,
    /// The player is on the right side while its left edge is less than
    /// this before the entity's right edge.
    pub right: i64,
    /// A falling player whose bottom edge is less than this past the
    /// entity's top edge isn't at a side.
    pub top: i64,
}

/// What kind of tile a cell is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum Tile {
    #[default]
    Empty = 0,
    /// Stops movement from every side.
    Solid = 1,
    /// Stops only an entity moving down onto its top row.
    Top = 2,
    Ladder = 3,
    Hazard = 4,
    Water = 5,
    Ice = 6,
    /// Solid, and a head hit tells it apart: a block with a coin in it.
    CoinBlock = 7,
    /// Solid, and a head hit tells it apart: a block with an item in it.
    ItemBlock = 8,
    /// Solid, and a head hit tells it apart: a brick, bumped or broken.
    Brick = 9,
    /// Passed through, and taken by a body's pickup points: a coin.
    Coin = 10,
}

impl Tile {
    pub const ALL: [Tile; 11] = [
        Tile::Empty,
        Tile::Solid,
        Tile::Top,
        Tile::Ladder,
        Tile::Hazard,
        Tile::Water,
        Tile::Ice,
        Tile::CoinBlock,
        Tile::ItemBlock,
        Tile::Brick,
        Tile::Coin,
    ];

    /// Whether it stops movement from every side: solid, or a block or
    /// brick, which are solid to everything but a head hit.
    pub fn solid(self) -> bool {
        matches!(
            self,
            Tile::Solid | Tile::CoinBlock | Tile::ItemBlock | Tile::Brick
        )
    }

    pub fn from_u8(v: u8) -> Option<Tile> {
        Self::ALL.get(usize::from(v)).copied()
    }
}

/// Platformer `level.pickups`: one group of places the player takes by
/// touching them ([`crate::systems::pickups`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupGroup {
    /// Each place, x and y in level pixels.
    pub places: Vec<[i64; 2]>,
    /// Each place's counter: 0 while it is there, 1 once taken.
    pub counters: Vec<usize>,
    /// The player's place less this is compared with a place.
    pub offset: [i64; 2],
    /// The y window from a place's y, low and high, edges included.
    pub window: [i64; 2],
    /// Fired on a take.
    pub event: Option<EventKind>,
    /// Platformer `level.pickup_holes`: a taken place's hole, left, top,
    /// right and bottom from the place, edges included.
    pub hole: Option<[i64; 4]>,
    /// Platformer `level.pickup_cells`: each place's cells, by index into
    /// the level's tiles, which show an empty cell's picture once it is
    /// taken (presentation only). Empty without the option.
    pub cells: Vec<Vec<u32>>,
    /// Platformer `level.pickup_by_hook`: [`crate::systems::pickups`]
    /// takes none of these places; a hook sets the counters.
    pub by_hook: bool,
}

/// A level's camera settings, for every camera while it plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelCamera {
    /// The axes followed, x and y, instead of the camera's.
    pub axis: Option<[bool; 2]>,
    pub lead: Option<[Fx; 2]>,
    /// The most a camera moves in a frame.
    pub step: Option<Fx>,
    /// The followed entity doesn't move while a camera is more than a
    /// step from its target.
    pub hold_player: bool,
    /// Feature `level.camera.keep_in_view`: the bounds in place of every
    /// camera's `keep_in_view` (and its rows) in this level.
    pub keep_in_view: Option<[i64; 2]>,
    /// Feature `level.camera.keep_in_view_rows`: `(rows, bounds)` in place
    /// of `keep_in_view` while the entity's row is in `rows`, as a
    /// camera's.
    pub keep_in_view_rows: Vec<([i64; 2], [i64; 2])>,
}

/// What a touch that isn't a stomp does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    KillPlayer,
    HurtPlayer,
    Collect,
    Kick,
    Nothing,
    /// The entity leaves and `enemy.defeated` fires (platformer 0.4.10).
    Defeat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    pub name: String,
    /// The checkpoint `restart-level` starts from once the first camera's
    /// x is at least `.0`: the camera's x there and the player's place
    /// (core 0.4.24).
    pub halfway: Option<(Fx, [Fx; 2])>,
    /// Pixels per grid cell, each way.
    pub tile_size: i64,
    /// Width and height in cells.
    pub size: [i64; 2],
    /// Each cell's kind, row by row.
    pub tiles: Vec<Tile>,
    /// Core `level.tile_surfaces`: each cell's standing row inside it for
    /// a top-only or ladder cell, [`NO_SURFACE`] for none, row by row; empty
    /// when the level has no surfaces. A body on ground follows them as it
    /// moves across, a pixel at a time.
    pub surfaces: Vec<u8>,
    /// Platformer `level.ladders`: each ladder's box, `[x0, y0, x1, y1]`
    /// in level pixels, edges included; empty for none.
    pub ladders: Vec<[i64; 4]>,
    /// Platformer `level.ladder_tops`: the ladder tops' boxes, as
    /// `ladders`; empty for none.
    pub ladder_tops: Vec<[i64; 4]>,
    /// Platformer `level.ladder_stops`: the boxes where a climb is
    /// refused, as `ladders`; empty for none.
    pub ladder_stops: Vec<[i64; 4]>,
    /// Platformer `level.ground`: the ground's one-row boxes, `[x0, y0,
    /// x1, y1]` in level pixels, edges included; empty for none, when a
    /// body follows the tile surfaces instead.
    pub ground: Vec<[i64; 4]>,
    /// Platformer `level.ground`: the columns from a body's x whose feet
    /// stand on the ground's boxes, first and last.
    pub ground_feet: [i64; 2],
    /// Platformer `level.ground_land`: a falling jump lands on the first
    /// ground box its feet meet on a row up to this many pixels above the
    /// row under its tile hitbox ([`crate::world::ground_land_row`]).
    pub ground_land: Option<i64>,
    /// Platformer `level.ground_boxes`: the ground is one box a row, so the
    /// walk takes no step where the feet meet none and a fall off a ledge
    /// drops its `speed` on its first frame, not its `first` (Donkey Kong's
    /// $D8AD on 75m, where 25m's $D91A steps a pixel, $D8EB).
    pub ground_boxes: bool,
    /// Platformer `level.ground_land_rising`: a jump lands on the ground
    /// its feet meet on any frame after its takeoff, rising too.
    pub ground_land_rising: bool,
    /// Platformer `level.ground_bodies`: the kinds (by index) whose bodies
    /// are ground too, each with its box `[x0, y0, x1, y1]` from the
    /// body's place, edges included; tested after `ground`. A body whose
    /// ground test meets one rides it (its `riding`).
    pub ground_bodies: Vec<(usize, [i64; 4])>,
    /// Platformer `level.pickups`: the groups of places the player takes
    /// by touching them, in the order they are tested; empty for none.
    pub pickups: Vec<PickupGroup>,
    pub spawns: Vec<Spawn>,
    pub kill_below: Option<Fx>,
    /// A player crossing `kill_below` fires this instead of dying, and goes
    /// on (platformer 0.4.9).
    pub pit_event: Option<EventKind>,
    pub goal_x: Option<Fx>,
    /// A player whose top edge is at or below this passes under the goal
    /// (platformer 0.4.13).
    pub goal_under: Option<Fx>,
    /// What a hazard tile does: the touch (kill or hurt) and the damage.
    pub hazard: Option<(Touch, i64)>,
    /// Gravity and fall limit in water tiles.
    pub water: Option<(Fx, Fx)>,
    /// Acceleration and deceleration on ice tiles.
    pub ice: Option<(Fx, Fx)>,
    /// The level's place in its stage, for state mappings.
    pub origin: [Fx; 2],
    /// Camera settings for this level.
    pub camera: Option<LevelCamera>,
    /// Values counters take when the level starts, by counter index.
    pub counters: Vec<(usize, i64)>,
    /// The level `next-level` goes to.
    pub next: Option<usize>,
    /// Entities but the player appear when their spawn's x is within this
    /// many pixels right of the view; without it, when the level starts.
    pub spawn_ahead: Option<Fx>,
    /// Entities but the player go when their right edge is this many
    /// pixels left of the view.
    pub despawn_behind: Option<Fx>,
    /// And when their left edge is this many pixels right of it.
    pub despawn_ahead: Option<Fx>,
    /// Those that go by the view wait at their spawn to appear again.
    pub respawn: bool,
    /// At most this many bodies but the player (and `no_slot` kinds) are
    /// active at once by spawning by the view.
    pub spawn_slots: Option<u32>,
    /// The width a kind's touches use here in place of its hitbox's: the
    /// kind, then the width in pixels.
    pub hit_widths: Vec<(usize, Fx)>,
    /// Despawning measures the entity's position in whole pixels, both
    /// ways, rather than its far edge behind and near edge ahead.
    pub despawn_by_position: bool,
    /// Despawning tests after the enemies system moves everything, not in
    /// the spawn system.
    pub despawn_after_move: bool,
    /// Spawns appear by the view in order, one a free slot a frame, and
    /// one already left of the view's last column at its turn is dropped.
    pub spawn_in_order: bool,
    /// A body that appears by the view first moves on the next frame.
    pub appear_still: bool,
    /// The slots are numbered and a body that appears takes the lowest
    /// free one, with the speed up and down and the fraction across its
    /// last body left (core 0.4.11): [`crate::world::World::slot_rest`].
    pub slot_rest: bool,
    /// The level's `hard_spawns` (platformer 0.4.26): Super Mario Bros.'
    /// SecondaryHardMode, which a launcher's `hard_mask` reads too.
    pub hard: bool,
    /// How the level is drawn ahead of the view, for `drawn` spawns.
    pub draw: Option<Draw>,
    /// The column whose drawing stops the scroll.
    pub scroll_stop: Option<ScrollStop>,
    /// Where the level sends the player back as it is drawn.
    pub loop_back: Option<LoopBack>,
    /// The ways out the player can take, in order.
    pub exits: Vec<Exit>,
}

/// A column that stops the scroll when it is drawn (`scroll_stop`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollStop {
    /// Where the column is along the level's axis, in pixels.
    pub at: Fx,
    /// The kinds that go as it is drawn.
    pub removes: Vec<usize>,
}

/// A level that sends the player back when its draw reaches a check and the
/// player isn't where the check wants (Super Mario Bros.' ProcLoopCommand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBack {
    /// Pixels the player and the view go back: a whole number of the
    /// draw's steps.
    pub back: i64,
    /// Columns behind the draw that stay as they are, moved back with the
    /// view (the columns a game keeps in its tile buffer).
    pub keep: i64,
    pub checks: Vec<LoopCheck>,
}

/// One place a [`LoopBack`] tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopCheck {
    /// The column, in pixels, the draw has reached.
    pub x: i64,
    /// The player's whole pixels down that pass, standing.
    pub y: i64,
}

/// An exit the `exits` system takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    /// The level it goes to.
    pub to: usize,
    /// A box in the level, in pixels, that the player's hitbox overlaps.
    pub at: Rect,
    /// The action that must be held.
    pub hold: Option<usize>,
    pub standing: bool,
    /// Where the player comes out; without it, its spawn there.
    pub enter: Option<[Fx; 2]>,
    /// Taking it starts `to` as a level, as `next-level` does.
    pub start: bool,
    /// Taken only while this holds.
    pub guard: Option<Guard>,
    /// The axis (0 x, 1 y) whose place in the view the player keeps.
    pub keep: Option<usize>,
    /// Frames the exit takes, frozen.
    pub scroll: u32,
}

/// A cell with no standing row of its own in [`Level::surfaces`].
pub const NO_SURFACE: u8 = u8::MAX;

impl Level {
    /// The standing row inside the cell holding pixel `x`, `y`, when the
    /// level has surfaces and that cell has one.
    pub fn surface(&self, x: i64, y: i64) -> Option<i64> {
        let c = self.cell(x, y)?;
        let s = *self.surfaces.get(c)?;
        (s != NO_SURFACE).then_some(i64::from(s))
    }

    /// Width and height in pixels.
    pub fn pixels(&self) -> [i64; 2] {
        [self.size[0] * self.tile_size, self.size[1] * self.tile_size]
    }

    /// The cell index of the pixel at `x`, `y`, if it's inside the grid.
    pub fn cell(&self, x: i64, y: i64) -> Option<usize> {
        if x < 0 || y < 0 {
            return None;
        }
        let (col, row) = (x / self.tile_size, y / self.tile_size);
        if col >= self.size[0] || row >= self.size[1] {
            return None;
        }
        Some((row * self.size[0] + col) as usize)
    }

    /// The top left pixel of cell `c`, the inverse of [`Level::cell`].
    pub fn cell_pixel(&self, c: usize) -> (i64, i64) {
        let c = c as i64;
        (
            (c % self.size[0]) * self.tile_size,
            (c / self.size[0]) * self.tile_size,
        )
    }

    /// The kind of the cell the pixel at `x`, `y` is in, as the level was
    /// built. Everything outside the grid is empty. A running game's
    /// changed cells are [`crate::world::World::tile_at`]'s.
    pub fn tile_at(&self, x: i64, y: i64) -> Tile {
        self.cell(x, y).map_or(Tile::Empty, |c| self.tiles[c])
    }

    /// Whether the pixel at `x`, `y` is in a solid cell, as built.
    pub fn solid_at(&self, x: i64, y: i64) -> bool {
        self.tile_at(x, y).solid()
    }

    /// The axes the level's cameras follow, given a camera's own.
    pub fn axes(&self, camera: &Camera) -> [bool; 2] {
        self.camera
            .as_ref()
            .and_then(|c| c.axis)
            .unwrap_or([camera.axis_x, camera.axis_y])
    }

    /// Whether spawning by the view runs along y: the level's camera
    /// follows y only.
    pub fn vertical(&self) -> bool {
        self.camera
            .as_ref()
            .and_then(|c| c.axis)
            .is_some_and(|a| a == [false, true])
    }
}

/// One entity placed in a level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawn {
    pub kind: usize,
    pub x: Fx,
    pub y: Fx,
    /// Where a patrol turns back.
    pub patrol_end: Option<Fx>,
    /// It appears as the level's `draw` draws its column.
    pub drawn: bool,
    /// It is placed this far past the view's last column along the level's
    /// axis when it appears (core 0.4.11).
    pub at_edge: Option<Fx>,
    /// It moves on the frame it appears, even with `appear_still` (core
    /// 0.4.11).
    pub appear_moving: bool,
}

/// How a level is drawn ahead of the view (core 0.4.10): every column
/// left of `ahead` past the view's far edge when the level is placed,
/// then a set of `step` pixels each time the view's near edge passes a
/// whole step, one tile column every `every` frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub ahead: Fx,
    pub step: i64,
    pub every: u32,
    /// Frames after a set starts on which only drawn spawns appear.
    pub pause: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Camera {
    pub name: String,
    /// The kind it follows: the first entity of that kind.
    pub follow: usize,
    pub axis_x: bool,
    pub axis_y: bool,
    pub size: [i64; 2],
    pub lead: [Fx; 2],
    pub clamp_to_level: bool,
    pub scroll_back: bool,
    /// Moves by the followed entity's move instead of keeping it `lead`
    /// from the corner (core 0.4.4).
    pub by_move: Option<ByMove>,
    /// The followed entity's whole place in the view across stays within
    /// these (core 0.4.12).
    pub keep_in_view: Option<[i64; 2]>,
    /// `keep_in_view`'s first bound acts when the place reaches it (core
    /// 0.4.17).
    pub keep_in_view_touch: bool,
    /// Feature `camera.keep_in_view_turn`: in the air, a bound reached
    /// turns the entity around.
    pub keep_in_view_turn: bool,
    /// Feature `camera.keep_in_view_rows`: `(rows, bounds)`, the bounds
    /// in place of `keep_in_view` while the entity's row is in `rows`.
    pub keep_in_view_rows: Vec<([i64; 2], [i64; 2])>,
}

/// A camera that moves forward (right or down) by the followed entity's
/// whole-pixel move this frame, by where the entity was in the view on
/// the frame before: not at all while that place is under `from`, the
/// move less `short_by` (but at least a pixel) from `from`, and the whole
/// move from `full` on. A move back or none doesn't move it. Super Mario
/// Bros.' ScrollHandler, with 80, 112 and 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByMove {
    pub from: Fx,
    pub full: Fx,
    pub short_by: Fx,
    /// Frames, less one, it doesn't move after the followed entity is
    /// pushed out of a wall, from the next frame (core 0.4.11):
    /// [`crate::world::World::camera_hold`].
    pub push_hold: u32,
    /// On a frame a state stops the followed entity's system and not the
    /// camera's, it moves by the entity's last move again (core 0.4.12):
    /// [`crate::world::World::last_move`].
    pub keep_move: bool,
    /// `push_hold` counts down on the frames the counters system runs,
    /// whether the camera runs or not (core 0.4.23).
    pub hold_with_counters: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub name: String,
    pub initial: usize,
    pub states: Vec<State>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub name: String,
    pub freeze: bool,
    pub part: Part,
    pub transitions: Vec<Transition>,
}

/// What a state stops besides a whole freeze, and how it counts its time
/// (core 0.4.12). The game's states and the entities' have the same.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Part {
    /// A part freeze: these systems don't run on a frame that starts in
    /// the state. A whole `freeze` stops every system that moves anyway.
    pub stops: Vec<System>,
    /// Of the systems it stops, these stop for the rest of the frame it is
    /// entered in too.
    pub stops_at_once: Vec<System>,
    /// Counters, by index, whose `every` changes wait at their change
    /// while a frame starts in the state.
    pub holds: Vec<usize>,
    /// Entering it keeps the frames counted in the state before.
    pub keep_time: bool,
    /// Feature `state.stops_machines`: a frame that begins in the state
    /// doesn't run `entity_states`.
    pub stops_machines: bool,
    /// Feature `state.frees_at_once`, a game machine's state: systems
    /// that run for the rest of the frame on which the state is left,
    /// though the frame began frozen in it.
    pub frees_at_once: Vec<System>,
    /// An entity's state: a camera `by_move` keeps the entity's place in
    /// its view only on frames whose number is a multiple of this; 0 is
    /// every frame.
    pub seen_every: u32,
    /// An entity's state: what entering it does to the entity's standing
    /// (core 0.4.13).
    pub ground: Option<Ground>,
    /// An entity's state: the speed across and up and down entering it
    /// sets (core 0.4.15).
    pub enter_speed: Option<[Fx; 2]>,
    /// An entity's state (feature `state.step_word`): mechanic word 0 goes
    /// up by one each frame in it, back to 0 at this; 0 for none.
    pub step_word: i64,
    /// With `enter_speed`, the speed up and down keeps its fraction below
    /// a pixel and only its whole pixels are set (core 0.4.15).
    pub enter_keeps_fraction: bool,
}

/// What entering a state does to an entity's standing (core 0.4.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// Off the ground.
    Leave,
    /// On the ground, if there is ground under it.
    Land,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub to: usize,
    pub on: Option<EventKind>,
    pub after: Option<u32>,
    /// With `after`: taken only on a frame whose number is a multiple of
    /// this (core 0.4.12).
    pub align: Option<u32>,
    /// With `after`: taken only on a frame whose counter (by index) has
    /// just had its first `every` clock come round (feature `transition.align_to`).
    pub align_to: Option<usize>,
    /// An entity's machine: taken once the body's y is at or past this,
    /// and its `after` is up with one (feature `transition.below`).
    pub below: Option<Fx>,
    /// An entity's machine, with `after` (feature `transition.word`):
    /// taken only while mechanic word `.0` is at least `.1` (if any) and
    /// below `.2` (if any).
    pub word: Option<(usize, Option<i64>, Option<i64>)>,
    /// Feature `transition.pressed`: taken on a frame this input action
    /// (by index) is newly pressed, and with `after` only once its time is
    /// up.
    pub pressed: Option<usize>,
    /// Feature `transition.held`: taken on every frame this input action
    /// (by index) is held, and with `after` only once its time is up.
    pub held: Option<usize>,
    /// Taken only while this holds.
    pub guard: Option<Guard>,
    pub actions: Vec<Do>,
}

/// A condition on a counter: its value `cmp` `value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guard {
    pub counter: usize,
    pub cmp: Cmp,
    pub value: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

impl Cmp {
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Self::Lt => a < b,
            Self::Le => a <= b,
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Ge => a >= b,
            Self::Gt => a > b,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counter {
    pub name: String,
    pub start: i64,
    pub changes: Vec<Change>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub wrap: bool,
    /// Its `every` changes count on while the world is frozen.
    pub runs_when_frozen: bool,
    /// It's a shift register, stepped every frame the `counters` system runs.
    pub shift: Option<Shift>,
    /// A level start that sets it also starts its `every` clocks over
    /// (core 0.4.24).
    pub restarts_clock: bool,
    /// Feature `counter.high_of`: it keeps the highest value this counter
    /// has had, raised as that counter changes (a top score).
    pub high_of: Option<usize>,
}

/// A linear feedback shift register in a counter's low `bits` bits: each step
/// moves them one bit down, and the top bit becomes the exclusive-or of the
/// old bits at `taps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shift {
    pub bits: u32,
    pub taps: Vec<u32>,
}

impl Shift {
    /// The value after one step.
    pub fn step(&self, v: i64) -> i64 {
        let v = v as u64;
        let top = self.taps.iter().fold(0, |t, b| t ^ (v >> b)) & 1;
        ((v >> 1) | (top << (self.bits - 1))) as i64
    }
}

/// One way a counter changes: on an event, or every so many frames the
/// world isn't frozen (any frame, for a counter that runs when frozen),
/// by an amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    On(EventKind, i64),
    Every(u32, i64),
    /// On an event that concerns an entity of one of `of`'s kinds (any,
    /// when empty), `after` frames later (core 0.4.13), while its guard
    /// holds (feature `change.if`).
    When(OnChange),
}

/// A change on an event with a filter by kind or a delay (core 0.4.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnChange {
    pub on: EventKind,
    pub by: i64,
    /// Entity kinds, by index into [`Spec::kinds`].
    pub of: Vec<usize>,
    pub after: u32,
    /// Feature `change.if`: the change is made only while this holds,
    /// tested as the event fires.
    pub guard: Option<Guard>,
}
