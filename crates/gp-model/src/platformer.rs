//! The platformer genre extension, version [`VERSION`]: running, jumping,
//! climbing, walking, patrolling, flying, hopping and path-following
//! enemies, falling, stomping, standing on and riding entities, kicking,
//! collecting, health and damage, knockback, shooting and spawning,
//! projectiles hitting entities, carrying and throwing, pits, hazards,
//! water, ice and goals, swinging around a point, leaving the view and
//! springboards. Its JSON Schema is `schema/platformer.json` (`gp model
//! schema platformer`).
//!
//! A model uses it by declaring `extensions = { platformer = "0.4" }`.
//! The version bumps only for a breaking change, once per release. Each
//! option added since 0.4.0 (`launcher`, `leap.place_on_appear`,
//! `contact.touch_edges` and the rest) is a named feature in
//! [`crate::feature_list`], with the version it appeared in kept as a
//! note; all are optional and off by default, so a model without them
//! runs as before. A model that names a patch from before features, such
//! as "0.4.24", is read as "0.4".
//! It adds components to entities, fields to levels, per-spawn parameters,
//! and the events and actions its rules can name. [`EXTENSION`] lists them,
//! which is what any extension plugin declares.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fixed::Fixed;

pub const NAME: &str = "platformer";
/// The extension's version, `major.minor`: it bumps only for a breaking
/// change ([`crate::features`]).
pub const VERSION: &str = "0.4";

/// What an extension adds to the core, so a model's use of it can be
/// checked. A plugin extension declares the same.
#[derive(Debug, Clone, Copy)]
pub struct Extension {
    pub name: &'static str,
    pub version: &'static str,
    /// The core schema versions it works with, as `major.minor`.
    pub core: &'static str,
    /// The systems it adds, which `model.systems` puts in order with the
    /// core's.
    pub systems: &'static [&'static str],
    pub entity_components: &'static [&'static str],
    pub level_fields: &'static [&'static str],
    pub spawn_params: &'static [&'static str],
    pub events: &'static [&'static str],
    pub actions: &'static [&'static str],
}

pub const EXTENSION: Extension = Extension {
    name: NAME,
    version: VERSION,
    core: "0.4",
    systems: SYSTEMS,
    entity_components: &[
        "run",
        "jump",
        "patrol",
        "walk",
        "fall",
        "fly",
        "hop",
        "path",
        "contact",
        "climb",
        "health",
        "knockback",
        "hits",
        "shoot",
        "carry",
        "carryable",
        "oscillate",
        "leave_view",
        "leap",
        "launcher",
        "shuttle",
        "orbit",
        "orbit_part",
        "springboard",
        "stream",
        "invincible",
        "locks",
        "bounce",
        "touch_points",
    ],
    level_fields: &[
        "kill_below",
        "pit_event",
        "goal",
        "hazard",
        "water",
        "ice",
        "hard_spawns",
        "ladders",
        "ground",
    ],
    spawn_params: &["patrol_end"],
    events: &[
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
    ],
    actions: &["kill-player"],
};

/// The platformer's systems:
///
/// - `platformer.player`: the player runs and jumps, against tiles, then
///   falls into a pit (`kill_below`) or reaches the goal.
/// - `platformer.enemies`: every other entity patrols, walks and falls.
/// - `platformer.contact`: the player's touches with other entities.
pub const SYSTEMS: &[&str] = &[
    "platformer.player",
    "platformer.enemies",
    "platformer.contact",
];

/// The components a platformer entity can have. Each is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer platformer extension 0.4.5: entity components")]
pub struct EntityComponents {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Run>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<Jump>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patrol: Option<Patrol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walk: Option<Walk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fall: Option<Fall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fly: Option<Fly>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hop: Option<Hop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Path>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<Contact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub climb: Option<Climb>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knockback: Option<Knockback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits: Option<Hits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoot: Option<Shoot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carry: Option<Carry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carryable: Option<Carryable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oscillate: Option<Oscillate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leave_view: Option<LeaveView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leap: Option<Leap>,
    /// Since 0.4.28: it fires another entity on its own timer and a shift
    /// counter's bits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<Launcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuttle: Option<Shuttle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<Orbit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit_part: Option<OrbitPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub springboard: Option<Springboard>,
    /// A level marker that sends entities in from past the view's edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<Stream>,
    /// Since 0.4.10, a player's: invincible while a counter holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invincible: Option<Invincible>,
    /// A player's actions that do nothing while a counter holds (feature
    /// `power.locks`): Donkey Kong's hammer, held while its timer runs,
    /// takes away jumping and climbing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locks: Option<Locks>,
    /// Since 0.4.10: bouncing off the ground at a point, and leaving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce: Option<Bounce>,
    /// A player's points, in pixels from its position, that a
    /// `contact.near` tests. Super Mario Bros.' FirebarCollision tests
    /// y+24 for small Mario and y, y+12 and y+24 for big Mario, 4 pixels in
    /// from his left. Default none: no `contact.near` touches this player.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touch_points: Vec<[i32; 2]>,
}

/// Feature `power.locks`: while `while` holds (a counter compared with a
/// number, as `invincible`'s), the named actions count as not held and not
/// pressed for this entity's jump and climb. Donkey Kong's hammer: with
/// its timer running, A doesn't jump and Up doesn't take a ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Locks {
    #[serde(rename = "while")]
    pub while_: crate::model::Guard,
    /// The actions taken away, by name.
    pub actions: Vec<String>,
}

/// Since 0.4.10: a player that is invincible while `while` holds (a
/// counter compared with a number, as a transition's `if` is). While it
/// does, a touch does the toucher's `contact.invincible` instead of a
/// stomp or its `otherwise`: by default an entity whose touch would hurt
/// or kill the player, or kick it, is defeated (Super Mario Bros.'
/// StarInvincibleTimer: CheckForPUpCollision jumps to ShellOrBlockDefeat
/// before HandlePECollisions looks at a stomp).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Invincible {
    #[serde(rename = "while")]
    pub while_: crate::model::Guard,
}

/// Since 0.4.10: after it moves, the tile at `point` (from its position)
/// is looked at. A tile there that isn't empty, water or a ladder, met
/// while moving down for the first frame in a row, bounces it: its speed
/// up and down becomes `speed`, its fraction kept (FireballBGCollision
/// stores $fd in Fireball_Y_Speed and leaves the force alone), and its
/// whole height is rounded down to a
/// multiple of `snap`, its fraction kept. One met while moving up, or on
/// the frame after a bounce, removes it. Super Mario Bros.' fireballs
/// (FireballBGCollision, BlockBufferChk_FBall): a wall is met the same way,
/// so a fireball dies against one. Above `from_y` nothing is looked at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounce {
    pub point: [Fixed; 2],
    pub speed: Fixed,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub snap: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_y: Option<Fixed>,
}

impl EntityComponents {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Moving left and right with the input actions `left` and `right`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Top speed walking.
    pub walk_speed: Fixed,
    /// Top speed while `run_action` is held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_speed: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_action: Option<String>,
    /// Speed gained per frame toward the top speed. Without it the top
    /// speed is reached at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceleration: Option<Fixed>,
    /// `acceleration` instead while `run_action` is held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_acceleration: Option<Fixed>,
    /// Speed lost per frame with no direction held. Without it the entity
    /// stops at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deceleration: Option<Fixed>,
    /// Speed lost per frame while the direction held is against the
    /// entity's motion, on the ground: a skid. Without it a turn uses
    /// `acceleration`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skid_deceleration: Option<Fixed>,
    /// Can change direction and speed in the air, as on the ground.
    pub air_control: bool,
    /// The first steps from standing: `frames` frames at `speed` before
    /// the speed moves toward the top speed as usual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<RunStart>,
    /// The top speed in the air while a direction is held, instead of
    /// `walk_speed`, with `air_control`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_speed: Option<Fixed>,
    /// Holding a direction into a wall the tile hitbox touches clears the
    /// position's fraction, as a game that pushes its player back to the
    /// wall's edge each frame does.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wall_snap: bool,
    /// Since 0.4.2: the entity moves by its speed rounded down to a
    /// multiple of this step, and the rest of the speed is kept apart and
    /// carried to the next frame's speed (the state's `vx` is the whole
    /// steps and `vx_frac` the rest). With nothing held, `deceleration`
    /// slows it only while its whole steps aren't 0, so a stop keeps the
    /// rest. Speeds compared with `skid_turn_below` and the bands are the
    /// whole steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_step: Option<Fixed>,
    /// Since 0.4.2, with `move_step` and `acceleration`: a direction held
    /// adds its rate whatever the speed, and the top speed then caps the
    /// whole steps only, keeping the fraction, so the speed wobbles just
    /// above the top speed (just below it going left).
    #[serde(default, skip_serializing_if = "is_false")]
    pub cap_whole: bool,
    /// Since 0.4.2: what makes a skid. `speed` (the default): a direction
    /// held against the speed, on the ground, slows by
    /// `skid_deceleration`. `last_motion`: the rate in use (any of them)
    /// doubles while the entity faces away from the way it last moved,
    /// so its first step after a level start, when it hasn't moved yet,
    /// is doubled too. The facing is the one the frame starts with.
    #[serde(default, skip_serializing_if = "SkidBy::is_speed")]
    pub skid_by: SkidBy,
    /// Since 0.4.2: on the ground (not on a frame that jumps), a direction
    /// held against the way it last moved, while the speed is below this,
    /// sets the speed to 0 at once and turns; the frame's rate then applies
    /// the new way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skid_turn_below: Option<Fixed>,
    /// Since 0.4.11, with `skid_turn_below`: input actions that steer for
    /// the quick turn as a direction does. While one of them is held and
    /// no direction is, or a direction that isn't the way it last moved,
    /// a speed below `skid_turn_below` is set to 0 at once and the entity
    /// turns to the way it faces, on the ground and not on a frame that
    /// jumps. Super Mario Bros.' GetPlayerAnimSpeed takes any button but A
    /// as steering (ChkSkid), so B held after letting go of the direction
    /// stops Mario dead below $0b.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skid_turn_with: Vec<String>,
    /// Since 0.4.2: while the run doesn't count, at or above a band's
    /// `from` speed its `rate` replaces the walking rates (`acceleration`,
    /// `deceleration` and `skid_deceleration`): slowing down faster from a
    /// high speed. In order of `from`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decel_bands: Vec<DecelBand>,
    /// Since 0.4.2, with `decel_bands`: once the speed reaches this on the
    /// ground, the last band's rate holds, whatever the speed, until a
    /// direction is held the way it last moved on the ground while the
    /// speed is below this. A frame that jumps changes neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decel_hold_from: Option<Fixed>,
    /// Since 0.4.2, with `run_speed`: each frame the run action counts on
    /// the ground (not a frame that jumps) sets a count to this, and the
    /// count goes down by one as each frame starts; while it is above 0
    /// the run still counts. So the run is kept
    /// for this many frames, less one, after the action is let go.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub run_hold_frames: u32,
    /// Since 0.4.2, with `run_speed`: the run action (and its hold) counts
    /// only while the direction held is the way the entity last moved, or
    /// while nothing is held before it has moved at all. A run pressed with
    /// a direction from a level start, or while skidding, walks first.
    #[serde(default, skip_serializing_if = "is_false")]
    pub run_with_motion: bool,
    /// Since 0.4.3, with `jump`: the frame a jump starts from the ground is
    /// a frame in the air for the run, as in a game that sets the jump
    /// state before it moves the player across. With nothing held the
    /// speed is kept, and a direction held uses the air's rules
    /// (`air_run_from` included) and doesn't slow the first steps.
    #[serde(default, skip_serializing_if = "is_false")]
    pub takeoff_in_air: bool,
    /// Since 0.4.3: the facing changes only on the ground. In the air a
    /// direction held still moves the speed, but the entity keeps the way
    /// it faced, so with `skid_by = "last_motion"` pressing back in the air
    /// doesn't double the rate.
    #[serde(default, skip_serializing_if = "is_false")]
    pub face_on_ground: bool,
    /// Since 0.4.3, with `run_speed` and `air_control`: in the air the run
    /// counts (its top speed and `run_acceleration`) while the speed as the
    /// frame starts (whole steps with `move_step`, without its sign) is at
    /// least this, and doesn't count below it, whatever the run action and
    /// its hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_run_from: Option<Fixed>,
    /// Since 0.4.5: walls are found at points instead of across the tile
    /// hitbox, after the vertical move. The move across doesn't stop at a
    /// tile; once the frame's moves are done, the first of the points (left
    /// ones first) in a cell that stops movement pushes the entity one
    /// pixel away from it and stops its speed across, unless it moves away
    /// from that side. A machine state's `set` can change them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls: Option<Walls>,
    /// Since 0.4.8: on the frame after a wall or the feet push the entity
    /// out to one side, a direction held toward that side doesn't change
    /// its speed, as if nothing were held, as Super Mario Bros.'
    /// ImpedePlayerMove clears that side's bit of Player_CollisionBits and
    /// ImposeFriction masks the buttons with it the next frame. The side is
    /// world state (`pushed`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub push_masks: bool,
    /// Since 0.4.11: the run's speed tests (`decel_bands`,
    /// `decel_hold_from`, `skid_turn_below`, `air_run_from`) and the jump's
    /// `bands` read the speed as the run itself last left it, before a
    /// wall stopped it, instead of the speed as the frame starts. The run
    /// leaves it each frame on the ground, and in the air while a
    /// direction is held. Super Mario Bros.' ImposeFriction writes
    /// Player_XSpeedAbsolute and ImpedePlayerMove nulls Player_X_Speed
    /// only, so the frame after Mario runs into a wall still reads his
    /// running speed. It is world state (`seen_speed`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub speed_before_walls: bool,
    /// Since 0.4.15: both directions held where the facing turns face the
    /// entity neither way, and count as a way of their own for the run's
    /// tests against the way it last moved, though right still wins for
    /// the speed. A `skid_by = "last_motion"` then doubles the rate on the
    /// next frame whichever way it moved, and a slow skid leaves it last
    /// moved both ways. Super Mario Bros.' OnGroundStateSub stores
    /// Left_Right_Buttons in PlayerFacingDir as it is, 3 with both held,
    /// which X_Physics compares with Player_MovingDir. It is world state
    /// (`facing` and `moved` are `both`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub face_both: bool,
    /// Since 0.4.16: the run does nothing, so the speed across stays as it
    /// is and no input reaches it. For a machine state to set: while
    /// Super Mario Bros.' PlayerDeath runs, PlayerCtrlRoutine loads no
    /// buttons and GameEngine clears Left_Right_Buttons every frame, so
    /// ImposeFriction doesn't run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub off: bool,
    /// Feature `run.step_cycle`: the whole pixels the entity moves on each
    /// walking frame on the ground, by a step counter it keeps in its
    /// mechanic word 0. A walking frame (a direction held, on the ground)
    /// first advances the counter by one, back to 0 at the list's length,
    /// then moves the entry at the counter, and `walk_speed` no longer
    /// moves it. The counter also advances on every frame in the air and
    /// of a `jump.land_hold`, and not while it stands. Donkey Kong's walk
    /// routine steps one counter, $88, through 15 frames, moving on the
    /// bits of the pattern $DB/$36: 2 px every 3 frames, the same both
    /// ways, with the phase kept through stops and jumps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_cycle: Vec<u32>,
    /// Feature `run.air_step_cycle`, with `step_cycle` and without
    /// `air_control`: in the air the entity moves across by this list
    /// instead of its speed, the way it took off (the way held on the
    /// takeoff frame; none held, it doesn't move across), by a counter of
    /// its own in mechanic word 1, which a frame that moves advances first.
    /// Donkey Kong's jump moves Mario a pixel every other frame by the
    /// toggle $9E: `[1, 0]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub air_step_cycle: Vec<u32>,
    /// Feature `run.lone_direction`: left or right moves the entity only
    /// held alone, with neither of its climb's up and down actions held.
    /// Donkey Kong's walk and hammer routines move Mario only while the
    /// pad's latched byte $56 is exactly 1 or 2 ($D1CF, $D6DE), so Up or
    /// Down with a direction stands him still.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lone_direction: bool,
    /// Feature `run.face_way`: the facing is a way the game keeps from
    /// the pad: the frame a jump takes off (with `takeoff_in_air`) turns
    /// the entity to a lone direction held, a landing hold (`jump.land_hold`)
    /// keeps it, a turn at the view's edge in the air
    /// (`camera.keep_in_view_turn`) turns it with the speed, and a level's
    /// restart keeps it. Donkey Kong's pad routine $D175 writes $57 from
    /// Left or Right held alone on every frame but the jump's and the
    /// walk-off fall's ($CEC1-$CED3), the jump's edge turn writes it
    /// ($D55F-$D56E), the stage setup leaves it, and $F070 mirrors his
    /// pictures by it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub face_way: bool,
    /// Feature `run.both_left`: left and right held together turn the
    /// entity left and move it nowhere, on the ground and (with
    /// `climb.lone_way`) on a ladder. Donkey Kong's pad routine $D175
    /// writes $57 left for the pad's Left and Right bits together and
    /// leaves $56 0, so Mario faces left and stands (wallW's pad probes,
    /// 2026-11-04: x kept at 120 on girder 1 with both held).
    #[serde(default, skip_serializing_if = "is_false")]
    pub both_left: bool,
}

/// Wall points (since 0.4.5), x and y from the entity's position in whole
/// pixels, tested in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Walls {
    /// Points on the left side: a wall there pushes the entity right.
    pub left: Vec<[Fixed; 2]>,
    /// Points on the right side: a wall there pushes the entity left.
    pub right: Vec<[Fixed; 2]>,
}

/// What makes a skid (since 0.4.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkidBy {
    /// A direction held against the speed, on the ground.
    #[default]
    Speed,
    /// Facing away from the way it last moved: the rate in use doubles.
    LastMotion,
}

impl SkidBy {
    fn is_speed(&self) -> bool {
        *self == SkidBy::Speed
    }
}

/// A walking rate from a speed up (since 0.4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecelBand {
    /// At or above this speed.
    pub from: Fixed,
    pub rate: Fixed,
}

/// Slow first steps from standing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStart {
    pub speed: Fixed,
    pub frames: u32,
}

/// Jumping and falling. Vertical speed is positive downward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Jump {
    /// The input action that jumps, newly pressed while standing.
    pub action: String,
    /// The vertical speed a jump sets (negative: up).
    pub speed: Fixed,
    /// Added to the vertical speed every frame, on the ground and off it
    /// (off it only, with `no_gravity_on_ground`).
    pub gravity: Fixed,
    /// Gravity instead while the action is held and the entity is rising,
    /// for jumps whose height depends on how long the button is held.
    /// Without it, holding longer changes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity_held: Option<Fixed>,
    /// The fastest the entity falls.
    pub max_fall: Fixed,
    /// Since 0.4.3: the fall limit acts only once the speed's whole part
    /// is at least `max_fall` and its fraction at least one half, and then
    /// sets the speed to `max_fall`, so a fall goes a little past the limit
    /// on some frames. `max_fall` must be whole.
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_fall_from_half: bool,
    /// Since 0.4.5: a frame that starts on the ground and doesn't start a
    /// jump adds no gravity and doesn't move the entity up or down, as in a
    /// game that moves a standing player only across. If the ground under
    /// it has gone by the end of the frame it is in the air, and gravity
    /// starts on the next frame.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_gravity_on_ground: bool,
    /// Since 0.4.5: landing on a tile sets the whole part of the height
    /// to the tile's top and keeps its fraction, and a jump's takeoff
    /// clears the fraction, as in a game that masks the height's whole
    /// byte on landing and keeps its fraction byte apart.
    #[serde(default, skip_serializing_if = "is_false")]
    pub land_keeps_fraction: bool,
    /// Since 0.4.6: the entity's fall gravity is kept from its last jump
    /// (its band's `gravity`, or `gravity` above), after it lands too, and
    /// a fall that isn't a jump (off a ledge, or from ground that left)
    /// uses it, as in a game that keeps one falling force for the player
    /// and sets it only when a jump starts. The held phase still uses
    /// `gravity_held`. It is world state (`fall_gravity`), which a state
    /// mapping can read from the original.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_fall_gravity: bool,
    /// Since 0.4.6, with `keep_fall_gravity`: the fall gravity before the
    /// entity's first jump, from when it is placed at a level start or
    /// through an exit. Without it, `gravity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_fall_gravity: Option<Fixed>,
    /// Since 0.4.5: a landing on a tile only while the tile hitbox's
    /// bottom row is less than this many pixels into the tile. Deeper, the
    /// entity doesn't land: it is pushed one pixel back the way it last
    /// moved, as against a wall, its speed across becomes 0, and its wall
    /// points aren't tested that frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub land_depth: Option<u32>,
    /// Since 0.4.5: ground points, the columns (x from the entity's
    /// position, whole pixels) where the tile hitbox's bottom row looks
    /// for ground, instead of across the hitbox's width. A machine state's
    /// `set` can change them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feet: Option<Vec<Fixed>>,
    /// Jumps that depend on the horizontal speed at takeoff: the first band
    /// whose `below` is above the speed gives `speed`, `gravity` and
    /// `gravity_held` for the whole jump, and a speed at or above every
    /// band's uses the values above. In order of `below`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<JumpBand>,
    /// The position moves by the old vertical speed before gravity is
    /// added, as Super Mario Bros. does, rather than after.
    #[serde(default, skip_serializing_if = "is_false")]
    pub move_first: bool,
    /// Frames after walking off a ledge in which a jump still works.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub coyote_frames: u32,
    /// Frames a jump pressed in the air is remembered until landing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub buffer_frames: u32,
    /// Once the action isn't held, a rise faster than this (negative: up)
    /// is cut to it, every frame: a jump cut short by letting go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_cap: Option<Fixed>,
    /// Since 0.4.5: one point over the head decides what a rise into a
    /// tile does, instead of the tile hitbox's top edge. A machine state's
    /// `set` can change it (a bigger player's head is higher).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Head>,
    /// Since 0.4.8: the entity meets tiles (landing, the head, the feet's
    /// depth and the wall points) only while its y, in whole pixels, is at
    /// least the first and below the second; elsewhere it goes through
    /// them. Super Mario Bros.' PlayerBGCollision does nothing while
    /// Mario's high byte is off the screen or his y is $cf or more, so he
    /// falls into a pit past the ground's side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collide_y: Option<[i64; 2]>,
    /// Since 0.4.18: points that take a tile by touching it, as a coin. A
    /// machine state's `set` can change them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pickup: Option<Pickup>,
    /// Feature `jump.takeoff_fraction`: a takeoff sets the height's
    /// fraction to this (from 0, below 1), as a game that keeps a jump's
    /// height in a fraction byte of its own and starts it at a set value.
    /// Donkey Kong's jump routine starts its fraction at $F0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takeoff_fraction: Option<Fixed>,
    /// Feature `jump.land_hold`: for this many frames after the frame a
    /// jump lands, the entity neither walks, jumps nor takes a ladder; it
    /// stands. Mechanic word 2 counts the frames left. Donkey Kong keeps
    /// Mario in his jump state for 4 frames after landing ($94 from $F0 to
    /// $F4), and the jump routine runs no ladder test (clear-25m clip 1,
    /// frames 3086-3089).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub land_hold: u32,
    /// Feature `jump.ledge_fall`: a fall that isn't a jump (walking off a
    /// ledge) drops straight down at a fixed speed, and may kill on landing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledge_fall: Option<LedgeFall>,
    /// Feature `jump.fatal_drop`: a jump that lands this many pixels or
    /// more below its takeoff kills the player, when its `land_hold` ends
    /// (on landing without one). Mechanic word 3 keeps the takeoff height.
    /// Donkey Kong sets $95 to $FF 16 px below the takeoff and kills after
    /// the 4-frame hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fatal_drop: Option<u32>,
    /// Feature `jump.fatal_drop_in_air`: with `fatal_drop`, mechanic word
    /// 3 keeps the height after the takeoff frame's move, and the drop is
    /// tested on every later frame in the air, on the height the frame's
    /// move reaches before a landing puts the body on the ground. Once it
    /// holds, the word is -1000 and the landing kills. Donkey Kong's jump
    /// routine keeps $95 from $01 after the arc's first step ($D570-$D57C,
    /// $D5EA), sets it to $FF once the step's height less 16 is at or past
    /// it ($D5BF-$D5CA), before the landing test ($D5CC), and kills on $FF
    /// as the hold ends ($D615).
    #[serde(default, skip_serializing_if = "is_false")]
    pub fatal_drop_in_air: bool,
}

/// A fall off a ledge at a fixed speed (feature `jump.ledge_fall`). The
/// frame the body walks off it moves `first` pixels down (or `speed`), and
/// `speed` on each frame after, with no gravity and no move across; the
/// run gets no input, and a step counter (`run.step_cycle`) steps on each
/// frame of the fall. With `kills` the landing kills the player. Donkey
/// Kong's state 8 ($D697): 2 a frame, 3 on the first, and death on landing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgeFall {
    pub speed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub kills: bool,
}

/// Pickup points (since 0.4.18). After the frame's moves, while the entity
/// meets tiles, the first point in a cell of kind `tile` turns the cell
/// into `becomes` and fires `event` for the entity. It takes one cell a
/// frame, and the wall points aren't tested on that frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pickup {
    /// The kind of cell taken.
    pub tile: crate::model::TileKind,
    /// The points, x and y from the entity's position, in whole pixels,
    /// tested in order.
    pub points: Vec<[Fixed; 2]>,
    /// An event fired for the entity: the core's, an extension's or the
    /// model's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The kind the cell becomes.
    #[serde(default = "pickup_becomes")]
    pub becomes: crate::model::TileKind,
}

fn pickup_becomes() -> crate::model::TileKind {
    crate::model::TileKind::Empty
}

/// A head point (since 0.4.5). After the vertical move, while the entity
/// rises (its vertical speed below 0), its y is at least `above` and at
/// least `row_from` pixels into its tile row, the cell `point` is in
/// decides: a cell that stops movement sets the vertical speed's whole
/// part to its hit's `speed`, fires its `event` for the entity and turns
/// into `becomes`. The entity isn't moved. The tile hitbox still stops
/// the entity from the sides and lands it, but no longer stops a rise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Head {
    /// The point, x and y from the entity's position, in whole pixels.
    pub point: [Fixed; 2],
    /// No head test while the entity's y is less than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<Fixed>,
    /// A head test only when the entity's y, in the tile row it's in, is
    /// at least this many pixels down (y mod the tile size).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub row_from: u32,
    /// A hit sets the speed's whole part and keeps its fraction, rather
    /// than setting the whole speed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_fraction: bool,
    /// What a hit does by the cell's kind, first match. A cell that stops
    /// movement and isn't listed sets the speed's whole part to 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<HeadHit>,
    /// Feature `head.busy_while`: while this holds, every cell that stops movement is
    /// hit as a `solid` one: no event, no change to the cell. Super Mario
    /// Bros.' PlayerBGCollision reads BlockBounceTimer, which a block hit
    /// sets to 16 frames, and takes no block hit while it runs (NYSpd).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_while: Option<crate::model::Guard>,
}

/// What a head hit does to the cell above the one hit (feature `head.above`): when
/// that cell is `tile`, it becomes `becomes` and `event` fires for the
/// entity. Super Mario Bros.' BumpBlock and BrickShatter call
/// CheckTopOfBlock, which takes a coin sitting on the block that was hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeadAbove {
    pub tile: crate::model::TileKind,
    #[serde(default = "pickup_becomes")]
    pub becomes: crate::model::TileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

/// What a head hit on one kind of cell does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeadHit {
    /// A kind that stops movement: `solid`, `coin_block`, `item_block` or
    /// `brick`.
    pub tile: crate::model::TileKind,
    /// The vertical speed's whole part after the hit (positive: down).
    pub speed: Fixed,
    /// An event fired for the entity: the core's, an extension's or the
    /// model's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The kind the cell becomes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub becomes: Option<crate::model::TileKind>,
    /// Since 0.4.6: frames after the hit's frame that the cell stays
    /// solid, as a plain `solid` cell, before it becomes `becomes` (or the
    /// kind it was, without one), as in a game that keeps a bumped block's
    /// cell solid while the block bounces. A body that hits it meanwhile
    /// gets the `solid` hit. It is world state, counted down on frames
    /// that aren't frozen.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hold: u32,
    /// Feature `head.above`: what the hit does to the cell above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<HeadAbove>,
}

/// A jump band: the values a jump uses when the horizontal speed at
/// takeoff is below `below`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JumpBand {
    /// The horizontal speed (without its sign) the band reaches up to, not
    /// included.
    pub below: Fixed,
    pub speed: Fixed,
    pub gravity: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity_held: Option<Fixed>,
}

/// Walking at a steady speed, turning at walls, and at ledges with
/// `turn_at_ledges`. It starts facing the player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Walk {
    /// Pixels per frame, without its sign.
    pub speed: Fixed,
    #[serde(default, skip_serializing_if = "is_false")]
    pub turn_at_ledges: bool,
    /// Since 0.4.8: walls are met only at the tile hitbox's top row, as
    /// Super Mario Bros.' DoEnemySideCheck tests two points at y + 20 while
    /// the hitbox reaches the foot at y + 24; a hopper that has sunk into
    /// the ground (`hop.land_first`) doesn't take the ground for a wall.
    #[serde(default, skip_serializing_if = "is_false")]
    pub walls_at_top: bool,
    /// Since 0.4.26: a wall doesn't stop the move. It moves, and if the leading edge of
    /// the tile hitbox's top row is then in a solid tile it turns where it
    /// stands, as Super Mario Bros.' DoEnemySideCheck runs after the move
    /// and only reverses the speed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub turn_after_move: bool,
    /// With `turn_at_ledges`: the ledge is looked for after the move, at
    /// this many pixels across the entity whichever way it faces, in place
    /// of the leading bottom corner before the move, as Super Mario Bros.'
    /// ChkUnderEnemy reads one point under the enemy's middle and
    /// ChkForRedKoopa only reverses the speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledge_at: Option<Fixed>,
    /// Walkers with this turn each other when their hitboxes touch, as
    /// Super Mario Bros.' EnemiesCollision and EnemyTurnAround do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bump: Option<WalkBump>,
}

/// `walk.bump`: when two walkers test their touch. The test takes each one's
/// place before this frame's moves, and a pair turns once a touch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalkBump {
    /// A touch is tested on frames where the frame less `phase` is a
    /// multiple of this; 0 never, for a state that leaves the bumping.
    pub every: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub phase: u32,
    /// Only a body with its x this far from the view's left edge, first to
    /// last, takes part, as EnemyOffscrBitsMasked leaves the test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<[i64; 2]>,
}

/// Falling for an entity that doesn't jump: `gravity` is added to its
/// vertical speed every frame, up to `max_fall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fall {
    pub gravity: Fixed,
    pub max_fall: Fixed,
    /// Since 0.4.3: the fall limit acts from one half past it, as
    /// `jump.max_fall_from_half`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_fall_from_half: bool,
    /// The position moves by the old vertical speed before gravity is
    /// added, as Super Mario Bros. does, rather than after.
    #[serde(default, skip_serializing_if = "is_false")]
    pub move_first: bool,
    /// Since 0.4.19: the columns where the ground is looked for, in pixels
    /// from the entity's x, as `jump.feet`. Without them it is every column
    /// of the tile hitbox. Super Mario Bros.' ChkUnderEnemy tests one point,
    /// x + 8, so a goomba drops off a ledge once its middle is past the
    /// edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feet: Option<Vec<Fixed>>,
    /// Since 0.4.19: a body on the ground takes no gravity. It only looks
    /// for the ground, and falls from the frame after it has gone. Placed
    /// in a numbered slot it takes the slot's height fraction (since
    /// 0.4.21), and on the ground no speed from the slot. Super Mario
    /// Bros.' enemy in state 0 skips MoveD_EnemyVertically, ChkForRedKoopa's
    /// change to state 1 moves it from the next frame, nothing clears
    /// Enemy_YMF_Dummy, and SetBBox runs on into InitVStf.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stands_still: bool,
}

/// Moving sideways at a steady speed, through tiles: a bullet or a
/// swimming fish. Since 0.4.10 it may have `fall` too, for a projectile
/// thrown in an arc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fly {
    /// Pixels per frame, without its sign.
    pub speed: Fixed,
    /// Since 0.4.10: it keeps the speed across it has (one a start carries
    /// from the original, or `shoot.vx`), and takes `speed` the way it
    /// faces only while that is 0. Super Mario Bros.' fireball moves by
    /// whatever Fireball_X_Speed holds.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_speed: bool,
    /// Which way it goes; default toward the player when it spawns.
    #[serde(default, skip_serializing_if = "Heading::is_player")]
    pub heading: Heading,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Heading {
    /// Toward the player when it spawns.
    #[default]
    Player,
    Left,
    Right,
}

impl Heading {
    fn is_player(&self) -> bool {
        *self == Heading::Player
    }
}

/// Jumping again on landing: the vertical speed becomes `speed` (negative,
/// up) whenever the entity lands. Needs `fall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hop {
    pub speed: Fixed,
    /// Since 0.4.1: the speeds of its hops in turn, from the first, then
    /// again from the start (three small hops and a big one), instead of
    /// `speed`. The count of hops is kept in the body's mechanic words, so
    /// an entity with a pattern has no `ext` hook.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pattern: Vec<Fixed>,
    /// Since 0.4.1: it hops only on frames whose number, plus `phase` times
    /// the body's index, is a multiple of `every`: a hop on a beat.
    /// Standing on other frames, it waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub phase: u32,
    /// Since 0.4.8: the landing comes before the move, as Super Mario
    /// Bros.' EnemyJump runs in EnemyToBGCollisionDet before
    /// MoveJumpingEnemy. At the start of its turn, standing on ground
    /// (its bottom row in a tile that stops it) with its speed's whole
    /// part anything but -2, -1 or 0 (the game adds 2 to Enemy_Y_Speed and
    /// compares it with 3 unsigned), it is placed on that tile's top, its
    /// height's fraction kept, and takes the hop's speed; then it moves by
    /// its `fall`, and a move into the ground doesn't stop it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub land_first: bool,
    /// Since 0.4.26: with `land_first`, the landing still happens on a frame a freeze or
    /// a state's `stops` or `stops_at_once` keeps the enemies from moving,
    /// as Super Mario Bros.' RunNormalEnemies runs EnemyToBGCollisionDet
    /// before its TimerControl check, which skips only EnemyMovementSubs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub land_while_stopped: bool,
}

/// Swinging around a center on one axis (since 0.4.1): each frame its
/// speed along the axis moves by `force` toward the center, up to
/// `max_speed` either way, and it moves by its speed, through tiles. The
/// center is `center` pixels from where it's placed, or `low_center` when
/// it's placed at or past `low_at` along the axis. A flying enemy that bobs
/// around a point, or a lift that swings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Oscillate {
    pub axis: PathAxis,
    pub center: Fixed,
    pub force: Fixed,
    pub max_speed: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_at: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_center: Option<Fixed>,
    /// Since 0.4.23: it starts with no speed, whatever its numbered slot's
    /// last body left there (`slot_rest`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub starts_at_rest: bool,
    /// Since 0.4.23: it moves by the speed it had before the force is
    /// added, which is chosen by where it was before the move.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub move_first: bool,
    /// At no speed along the axis, its position's fraction is cleared, and
    /// short of where it's placed it takes no force and moves a pixel
    /// toward that place on frames that are multiples of this number
    /// (Super Mario Bros.' ProcMoveRedPTroopa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_creep_every: Option<u32>,
}

/// Leaving the level once it is off the view (since 0.4.1): removed on the
/// first frame its position along `axis` (whole pixels from the view's
/// near edge) is under `margin`, or at or past the view's size less
/// `margin`. A projectile that is gone once it leaves the screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeaveView {
    pub margin: Fixed,
    /// Since 0.4.10: the margin at the view's far edge (right or bottom),
    /// when it isn't `margin`. Super Mario Bros.' fireball goes once any
    /// of it is off the screen: its left edge past the left, or its right
    /// edge, 8 pixels on, past the right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub far_margin: Option<Fixed>,
    /// Since 0.4.24: it also leaves at or past the view's height less this
    /// margin, whatever the `axis`. Super Mario Bros.' fireball goes at
    /// height 240 (0 here), where FBall_OffscreenBits' vertical half sets
    /// a bit FireballObjCore tests, as well as past the sides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_margin: Option<Fixed>,
    #[serde(default = "axis_x", skip_serializing_if = "is_axis_x")]
    pub axis: PathAxis,
}

/// Leaping again and again from a start point (since 0.4.1): placed at
/// `from` (a y, or where it's placed without one), it leaps at `speed`
/// (negative is up; a drop starts down) and is pulled by `gravity`, on frames whose number
/// is a multiple of `every`, falling at most `max_fall`, through tiles.
/// With `rise`, it first rises at `rise.speed` until `rise.height` above
/// the start, then leaps. It starts again from the start when its wait is
/// over (`interval` frames times one of `waits`, drawn at random as the
/// leap starts) or once it is `below` pixels under the start. A lava
/// bubble, or a log that drops in a waterfall. The wait and the phase are
/// kept in the body's mechanic words 0 and 1, so it has no `ext` hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Leap {
    pub speed: Fixed,
    pub gravity: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fall: Option<Fixed>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub every: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Fixed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rise: Option<LeapRise>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub interval: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waits: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<Fixed>,
    /// Since 0.4.17: the old speed moves it and gravity comes after.
    #[serde(default, skip_serializing_if = "is_false")]
    pub move_first: bool,
    /// Since 0.4.17: as `jump.max_fall_from_half`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub max_fall_from_half: bool,
    /// Since 0.4.17: a counter with an `every`. The wait counts that
    /// shared beat's ticks instead of `interval` frames, and a new body
    /// waits one tick before its first leap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<String>,
    /// Since 0.4.17: a new leap keeps the fraction of the height.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_fraction: bool,
    /// Since 0.4.26: a body is at its start point on the frame it appears,
    /// as InitPodoboo places it while the area parser fills the slot.
    #[serde(default, skip_serializing_if = "is_false")]
    pub place_on_appear: bool,
    /// Since 0.4.26: a byte of a shift counter picks the wait and the speed's fraction,
    /// instead of the generator. Needs `beat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<LeapDraw>,
}

/// Where a leap reads its random byte: `counter`, a counter with a `shift`,
/// from bit `at`, `slot_step` bits lower for each numbered slot after the
/// first. The byte modulo the length of `waits` picks the wait, and the byte
/// or'd with `fraction` is the launch speed's fraction, in 256ths, added to
/// `speed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeapDraw {
    pub counter: String,
    pub at: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub slot_step: u32,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub fraction: u8,
}

/// Since 0.4.28: a launcher, an object that fires the entity `fires` on its
/// own timer and a shift counter's bits, aimed at the player's side (Super
/// Mario Bros.' bullet bill cannons: SetupCannon, ProcessCannons and
/// BulletBillHandler).
///
/// The level's launchers keep `table` entries. One that appears (a `drawn`
/// spawn, as its column is drawn) takes the entry the counter `next` names,
/// ending the launcher that had it and keeping that one's timer, and moves
/// `next` on, back to 0 after the last. One placed left of `from_x` takes its
/// entry and fires nothing, so it has no body. A launcher that appears is
/// picked from the next frame on (the area parser runs after the enemies).
///
/// Each frame after the other enemies, the level's first `slots` numbered
/// slots (`slot_rest`) take a turn each, the last first. A slot that holds a
/// shot gives the shot its turn: shots move there, not with the other
/// enemies. A free slot reads a byte of `counter` from bit `at`, `slot_step`
/// bits lower for each slot after the first, and'ed with `mask` (`hard_mask`
/// in a level with `hard_spawns`). A value below `table` picks that entry's
/// launcher: a timer above 0 counts down by 1, and at 0 it fires, unless the
/// enemies are frozen. The shot appears at the launcher moved by `offset`,
/// in that slot with the slot's rest fraction across, and doesn't move that
/// frame; the timer is set to `rearm`. A freeze counts timers down too.
///
/// On its first turn a shot goes if it is past the view's right edge,
/// `behind` pixels or more left of its left edge, or within `near` pixels of
/// the player. The test is on the whole-pixel difference modulo `near_wrap`
/// (none when 0), with 1 added when the shot is at or right of the player:
/// `(d + near + carry) mod wrap < 2 near`. Otherwise it faces the player's
/// side and flies. The launcher keeps its timer and entry in mechanic words 0
/// and 1, and a shot keeps "aimed" in its word 0, so neither has an `ext`
/// hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    pub fires: String,
    pub counter: String,
    pub at: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub slot_step: u32,
    pub mask: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_mask: Option<u32>,
    pub slots: u32,
    pub table: u32,
    pub next: String,
    pub rearm: u32,
    #[serde(default, skip_serializing_if = "is_zero_pair")]
    pub offset: [Fixed; 2],
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub from_x: u32,
    pub near: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub near_wrap: u32,
    pub behind: u32,
}

/// A level marker that sends entities in (Super Mario Bros.' frenzy objects).
/// Once it appears, and while a slot of the level is free, a new `entity`
/// comes in `past_edge` pixels past the view's last column, then the next
/// after `wait` frames plus the next row of the entity's `times` table, in
/// turn. A shift counter's byte (`draw`, read as a leap's, by the new body's
/// slot) picks its row of the `heights` table, which is its y; its `ext`
/// word 0 is that row and word 3 is 1, so a hook sees it is set up. The
/// marker needs `no_slot`, and it ends once a body of `until` is in the
/// level. It keeps the wait and the turn in its mechanic words 0 and 1.
/// Needs a level with `spawn_slots`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    pub entity: String,
    pub past_edge: Fixed,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub wait: u32,
    pub draw: LeapDraw,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// Moving back and forth by a counter (since 0.4.1): on frames whose
/// number is a multiple of `every`, it counts a move, turns once the count
/// passes `turn_after` (and counts from 0 again), and moves `step` pixels
/// along `axis` the way it's going, right or down first, through tiles.
/// With `sway`, each move also goes `sway.step` across the other axis,
/// toward the far side while the frame number divided by `sway.half` is
/// odd and back otherwise: a flier that bobs as it goes. The count and the
/// way are the body's mechanic words 0 and 1, so it has no `ext` hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shuttle {
    pub axis: PathAxis,
    pub step: Fixed,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub every: u32,
    pub turn_after: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sway: Option<Sway>,
}

/// A pivot that turns an angle and carries parts around it (since 0.4.1).
/// On its first frame it moves by `place` and spawns `parts` of the entity
/// `part` where it is. Then each frame (only while its x is inside the view
/// with `in_view`) its angle moves by `spin` steps of the `angles` in a
/// turn (negative: the other way); landing on an angle in `skip` it goes a
/// step on. The
/// angle is kept in 256ths of a step in the body's mechanic word 0. A
/// firebar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    pub spin: Fixed,
    #[serde(default = "thirty_two", skip_serializing_if = "is_thirty_two")]
    pub angles: u32,
    pub parts: u32,
    pub part: String,
    #[serde(default, skip_serializing_if = "is_zero_pair")]
    pub place: [Fixed; 2],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_view: bool,
    /// Since 0.4.20: it moves by `place` and spawns its parts on the frame
    /// it appears, in the spawn system, rather than on its first turn in
    /// the enemies system, so its first turn already moves its angle.
    /// Super Mario Bros.' InitShortFirebar runs as the area parser puts
    /// the firebar in its slot, and ProcFirebar spins it from the next
    /// frame.
    #[serde(default, skip_serializing_if = "is_false")]
    pub place_on_appear: bool,
    /// In a level with `slot_rest`, a new pivot takes its whole angle from
    /// its slot's speed up and down, wrapped to `angles`, and a pivot that
    /// leaves puts its whole angle there. Super Mario Bros.'
    /// InitShortFirebar clears FirebarSpinState_Low only, and
    /// FirebarSpinState_High is the slot's Enemy_Y_Speed. Default off: it
    /// starts at angle 0.
    #[serde(default, skip_serializing_if = "is_false")]
    pub angle_in_slot: bool,
}

/// A part carried by an `orbit` pivot (since 0.4.1): on its first frame it
/// finds its pivot, the nearest body with an `orbit` within `within`
/// pixels of where it was placed, and which part it is (the parts placed
/// there before it). Each frame it's at the pivot plus an offset from a
/// quarter-turn table: the entity's `ext_tables` `offsets` (one run of
/// entries per part, a quarter turn and one long), `rows` (where each
/// part's run starts) and `signs` (each quarter turn's signs, bit 0 for x
/// and bit 1 for y). x takes the angle folded into a quarter turn and y the
/// angle a quarter turn on. It leaves when its pivot does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrbitPart {
    pub within: Fixed,
}

fn thirty_two() -> u32 {
    32
}
fn is_thirty_two(n: &u32) -> bool {
    *n == 32
}

/// A springboard (since 0.4.4): a player that stands on it (its feet on
/// the springboard's top as placed, or riding it through `contact.stand`)
/// is held for the frames of its `steps`, then thrown up at `speed`. Each
/// frame of a step moves the player by the step's `move` (positive down)
/// and puts the springboard at its `offset` below where it was placed.
/// While held the player doesn't move by its speed, jump or fall, though
/// its run still changes its speed. The press's last frame throws it,
/// higher with a `boost`. With `throw_late` the thrown player stands one
/// more frame, moving across but not up, and rises from the frame after,
/// as when a game throws after the player's move. The press is kept in
/// the body's mechanic words 0 to 2, so a springboard has no `ext` hook.
/// Its `on_ground` is false while it presses, for its sprite's clips.
/// Super Mario Bros.' jumpspring, or any spring or trampoline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Springboard {
    /// The press's steps, in order.
    pub steps: Vec<SpringStep>,
    /// The vertical speed of a plain throw (negative, up).
    pub speed: Fixed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost: Option<SpringBoost>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub throw_late: bool,
}

/// One step of a springboard's press.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpringStep {
    /// How many frames it lasts.
    pub frames: u32,
    /// The player's move each frame, in pixels (positive down).
    #[serde(rename = "move", default, skip_serializing_if = "is_zero_fixed")]
    pub moves: Fixed,
    /// The springboard's place below where it was placed, in pixels.
    #[serde(default, skip_serializing_if = "is_zero_fixed")]
    pub offset: Fixed,
}

/// A higher throw: from step `from_step` (counted from 0) on, `action`
/// newly pressed (or held, with `held`) on any frame of the press makes
/// the throw `speed` instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpringBoost {
    pub speed: Fixed,
    pub action: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub from_step: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub held: bool,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}
fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}
fn is_zero_fixed(v: &Fixed) -> bool {
    *v == Fixed::ZERO
}

/// A shuttle's sway across its axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sway {
    pub step: Fixed,
    pub half: u32,
}

/// A slow rise before a leap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeapRise {
    pub speed: Fixed,
    pub height: Fixed,
}

fn axis_x() -> PathAxis {
    PathAxis::X
}
fn is_axis_x(a: &PathAxis) -> bool {
    *a == PathAxis::X
}

/// Moving back and forth along one axis from the spawn: out by `distance`
/// (negative: left or up) at `speed`, waiting `pause` frames at each end,
/// and back. With `wait_near`, it doesn't leave the spawn end while the
/// player's x is within that many pixels of its own. With `every` (since
/// 0.4.7), it moves only on frames whose number plus `phase` is a multiple
/// of `every`. No gravity, through tiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Path {
    pub axis: PathAxis,
    pub distance: Fixed,
    /// Pixels per frame, without its sign.
    pub speed: Fixed,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pause: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_near: Option<Fixed>,
    /// Since 0.4.10, with `wait_near`: the distance across, in whole
    /// pixels, is taken modulo this before it is compared, as Super Mario
    /// Bros.' ChkPlayerNearPipe compares only PlayerEnemyDiff's low byte
    /// (256), so a plant a page and a few pixels from Mario waits. 0 for
    /// none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wait_near_wrap: u32,
    /// Since 0.4.7: it moves only on frames whose number plus `phase` is a
    /// multiple of this (Super Mario Bros.' piranha plant moves a pixel on
    /// odd frames). 0 or 1 for every frame.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub every: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub phase: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PathAxis {
    X,
    Y,
}

/// Walking back and forth between the spawn's `x` and its `patrol_end`
/// parameter, turning at each end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Patrol {
    /// Pixels moved each time it moves.
    pub step: Fixed,
    /// It moves once every this many frames.
    pub period: u32,
}

/// What happens when the player touches this entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    /// A falling player whose bottom row is within this many rows of the
    /// top of this entity's hitbox stomps it: `enemy.stomped` fires and the
    /// entity is removed, unless `keep_on_stomp`. Without it the entity
    /// can't be stomped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stomp_depth: Option<Fixed>,
    /// The player's vertical speed after a stomp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce: Option<Fixed>,
    /// A stomp leaves the entity in place, for its state machine to change
    /// (a Koopa into its shell, a Goomba flattened for a while).
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_on_stomp: bool,
    /// What any other touch does.
    pub otherwise: Touch,
    /// How much a `hurt-player` touch takes from the player's `health`.
    /// Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
    /// A falling player lands on the entity's hitbox top instead of
    /// stomping it, and rides it while it moves: `enemy.stood_on` fires on
    /// landing. The touch below the top is still `otherwise`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stand: bool,
    /// Since 0.4.9: the touch is tested only on frames whose number is a
    /// multiple of this (the frame count from power-on), as Super Mario
    /// Bros.' PlayerEnemyCollision leaves on frames whose FrameCounter is
    /// odd. Default every frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    /// Since 0.4.9, with `every`: the frames tested are those whose number
    /// less this is a multiple of `every` (PlayerHammerCollision runs on
    /// odd frames: `every = 2`, `phase = 1`). Default 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub phase: u32,
    /// Since 0.4.10: what a touch does while the player is invincible
    /// (its `invincible`). Default `defeat` for an entity whose touch would
    /// hurt or kill the player or kick it, or that can be stomped, and
    /// `otherwise` for the rest. Super Mario Bros.' firebars and hammers
    /// do nothing to a player with the star.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invincible: Option<Touch>,
    /// Since 0.4.14: the touch's test takes each hitbox one pixel wider and
    /// taller, so two that only meet at their edges touch. Super Mario
    /// Bros.' SprObjectCollisionCore compares a box's left and right as
    /// offsets from BoundBoxCtrlData, each a pixel in the box, and counts
    /// equal as a touch. Default off: hitboxes touch when they share a pixel.
    #[serde(default, skip_serializing_if = "is_false")]
    pub touch_edges: bool,
    /// Since 0.4.25: a state's `stops_at_once` that this touch sets off
    /// leaves the bodies in earlier numbered slots to finish their move.
    /// Super Mario Bros.' RunNormalEnemies tests a touch inside the enemy
    /// loop, so TimerControl stops the enemy that hurt Mario and those after
    /// it, and those before it have moved. Default off, and a body without a
    /// numbered slot stops them all.
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_turn: bool,
    /// The touch is tested in the entity's own turn, after its move, not
    /// before the enemies move. Super Mario Bros.' BulletBillHandler moves a
    /// cannon's bill and then calls PlayerEnemyCollision, in ProcessCannons
    /// after the enemy loop, so all the enemies have moved on the frame a
    /// bill hits Mario. Default off.
    #[serde(default, skip_serializing_if = "is_false")]
    pub after_move: bool,
    /// Since 0.4.22: the entity's sides stop the player. A touch at a side
    /// impedes the player as a wall does: its speed across becomes 0 and
    /// it moves one pixel away, unless it is already moving away. Super
    /// Mario Bros.' PlatformSideCollisions calls ImpedePlayerMove. Default
    /// off: a touch doesn't move the player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<Sides>,
    /// Since 0.4.26, with `stand`: how the player rides the entity. The player is placed
    /// on the entity after the entity's own move on the same frame, keeping
    /// the fraction of its height, as Super Mario Bros.' MoveSmallPlatform
    /// and ChkYPCollision run PositionPlayerOnVPlat after the platform moves.
    /// It needs `platformer.contact` before `platformer.enemies` in the
    /// system order. Default off: the player is placed in the contact
    /// system, on a whole pixel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ride: Option<Ride>,
    /// The touch is tested by the player's `touch_points` near the entity's
    /// position, in the entity's turn after it moves, in place of the
    /// hitboxes in the contact system. Super Mario Bros.' FirebarCollision
    /// runs inside ProcFirebar. Default off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<Near>,
}

/// How a touch tests points of the player (`contact.near`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Near {
    /// A point touches while it is less than this many pixels from the
    /// entity's position on both axes.
    pub within: u32,
    /// An entity this many pixels or more from the view's left edge touches
    /// nothing (FirebarCollision's `cmp #$f0`). Default no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<u32>,
}

/// How the player rides an entity it stands on (`contact.ride`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ride {
    /// A second box this many pixels below the first, tested after it.
    /// Super Mario Bros.' SmallPlatformCollision moves the box down $80 and
    /// tests again. Default 0: one box.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub copy_below: u32,
    /// The boxes' heights wrap at this y, as the game's height byte does.
    /// Default 0: no wrap.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wrap: u32,
    /// A box whose top is above this y isn't tested (`cmp #$20`). Default 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub above: u32,
}

/// Where a touch counts as a side of the entity (`contact.sides`, since
/// 0.4.22), in pixels, measured with the touch's edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sides {
    /// The player is on the left side while its right edge is less than
    /// this past the entity's left edge.
    pub left: u32,
    /// The player is on the right side while its left edge is less than
    /// this before the entity's right edge.
    pub right: u32,
    /// A falling player whose bottom edge is less than this past the
    /// entity's top edge isn't at a side.
    pub top: u32,
}

/// Climbing ladders: tiles of kind `ladder`. `up` held with the hitbox's
/// middle on a ladder, or `down` held standing on a ladder's top, starts a
/// climb: the entity moves to the ladder's middle across and has no
/// gravity, moving `speed` a frame while `up` or `down` is held. It stops
/// climbing when its middle leaves the ladder, when it lands, or when
/// `release` is pressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Climb {
    pub speed: Fixed,
    pub up: String,
    pub down: String,
    /// The action that lets go. Default none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// A ladder cell with no ladder above it stands like a `top` tile for
    /// this entity while it isn't climbing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stand_on_top: bool,
    /// Pixels it moves down into the ladder when it starts climbing down
    /// from the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_down: Option<Fixed>,
    /// Holding this action keeps it still on the ladder (firing, in Mega
    /// Man 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_still: Option<String>,
    /// Feature `climb.up_cycle`: whole pixels moved up on each count of
    /// the body's step counter (its mechanic word 0, shared with
    /// `run.step_cycle`), in place of `speed`. A frame up is held at a
    /// ladder steps the counter, and the climb starts on a count that
    /// moves (Donkey Kong's $88 pattern at $D9E8, $24 and $49).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub up_cycle: Vec<u32>,
    /// Feature `climb.down_cycle`: the same for climbing down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub down_cycle: Vec<u32>,
    /// Feature `climb.by_feet`: the ladder that holds the climber is the
    /// row of pixels under its tile hitbox, not its middle. It climbs while
    /// that row is in a ladder or a cell with a surface, stops where it
    /// isn't (a broken ladder's top), and stands once it reaches a surface
    /// row. Down starts a climb from a surface with a ladder under it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub by_feet: bool,
    /// Feature `climb.down_holds`: `down` held while standing and not
    /// climbing keeps the body still: the run gets no input and the step
    /// counter doesn't step. Up held walks as usual (Donkey Kong's walk
    /// routine looks for a ladder down instead of walking). A jump comes
    /// first: the takeoff frame and a landing hold aren't held.
    #[serde(default, skip_serializing_if = "is_false")]
    pub down_holds: bool,
    /// Feature `climb.down_through`, with `by_feet`: down newly pressed
    /// also starts a climb from standing with the feet in a ladder cell (a
    /// ladder's foot), and climbing down tests nothing on the way: it goes on until
    /// the feet reach a surface row, and the height wraps at this many
    /// pixels, as an 8-bit coordinate does. Neither `kill_below` nor a
    /// goal touches a body climbing so. Donkey Kong looks for a girder
    /// only of the floor its y + 8 is in, and Down at a ladder's foot
    /// climbs down through the floor, round the screen and onto the top
    /// girder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_through: Option<u32>,
    /// Feature `climb.grab`, `[x0, y0, x1, y1]` from the entity's place, in
    /// whole pixels: in a level with `ladders`, a climb up starts only
    /// where this box (edges included) meets one of the level's ladder
    /// boxes below the box's top row, instead of by the cell at the
    /// entity's middle. Standing on a ladder's top, up does nothing (the
    /// game's climb routine finds the top in a list of its own, $D50A).
    /// Donkey Kong's ladder routine ($D28B) tests Mario's feet box against
    /// each ladder's box ($D8AD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab: Option<[i32; 4]>,
    /// With `grab` (feature `climb.grab`): a climb up by a ladder box
    /// starts with the entity's x at the box's left plus this, instead of
    /// the middle of the cell. Donkey Kong's $D2B9 puts Mario's sprite at
    /// the ladder's x less 4 ($A1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_snap: Option<i32>,
    /// Feature `climb.down_through_step`, with `down_through`: the press of
    /// down at a ladder's foot climbs down through the floor only while the
    /// entity's last move across stepped it up a surface. Donkey Kong's
    /// walk tests the ground ($D20B) after the move across and before the
    /// step up, so $5A is 0 after a step up the slope, and $D432 skips its
    /// ladder-top test then (the ladder-zip, TASVideos 2299S).
    #[serde(default, skip_serializing_if = "is_false")]
    pub down_through_step: bool,
    /// Feature `climb.top_test`, with `grab`: a climb from standing asks
    /// the level's `ladder_tops` and `ladder_stops` by the grab box (edges
    /// included). Down starts only where the box meets one of the level's
    /// ladder boxes, and a top and no stop, at the ladder's left plus
    /// `grab_snap`; up doesn't start where it meets a top or a stop; and
    /// climbing, up doesn't move where it meets a stop. With
    /// `down_through_step`, a press of down after a step up skips the top
    /// and stop test but still needs the ladder box. Donkey Kong's ladder routine
    /// asks $D50A on the girder while $5A is set ($D397 up, $D43B down)
    /// and before each move ($D3AF, $D45A): 1 in a top's box, 2 in a
    /// stop's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub top_test: bool,
    /// Feature `climb.lone_way`: the climb reads one way from the pad, as
    /// the walk does with `run.lone_direction`: up or down starts or moves
    /// a climb only held alone, and on a ladder left or right held alone
    /// (both, with `run.both_left`) turns the climber to it without moving
    /// it. Donkey Kong's $D175 latches $56 only for a lone direction and
    /// writes $57 from Left or Right on every frame but a jump's or a
    /// fall's, the ladder's too, and the ladder routine ($D37E) climbs on
    /// $56 = 8 or 4 (wallW's ladder probes, 2026-11-04: Left on a ladder
    /// faces left on the frame it is pressed; Down and Left stand still).
    #[serde(default, skip_serializing_if = "is_false")]
    pub lone_way: bool,
}

/// Hit points. The player's are a counter, so rules and the HUD read them;
/// any other entity's are its own, per instance. A hurt takes `damage`
/// from them, unless the entity is invulnerable from an earlier one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// The player's: the counter that holds its hit points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<String>,
    /// Any other entity's: its hit points when it appears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    /// Frames after a hurt in which no hurt counts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub invulnerable: u32,
    /// Hits do nothing (the hitter still goes, unless `hits.keep`): a
    /// shield, usually set by a state.
    #[serde(default, skip_serializing_if = "is_false")]
    pub shield: bool,
    /// Damage taken from each hitting entity, by entity name, instead of
    /// its `hits.damage`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub damage_from: std::collections::BTreeMap<String, u32>,
    /// Since 0.4.12: what a defeat by a hit or a touch leaves in its place,
    /// instead of nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defeat: Option<Defeat>,
}

/// What a defeat leaves (0.4.12): a body of another entity at the same
/// place, which keeps the defeated one's numbered slot until it leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Defeat {
    /// The entity it becomes.
    pub becomes: String,
    /// Pixels it moves up at once.
    pub rise: Fixed,
    /// The whole part of its speed up and down; the fraction is kept.
    pub speed: Fixed,
}

/// Being knocked back by a hurt that counts: for `frames` frames the entity
/// moves `speed` a frame against the way it faces, its run input ignored,
/// and on the first its vertical speed becomes `rise`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Knockback {
    pub speed: Fixed,
    pub rise: Fixed,
    pub frames: u32,
}

/// Hitting other entities: a projectile, or a thrown enemy. Touching an
/// entity with `health` whose role is in `roles` takes `damage` from it
/// (or its `damage_from` for this entity) and fires `enemy.hit` for it;
/// one out of hit points fires `enemy.defeated` and leaves the level. The
/// hitter leaves too, unless `keep`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hits {
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub damage: u32,
    /// The roles it hits. Default `enemy` and `boss`.
    #[serde(default = "enemy_roles", skip_serializing_if = "is_enemy_roles")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep: bool,
    /// Since 0.4.10: hits are tested only on frames whose number less
    /// `phase` is a multiple of this, as `contact.every` (Super Mario
    /// Bros.' FireballEnemyCollision runs on even frames).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub phase: u32,
    /// Since 0.4.10: the hitter moves (its components, as the enemies
    /// system moves it) in the contact system, just before its hits are
    /// tested, and not in the enemies system, so it hits the others where
    /// they were before they move. Super Mario Bros. runs FireballObjCore,
    /// which moves a fireball and tests it against the enemies, ahead of
    /// its enemy loop.
    #[serde(default, skip_serializing_if = "is_false")]
    pub move_first: bool,
}

/// Spawning another entity: when `action` is pressed, or every `every`
/// frames, while fewer than `max` of it are in the level. It appears
/// `ahead` from this entity's position, x mirrored when this one faces
/// left, facing the same way. One of `action` and `every`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shoot {
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero_pair")]
    pub ahead: [Fixed; 2],
    pub max: u32,
    /// Since 0.4.10: `ahead` across isn't mirrored for a shooter facing
    /// left (Super Mario Bros.' fireball starts 4 pixels right of Mario
    /// whichever way he faces).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unmirrored: bool,
    /// Since 0.4.10: the new body's speed up and down (a fireball starts
    /// falling at 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vy: Option<Fixed>,
    /// Since 0.4.10: the new body's speed across, mirrored for a shooter
    /// facing left (a fireball's $40 or $c0, 4 pixels a frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vx: Option<Fixed>,
    /// The new body's speed across, signed and not mirrored, for a shooter
    /// facing both ways (`run.face_both`). FireballObjCore indexes
    /// FireballXSpdData by PlayerFacingDir less 1, so a facing of 3 reads
    /// the byte after the table, the routine's own first opcode, $86: 8
    /// pixels left less $60 sixteenths of the next pixel, -7.625 a frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vx_both: Option<Fixed>,
    /// Since 0.4.10, a player's: it shoots after it moves this frame, from
    /// where it has got to, as Super Mario Bros.' ProcFireball_Bubble runs
    /// in GameEngine after GameRoutines has moved Mario.
    #[serde(default, skip_serializing_if = "is_false")]
    pub after_move: bool,
    /// Since 0.4.10: the new body starts from the shooter's place in whole
    /// pixels, its fractions 0 (FireballObjCore copies Player_X_Position
    /// and Player_Y_Position, not their fractions).
    #[serde(default, skip_serializing_if = "is_false")]
    pub whole: bool,
    /// Since 0.4.12: the `max` shots take numbered slots in turn, nothing
    /// is shot while the slot whose turn it is holds one, and a new shot
    /// sets only the whole parts of its place and speed down, keeping the
    /// fractions its slot's last shot left. FireballObjCore never clears
    /// SprObject_YMF_Dummy or SprObject_Y_MoveForce, and
    /// ProcFireball_Bubble picks the slot by FireballCounter's low bit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_slots: bool,
}

/// Lifting, carrying and throwing `carryable` entities. `action` pressed
/// while standing on one lifts it: for `lift_frames` frames the carrier
/// doesn't move, then the entity rides at `offset` from the carrier's
/// position. `action` pressed again throws it: it takes `throw` (x
/// mirrored when facing left), or `throw_moving` when the carrier's speed
/// across is `moving_above` or more. `item.lifted` and `item.thrown` fire,
/// each concerning the item and the carrier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Carry {
    pub action: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lift_frames: u32,
    pub offset: [Fixed; 2],
    pub throw: [Fixed; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throw_moving: Option<[Fixed; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving_above: Option<Fixed>,
}

/// An entity a `carry` can lift.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Carryable {
    /// Its offset from the carrier instead of the carry's, such as a
    /// taller item's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[Fixed; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Touch {
    /// The player dies: `player.died` fires.
    KillPlayer,
    /// `player.hurt` fires, for the player's state machine to decide what
    /// a hurt does (shrink, or die with `kill-player`).
    HurtPlayer,
    /// The player collects it: it's removed and `item.collected` fires.
    Collect,
    /// The player kicks it: `enemy.kicked` fires, for its state machine.
    Kick,
    Nothing,
    /// Since 0.4.10: the entity is defeated: it leaves and
    /// `enemy.defeated` fires for it.
    Defeat,
}

/// Fields a platformer level can have.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer platformer extension 0.4.0: level fields")]
pub struct LevelFields {
    /// A player whose top edge is at or below this y has fallen into a pit
    /// and dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_below: Option<Fixed>,
    /// Since 0.4.9: a player falling into the pit fires this event (one of
    /// `model.events`) on the frame its top edge first reaches
    /// `kill_below`, instead of dying, and goes on moving, so the model's
    /// rules say what a fall does and when the life ends. Super Mario
    /// Bros.' PlayerHole locks the screen once Mario is below it and ends
    /// the life only later, while he and the enemies go on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pit_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Goal>,
    /// What a `hazard` tile does to the player touching it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hazard: Option<Hazard>,
    /// Falling in `water` tiles: these replace the entity's gravity and
    /// fall limit while its hitbox's middle is in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water: Option<Water>,
    /// Running on `ice` tiles: these replace the entity's acceleration and
    /// deceleration while it stands on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ice: Option<Ice>,
    /// Since 0.4.26: the level's `spawn_objects` list also places the
    /// objects it marks for hard mode, which are left out otherwise.
    /// Super Mario Bros.' ProcessEnemyData skips them until
    /// SecondaryHardMode is set, from level 5-3 on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hard_spawns: bool,
    /// Feature `level.ladders`: the level's ladders as boxes, read from two
    /// `byte-rows` slots, for a climber's `climb.grab`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladders: Option<Ladders>,
    /// Feature `level.ladder_tops`: the boxes of the level's ladder tops,
    /// read from two `byte-rows` slots as `ladders` is, for a climber's
    /// `climb.top_test`. Donkey Kong's $D50A tests Mario's feet box against
    /// each stage's list of whole ladders' top rows ($C127 and $C143 for
    /// 25m).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder_tops: Option<Ladders>,
    /// Feature `level.ladder_stops`: the boxes where a climb is refused,
    /// read as `ladders` is, for a climber's `climb.top_test`. Donkey
    /// Kong's $D50A refuses a climb where Mario's feet box meets one of the
    /// broken ladders' zones ($C161 and $C16E for 25m).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder_stops: Option<Ladders>,
    /// Feature `level.ground`: the level's ground as one-row boxes read
    /// from runs in two `byte-rows` slots, which a body on the ground
    /// follows by its feet in place of the tile surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground: Option<Ground>,
    /// Feature `level.pickups`: groups of places the player takes by
    /// touching them, read from a byte table, each kept in a counter. A
    /// group may leave a hole where a place was taken (feature
    /// `level.pickup_holes`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pickups: Vec<Pickups>,
}

/// Feature `level.pickups`: `count` places, their x from index `x_at` of
/// the byte table `table` (a slot's column, `slot.column`) on and their y
/// from `y_at` on. Place k is in the counter `<counter>_<k>`, 0 while it
/// is there and 1 once taken; the level's counters set them at its start.
/// After the player's moves, on a frame it moved across or is in a jump,
/// its place less `offset` touches the first place not taken whose x is
/// its x and whose y is within `window` of its y (place y + window[0] to
/// place y + window[1], edges included), unless it is falling off a ledge
/// or out: the place's counter becomes 1 and `event` fires. Groups are
/// tested in order and one touch a frame ends the test. Donkey Kong's
/// $EE79 tests Mario's sprite ($0200, $0203: his place less 5) against
/// 100m's 8 rivets ($C5AE and $C5C2 from entry 11, a window of 16 above)
/// and then the stage's 3 prizes ($EEF0, entry $C5FA[$53], exact), on a
/// frame his step beat $BE is set, and returns after the first it finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pickups {
    /// A byte table: `slot.column`.
    pub table: String,
    /// The index in `table` of the first place's x.
    pub x_at: u32,
    /// The index in `table` of the first place's y.
    pub y_at: u32,
    /// How many places.
    pub count: u32,
    /// The counters' name: place k's is `<counter>_<k>`.
    pub counter: String,
    /// What the player's place less this is compared with: Donkey Kong's
    /// sprite is 5 left of and above Mario's place.
    #[serde(default)]
    pub offset: [i32; 2],
    /// The y window from a place's y, low and high, edges included; [0, 0]
    /// for the same y.
    #[serde(default)]
    pub window: [i32; 2],
    /// An event (one of `model.events`) fired on a take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Feature `level.pickup_holes`: a place taken is a hole, this box
    /// (left, top, right, bottom from the place, edges included): a touch
    /// of it, as a take's but not in a jump, starts the player's fall off
    /// a ledge (`jump.ledge_fall`) on the next frame, and that fall doesn't
    /// land on a ground box (`level.ground`) on the hole's rows while the
    /// player's feet are within the hole. Donkey Kong's $EED9 puts Mario in
    /// his fall ($96 = 8) on a removed rivet's spot unless he is jumping;
    /// the fall ($D697) goes 2 pixels a frame down past the floor, whose
    /// box test ($D2CB) finds no floor under him there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole: Option<[i32; 4]>,
    /// Feature `level.pickup_cells`: the level's cells a place covers,
    /// which show an empty cell's picture (`present.cells`, kind `empty`)
    /// once it is taken. Presentation only: the cells' kinds stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<PickupCells>,
    /// Feature `level.pickup_by_hook`: the engine takes none of the
    /// group's places; an `ext` hook sets its counters, and the group gives
    /// the places' cells (`level.pickup_cells`). Original Edition's 50m
    /// takes a prize on a climb's step too ($EEF0 on $BE), which the
    /// `dk_prizes` hook models, and $EF38 blanks its cells as on 100m.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub by_hook: bool,
}

/// Feature `level.pickup_cells`: place k's first cell is the number made
/// of the bytes at `at[0] + k` (high) and `at[1] + k` (low) of the group's
/// `table`, less `base`, counted row by row across the level's width; it
/// covers `size` cells across and down from there, less any outside the
/// level. Donkey Kong's $EF38
/// writes blank tiles ($24) over a rivet taken (1 by 1, as $CC = $11 at
/// $EEB6 asks) or a prize (2 by 2, $CC = $22 at $EF0C), at the name-table
/// address $C5D6 (high) and $C5E9 (low) give the place's entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickupCells {
    /// The indexes in `table` of the first place's high and low byte.
    pub at: [u32; 2],
    /// The number of the level's first cell.
    #[serde(default)]
    pub base: u32,
    /// Cells across and down.
    pub size: [u32; 2],
}

/// Feature `level.ground`: each row of `runs` is a run of boxes, its first
/// five bytes x, y, shape, the step across from one box to the next (a
/// signed byte) and the count; each box is `rise` pixels above the last,
/// and x and y wrap as bytes. The shape divided by `shape_step` is a row
/// of `shapes`, whose four bytes are the box's left, top, right and bottom
/// from its place, edges included. A body on the ground keeps its height
/// while its feet, the columns `feet` from its x on the row under its tile
/// hitbox, meet a box; once they meet none it moves to the nearest row a
/// pixel or two up or down whose box they meet. Donkey Kong's walk tests
/// Mario's feet box ($C186: his sprite +8 to +10, +17 down) against the
/// girder pieces of his row ($D91A, the runs at $C08C, each piece a pixel
/// higher) after each move across and steps a pixel when it finds none
/// ($D8EB).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ground {
    pub runs: String,
    pub shapes: String,
    #[serde(default = "one_step")]
    pub shape_step: u32,
    #[serde(default = "one_rise")]
    pub rise: i32,
    pub feet: [i32; 2],
    /// Feature `level.ground_land`: a jump falling (its vertical speed 0
    /// or more) lands on the first box its feet meet on a row from this
    /// many pixels above the row under its tile hitbox down to that row,
    /// wherever it was the frame before, and the tiles don't land it. A
    /// ledge fall (`jump.ledge_fall`) lands the same way, where it is.
    /// Donkey Kong's jump tests Mario's feet box, +10 to +12 down from his
    /// height, against the girder pieces in their list's order ($D5CC,
    /// $D2CB, $D91A) and puts him on the first it meets ($D5D3); his fall
    /// off a girder's end tests the same box and dies where it meets one
    /// ($D6BC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub land: Option<u32>,
    /// Feature `level.ground_land_rising`: with `land`, a jump lands on a
    /// box its feet meet on any frame after its takeoff, rising too. Donkey
    /// Kong's jump runs its ground test ($D5CC) on every frame but the
    /// takeoff's ($D5BB) whichever way the arc goes, so a jump whose feet
    /// rise into a box's top lands there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub land_rising: bool,
    /// Feature `level.ground_bodies`: entities whose box is ground too, so
    /// the ground can move. A ground test meets the level's boxes first,
    /// then these bodies' boxes in the bodies' order. A body whose test
    /// meets one of these rides it: it moves by the same step whenever
    /// that body moves, and it stays standing without a test until it
    /// moves across or leaves the ground. The ride ends at the next test
    /// that meets no box at all (the walk's after a move across, each
    /// frame of a jump after its takeoff, a ledge fall's); a test that
    /// meets one of the level's boxes leaves it as it was. Donkey Kong's
    /// 75m: when the stage's boxes hold nothing ($D316), $D326 tests the
    /// six elevator platforms against Mario's feet box and sets $DA by the
    /// column it meets, or clears it when it meets none; $E834 then moves
    /// Mario's sprite with the column on each of its steps while $DA holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<GroundBody>,
    /// Feature `level.ground_boxes`: each row of `runs` is one box, its
    /// first three bytes x, y and shape, with no step across or count.
    /// Donkey Kong's ground on every stage but 25m ($D2E2 reads the
    /// stage's rows through the pointer at $C46B and tests Mario's feet box
    /// against each, $D8AD; 75m's at $C20C).
    #[serde(default, skip_serializing_if = "is_false")]
    pub boxes: bool,
}

fn one_rise() -> i32 {
    1
}

/// Feature `level.ground_bodies`: an entity kind whose bodies are ground,
/// and its box: left, top, right and bottom from the body's place, edges
/// included. Donkey Kong's elevator platform is the box at $C2C8: its
/// sprite's x to x + 18 on the row under its y ($D326 with $2A).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroundBody {
    pub entity: String,
    #[serde(rename = "box")]
    pub edges: [i32; 4],
}

/// Feature `level.ladders`: each row of `places` is a ladder, its first
/// three bytes its x, its y and its shape; the shape divided by
/// `shape_step` is a row of `shapes`, whose four bytes are the box's left,
/// top, right and bottom from the ladder's x and y, edges included.
/// Donkey Kong keeps each stage's ladders this way ($C0E3 and $C10B for
/// 25m).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ladders {
    pub places: String,
    pub shapes: String,
    #[serde(default = "one_step")]
    pub shape_step: u32,
    /// Feature `level.ladder_lists`: `places` names a byte table's column
    /// (`slot.column`) that holds the game's own list, rows of three bytes
    /// from this index up to a row that starts with `end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub places_at: Option<u32>,
    /// Feature `level.ladder_lists`: `shapes` names a byte table's column
    /// whose rows of four bytes start at this index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shapes_at: Option<u32>,
    /// Feature `level.ladder_lists`: the byte that ends a list read by
    /// `places_at`. Default 254 ($FE, Donkey Kong's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
}

fn one_step() -> u32 {
    1
}

/// A hazard tile's touch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hazard {
    /// `kill-player` or `hurt-player`.
    pub touch: Touch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Water {
    pub gravity: Fixed,
    pub max_fall: Fixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ice {
    pub acceleration: Fixed,
    pub deceleration: Fixed,
}

impl LevelFields {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Reaching the goal fires `goal.reached`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    /// The player's left edge reaching this x.
    pub x: Fixed,
    /// Since 0.4.13: a player whose top edge is at or below this y passes
    /// under the goal and doesn't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<Fixed>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}
fn is_one(n: &u32) -> bool {
    *n == 1
}
fn one() -> u32 {
    1
}
fn enemy_roles() -> Vec<String> {
    vec!["enemy".into(), "boss".into()]
}
fn is_enemy_roles(r: &Vec<String>) -> bool {
    r == enemy_roles().as_slice()
}
fn is_zero_pair(p: &[Fixed; 2]) -> bool {
    *p == [Fixed::ZERO; 2]
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// The whole extension's schema, for `gp model schema platformer`.
#[derive(JsonSchema)]
#[schemars(title = "Game Printer platformer extension 0.4.5")]
pub struct Schema {
    /// Components it adds to entities.
    pub entity: EntityComponents,
    /// Fields it adds to levels.
    pub level: LevelFields,
}
