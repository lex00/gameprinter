//! Many starts, found in a recording without anyone choosing them
//! (docs/interpreter.md, "Many starts"): for each level the recording
//! reaches, the first quiet frame after the level starts, and a few more
//! frames through the level where the player stands next to something
//! (the first pit ahead, the first wall, the first block overhead, the
//! first enemy of each kind), so the same presses meet different things.
//!
//! One pass over the input finds them all. Each start is a frame of the
//! original the model can be set to: a polled frame after the mapping's
//! start, with the model's frame there counted the way the lockstep counts
//! it ([`crate::lockstep::start_at`]), so a start taken from a snapshot of
//! the original there is the same start a replay takes. A start the
//! rebuilt game can't begin from says why (`cant`) and is kept, not
//! dropped.

use serde::{Deserialize, Serialize};

use gp_runtime::{InputFrame, LoadHow, Runtime, Value};
use gp_sim::spec::Tile;
use gp_sim::{Fx, Spec, World};

use crate::mapping::Mapping;
use crate::translate::{self, Translation};

/// What a start is next to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Near {
    /// The level's first quiet frame.
    LevelStart,
    /// A column with no ground under it ahead of the player.
    Pit,
    /// A solid tile ahead at the player's height.
    Wall,
    /// A solid tile over the player's head.
    Block,
    /// A living entity of this kind ahead.
    Enemy { entity: String },
    /// A context of the clip corpus (`gp_interp::corpus`): `words` is how
    /// it reads after the level's name, "standing still".
    Context { context: String, words: String },
}

impl Near {
    /// In game words, after the level's name: "World 1-2, before the first pit".
    pub fn words(&self) -> String {
        match self {
            Near::LevelStart => "start".into(),
            Near::Pit => "before the first pit".into(),
            Near::Wall => "at the first wall".into(),
            Near::Block => "under the first block".into(),
            Near::Enemy { entity } => format!("near the first {}", entity.replace('_', " ")),
            Near::Context { words, .. } => words.clone(),
        }
    }

    /// A short key for IDs: `start`, `pit`, `enemy-goomba`.
    pub fn key(&self) -> String {
        match self {
            Near::LevelStart => "start".into(),
            Near::Pit => "pit".into(),
            Near::Wall => "wall".into(),
            Near::Block => "block".into(),
            Near::Enemy { entity } => format!("enemy-{entity}"),
            Near::Context { context, .. } => context.replace('[', "-").replace(']', ""),
        }
    }
}

/// One start found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundStart {
    /// The original's frame (after that many steps from power-on).
    pub frame: u64,
    /// The model's frame there, as the lockstep counts it.
    pub model_frame: u64,
    /// The model level the original is in.
    pub level: String,
    pub near: Near,
    /// "Mario standing on the ground at 3.7 s, nothing near".
    pub says: String,
    /// Standing still (else only on the ground).
    pub still: bool,
    /// Why the rebuilt game can't start here, when it can't: "enemies
    /// aren't carried over yet: the original has a goomba here".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cant: Option<String>,
}

/// How many starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    /// Starts after the level's own, at most, in each level.
    pub extra_per_level: usize,
    /// Frames at least between two starts of a level.
    pub spacing: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            extra_per_level: 3,
            spacing: 120,
        }
    }
}

/// What a pass found: the starts in frame order, and each level the
/// input reaches with the frame it is first played in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub starts: Vec<FoundStart>,
    pub levels: Vec<(String, u64)>,
}

/// Frames on the ground before a frame counts as quiet.
const STILL_FOR: u32 = 10;
/// Nothing within this many pixels of a level's start.
const NEAR: i64 = 80;
/// A level start waits this long for the player to stand still before it
/// takes a frame where the player only stands on the ground.
const WAIT_STILL: u64 = 180;
/// Features are looked for every this many polled frames.
const EVERY: u64 = 4;

pub(crate) fn px(v: &Value) -> Option<i64> {
    match v {
        Value::Fixed(r) => Some(r >> 16),
        Value::Int(n) => Some(*n),
        _ => None,
    }
}

pub fn holds<O: Runtime>(o: &O, cond: &gp_runtime::select::Condition) -> Result<bool, String> {
    cond.eval(&mut |sel| {
        o.query(&sel.to_string())?
            .first()
            .and_then(|e| e.value.as_int())
            .ok_or(format!("{sel} names no number"))
    })
}

fn read<O: Runtime>(o: &O, sel: &str) -> Result<i64, String> {
    o.query(sel)?
        .first()
        .and_then(|e| e.value.as_int())
        .ok_or(format!("{sel} names no number"))
}

/// The original's living slot objects of a kind the mapping names: the
/// entity and its x, by the slots' `x` field.
pub(crate) fn slot_objects<O: Runtime>(
    o: &O,
    mapping: &Mapping,
) -> Result<Vec<(String, Option<i64>)>, String> {
    let Some(s) = &mapping.slots else {
        return Ok(Vec::new());
    };
    let xf = s.fields.iter().find(|f| f.path == "x");
    let mut out = Vec::new();
    for i in 0..s.count {
        let flag = s.flag_of(i, |a| read(o, &format!("ram[{a:#06x}]")).map(|v| v as u8))?;
        if flag == 0 {
            continue;
        }
        let id = s.id_of(i, |a| read(o, &format!("ram[{a:#06x}]")).map(|v| v as u8))?;
        let Some(k) = s
            .kinds
            .iter()
            .find(|k| k.ids.contains(&id) && s.flag_fits(k, flag))
        else {
            continue;
        };
        let x = match xf {
            Some(f) => {
                let mut bytes = Vec::new();
                for sel in f.slot_selectors(i) {
                    bytes.push(read(o, &sel)? as u8);
                }
                px(&f.value(&bytes))
            }
            None => None,
        };
        out.push((k.entity.clone(), x));
    }
    Ok(out)
}

/// The player's body in a translated world, if it is alive in the level.
pub(crate) fn player_body<'a>(
    spec: &Spec,
    w: &'a World,
    pk: usize,
) -> Option<&'a gp_sim::world::Body> {
    let p = w.bodies.iter().find(|b| b.kind == pk)?;
    let rows = spec
        .levels
        .get(w.level)
        .map_or(i64::MAX, |l| l.size[1].saturating_mul(l.tile_size));
    (p.alive && p.y >= Fx::ZERO && p.y.floor() < rows).then_some(p)
}

/// Whether every rule of a translated world is in its first state: not
/// dying, won or paused, whose time in the state the mapping doesn't
/// carry over.
pub(crate) fn in_play(spec: &Spec, w: &World) -> bool {
    spec.machines
        .iter()
        .zip(&w.machines)
        .all(|(m, s)| s.state == m.initial)
}

/// What the player stands next to in a translated world, nearest kind
/// first: a pit ahead, a wall ahead, a block overhead.
fn tiles_near(spec: &Spec, w: &World, p: &gp_sim::world::Body) -> Vec<Near> {
    let Some(level) = spec.levels.get(w.level) else {
        return Vec::new();
    };
    let [width, height] = level.pixels();
    let hb = spec.kind(p).tile_hitbox;
    let left = p.x.add(hb.x).floor();
    let right = p.x.add(hb.x).add(hb.w).floor();
    let top = p.y.add(hb.y).floor();
    let feet = p.y.add(hb.y).add(hb.h).floor();
    let dir: i64 = if p.facing_left { -1 } else { 1 };
    let front = if p.facing_left { left } else { right };
    let ground = |t: Tile| t.solid() || t == Tile::Top;
    let mut out = Vec::new();
    // A pit: a column ahead with nothing to stand on down to the bottom.
    for dx in (8..=72).step_by(4) {
        let x = front + dx * dir;
        if x < 0 || x >= width {
            break;
        }
        let floor = (feet + 1..height)
            .step_by(4)
            .any(|y| ground(w.tile_at(spec, x, y)));
        if !floor {
            out.push(Near::Pit);
            break;
        }
    }
    // A wall: solid ahead at the player's height.
    let mid = (top + feet) / 2;
    if (1..=24).any(|dx| {
        let x = front + dx * dir;
        w.tile_at(spec, x, mid).solid() || w.tile_at(spec, x, feet - 4).solid()
    }) {
        out.push(Near::Wall);
    }
    // A block overhead, within a jump.
    let center = (left + right) / 2;
    if (16..=72).step_by(8).any(|dy| {
        [-8, 0, 8, 16]
            .iter()
            .any(|dx| w.tile_at(spec, center + dx * dir, top - dy).solid())
    }) {
        out.push(Near::Block);
    }
    out
}

/// Why the rebuilt game can't start from a translation, or `None`.
/// `gaps` are the loaded objects the start can't carry
/// ([`crate::carry::gaps`]).
pub(crate) fn cant_start(
    spec: &Spec,
    t: &Result<Translation, String>,
    pk: Option<usize>,
    name: &str,
    gaps: &[crate::carry::Gap],
) -> Option<String> {
    let t = match t {
        Ok(t) => t,
        Err(e) => return Some(format!("the original's state there doesn't translate: {e}")),
    };
    if spec.levels.len() > 1
        && t.notes
            .iter()
            .any(|n| n.path == "level" && n.how == LoadHow::Missing)
    {
        return Some("the mapping can't say which level this is".into());
    }
    if let Some(pk) = pk
        && player_body(spec, &t.world, pk).is_none()
    {
        return Some(format!("{name} isn't in the level here"));
    }
    // An object the original has loaded that the start can't carry as it
    // is: a start from here would be wrong where the notes don't say.
    crate::carry::says(gaps)
}

/// Why the rebuilt game can't begin at the original's state now, or
/// `None` when it can: the same test the starts found in a recording pass
/// (a translation that loads, the player in the level, every rule in its
/// first state, every loaded object one the start carries), for a caller
/// that picks its own frames, such as `gp capture dense`
/// (docs/research/dense-clips.md).
pub fn cant_start_here<O: Runtime>(
    o: &O,
    spec: &Spec,
    mapping: &Mapping,
    player: &str,
) -> Result<Option<String>, String> {
    // The slots first: reading them is cheap, and one object the start
    // can't carry is enough to refuse, so most such frames need no
    // translation.
    let gaps = crate::carry::gaps(o, spec, mapping)?;
    if let Some(why) = crate::carry::says(&gaps) {
        return Ok(Some(why));
    }
    let base = World::power_on(spec);
    let t = translate::from_original(o, spec, &base, mapping);
    if let Ok(tw) = &t
        && !in_play(spec, &tw.world)
    {
        return Ok(Some("a rule of the game isn't in its first state".into()));
    }
    let pk = spec.kinds.index_of(player);
    Ok(cant_start(spec, &t, pk, player, &gaps))
}

/// One level visit's progress.
struct Visit<O> {
    level: String,
    from: u64,
    /// The level's start, once taken.
    started: bool,
    /// A frame on the ground with nothing near, kept until the player
    /// stands still or the wait ends: the original there, its frame, the
    /// model's frame, and the start.
    fallback: Option<(O, FoundStart)>,
    used: Vec<Near>,
    extras: usize,
    last: u64,
}

/// Finds the starts of `inputs` on `original` (at power-on): the first
/// quiet frame of each level it reaches, and up to `opts.extra_per_level`
/// frames on the ground next to a pit, a wall, a block or an enemy of a
/// kind not met yet in that level, at least `opts.spacing` frames apart.
/// A level played again in the same input (after a death, or later) adds
/// nothing. `found`
/// hears each start with the original standing there, for a snapshot or a
/// picture. `rate` is the model's frame rate, for the seconds in a
/// sentence. `have` are the starts earlier inputs found, by level and what
/// each is next to: a level whose start is among them gets only more
/// starts next to things they don't have, up to the same count.
#[allow(clippy::too_many_arguments)]
pub fn find_starts<O: Runtime>(
    original: &O,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    player: &str,
    rate: (u64, u64),
    opts: Options,
    have: &[(String, Near)],
    found: &mut dyn FnMut(&O, &FoundStart) -> Result<(), String>,
) -> Result<Found, String> {
    let idx = |field: &str| {
        mapping
            .fields
            .iter()
            .position(|f| f.path == format!("entities.{player}[0].{field}"))
    };
    let (Some(ground), Some(xi)) = (idx("on_ground"), idx("x")) else {
        return Err(format!(
            "the state mapping has no on_ground or x for {player}, so a quiet frame can't be found"
        ));
    };
    let vx = idx("vx");
    let cond = mapping
        .start
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()?;
    let pk = spec.kinds.index_of(player);
    let name = {
        let mut c = player.chars();
        c.next()
            .map(|f| {
                f.to_uppercase()
                    .chain(c)
                    .collect::<String>()
                    .replace('_', " ")
            })
            .unwrap_or_default()
    };
    let secs = |f: u64| f as f64 * rate.1 as f64 / rate.0.max(1) as f64;
    let base = World::power_on(spec);
    let mut o = original.fork();
    let mut out = Found::default();
    let mut started = false;
    let mut model_frame = 0u64;
    let mut still = 0u32;
    let mut grounded = 0u32;
    let mut last_x: Option<Value> = None;
    let mut visit: Option<Visit<O>> = None;
    let mut seen: Vec<String> = Vec::new();
    let mut polled_n = 0u64;

    let mut take = |out: &mut Found, o: &O, s: FoundStart| -> Result<(), String> {
        found(o, &s)?;
        out.starts.push(s);
        Ok(())
    };

    for input in inputs {
        let events = o.step(std::slice::from_ref(input))?;
        let f = o.frame();
        let polled = crate::lockstep::polled(&events);
        if !started {
            match &cond {
                Some(c) => {
                    if holds(&o, c)? {
                        started = true;
                    }
                    continue;
                }
                None => {
                    if !polled {
                        continue;
                    }
                    started = true;
                }
            }
        }
        if !polled {
            continue;
        }
        model_frame += 1;
        polled_n += 1;
        let playing = match &cond {
            Some(c) => holds(&o, c)?,
            None => true,
        };
        let v = translate::read_original(&o, mapping)?;
        let on = matches!(v[ground], Value::Flag(true));
        let moving = vx.is_some_and(|i| v[i].as_int().is_some_and(|n| n != 0))
            || last_x.as_ref().is_some_and(|lx| *lx != v[xi]);
        last_x = Some(v[xi].clone());
        grounded = if on && playing { grounded + 1 } else { 0 };
        still = if on && playing && !moving {
            still + 1
        } else {
            0
        };
        if !playing {
            continue;
        }
        // A model of one level is in it whatever the mapping says.
        let Some(level) = translate::level_of(&o, mapping, &v)?
            .map(|(l, _)| l)
            .or_else(|| (spec.levels.len() == 1).then(|| spec.levels[0].name.clone()))
        else {
            continue;
        };
        if visit.as_ref().is_none_or(|vs| vs.level != level) {
            // A level done: its start taken from the fallback if it never
            // stood still.
            if let Some(vs) = visit.take()
                && !vs.started
                && let Some((fo, s)) = vs.fallback
            {
                take(&mut out, &fo, s)?;
            }
            if seen.contains(&level) {
                visit = Some(Visit {
                    level: level.clone(),
                    from: f,
                    started: true,
                    fallback: None,
                    used: Vec::new(),
                    extras: usize::MAX,
                    last: f,
                });
                continue;
            }
            seen.push(level.clone());
            out.levels.push((level.clone(), f));
            // Starts an earlier input found here: its own start is taken,
            // and what they are next to is met.
            let before: Vec<Near> = have
                .iter()
                .filter(|(l, _)| *l == level)
                .map(|(_, n)| n.clone())
                .collect();
            visit = Some(Visit {
                level: level.clone(),
                from: f,
                started: before.contains(&Near::LevelStart),
                fallback: None,
                extras: before.iter().filter(|n| **n != Near::LevelStart).count(),
                used: before,
                last: 0,
            });
        }
        let vs = visit.as_mut().expect("a visit");
        if vs.extras == usize::MAX || grounded < STILL_FOR {
            continue;
        }
        if !vs.started {
            if still < STILL_FOR && vs.fallback.is_some() {
                if f >= vs.from + WAIT_STILL {
                    let (fo, s) = vs.fallback.take().expect("a fallback");
                    vs.started = true;
                    vs.last = s.frame;
                    take(&mut out, &fo, s)?;
                }
                continue;
            }
            let t = translate::from_original(&o, spec, &base, mapping);
            let quiet = match (&t, pk) {
                (Ok(t), _) if !in_play(spec, &t.world) => false,
                (Ok(t), Some(pk)) => player_body(spec, &t.world, pk).is_some_and(|p| {
                    !t.world.bodies.iter().any(|b| {
                        b.kind != pk
                            && b.alive
                            && !b.waiting
                            && (b.x.floor() - p.x.floor()).abs() < NEAR
                    })
                }),
                (Ok(_), None) => true,
                (Err(_), _) => false,
            };
            if !quiet {
                continue;
            }
            let is_still = still >= STILL_FOR;
            let gaps = crate::carry::gaps(&o, spec, mapping)?;
            let s = FoundStart {
                frame: f,
                model_frame,
                level: level.clone(),
                near: Near::LevelStart,
                says: if is_still {
                    format!(
                        "{name} standing on the ground at {:.1} s, nothing near",
                        secs(f)
                    )
                } else {
                    format!(
                        "{name} on the ground at {:.1} s, nothing near (moving: never standing still as the level starts)",
                        secs(f)
                    )
                },
                still: is_still,
                cant: cant_start(spec, &t, pk, &name, &gaps),
            };
            if is_still {
                vs.started = true;
                vs.fallback = None;
                vs.last = f;
                take(&mut out, &o, s)?;
            } else {
                vs.fallback = Some((o.fork(), s));
            }
            continue;
        }
        if vs.extras >= opts.extra_per_level
            || f < vs.last + opts.spacing
            || !polled_n.is_multiple_of(EVERY)
        {
            continue;
        }
        // What the player is next to here, not met yet in this level.
        let t = translate::from_original(&o, spec, &base, mapping);
        let slots = slot_objects(&o, mapping)?;
        let px_now = px(&v[xi]);
        let mut near: Vec<Near> = Vec::new();
        if let (Ok(tw), Some(pk)) = (&t, pk)
            && in_play(spec, &tw.world)
            && let Some(p) = player_body(spec, &tw.world, pk)
        {
            near.extend(tiles_near(spec, &tw.world, p));
            let dir: i64 = if p.facing_left { -1 } else { 1 };
            // Ahead by 16 to 160 px, in the units the player's x is in.
            let ahead = |x: i64, from: i64| (16..=160).contains(&((x - from) * dir));
            if mapping.slots.is_some() {
                if let Some(pxn) = px_now {
                    for (e, x) in &slots {
                        if x.is_some_and(|x| ahead(x, pxn)) {
                            near.push(Near::Enemy { entity: e.clone() });
                        }
                    }
                }
            } else {
                for b in &tw.world.bodies {
                    if b.kind != pk && b.alive && !b.waiting && ahead(b.x.floor(), p.x.floor()) {
                        near.push(Near::Enemy {
                            entity: spec.kinds[b.kind].name.clone(),
                        });
                    }
                }
            }
        }
        let Some(n) = near.into_iter().find(|n| !vs.used.contains(n)) else {
            continue;
        };
        vs.used.push(n.clone());
        vs.extras += 1;
        vs.last = f;
        let what = match &n {
            Near::Pit => "a pit ahead".to_owned(),
            Near::Wall => "a wall ahead".to_owned(),
            Near::Block => "a block overhead".to_owned(),
            Near::Enemy { entity } => format!("a {} ahead", entity.replace('_', " ")),
            Near::LevelStart | Near::Context { .. } => "nothing near".to_owned(),
        };
        let s = FoundStart {
            frame: f,
            model_frame,
            level: level.clone(),
            says: format!(
                "{name} {} at {:.1} s, {what}",
                if still >= STILL_FOR {
                    "standing on the ground"
                } else {
                    "on the ground"
                },
                secs(f)
            ),
            still: still >= STILL_FOR,
            cant: cant_start(spec, &t, pk, &name, &crate::carry::gaps(&o, spec, mapping)?),
            near: n,
        };
        take(&mut out, &o, s)?;
    }
    if let Some(vs) = visit
        && !vs.started
        && let Some((fo, s)) = vs.fallback
    {
        take(&mut out, &fo, s)?;
    }
    out.starts.sort_by_key(|s| s.frame);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_is_named_in_game_words() {
        assert_eq!(Near::LevelStart.words(), "start");
        assert_eq!(Near::Pit.words(), "before the first pit");
        assert_eq!(
            Near::Enemy {
                entity: "koopa_green".into()
            }
            .words(),
            "near the first koopa green"
        );
        assert_eq!(
            Near::Enemy {
                entity: "goomba".into()
            }
            .key(),
            "enemy-goomba"
        );
    }
}
