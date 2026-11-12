//! Where an entity's sprite was seen in the recorded runs: the evidence an
//! edit works from (docs/research/agent-sprite-editor.md). Each of the
//! source's baselines is replayed in the interpreter on its recorded input,
//! and every frame the entity is active is filed under its pose: its own
//! machine state, standing or rising or falling, moving or still, and
//! facing. The game's rule state (playing, dying) and the events that
//! touch the entity are kept with the frames, so a preview can jump to the
//! frames where an edit shows (the jump, the stomp, the death).
//!
//! This is the model's replay of the recorded input, which agrees with the
//! original frame for frame where validation passes. The original's own
//! drawn data (OAM, palette RAM, the tiles behind the sprite from `gp
//! track`) is the next source; the design names it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use gp_interp::{Interp, Lifted};
use serde_json::{Value, json};

use crate::place::Target;

/// The most jump points kept per pose.
pub const MAX_MOMENTS: usize = 8;

#[derive(Default)]
struct Seen {
    frames: u64,
    first: Option<(String, u64)>,
    moments: Vec<Value>,
}

/// The poses and moments the entity was seen in, per baseline, and its
/// states never seen. Err when the source has no baselines; `seen` is 0
/// when the entity never appears in them.
pub fn contexts(t: &Target, assets: &Path, entity: &str) -> Result<Value, String> {
    let lifted = Lifted::read_dir(assets, &t.model)?;
    let mut poses: BTreeMap<String, Seen> = BTreeMap::new();
    let mut states_seen: BTreeSet<String> = BTreeSet::new();
    let mut rules_seen: BTreeSet<String> = BTreeSet::new();
    let mut per_run = Vec::new();
    let mut all_states: Vec<String> = Vec::new();
    let mut total = 0u64;
    for (name, input) in &t.runs {
        let mut rt = Interp::load(&t.model, &lifted)?;
        let kind = rt
            .spec()
            .kinds
            .iter()
            .position(|k| k.name == entity)
            .ok_or(format!("the simulation has no kind `{entity}`"))?;
        if all_states.is_empty()
            && let Some(m) = &rt.spec().kinds[kind].machine
        {
            all_states = m.states.iter().map(|s| s.name.clone()).collect();
        }
        let mut seen_here = 0u64;
        for f in input {
            let events = rt.advance(f)?;
            let (spec, w) = (rt.spec(), rt.world());
            let rule = spec
                .machines
                .first()
                .zip(w.machines.first())
                .map(|(m, s)| m.states[s.state].name.clone());
            for (i, b) in w.of_kind(kind) {
                if !b.active() {
                    continue;
                }
                seen_here += 1;
                let state = spec.kinds[kind]
                    .machine
                    .as_ref()
                    .map_or("default".to_owned(), |m| m.states[b.state].name.clone());
                let motion = if b.on_ground {
                    "ground"
                } else if b.vy.0 < 0 {
                    "rising"
                } else {
                    "falling"
                };
                let moving = if b.vx.0 != 0 { "moving" } else { "still" };
                let facing = if b.facing_left { "left" } else { "right" };
                let key = format!("{state}/{motion}/{moving}/{facing}");
                states_seen.insert(state);
                if let Some(r) = &rule {
                    rules_seen.insert(r.clone());
                }
                let s = poses.entry(key).or_default();
                s.frames += 1;
                if s.first.is_none() {
                    s.first = Some((name.clone(), w.frame));
                }
                let mine: Vec<&str> = events
                    .iter()
                    .filter(|e| e.entity == Some(i))
                    .filter_map(|e| spec.events.get(e.kind.0).map(String::as_str))
                    .collect();
                if !mine.is_empty() && s.moments.len() < MAX_MOMENTS {
                    s.moments.push(json!({
                        "baseline": name, "frame": w.frame, "events": mine, "rule": rule,
                    }));
                }
            }
        }
        total += seen_here;
        per_run.push(json!({ "baseline": name, "frames": input.len(), "seen": seen_here }));
    }
    let poses: Vec<Value> = poses
        .into_iter()
        .map(|(k, s)| {
            let (b, f) = s.first.unwrap_or_default();
            json!({ "pose": k, "frames": s.frames, "first": { "baseline": b, "frame": f }, "moments": s.moments })
        })
        .collect();
    let unseen: Vec<&String> = all_states
        .iter()
        .filter(|s| !states_seen.contains(*s))
        .collect();
    Ok(json!({
        "seen": total,
        "baselines": per_run,
        "poses": poses,
        "rule_states": rules_seen,
        "states_unseen": unseen,
    }))
}
