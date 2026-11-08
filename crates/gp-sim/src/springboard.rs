//! The springboard (platformer 0.4.4): a body that holds a player who
//! stands on it for the frames of its steps, moving the player and itself
//! a set amount each frame, and then throws the player up, higher when an
//! action is pressed during the press. Super Mario Bros.' jumpspring
//! (JumpspringHandler, ChkForLandJumpSpring) is one; any game with a
//! spring or a trampoline that holds the player can say the same.
//!
//! It runs in two places. In `platformer.enemies`, each springboard's turn
//! ([`step`]) starts a press under a player standing on it and runs one
//! frame of the press. In `platformer.player`, a player a springboard
//! holds ([`holder`]) changes its speed by its run but doesn't move by it,
//! can't jump and has no gravity, and a player thrown by a springboard
//! with `throw_late` ([`thrown_late`]) stands one more frame.
//!
//! A springboard keeps its press in its body's mechanic words: word 0 is
//! the frames of the press run so far (0 at rest), word 1 the player it
//! holds plus one, word 2 is 1 once the boost counts. A body that is a
//! springboard has no `ext` hook. Its `on_ground` is true at rest and
//! false while it presses, so a sprite clip can show the press.

use crate::spec::{Kind, Role, Spec, Springboard};
use crate::world::{Body, World};

/// The frames of a whole press.
pub fn frames(s: &Springboard) -> i64 {
    s.steps.iter().map(|st| i64::from(st.frames)).sum()
}

/// The step frame `n` of the press is in, counted from 0.
fn step_at(s: &Springboard, n: i64) -> usize {
    let mut left = n;
    for (i, st) in s.steps.iter().enumerate() {
        left -= i64::from(st.frames);
        if left < 0 {
            return i;
        }
    }
    s.steps.len().saturating_sub(1)
}

/// The springboard holding player `i`, if one is: an active body with a
/// springboard, pressing, whose word 1 names `i`.
pub fn holder(spec: &Spec, w: &World, i: usize) -> Option<usize> {
    let mark = i as i64 + 1;
    w.bodies.iter().position(|b| {
        b.active() && b.ext[0] > 0 && b.ext[1] == mark && spec.kind(b).springboard.is_some()
    })
}

/// Whether any entity of the model, in any state, is a springboard that
/// throws late.
fn any_throw_late(spec: &Spec) -> bool {
    let late = |k: &Kind| k.springboard.as_ref().is_some_and(|s| s.throw_late);
    spec.kinds.iter().any(|k| {
        late(k)
            || k.machine
                .as_ref()
                .is_some_and(|m| m.states.iter().any(|s| late(&s.kind)))
    })
}

/// Whether player `i` starts this frame thrown by a springboard that
/// throws late: in a model with one, standing with an upward speed, and
/// not jumping this frame (a jump from there is an ordinary jump). Such a
/// player runs as on the ground but doesn't move up, and leaves the ground
/// ([`leave`]); it rises from the next frame.
pub fn thrown_late(spec: &Spec, w: &World, i: usize) -> bool {
    let b = &w.bodies[i];
    if !b.on_ground || b.vy >= crate::Fx::ZERO || !any_throw_late(spec) {
        return false;
    }
    let jumps = spec
        .kind(b)
        .jump
        .as_ref()
        .is_some_and(|j| w.pressed(spec, j.action) || b.buffer > 0);
    !jumps
}

/// A thrown player leaves the ground: in the air after a jump, with no
/// held gravity and no coyote time.
fn leave(b: &mut Body) {
    b.on_ground = false;
    b.air = u32::MAX;
    b.held_phase = false;
    b.buffer = 0;
}

/// [`thrown_late`]'s frame: the player leaves the ground where it is.
pub fn leave_late(w: &mut World, i: usize) {
    leave(&mut w.bodies[i]);
}

/// Whether player body `p` stands on springboard body `s`: on the ground
/// and riding it, or with its tile hitbox's bottom on the springboard's
/// top as placed and the two overlapping across.
fn stands_on(spec: &Spec, w: &World, p: usize, s: usize) -> bool {
    let (pb, sb) = (&w.bodies[p], &w.bodies[s]);
    if !pb.active() || !pb.on_ground || pb.climbing || pb.vy < crate::Fx::ZERO {
        return false;
    }
    if pb.riding == s as u32 + 1 {
        return true;
    }
    let (pk, sk) = (spec.kind(pb), spec.kind(sb));
    let th = pk.tile_hitbox;
    let bottom = pb.y.floor() + th.y.floor() + th.h.floor();
    let top = sb.home[1].floor() + sk.hitbox.y.floor();
    let (pl, pr) = (
        pb.x.floor() + th.x.floor(),
        pb.x.floor() + th.x.floor() + th.w.floor(),
    );
    let (sl, sr) = (
        sb.x.floor() + sk.hitbox.x.floor(),
        sb.x.floor() + sk.hitbox.x.floor() + sk.hitbox.w.floor(),
    );
    bottom == top && pl < sr && sl < pr
}

/// A springboard's turn, in `platformer.enemies`. At rest, it starts a
/// press under the first player standing on it. Pressing (the frame it
/// starts included), the frame's step moves the player by its `move` and
/// puts the springboard at its `offset` from where it was placed; from the
/// boost's step on, its action (newly pressed, or held with `held`) turns
/// the boost on. The press's last frame throws the player at the boost's
/// speed or the plain one, and the springboard rests. Without
/// `throw_late` the player leaves the ground then; with it the player
/// stands one more frame ([`thrown_late`]).
pub fn step(spec: &Spec, w: &mut World, i: usize) {
    let Some(s) = spec.kind(&w.bodies[i]).springboard.as_ref() else {
        return;
    };
    let total = frames(s);
    if w.bodies[i].ext[0] <= 0 {
        let player = (0..w.bodies.len()).find(|&p| {
            p != i && spec.kinds[w.bodies[p].kind].role == Role::Player && stands_on(spec, w, p, i)
        });
        let Some(p) = player else {
            w.bodies[i].on_ground = true;
            return;
        };
        let b = &mut w.bodies[i];
        b.ext = [0, p as i64 + 1, 0, b.ext[3]];
    }
    let p = (w.bodies[i].ext[1] - 1) as usize;
    if total <= 0 || p >= w.bodies.len() || !w.bodies[p].active() {
        rest(w, i);
        return;
    }
    let n = w.bodies[i].ext[0];
    let k = step_at(s, n);
    let st = s.steps[k];
    w.bodies[p].y = w.bodies[p].y.add(st.moves);
    w.bodies[p].vy = crate::Fx::ZERO;
    w.bodies[p].on_ground = true;
    if let Some(boost) = s.boost
        && k as u32 >= boost.from_step
        && (if boost.held {
            w.held(spec, boost.action)
        } else {
            w.pressed(spec, boost.action)
        })
    {
        w.bodies[i].ext[2] = 1;
    }
    let home = w.bodies[i].home[1];
    w.bodies[i].y = home.add(st.offset);
    if n + 1 >= total {
        let speed = match s.boost {
            Some(boost) if w.bodies[i].ext[2] != 0 => boost.speed,
            _ => s.speed,
        };
        let pb = &mut w.bodies[p];
        pb.vy = speed;
        if !s.throw_late {
            leave(pb);
        }
        rest(w, i);
        w.bodies[i].on_ground = true;
    } else {
        w.bodies[i].ext[0] = n + 1;
        w.bodies[i].on_ground = false;
    }
}

/// Puts a springboard at rest.
fn rest(w: &mut World, i: usize) {
    let b = &mut w.bodies[i];
    b.ext = [0, 0, 0, b.ext[3]];
}
