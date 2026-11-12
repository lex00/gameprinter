//! Reachability checks ([`Kind::Reach`]): a level is playable under its
//! player's mechanics when the goal can be reached from the spawn by the
//! player's own moves, found by `gp_interp::reach` on the model with its
//! hooks (docs/mechanics.md, "Reachability"). They check the model alone,
//! so they run once per validation, not per recording, and they're
//! deterministic, so a level a model change makes unreachable fails CI.
//!
//! The search samples moves and ignores enemies, exits and momentum
//! carried between moves (gp_interp::reach), so a fail means no sampled
//! move gets there. Its moves walk and jump, so for a player that climbs
//! a miss is left unmeasured.

use gp_interp::variant::Sim;
use gp_sim::Spec;

use crate::compile::{Kind, Plan};
use crate::evaluate::{Against, Outcome, Status};

/// The reach checks of `plan` on the compiled model `spec` with its hooks:
/// one outcome per level whose `reachable` quantity a contract level that
/// checks it governs.
pub fn outcomes(plan: &Plan, spec: &Spec, sim: &dyn Sim) -> Vec<Outcome> {
    let mut out = Vec::new();
    for check in plan.checks.iter().filter(|c| c.kind == Kind::Reach) {
        let Some((g, _)) = check.levels.get("reachable") else {
            continue;
        };
        let mut o = Outcome {
            part: check.part.clone(),
            kind: Kind::Reach.name().to_owned(),
            quantity: "reachable".into(),
            case: None,
            against: Against::Model,
            level: "none".into(),
            status: Status::Unexercised,
            samples: 0,
            expected: None,
            actual: None,
            tolerance: None,
            frame: None,
            detail: None,
        };
        if !Kind::Reach.checks_at(g.level) {
            continue;
        }
        let Some(index) = spec.levels.iter().position(|l| l.name == check.name) else {
            o.detail = Some("the compiled model has no such level".into());
            out.push(o);
            continue;
        };
        o.level = g.level.name().into();
        o.expected = Some("goal reachable".into());
        match gp_interp::reach::reach(spec, index, sim) {
            Ok(r) => {
                o.samples = r.moves;
                match &r.goal_path {
                    Some(path) => {
                        o.status = Status::Pass;
                        o.actual = Some("goal reachable".into());
                        o.detail = Some(format!(
                            "from the spawn in {} stands ({} moves, {} frames searched)",
                            path.len(),
                            r.moves,
                            r.frames
                        ));
                    }
                    // The search's moves walk and jump; none climbs, so a
                    // goal a ladder leads to (Donkey Kong's Pauline) is
                    // never found, and a miss says nothing about a player
                    // that climbs.
                    None if climbs(spec) => {
                        o.detail = Some(format!(
                            "not measured: the player climbs, and the search's moves only walk and jump ({} places reached, {} moves); a clip that reaches the goal is the measure",
                            r.stands.len(),
                            r.moves
                        ));
                    }
                    None => {
                        o.status = Status::Fail;
                        let furthest = r.stands.iter().map(|s| s[0]).max().unwrap_or(0);
                        o.actual = Some("not reached".into());
                        o.detail = Some(format!(
                            "no sampled move reaches the goal; {} places reached, the furthest at x {furthest} ({} moves, {} deaths)",
                            r.stands.len(),
                            r.moves,
                            r.deaths
                        ));
                    }
                }
            }
            Err(e) => o.detail = Some(e),
        }
        out.push(o);
    }
    out
}

/// Whether the model's player has a climb, which the search's moves
/// don't use.
fn climbs(spec: &Spec) -> bool {
    spec.kinds
        .iter()
        .any(|k| k.role == gp_sim::spec::Role::Player && k.climb.is_some())
}

/// Whether any level's reachability is checked, so a caller knows to
/// compile the model.
pub fn wanted(plan: &Plan) -> bool {
    plan.checks.iter().any(|c| {
        c.kind == Kind::Reach
            && c.levels
                .get("reachable")
                .is_some_and(|(g, _)| Kind::Reach.checks_at(g.level))
    })
}
