//! Progressive asset generation (docs/host.md, "Progressive generation"):
//! fill a working copy's presentation slots in an order the catalog
//! implies, so each asset is made and reviewed against what is already
//! there.
//!
//! The order:
//!
//! 1. Palettes slots. Every sprite's colors, and the critic's contrast and
//!    harmony checks, depend on them.
//! 2. For each tiles slot, a base sheet (stub tiles) when the slot still
//!    holds an original or nothing generated, so no tile of an original is
//!    left in it.
//! 3. For each tiles slot, its sprites by role: player, then enemies and
//!    bosses, characters, pickups, projectiles, hazards, then the rest,
//!    ties by entity name. The player comes first because the other sprites
//!    are reviewed against it (size and style).
//!
//! Each sprite step tries up to `tries` seeds without placing anything,
//! reads the critic's review of each, and places the best (the first to
//! pass with `min_score`, else the highest score). Slots that the
//! simulation reads are left to `model_edit`.

use gp_model::Model;
use gp_model::model::AssetKind;
use serde_json::{Value, json};

use crate::catalog::Status;
use crate::generate::{self, DEFAULT_MIN_SCORE, GenerateArgs};
use crate::{Config, Reply, place};

/// One step of the order.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub slot: String,
    pub entity: Option<String>,
    pub role: Option<String>,
    pub generator: &'static str,
    pub why: String,
}

impl Step {
    fn json(&self, n: usize) -> Value {
        json!({
            "n": n,
            "slot": self.slot,
            "entity": self.entity,
            "role": self.role,
            "generator": self.generator,
            "why": self.why,
        })
    }
}

/// Where a role falls in the order; lower first.
pub fn role_rank(role: &str) -> u8 {
    match role {
        "player" => 0,
        "enemy" | "boss" => 1,
        "npc" => 2,
        "collectible" | "power-up" | "goal" => 3,
        "projectile" => 4,
        "hazard" => 5,
        _ => 6,
    }
}

/// The order for a model, from what its slots hold: `base_needed` says, for
/// a tiles slot, whether to make a base sheet first.
pub fn order(model: &Model, base_needed: &dyn Fn(&str) -> bool) -> Vec<Step> {
    let sim = gp_interp::lifted::simulation_slots(model);
    let is_art = |name: &str| !sim.iter().any(|s| s.as_str() == name);
    let mut steps = Vec::new();
    // Slots filled from one asset share its file: one base step each.
    let mut seen = std::collections::BTreeSet::new();
    let mut first_of = |slot: &gp_model::model::AssetSlot| match &slot.from {
        Some(f) => seen.insert((f.source.clone(), f.asset.clone())),
        None => true,
    };
    for (name, slot) in &model.assets {
        if slot.kind == AssetKind::Palettes && is_art(name) && first_of(slot) {
            steps.push(Step {
                slot: name.clone(),
                entity: None,
                role: None,
                generator: "stub",
                why: "palettes come first: sprite colors and the critic's contrast checks depend on them".into(),
            });
        }
    }
    for (name, slot) in &model.assets {
        if slot.kind != AssetKind::Tiles || !is_art(name) {
            continue;
        }
        if first_of(slot) && base_needed(name) {
            steps.push(Step {
                slot: name.clone(),
                entity: None,
                role: None,
                generator: "stub",
                why: "a base sheet, so nothing of the original stays in the slot".into(),
            });
        }
        let mut sprites: Vec<(u8, &String, &str)> = model
            .entities
            .iter()
            .filter(|(_, e)| e.core.sprite.as_ref().is_some_and(|s| &s.slot == name))
            .map(|(n, e)| (role_rank(&e.core.role), n, e.core.role.as_str()))
            .collect();
        sprites.sort();
        for (_, entity, role) in sprites {
            steps.push(Step {
                slot: name.clone(),
                entity: Some(entity.clone()),
                role: Some(role.to_owned()),
                generator: "shapes",
                why: format!("`{entity}` ({role}), after the sprites before it, so the critic compares it with them"),
            });
        }
    }
    steps
}

/// The order for a working copy.
fn plan(config: &Config, copy: &str) -> Result<(Model, Vec<Step>), String> {
    let dir = config.existing_copy(copy)?;
    let text = std::fs::read_to_string(dir.join(gp_model::MODEL_FILE))
        .map_err(|e| format!("reading {}: {e}", gp_model::MODEL_FILE))?;
    let model = Model::parse(&text).map_err(|e| format!("{}: {e}", gp_model::MODEL_FILE))?;
    let base = |slot: &str| {
        place::target(config, copy, slot, None)
            .map(|t| matches!(t.state.status, Status::Original))
            .unwrap_or(false)
    };
    let steps = order(&model, &base);
    Ok((model, steps))
}

fn step_args(a: &GenerateArgs, s: &Step, seed: u64, place: bool, critic: &str) -> GenerateArgs {
    GenerateArgs {
        copy: a.copy.clone(),
        slot: Some(s.slot.clone()),
        entity: s.entity.clone(),
        generator: Some(s.generator.to_owned()),
        seed,
        hue_shift: None,
        prompt: a.prompt.clone(),
        place,
        author: a.author.clone(),
        critic: Some(critic.to_owned()),
        min_score: a.min_score,
        progressive: false,
        plan_only: false,
        tries: None,
    }
}

pub fn run(config: &Config, a: GenerateArgs) -> Result<Reply, String> {
    let tries = a.tries.unwrap_or(4).clamp(1, 16);
    let min = a.min_score.unwrap_or(DEFAULT_MIN_SCORE);
    let off = a.critic.as_deref() == Some("off");
    let (_, steps) = plan(config, &a.copy)?;
    if steps.is_empty() {
        return Err("the copy has no presentation slot of a kind assets_generate fills".into());
    }
    let listed: Vec<Value> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| s.json(i + 1))
        .collect();
    if a.plan_only {
        return Ok(Reply::ok(
            json!({ "ok": true, "written": false, "plan": listed }),
        ));
    }
    let mut done = Vec::new();
    let mut failed = false;
    for (i, s) in steps.iter().enumerate() {
        // Whole-slot stubs have one possible result; sprites try seeds.
        let seeds = if s.generator == "shapes" && !off {
            tries
        } else {
            1
        };
        let mut best: Option<(u64, f64, bool, Value)> = None;
        for k in 0..u64::from(seeds) {
            let seed = a.seed + k;
            let r = generate::generate(
                config,
                step_args(&a, s, seed, false, if off { "off" } else { "advise" }),
            )?;
            if r.is_error {
                return Ok(Reply::refused(json!({
                    "ok": false,
                    "stage": "generate",
                    "step": s.json(i + 1),
                    "error": r.json,
                    "done": done,
                })));
            }
            let score = r.json["critic"]["score"].as_f64().unwrap_or(1.0);
            let pass = r.json["critic"]["pass"].as_bool().unwrap_or(true);
            let better = best
                .as_ref()
                .is_none_or(|(_, bs, bp, _)| (pass, score) > (*bp, *bs));
            if better {
                best = Some((seed, score, pass, r.json["critic"].clone()));
            }
            if pass && score >= min {
                break;
            }
        }
        let (seed, score, pass, review) = best.expect("at least one seed");
        let placed = generate::generate(config, step_args(&a, s, seed, true, "advise"))?;
        let ok = placed.json["written"].as_bool().unwrap_or(false);
        failed |= !ok;
        done.push(json!({
            "step": s.json(i + 1),
            "seed": seed,
            "seeds_tried": (seed - a.seed + 1),
            "score": score,
            "review_passed": pass,
            "met_min_score": pass && score >= min,
            "problems": review["checks"].as_array().map(|c| c.iter()
                .filter(|c| matches!(c["verdict"].as_str(), Some("warn" | "fail")))
                .map(|c| format!("{}: {}", c["name"].as_str().unwrap_or(""), c["message"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()),
            "written": ok,
            "simulation_unchanged": placed.json["simulation"]["unchanged"],
            "placement": if ok { Value::Null } else { placed.json },
        }));
        if !ok {
            break;
        }
    }
    let scores: Vec<f64> = done.iter().filter_map(|d| d["score"].as_f64()).collect();
    let mean = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    };
    let out = json!({
        "ok": !failed,
        "written": !done.is_empty(),
        "min_score": min,
        "steps": steps.len(),
        "mean_score": (mean * 1000.0).round() / 1000.0,
        "all_met_min_score": done.iter().all(|d| d["met_min_score"].as_bool().unwrap_or(false)),
        "done": done,
    });
    Ok(if failed {
        Reply::refused(out)
    } else {
        Reply::ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_rank_player_first_and_hazards_late() {
        let mut roles = ["hazard", "collectible", "enemy", "player", "decor", "npc"];
        roles.sort_by_key(|r| role_rank(r));
        assert_eq!(
            roles,
            ["player", "enemy", "npc", "collectible", "hazard", "decor"]
        );
    }
}
