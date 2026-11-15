//! The interpreter's state named by model paths (docs/interpreter.md):
//! what `query`, `sample` and run-until conditions read, and what a state
//! mapping compares with the original.
//!
//! ```text
//! frame                          frames since power-on
//! level                          the current level's name
//! rng                            the random number generator's state
//! entities.<kind>[i].id          the entity's ID, as events name it
//! entities.<kind>[i].x, .y       position (top left corner), pixels
//! entities.<kind>[i].stage_x, .stage_y  the same plus the level's origin:
//!                                the place in a stage of rooms
//! entities.<kind>[i].hp          damage taken (an entity's own health)
//! entities.<kind>[i].climbing    on a ladder
//! entities.<kind>[i].carrying    the ID of the entity it carries, or -1
//! entities.<kind>[i].vx, .vy     speed, pixels per frame
//! entities.<kind>[i].on_ground   standing on something solid
//! entities.<kind>[i].fall_gravity the gravity its falls use, kept from
//!                                its last jump (`jump.keep_fall_gravity`)
//! entities.<kind>[i].seen_speed  the speed without its sign as the run
//!                                last left it, before a wall stopped it
//!                                (`run.speed_before_walls`)
//! entities.<kind>[i].facing      "left" or "right"
//! entities.<kind>[i].alive       false once stomped, collected or gone
//! entities.<kind>[i].waiting     not yet appeared (spawn_ahead)
//! entities.<kind>[i].state, .time its own machine's state and time
//! entities.<kind>[i].ext[0] to [3] its mechanic words, for an entity with
//!                                a hook or a component that counts in
//!                                them (`hop.pattern`, `leap`, `shuttle`,
//!                                `orbit`, `orbit_part`, `run.step_cycle`,
//!                                `jump.land_hold`)
//! entities.<kind>[i].home_x, .home_y  a path's spawn end, for an entity
//!                                with a `path`
//! entities.<kind>[i].path_back   on its way back to the spawn end, or
//!                                waiting at the far end to go back
//! entities.<kind>[i].wait        frames it still waits at an end
//! cameras.<name>.x, .y           the view's top left corner
//! cameras.<name>.stage_x, .stage_y  the same plus the level's origin
//! cameras.<name>.seen_x, .seen_y the followed entity's whole-pixel place
//!                                in the view, for a camera `by_move`
//! cameras.<name>.hold            frames a `by_move` with `push_hold` still
//!                                holds still
//! cameras.<name>.move_x, .move_y the followed entity's last whole-pixel
//!                                move, for a `by_move` with `keep_move`
//! rules.<name>.state             the current state's name
//! rules.<name>.time              frames since entering it
//! rules.<name>.states.<state>    true while in that state
//! counters.<name>
//! input.<action>                 the action held this frame (true) or not
//! ticks.<counter>[i]             frames counted toward the counter's i-th
//!                                `every` change, 0 just after it changes
//! ```
//!
//! An entity type's instances are a list in spawn order, so
//! `entities.slug[1].x` is the second slug's position and
//! `entities[*][*].alive` every entity's life.

use std::collections::BTreeMap;

use gp_model::path::{self, Step};
use gp_runtime::{Entry, Value};
use gp_sim::{Fx, Spec, World};

/// A body's `moved`, by its value: the way it last moved. `both` is
/// `run.face_both`'s alone; a body's `pushed` is one of the first three.
pub const MOVED: [&str; 4] = ["none", "right", "left", "both"];

/// The entity runs (in any of its states), so its state has the run's
/// own parts: `vx_frac`, `moved`, `run_hold` and `decel_hold`.
pub fn runs(kind: &gp_sim::spec::Kind) -> bool {
    kind.run.is_some()
        || kind
            .machine
            .as_ref()
            .is_some_and(|m| m.states.iter().any(|s| s.kind.run.is_some()))
}

/// The entity's run masks the side it was pushed out of (`run.push_masks`,
/// in any of its states), so its state has `pushed`.
pub fn masks_pushes(kind: &gp_sim::spec::Kind) -> bool {
    let masks = |k: &gp_sim::spec::Kind| k.run.as_ref().is_some_and(|r| r.push_masks);
    masks(kind)
        || kind
            .machine
            .as_ref()
            .is_some_and(|m| m.states.iter().any(|s| masks(&s.kind)))
}

/// The entity's speed tests read the speed the run left before walls
/// (`run.speed_before_walls`, in any of its states), so its state has
/// `seen_speed`.
pub fn sees_speed_before_walls(kind: &gp_sim::spec::Kind) -> bool {
    any_state(kind, |k| {
        k.run.as_ref().is_some_and(|r| r.speed_before_walls)
    })
}

/// Both directions held are a facing and a way of their own for the entity
/// (`run.face_both`, in any of its states), so its `facing` and `moved`
/// can be `both`.
pub fn faces_both(kind: &gp_sim::spec::Kind) -> bool {
    any_state(kind, |k| k.run.as_ref().is_some_and(|r| r.face_both))
}

/// The entity keeps its fall gravity from its last jump (in any of its
/// states), so its state has `fall_gravity`.
pub fn keeps_fall_gravity(kind: &gp_sim::spec::Kind) -> bool {
    let keeps = |k: &gp_sim::spec::Kind| k.jump.as_ref().is_some_and(|j| j.keep_fall_gravity);
    keeps(kind)
        || kind
            .machine
            .as_ref()
            .is_some_and(|m| m.states.iter().any(|s| keeps(&s.kind)))
}

/// Something of the entity (in any of its states) is true.
fn any_state(kind: &gp_sim::spec::Kind, has: impl Fn(&gp_sim::spec::Kind) -> bool) -> bool {
    has(kind)
        || kind
            .machine
            .as_ref()
            .is_some_and(|m| m.states.iter().any(|s| has(&s.kind)))
}

/// The entity keeps mechanic words (in any of its states): it has a hook,
/// or a component that counts in them. Its state has `ext[0]` to `ext[3]`.
pub fn keeps_words(kind: &gp_sim::spec::Kind) -> bool {
    any_state(kind, |k| {
        k.ext.is_some()
            || k.hop.as_ref().is_some_and(|h| !h.pattern.is_empty())
            || k.leap.is_some()
            || k.launcher.is_some()
            || k.launched
            || k.shuttle.is_some()
            || k.stream.is_some()
            || k.orbit.is_some()
            || k.orbit_part.is_some()
            || k.run.as_ref().is_some_and(|r| !r.step_cycle.is_empty())
            || k.jump
                .as_ref()
                .is_some_and(|j| j.land_hold > 0 || j.fatal_drop.is_some())
            || k.climb
                .as_ref()
                .is_some_and(|c| !c.up_cycle.is_empty() || !c.down_cycle.is_empty())
    })
}

/// The entity follows a path (in any of its states), so its state has the
/// path's own parts: `home_x`, `home_y`, `path_back` and `wait`.
pub fn follows_path(kind: &gp_sim::spec::Kind) -> bool {
    any_state(kind, |k| k.path.is_some())
}

/// The state as leaves, each a concrete path and its value, in path order.
pub fn leaves(spec: &Spec, w: &World) -> Vec<(Vec<Step>, Value)> {
    let key = |k: &str| Step::Key(k.to_owned());
    let mut out = Vec::new();
    head_leaves(spec, w, &mut out);
    let origin = spec.levels[w.level].origin;
    // Only the kinds with bodies are read.
    for k in 0..spec.kinds.len() {
        if w.of_kind(k).next().is_none() {
            continue;
        }
        let kind = &spec.kinds[k];
        for (n, (id, b)) in w.of_kind(k).enumerate() {
            body_leaves(kind, id, b, origin, &mut |field, item, value| {
                let mut p = vec![key("entities"), key(&kind.name), Step::Item(n), key(field)];
                if let Some(i) = item {
                    p.push(Step::Item(i));
                }
                out.push((p, value));
            });
        }
    }
    tail_leaves(spec, w, &mut out);
    out
}

/// The leaves before the entities: the frame, level, generator and input.
fn head_leaves(spec: &Spec, w: &World, out: &mut Vec<(Vec<Step>, Value)>) {
    let key = |k: &str| Step::Key(k.to_owned());
    out.push((vec![key("frame")], Value::Int(w.frame as i64)));
    out.push((
        vec![key("level")],
        Value::Text(spec.levels[w.level].name.clone()),
    ));
    out.push((vec![key("rng")], Value::Int(w.rng.state as i64)));
    for a in &spec.actions {
        out.push((
            vec![key("input"), key(&a.name)],
            Value::Flag(w.held & a.mask != 0),
        ));
    }
}

/// One body's leaves, in path order: each field's name, its position for
/// a list field (`ext[0]` to `[3]`) and its value.
fn body_leaves(
    kind: &gp_sim::spec::Kind,
    id: usize,
    b: &gp_sim::world::Body,
    origin: [Fx; 2],
    out: &mut impl FnMut(&str, Option<usize>, Value),
) {
    let fixed = |f: Fx| Value::Fixed(f.raw());
    out("id", None, Value::Entity(id as u64));
    out("x", None, fixed(b.x));
    out("y", None, fixed(b.y));
    out("stage_x", None, fixed(b.x.add(origin[0])));
    out("stage_y", None, fixed(b.y.add(origin[1])));
    out("hp", None, Value::Int(b.hp));
    out("climbing", None, Value::Flag(b.climbing));
    let carrying = i64::from(b.carrying) - 1;
    out("carrying", None, Value::Int(carrying));
    out("vx", None, fixed(b.vx));
    out("vy", None, fixed(b.vy));
    out("on_ground", None, Value::Flag(b.on_ground));
    if kind.jump.is_some() {
        out("jumping", None, Value::Flag(b.air == u32::MAX));
    }
    let facing = match (b.faced_both, b.facing_left) {
        (true, _) => "both",
        (false, true) => "left",
        (false, false) => "right",
    };
    out("facing", None, Value::Text(facing.into()));
    if runs(kind) {
        out("vx_frac", None, fixed(b.vx_frac));
        let moved = MOVED.get(usize::from(b.moved)).copied().unwrap_or("none");
        out("moved", None, Value::Text(moved.into()));
        out("run_hold", None, Value::Int(i64::from(b.run_hold)));
        out("decel_hold", None, Value::Flag(b.decel_hold));
    }
    if masks_pushes(kind) {
        let pushed = MOVED.get(usize::from(b.pushed)).copied().unwrap_or("none");
        out("pushed", None, Value::Text(pushed.into()));
    }
    if keeps_fall_gravity(kind) {
        out("fall_gravity", None, fixed(b.fall_gravity));
    }
    if sees_speed_before_walls(kind) {
        out("seen_speed", None, fixed(b.seen_speed));
    }
    if keeps_words(kind) {
        for (i, word) in b.ext.iter().enumerate() {
            out("ext", Some(i), Value::Int(*word));
        }
    }
    if follows_path(kind) {
        out("home_x", None, fixed(b.home[0]));
        out("home_y", None, fixed(b.home[1]));
        out("path_back", None, Value::Flag(b.path_back));
        out("wait", None, Value::Int(i64::from(b.wait)));
    }
    out("alive", None, Value::Flag(b.alive));
    out("waiting", None, Value::Flag(b.waiting));
    if let Some(m) = &kind.machine {
        out("state", None, Value::Text(m.states[b.state].name.clone()));
        out("time", None, Value::Int(i64::from(b.time)));
    }
}

/// The leaves after the entities: cameras, rules, counters and ticks.
fn tail_leaves(spec: &Spec, w: &World, out: &mut Vec<(Vec<Step>, Value)>) {
    let key = |k: &str| Step::Key(k.to_owned());
    let fixed = |f: Fx| Value::Fixed(f.raw());
    let origin = spec.levels[w.level].origin;
    for (k, (c, pos)) in spec.cameras.iter().zip(&w.cameras).enumerate() {
        for (i, axis) in ["x", "y"].into_iter().enumerate() {
            out.push((vec![key("cameras"), key(&c.name), key(axis)], fixed(pos[i])));
            out.push((
                vec![key("cameras"), key(&c.name), key(&format!("stage_{axis}"))],
                fixed(pos[i].add(origin[i])),
            ));
            if c.by_move.is_some() {
                let seen = w.seen.get(k).map_or(Fx::ZERO, |s| s[i]);
                out.push((
                    vec![key("cameras"), key(&c.name), key(&format!("seen_{axis}"))],
                    fixed(seen),
                ));
            }
        }
        if c.by_move.is_some_and(|m| m.keep_move) {
            let mv = w.last_move.get(k).copied().unwrap_or([0; 2]);
            for (i, axis) in ["move_x", "move_y"].into_iter().enumerate() {
                out.push((
                    vec![key("cameras"), key(&c.name), key(axis)],
                    Value::Int(mv[i]),
                ));
            }
        }
        if c.by_move.is_some_and(|m| m.push_hold > 0) {
            let hold = w.camera_hold.get(k).copied().unwrap_or(0);
            out.push((
                vec![key("cameras"), key(&c.name), key("hold")],
                Value::Int(i64::from(hold)),
            ));
        }
    }
    for (m, ms) in spec.machines.iter().zip(&w.machines) {
        let at = |k: &str| vec![key("rules"), key(&m.name), key(k)];
        out.push((at("state"), Value::Text(m.states[ms.state].name.clone())));
        out.push((at("time"), Value::Int(i64::from(ms.time))));
        for (i, s) in m.states.iter().enumerate() {
            let mut p = at("states");
            p.push(key(&s.name));
            out.push((p, Value::Flag(i == ms.state)));
        }
    }
    for (c, v) in spec.counters.iter().zip(&w.counters) {
        out.push((vec![key("counters"), key(&c.name)], Value::Int(*v)));
    }
    let mut tick = 0;
    for c in &spec.counters {
        let n = every_changes(c);
        for i in 0..n {
            let at = vec![key("ticks"), key(&c.name), Step::Item(i)];
            out.push((at, Value::Int(i64::from(w.ticks[tick]))));
            tick += 1;
        }
    }
}

/// How many `every` changes a counter has: its `ticks.<name>` list's
/// length.
pub fn every_changes(c: &gp_sim::spec::Counter) -> usize {
    c.changes
        .iter()
        .filter(|ch| matches!(ch, gp_sim::spec::Change::Every(..)))
        .count()
}

/// A value as it sits in the TOML tree paths resolve against: fixed-point
/// values as models write them, whole numbers as integers and fractions as
/// decimal strings, so a filter such as `[x=128]` matches.
fn to_toml(v: &Value) -> toml::Value {
    match v {
        Value::Int(n) => toml::Value::Integer(*n),
        Value::Entity(n) => toml::Value::Integer(*n as i64),
        Value::Fixed(raw) => {
            let f = Fx::from_raw(*raw);
            if f.frac() == 0 {
                toml::Value::Integer(f.floor())
            } else {
                toml::Value::String(f.to_string())
            }
        }
        Value::Flag(b) => toml::Value::Boolean(*b),
        Value::Text(t) => toml::Value::String(t.clone()),
        Value::Vec2(x, y) => {
            toml::Value::Array(vec![toml::Value::Integer(*x), toml::Value::Integer(*y)])
        }
    }
}

/// Inserts `value` at `path`, making tables and lists on the way.
fn insert(node: &mut toml::Value, path: &[Step], value: toml::Value) {
    let Some((first, rest)) = path.split_first() else {
        *node = value;
        return;
    };
    let child = match first {
        Step::Key(k) => {
            let t = node.as_table_mut().expect("a table");
            let fresh = match rest.first() {
                Some(Step::Item(_)) => toml::Value::Array(Vec::new()),
                _ => toml::Value::Table(toml::Table::new()),
            };
            t.entry(k.clone()).or_insert(fresh)
        }
        Step::Item(i) => {
            let a = node.as_array_mut().expect("a list");
            if a.len() <= *i {
                a.resize(*i + 1, toml::Value::Table(toml::Table::new()));
            }
            &mut a[*i]
        }
    };
    insert(child, rest, value);
}

/// Reads model paths from one frame's state.
#[derive(Clone)]
pub struct View {
    tree: toml::Value,
    values: BTreeMap<String, Value>,
    order: Vec<String>,
}

impl View {
    pub fn new(spec: &Spec, w: &World) -> Self {
        let mut tree = toml::Value::Table(toml::Table::new());
        let mut values = BTreeMap::new();
        let mut order = Vec::new();
        for (p, v) in leaves(spec, w) {
            insert(&mut tree, &p, to_toml(&v));
            let name = path::display(&p);
            order.push(name.clone());
            values.insert(name, v);
        }
        // Every entity type has a list, even with no instances.
        if !spec.kinds.is_empty() {
            let t = tree.as_table_mut().expect("a table");
            let entities = t
                .entry("entities")
                .or_insert(toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .expect("a table");
            for name in spec.kinds.names() {
                entities
                    .entry(name.to_owned())
                    .or_insert(toml::Value::Array(Vec::new()));
            }
        }
        Self {
            tree,
            values,
            order,
        }
    }

    /// Every leaf under every part the path names, in path order. A path
    /// that names nothing is an error.
    pub fn query(&self, text: &str) -> Result<Vec<Entry>, String> {
        let p = gp_model::Path::parse(text)?;
        let found = p.resolve(&self.tree);
        if found.is_empty() {
            return Err(format!("`{text}` names nothing in the model's state"));
        }
        let mut out = Vec::new();
        for c in found {
            let prefix = path::display(&c);
            for name in &self.order {
                let under = name == &prefix
                    || name
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['));
                if under {
                    out.push(Entry {
                        path: name.clone(),
                        value: self.values[name].clone(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// The one value a path names, as an integer, for conditions: flags are
    /// 0 or 1, and fixed-point values their whole part, so
    /// `entities.player[0].y >= 128` reads pixels.
    pub fn int(&self, text: &str) -> Result<i64, String> {
        let entries = self.query(text)?;
        match entries.as_slice() {
            [one] => match one.value {
                Value::Fixed(raw) => Ok(Fx::from_raw(raw).floor()),
                ref v => v.as_int().ok_or(format!("`{text}` is {v}, not a number")),
            },
            _ => Err(format!(
                "`{text}` names {} values; a condition needs one",
                entries.len()
            )),
        }
    }
}

/// Reads paths of one frame's state from the world itself, without the
/// [`View`]: what a batch read (`query-values` in wit/game/game.wit) does
/// for the paths it can. A path it reads gets the values [`View::query`]
/// gives for it, from the same leaves.
pub struct Leaves<'a> {
    spec: &'a Spec,
    w: &'a World,
    /// The leaves but the entities', by path, made on the first read of one.
    others: std::cell::OnceCell<BTreeMap<String, Value>>,
}

impl<'a> Leaves<'a> {
    pub fn new(spec: &'a Spec, w: &'a World) -> Self {
        Self {
            spec,
            w,
            others: std::cell::OnceCell::new(),
        }
    }

    /// What [`View::query`] gives for `text` when it is one leaf, or one
    /// field of one body or of every body of a kind
    /// (`entities.KIND[*].FIELD`), or names a body or kind that isn't
    /// there, or doesn't parse; `None` for any other path, which the view
    /// reads.
    pub fn values(&self, text: &str) -> Option<Result<Vec<Value>, String>> {
        let p = match gp_model::Path::parse(text) {
            Ok(p) => p,
            Err(e) => return Some(Err(e)),
        };
        let segs = &p.segments;
        if let [first, kind, field] = segs.as_slice()
            && first.name == "entities"
            && first.index.is_none()
        {
            let nothing = || Err(format!("`{text}` names nothing in the model's state"));
            return match self.body_field(kind, field)? {
                Some(values) => Some(Ok(values)),
                None => Some(nothing()),
            };
        }
        if !p.is_concrete() {
            return None;
        }
        let mut steps = Vec::new();
        for s in segs {
            steps.push(Step::Key(s.name.clone()));
            if let Some(path::Index::At(n)) = s.index {
                steps.push(Step::Item(usize::try_from(n).ok()?));
            }
        }
        let others = self.others.get_or_init(|| {
            let mut out = Vec::new();
            head_leaves(self.spec, self.w, &mut out);
            tail_leaves(self.spec, self.w, &mut out);
            out.into_iter()
                .map(|(p, v)| (path::display(&p), v))
                .collect()
        });
        others
            .get(&path::display(&steps))
            .map(|v| Ok(vec![v.clone()]))
    }

    /// `entities.KIND[N].FIELD` or `entities.KIND[*].FIELD`: `Some` of
    /// the values when FIELD is a leaf of every body named (`ext[i]` with
    /// its position), `Some(None)` when the kind or the body isn't there
    /// (the path names nothing), `None` for any other.
    fn body_field(
        &self,
        kind: &path::Segment,
        field: &path::Segment,
    ) -> Option<Option<Vec<Value>>> {
        let item = match field.index {
            None => None,
            Some(path::Index::At(i)) => Some(usize::try_from(i).ok()?),
            Some(_) => return None,
        };
        let Some(k) = self.spec.kinds.index_of(&kind.name) else {
            // The kind's name is no key of `entities`, whatever follows.
            return Some(None);
        };
        let bodies: Vec<(usize, &gp_sim::world::Body)> = match kind.index {
            Some(path::Index::At(n)) => match usize::try_from(n) {
                Ok(n) => self.w.of_kind(k).nth(n).into_iter().collect(),
                Err(_) => Vec::new(),
            },
            Some(path::Index::All) => self.w.of_kind(k).collect(),
            _ => return None,
        };
        if bodies.is_empty() {
            return Some(None);
        }
        let spec_kind = &self.spec.kinds[k];
        let origin = self.spec.levels[self.w.level].origin;
        let mut out = Vec::with_capacity(bodies.len());
        for (id, b) in bodies {
            let mut found = None;
            body_leaves(spec_kind, id, b, origin, &mut |f, i, v| {
                if found.is_none() && f == field.name && i == item {
                    found = Some(v);
                }
            });
            out.push(found?);
        }
        Some(Some(out))
    }
}
