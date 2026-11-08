//! Every model path in one of four classes, by rules over the schema
//! (docs/mechanics.md, "Classes"; the research is docs/research/
//! mechanics.md, section 2):
//!
//! | Class | What it is | Test |
//! |---|---|---|
//! | mechanic | A rule the simulation runs, the same for every level: which components an entity has, the system order, a machine's states and transitions | Its code is in `gp-sim` or a hook |
//! | parameter | A number or choice in a mechanic's signature | Changing it changes a measure, and no level's layout |
//! | content | Data a mechanic reads that places or shapes things in a level: grids, object lists, spawns, goals, exits, the level graph, and the asset slots levels read | It can be replaced by other data of its kind with no code changed, and a state hash changes |
//! | presentation | Data the simulation never reads: sprites, art slots, level palettes, the frame rate | A state hash doesn't change when it changes |
//!
//! Each path also names the mechanic it belongs to, by record name
//! (`platformer.jump`, `core.camera`; for an `ext` hook, `ext:<name>`, which
//! gp-interp resolves to the hook's record), so a proposal or a question
//! about a path can say which mechanic it is about. The evidence table and
//! descriptions aren't classified, as they aren't facts.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::model::{AssetKind, Model};
use crate::path::{Concrete, Step, display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Mechanic,
    Parameter,
    Content,
    Presentation,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mechanic => "mechanic",
            Self::Parameter => "parameter",
            Self::Content => "content",
            Self::Presentation => "presentation",
        }
    }
}

/// One path's class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Classified {
    pub path: String,
    pub class: Class,
    /// The mechanic it belongs to, by record name, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mechanic: Option<String>,
    /// Why, when the class needs a word: `interface`, `role`, `scale`,
    /// `mixed`, `ext table`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// The platformer components, which are written under an entity by their
/// own names.
const PLATFORMER: &[&str] = crate::platformer::EXTENSION.entity_components;

/// The slots a level or a hook reads, which are content; the rest are
/// presentation. The same rule as gp-interp's `simulation_slots`.
pub fn simulation_slots(model: &Model) -> BTreeSet<String> {
    let mut slots = BTreeSet::new();
    for level in model.levels.values() {
        slots.extend(level.core.grid.iter().cloned());
        slots.extend(level.core.tileset.iter().cloned());
        slots.extend(level.core.layout.iter().map(|l| l.slot.clone()));
        slots.extend(level.core.spawn_objects.iter().map(|l| l.slot.clone()));
        slots.extend(level.core.spawn.iter().filter_map(|s| s.table.clone()));
    }
    for e in model.entities.values() {
        for t in e.core.ext_tables.values() {
            let slot = t.split_once('.').map_or(t.as_str(), |(s, _)| s);
            slots.insert(slot.to_owned());
        }
        slots.extend(e.core.ext_rows.values().cloned());
    }
    slots
}

fn key(step: Option<&Step>) -> &str {
    match step {
        Some(Step::Key(k)) => k,
        _ => "",
    }
}

/// The class of one concrete path in `model`.
pub fn classify_path(model: &Model, path: &Concrete, sim: &BTreeSet<String>) -> Classified {
    use Class::*;
    let at = |i: usize| key(path.get(i));
    let mk = |class: Class, mechanic: Option<String>, note: Option<&'static str>| Classified {
        path: display(path),
        class,
        mechanic,
        note,
    };
    let m = |s: &str| Some(s.to_owned());
    match at(0) {
        "model" => match at(1) {
            "systems" | "view" | "extensions" => mk(Mechanic, None, None),
            "start_level" => mk(Content, None, None),
            _ => mk(Presentation, None, None),
        },
        // The pad's direction latch (`input.keep_direction`) is how the
        // game reads the pad, a mechanic a switch may turn off.
        "input" if at(1) == "keep_direction" => mk(Mechanic, m("core.input"), None),
        "input" => mk(Parameter, None, Some("interface")),
        "assets" => {
            let slot = at(1);
            let kind = model.assets.get(slot).map(|s| s.kind);
            if sim.contains(slot) {
                let note = (kind == Some(AssetKind::Metatiles)).then_some("mixed");
                let ext_table = model
                    .entities
                    .values()
                    .flat_map(|e| e.core.ext_tables.values().chain(e.core.ext_rows.values()))
                    .any(|t| t.split_once('.').map_or(t.as_str(), |(s, _)| s) == slot);
                mk(
                    Content,
                    None,
                    if ext_table { Some("ext table") } else { note },
                )
            } else {
                mk(Presentation, None, None)
            }
        }
        "entities" => {
            let entity = model.entities.get(at(1));
            let ext = entity
                .and_then(|e| e.core.ext.as_ref())
                .map(|x| format!("ext:{x}"));
            let part = at(2);
            match part {
                "role" => mk(Mechanic, None, Some("role")),
                "sprite" => mk(Presentation, None, None),
                "size" | "tile_hitbox" => mk(Parameter, m("core.tile_collision"), None),
                "tile_collision" => mk(Mechanic, m("core.tile_collision"), None),
                "hitbox" => mk(Parameter, m("platformer.contact"), None),
                "ext" => mk(Mechanic, ext, None),
                "ext_params" => mk(Parameter, ext, None),
                "ext_tables" | "ext_rows" => mk(Parameter, ext, Some("ext table")),
                "despawn_behind" | "despawn_ahead" => mk(Parameter, m("core.spawn_by_view"), None),
                "no_slot" | "extra_slots" => mk(Mechanic, m("core.spawn_by_view"), None),
                "machine" => machine(path, 3, "core.modes"),
                p if PLATFORMER.contains(&p) => {
                    let name = format!("platformer.{p}");
                    // The component's own switches (an action it reads) are
                    // its interface; the rest its parameters.
                    let note = matches!(at(3), "action" | "run_action").then_some("interface");
                    mk(Parameter, Some(name), note)
                }
                _ => mk(Mechanic, None, None),
            }
        }
        "levels" => match at(2) {
            "present" => mk(Presentation, None, None),
            "tile_size" => mk(Parameter, None, Some("scale")),
            "kill_below" => mk(Parameter, m("platformer.pit"), None),
            "spawn_ahead" | "despawn_behind" | "spawn_slots" | "draw" => {
                mk(Parameter, m("core.spawn_by_view"), None)
            }
            "spawn_in_order" | "appear_still" | "despawn_at" | "despawn_after_move"
            | "hard_spawns" | "scroll_stop" | "loop_back" => {
                mk(Mechanic, m("core.spawn_by_view"), None)
            }
            "goal" => mk(Content, m("platformer.goal"), None),
            "exit" => mk(Content, m("core.exits"), None),
            _ => mk(Content, None, None),
        },
        "cameras" => match at(2) {
            "size" | "lead" => mk(Parameter, m("core.camera"), None),
            _ => mk(Mechanic, m("core.camera"), None),
        },
        // The top-level `present` (the screen, the HUD, the changed cells'
        // and effects' pictures, sound) is drawn, never read by the
        // simulation.
        "present" => mk(Presentation, None, None),
        "rules" => machine(path, 2, "core.flow"),
        // Which level plays each of the original's stages (feature `stages`).
        "stages" => mk(Content, None, None),
        "counters" => match at(2) {
            "change" => match key(path.get(4)) {
                "on" => mk(Mechanic, m("core.counters"), None),
                _ => mk(Parameter, m("core.counters"), None),
            },
            "wrap" => mk(Mechanic, m("core.counters"), None),
            _ => mk(Parameter, m("core.counters"), None),
        },
        _ => mk(Mechanic, None, None),
    }
}

/// A machine's paths from `at` on: `set` values and `after` times are
/// parameters, the states and transitions themselves the mechanic.
fn machine(path: &Concrete, at: usize, mechanic: &str) -> Classified {
    let rest: Vec<&str> = path[at..]
        .iter()
        .filter_map(|s| match s {
            Step::Key(k) => Some(k.as_str()),
            Step::Item(_) => None,
        })
        .collect();
    let class = if rest.contains(&"set") || rest.last() == Some(&"after") {
        Class::Parameter
    } else {
        Class::Mechanic
    };
    Classified {
        path: display(path),
        class,
        mechanic: Some(mechanic.to_owned()),
        note: None,
    }
}

/// Every fact of a model, and the header's simulation parts
/// (`model.systems`, `model.view`, `model.extensions`, `model.start_level`,
/// `model.frame_rate`), with its class.
pub fn classify(model: &Model) -> Vec<Classified> {
    let tree = model.to_tree();
    let sim = simulation_slots(model);
    let mut out = Vec::new();
    for k in ["systems", "view", "extensions", "start_level", "frame_rate"] {
        if tree.get("model").and_then(|m| m.get(k)).is_some() {
            let p = vec![Step::Key("model".into()), Step::Key(k.into())];
            out.push(classify_path(model, &p, &sim));
        }
    }
    for (path, _) in crate::facts(&tree) {
        out.push(classify_path(model, &path, &sim));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mosshop() -> Model {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap()
    }

    fn class_of<'a>(all: &'a [Classified], path: &str) -> &'a Classified {
        all.iter()
            .find(|c| c.path == path)
            .unwrap_or_else(|| panic!("{path} isn't classified"))
    }

    #[test]
    fn moss_hops_paths_fall_into_the_research_tables_classes() {
        let m = mosshop();
        let all = classify(&m);
        let tree = m.to_tree();
        assert_eq!(
            all.len(),
            crate::facts(&tree).len() + 4,
            "facts and four header parts"
        );
        let c = class_of(&all, "entities.player.jump.speed");
        assert_eq!(
            (c.class, c.mechanic.as_deref()),
            (Class::Parameter, Some("platformer.jump"))
        );
        assert_eq!(
            class_of(&all, "entities.player.jump.action").note,
            Some("interface")
        );
        assert_eq!(class_of(&all, "model.systems").class, Class::Mechanic);
        assert_eq!(
            class_of(&all, "model.frame_rate").class,
            Class::Presentation
        );
        assert_eq!(class_of(&all, "levels.main.goal.x").class, Class::Content);
        assert_eq!(
            class_of(&all, "levels.main.kill_below").class,
            Class::Parameter
        );
        assert_eq!(class_of(&all, "cameras.main.lead").class, Class::Parameter);
        assert_eq!(class_of(&all, "rules.game.initial").class, Class::Mechanic);
        assert_eq!(
            class_of(&all, "rules.game.states.dying.transition[0].after").class,
            Class::Parameter
        );
        // The grid a level reads is content; the tiles are art.
        assert_eq!(class_of(&all, "assets.level.kind").class, Class::Content);
        assert_eq!(
            class_of(&all, "assets.tiles.kind").class,
            Class::Presentation
        );
        assert!(
            all.iter()
                .any(|c| c.class == Class::Presentation && c.path.contains(".sprite."))
        );
    }
}
