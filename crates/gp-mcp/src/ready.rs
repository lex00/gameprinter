//! Whether the model knows an entity's sprite well enough to edit it
//! (docs/research/agent-sprite-editor.md, "Complete enough to edit"). An
//! edit reads the model, not raw recordings, so the model has to hold every
//! part of the sprite an edit could disturb: its slot and tiles, its
//! palette, its layout, the pictures its states and clips draw, and where
//! it's drawn, each with evidence. Recordings are how those facts got
//! there, and the source's baselines are the preview's input.
//!
//! The answer lists each item as `ok`, `missing` (refuses the edit) or
//! `skipped` (a source of evidence this build doesn't have yet, such as
//! `gp gaps`), and what would fill each missing one.

use gp_model::model::AssetKind;
use serde_json::{Value, json};

use crate::place::Target;

fn item(name: &str, status: &str, detail: String, fill: Option<&str>) -> Value {
    let mut v = json!({ "item": name, "status": status, "detail": detail });
    if let Some(f) = fill {
        v["fill"] = json!(f);
    }
    v
}

/// The completeness check for one entity's sprite. `seen` is the number of
/// frames the recorded baselines show it, from [`crate::contexts`].
pub fn readiness(t: &Target, entity: &str, seen: u64) -> Value {
    let m = &t.model;
    let mut items = Vec::new();
    let Some(e) = m.entities.get(entity) else {
        return json!({ "ready": false, "items": [item("entity", "missing", format!("no entity `{entity}`"), None)] });
    };
    let Some(sp) = e.core.sprite.as_ref() else {
        return json!({ "ready": false, "items": [item("sprite", "missing", format!("`{entity}` has no sprite"), Some("model_edit: give it a sprite"))] });
    };

    let set_slot = sp.frame.as_ref().map(|f| f.slot.clone()).filter(|s| {
        m.assets
            .get(s)
            .is_some_and(|a| a.kind == AssetKind::SpriteSet)
    });
    items.push(if let Some(set) = set_slot {
        item(
            "tiles",
            "ok",
            format!(
                "frames of the sprite set `{set}` (gp sprites capture), laid out from `{}`",
                sp.slot
            ),
            None,
        )
    } else if sp.tiles.is_empty() {
        item(
            "tiles",
            "missing",
            "the sprite names a row of a rows slot, which isn't resolved to tiles for an edit"
                .into(),
            Some("gp sprites capture: frames as tile layouts in a sprite-set slot"),
        )
    } else {
        item(
            "tiles",
            "ok",
            format!(
                "{} tiles, {} a row, in `{}`",
                sp.tiles.len(),
                sp.width,
                sp.slot
            ),
            None,
        )
    });

    let palettes = m.assets.iter().find(|(_, s)| s.kind == AssetKind::Palettes);
    items.push(match palettes {
        None => item(
            "palette",
            "missing",
            "the model has no palettes slot, so the sprite's colors aren't known".into(),
            Some("lift the palettes and add a palettes slot"),
        ),
        Some((name, slot)) => {
            let roles = slot.item_roles.len();
            if roles > 0 && sp.palette as usize >= roles {
                item(
                    "palette",
                    "missing",
                    format!("palette {} is past `{name}`'s {roles} palettes", sp.palette),
                    None,
                )
            } else {
                item(
                    "palette",
                    "ok",
                    format!("palette {} of `{name}`", sp.palette),
                    None,
                )
            }
        }
    });

    items.push(
        if !sp.cells.is_empty() && sp.cells.len() != sp.tiles.len() {
            item(
                "layout",
                "missing",
                format!(
                    "{} cells for {} tiles: the tile layout doesn't say how each tile is drawn",
                    sp.cells.len(),
                    sp.tiles.len()
                ),
                Some("capture the layout from OAM (docs/capture.md, Sprite layout)"),
            )
        } else {
            item(
                "layout",
                "ok",
                if sp.cells.is_empty() {
                    "every tile drawn as it is".to_owned()
                } else {
                    format!("{} cells with their own flips or palettes", sp.cells.len())
                },
                None,
            )
        },
    );

    // Each state draws the entity's sprite unless it sets its own; a state
    // that sets one is a picture this edit doesn't touch.
    let mut own = Vec::new();
    let mut base = Vec::new();
    if let Some(machine) = &e.core.machine {
        for (name, s) in &machine.states {
            if s.set.contains_key("sprite") {
                own.push(name.clone());
            } else {
                base.push(name.clone());
            }
        }
    }
    items.push(item(
        "states",
        "ok",
        if e.core.machine.is_none() {
            "no state machine: one picture in every state".to_owned()
        } else {
            format!("drawn with this sprite in {base:?}; states with their own sprite: {own:?}")
        },
        None,
    ));
    items.push(item(
        "clips",
        "ok",
        if sp.clips.is_empty() {
            "no animation: the one frame".to_owned()
        } else {
            format!(
                "{} clips; an edit to the sprite's own frame shows where none holds",
                sp.clips.len()
            )
        },
        None,
    ));

    let spawned = m
        .levels
        .values()
        .any(|l| l.core.spawn.iter().any(|s| s.entity == entity));
    items.push(if spawned || seen > 0 {
        item(
            "drawn",
            "ok",
            format!(
                "{}; {seen} frames in the recorded baselines",
                if spawned { "spawned in a level" } else { "not spawned directly" }
            ),
            None,
        )
    } else {
        item(
            "drawn",
            "missing",
            "no level spawns it and no baseline shows it, so the model doesn't say where it's drawn".into(),
            Some("record a run that shows it (gp host record, gp record) and capture its spawns"),
        )
    });

    let prefix = format!("entities.{entity}");
    let unexplained: Vec<String> = gp_model::validate::evidence_for(m)
        .into_iter()
        .filter(|(p, ev)| {
            let s = gp_model::path::display(p);
            ev.is_none()
                && (s.starts_with(&format!("{prefix}.sprite"))
                    || s.starts_with(&format!("{prefix}.size"))
                    || s.starts_with(&format!("{prefix}.hitbox")))
        })
        .map(|(p, _)| gp_model::path::display(&p))
        .collect();
    items.push(if unexplained.is_empty() {
        item(
            "evidence",
            "ok",
            "every sprite, size and hitbox fact has evidence".into(),
            None,
        )
    } else {
        item(
            "evidence",
            "missing",
            format!("no evidence for {unexplained:?}"),
            Some("the capture loop: gp capture (track, derive, questions) or model_refine"),
        )
    });
    items.push(item(
        "gaps",
        "skipped",
        "gp gaps isn't in this build; when it is, nothing it lists may name this entity's sprites"
            .into(),
        None,
    ));

    let ready = items.iter().all(|i| i["status"] != "missing");
    json!({ "ready": ready, "items": items })
}
