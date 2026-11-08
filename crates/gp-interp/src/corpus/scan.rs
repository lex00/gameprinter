//! Starts for each context, found in a recording of the original
//! (docs/research/clip-corpus.md, section 3, "Finding a start for a
//! context"). One pass plays the input on the original and, on each frame
//! it reads the controller after the mapping's start holds, reads the
//! mapped measures; every fourth such frame it also translates the
//! original's state, for what only the level's tiles or the bodies can
//! say (a ledge, water, a ladder, an enemy ahead, carrying, a machine
//! state). A context is read from the original, never from the model, so
//! a broken model can't decide which clips exist.
//!
//! Each context keeps the first frame that holds in each level, up to a
//! few levels, so two starts can come from different levels. A start the
//! rebuilt game can't begin at is kept with the reason.

use gp_runtime::{InputFrame, Runtime, Value};
use gp_sim::spec::Tile;
use gp_sim::{Spec, World};

use crate::mapping::Mapping;
use crate::starts::{cant_start, holds, in_play, player_body, slot_objects};
use crate::translate;

/// A context to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// On the ground, not moving for 10 frames, nothing near.
    Still,
    /// On the ground for 10 frames with nothing near, at any speed: a
    /// start a lead-in can stand still from.
    Ground,
    WalkTop,
    RunTop,
    Ledge,
    AirRising,
    AirFalling,
    Enemy(String),
    Exit,
    Water,
    Ladder,
    Carrying,
    /// The player's machine in this state, by index.
    State(usize),
}

/// One frame a context holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub context: String,
    pub frame: u64,
    pub model_frame: u64,
    pub level: String,
    pub facing_left: bool,
    pub cant: Option<String>,
}

/// How many frames on the ground, or still, before a frame counts.
const FOR: u32 = 10;
/// Frames the speed must stay the same to be a top speed.
const PLATEAU: u32 = 4;
/// Nothing within this many pixels for a quiet start.
const NEAR: i64 = 80;
/// Translations every this many polled frames.
const EVERY: u64 = 4;

/// Finds the contexts `wants` (by ID) in `inputs` on `original` (at
/// power-on): the first frame each holds in each level, at most `per`
/// levels with a start the rebuilt game can begin at and one without.
/// `run_mask` is the run action's button bits, for telling a walk from a
/// run; `hit` hears each frame kept, with the original standing there.
#[allow(clippy::too_many_arguments)]
pub fn scan<O: Runtime>(
    original: &O,
    spec: &Spec,
    mapping: &Mapping,
    inputs: &[InputFrame],
    player: &str,
    run_mask: u32,
    wants: &[(String, Want)],
    per: usize,
    hit: &mut dyn FnMut(&O, &Hit) -> Result<(), String>,
) -> Result<Vec<Hit>, String> {
    let idx = |field: &str| {
        mapping
            .fields
            .iter()
            .position(|f| f.path == format!("entities.{player}[0].{field}"))
    };
    let (Some(gi), Some(xi)) = (idx("on_ground"), idx("x")) else {
        return Err(format!(
            "the state mapping has no on_ground or x for {player}, so no context can be read"
        ));
    };
    let (vxi, vyi, yi) = (idx("vx"), idx("vy"), idx("y"));
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
    let base = World::power_on(spec);
    let mut o = original.fork();
    let mut out: Vec<Hit> = Vec::new();
    let mut started = false;
    let mut model_frame = 0u64;
    let mut polled_n = 0u64;
    let (mut grounded, mut still, mut plateau) = (0u32, 0u32, 0u32);
    let mut last_x: Option<i64> = None;
    let mut last_y: Option<i64> = None;
    let mut last_v: Option<i64> = None;
    for input in inputs {
        let events = o.step(std::slice::from_ref(input))?;
        let polled = crate::lockstep::polled(&events);
        if !started {
            match &cond {
                Some(c) => {
                    if holds(&o, c)? {
                        started = true;
                    }
                    continue;
                }
                None if !polled => continue,
                None => started = true,
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
        let on = matches!(v[gi], Value::Flag(true));
        let x = match &v[xi] {
            Value::Fixed(r) => Some(*r),
            Value::Int(n) => Some(n << 16),
            _ => None,
        };
        let y = yi.and_then(|i| match &v[i] {
            Value::Fixed(r) => Some(*r),
            Value::Int(n) => Some(n << 16),
            _ => None,
        });
        // The speed across: mapped, or else how far x moved.
        let vx = match vxi.map(|i| &v[i]) {
            Some(Value::Fixed(r)) => Some(*r),
            Some(Value::Int(n)) => Some(n << 16),
            _ => match (x, last_x) {
                (Some(a), Some(b)) => Some(a - b),
                _ => None,
            },
        };
        let vy = match vyi.map(|i| &v[i]) {
            Some(Value::Fixed(r)) => Some(*r),
            Some(Value::Int(n)) => Some(n << 16),
            _ => match (y, last_y) {
                (Some(a), Some(b)) => Some(a - b),
                _ => None,
            },
        };
        let moving = vx.is_some_and(|s| s != 0) || last_x.is_some_and(|l| Some(l) != x);
        plateau = if on && playing && vx.is_some_and(|s| s != 0) && vx == last_v {
            plateau + 1
        } else {
            0
        };
        last_x = x;
        last_y = y;
        last_v = vx;
        grounded = if on && playing { grounded + 1 } else { 0 };
        still = if on && playing && !moving {
            still + 1
        } else {
            0
        };
        if !playing || !polled_n.is_multiple_of(EVERY) {
            continue;
        }
        let f = o.frame();
        let Some(level) = translate::level_of(&o, mapping, &v)?
            .map(|(l, _)| l)
            .or_else(|| (spec.levels.len() == 1).then(|| spec.levels[0].name.clone()))
        else {
            continue;
        };
        // Which wants could still take a frame in this level.
        let open: Vec<&(String, Want)> = wants
            .iter()
            .filter(|(id, _)| {
                let mine: Vec<&Hit> = out.iter().filter(|h| h.context == *id).collect();
                !mine.iter().any(|h| h.level == level)
                    && (mine.iter().filter(|h| h.cant.is_none()).count() < per
                        || !mine.iter().any(|h| h.cant.is_some()))
            })
            .collect();
        if open.is_empty() {
            continue;
        }
        let run_held = input.buttons & run_mask != 0 && run_mask != 0;
        // Left or right held (port 1: right is bit 7, left bit 6).
        let dir_held = input.buttons & 0xC0 != 0;
        let t = translate::from_original(&o, spec, &base, mapping);
        let Ok(tw) = &t else { continue };
        if !in_play(spec, &tw.world) {
            continue;
        }
        let Some(pk) = pk else { continue };
        let Some(p) = player_body(spec, &tw.world, pk) else {
            continue;
        };
        let slots = slot_objects(&o, mapping)?;
        let near = |d: i64| {
            tw.world.bodies.iter().any(|b| {
                b.kind != pk && b.alive && !b.waiting && (b.x.floor() - p.x.floor()).abs() < d
            })
        };
        let kind = spec.kind(p);
        let hb = kind.tile_hitbox;
        let left = p.x.add(hb.x).floor();
        let right = p.x.add(hb.x).add(hb.w).floor();
        let top = p.y.add(hb.y).floor();
        let feet = p.y.add(hb.y).add(hb.h).floor();
        let mid = ((left + right) / 2, (top + feet) / 2);
        let dir: i64 = if p.facing_left { -1 } else { 1 };
        let front = if p.facing_left { left } else { right };
        let ground = |t: Tile| t.solid() || t == Tile::Top;
        for (id, w) in open {
            let ok = match w {
                Want::Still => still >= FOR && !near(NEAR),
                Want::Ground => grounded >= FOR && !near(NEAR),
                Want::WalkTop => plateau >= PLATEAU && dir_held && !run_held,
                Want::RunTop => plateau >= PLATEAU && dir_held && run_held,
                Want::Ledge => {
                    grounded >= 4
                        && (2..=12).step_by(2).any(|d| {
                            let xx = front + d * dir;
                            !(feet + 1..feet + 17)
                                .step_by(4)
                                .any(|yy| ground(tw.world.tile_at(spec, xx, yy)))
                        })
                }
                Want::AirRising => !on && vy.is_some_and(|s| s < 0),
                Want::AirFalling => !on && vy.is_some_and(|s| s > 0),
                Want::Enemy(e) => {
                    let ahead = |bx: i64, from: i64| (16..=64).contains(&((bx - from) * dir));
                    on && if mapping.slots.is_some() {
                        let pxn = x.map(|r| r >> 16);
                        slots.iter().any(|(k, sx)| {
                            k == e && matches!((sx, pxn), (Some(a), Some(b)) if ahead(*a, b))
                        })
                    } else {
                        tw.world.bodies.iter().any(|b| {
                            b.alive
                                && !b.waiting
                                && spec.kinds[b.kind].name == *e
                                && ahead(b.x.floor(), p.x.floor())
                        })
                    }
                }
                Want::Exit => spec.levels.get(tw.world.level).is_some_and(|l| {
                    l.exits.iter().any(|ex| {
                        ex.hold.is_some() && {
                            let (ax, ay) = (ex.at.x.floor(), ex.at.y.floor());
                            let (aw, ah) = (ex.at.w.floor(), ex.at.h.floor());
                            left < ax + aw && right > ax && top < ay + ah && feet > ay
                        }
                    })
                }),
                Want::Water => tw.world.tile_at(spec, mid.0, mid.1) == Tile::Water,
                Want::Ladder => p.climbing || tw.world.tile_at(spec, mid.0, mid.1) == Tile::Ladder,
                Want::Carrying => p.carrying != 0,
                Want::State(s) => on && p.state == *s,
            };
            if !ok {
                continue;
            }
            let gaps = crate::carry::gaps(&o, spec, mapping)?;
            let mut cant = cant_start(spec, &t, Some(pk), &name, &gaps);
            if matches!(w, Want::AirRising | Want::AirFalling) {
                cant = Some(
                    "the jump's state in the air (its band and whether it is held) isn't carried over, so its clips start on the ground and jump".into(),
                );
            }
            let mine: Vec<&Hit> = out.iter().filter(|h| h.context == *id).collect();
            let room = match &cant {
                None => mine.iter().filter(|h| h.cant.is_none()).count() < per,
                Some(_) => !mine.iter().any(|h| h.cant.is_some()),
            };
            if !room {
                continue;
            }
            let h = Hit {
                context: id.clone(),
                frame: f,
                model_frame,
                level: level.clone(),
                facing_left: p.facing_left,
                cant,
            };
            hit(&o, &h)?;
            out.push(h);
        }
    }
    Ok(out)
}

/// A start at a frame of `inputs` the caller names (a clip recipe's, a
/// save point's): the original from power-on played to the first frame at
/// or after `frame` that reads the controller once the mapping's start
/// holds, and the model's frame there counted as [`scan`] counts it. The
/// start's snapshot and the model's frame are all a clip needs to begin
/// there without playing the input again.
pub fn at_frame<O: Runtime>(
    original: &O,
    mapping: &Mapping,
    inputs: &[InputFrame],
    frame: u64,
) -> Result<(O, u64), String> {
    at_frame_from(
        original.fork(),
        mapping,
        inputs,
        0,
        ScanAt::default(),
        frame,
    )
}

/// Where [`at_frame`]'s count stands after some frames: whether the
/// mapping's start has held, and the model's frames counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanAt {
    pub started: bool,
    pub model_frame: u64,
}

fn start_of(mapping: &Mapping) -> Result<Option<gp_runtime::select::Condition>, String> {
    mapping
        .start
        .as_deref()
        .map(gp_runtime::select::Condition::parse)
        .transpose()
}

/// One frame of [`at_frame`]'s count: whether the model stands for it.
fn scan_step<O: Runtime>(
    o: &mut O,
    cond: Option<&gp_runtime::select::Condition>,
    input: &InputFrame,
    s: &mut ScanAt,
) -> Result<bool, String> {
    let events = o.step(std::slice::from_ref(input))?;
    let polled = crate::lockstep::polled(&events);
    if !s.started {
        match cond {
            Some(c) => {
                if holds(o, c)? {
                    s.started = true;
                }
                return Ok(false);
            }
            None if !polled => return Ok(false),
            None => s.started = true,
        }
    }
    if !polled {
        return Ok(false);
    }
    s.model_frame += 1;
    Ok(true)
}

/// [`at_frame`]'s count over the first `n` frames of `inputs`, `o` left
/// after them: what [`at_frame_from`] goes on from (a start snapshot
/// there). An error when the start is found within them.
pub fn at_frame_prefix<O: Runtime>(
    o: &mut O,
    mapping: &Mapping,
    inputs: &[InputFrame],
    n: usize,
    frame: u64,
) -> Result<ScanAt, String> {
    let cond = start_of(mapping)?;
    let mut s = ScanAt::default();
    for input in &inputs[..n.min(inputs.len())] {
        if scan_step(o, cond.as_ref(), input, &mut s)? && o.frame() >= frame {
            return Err(format!("the start is found before frame {n}"));
        }
    }
    Ok(s)
}

/// [`at_frame`] on an original that stands after the first `at` frames of
/// `inputs`, with the count `s` there ([`at_frame_prefix`]).
pub fn at_frame_from<O: Runtime>(
    mut o: O,
    mapping: &Mapping,
    inputs: &[InputFrame],
    at: usize,
    mut s: ScanAt,
    frame: u64,
) -> Result<(O, u64), String> {
    let cond = start_of(mapping)?;
    for input in inputs.iter().skip(at) {
        if scan_step(&mut o, cond.as_ref(), input, &mut s)? && o.frame() >= frame {
            return Ok((o, s.model_frame));
        }
    }
    Err(format!(
        "the input ends at frame {} before it reaches a frame the model stands for at or after {frame}",
        o.frame()
    ))
}
