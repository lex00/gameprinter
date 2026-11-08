//! A model's mechanic catalog (docs/mechanics.md): every mechanic the
//! model uses, as the record its code declares (`gp_sim::mechanic` for the
//! components, `Ext::describe` for hooks), with the model paths that use
//! it, the class of every model path, and the problems found by holding the
//! model to the records: parameters a hook's record doesn't declare, tables
//! it doesn't mark, systems missing from `model.systems`, components an
//! entity can't carry together, and hooks with no record (opaque).
//!
//! The catalog is generated, never written by hand, so it can't drift from
//! the code. `gp model mechanics` prints it.

use std::collections::BTreeMap;

use gp_model::Model;
use gp_model::classify::{self, Class, Classified};
use gp_sim::Exts;
use gp_sim::mechanic::{self, Record, TableKind};
use serde::Serialize;

/// The catalog format's version. A change to what a record or the catalog
/// says bumps it.
pub const CATALOG_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize)]
pub struct Param {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub ty: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub unit: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub range: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// One mechanic as the catalog prints it: its record, and where the model
/// uses it.
#[derive(Debug, Clone, Serialize)]
pub struct Mechanic {
    pub name: &'static str,
    pub version: &'static str,
    pub carrier: &'static str,
    pub system: &'static str,
    pub class: &'static str,
    pub implementation: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub like: &'static str,
    pub about: &'static str,
    /// The `ext` name, for a hook's record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    pub params: Vec<Param>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tables: BTreeMap<&'static str, &'static str>,
    pub state: Vec<&'static str>,
    pub reads: Vec<&'static str>,
    pub writes: Vec<&'static str>,
    pub emits: Vec<&'static str>,
    pub consumes: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spawns: Vec<&'static str>,
    pub assumes: Vec<&'static str>,
    pub after: Vec<&'static str>,
    pub excludes: Vec<&'static str>,
    pub built_from: Vec<&'static str>,
    pub invariants: Vec<&'static str>,
    pub measures: BTreeMap<&'static str, &'static str>,
    pub envelope: BTreeMap<&'static str, &'static str>,
    /// The model paths that carry it, such as `entities.player.jump`.
    pub used_by: Vec<String>,
}

impl Mechanic {
    fn from_record(r: &Record, hook: Option<&str>) -> Self {
        Self {
            name: r.name,
            version: r.version,
            carrier: r.carrier.name(),
            system: r.system,
            class: r.class,
            implementation: r.implementation,
            like: r.like,
            about: r.about,
            hook: hook.map(str::to_owned),
            params: r
                .params
                .iter()
                .map(|p| Param {
                    name: p.name,
                    ty: p.ty,
                    unit: p.unit,
                    range: p.range,
                    optional: p.optional,
                })
                .collect(),
            tables: r.tables.iter().map(|t| (t.name, t.kind.name())).collect(),
            state: r.state.to_vec(),
            reads: r.reads.to_vec(),
            writes: r.writes.to_vec(),
            emits: r.emits.to_vec(),
            consumes: r.consumes.to_vec(),
            spawns: r.spawns.to_vec(),
            assumes: r.assumes.to_vec(),
            after: r.after.to_vec(),
            excludes: r.excludes.to_vec(),
            built_from: r.built_from.iter().map(|p| p.name()).collect(),
            invariants: r.invariants.to_vec(),
            measures: r.measures.iter().copied().collect(),
            envelope: r.envelope.iter().copied().collect(),
            used_by: Vec::new(),
        }
    }
}

/// What holding the model to the records found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// The model path it's about.
    pub path: String,
    pub message: String,
    /// `error` for a model and code that disagree; `warning` for a hook
    /// with no record or a record whose system the model doesn't list.
    pub severity: &'static str,
}

/// One hook that is game-specific code rather than a general mechanic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Debt {
    pub hook: String,
    pub record: &'static str,
    /// The general mechanic it instances, if it names one.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub like: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub catalog: &'static str,
    pub model: String,
    pub mechanics: Vec<Mechanic>,
    /// `ext` hooks with no record: gp has no crate for them, or the hook
    /// doesn't describe itself.
    pub opaque: Vec<String>,
    pub problems: Vec<Problem>,
    /// Game-specific code that isn't yet a general mechanic: each hook
    /// record whose `like` is empty, or names a general mechanic no
    /// component runs yet. Every hook is debt until its general mechanic
    /// is a component (or a primitive) another game can use.
    pub debt: Vec<Debt>,
    /// Model paths by class.
    pub counts: BTreeMap<&'static str, usize>,
    pub paths: Vec<Classified>,
}

impl Catalog {
    /// The mechanic named `name`.
    pub fn get(&self, name: &str) -> Option<&Mechanic> {
        self.mechanics.iter().find(|m| m.name == name)
    }

    /// How many problems are errors: the model and the code disagree.
    pub fn errors(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| p.severity == "error")
            .count()
    }
}

/// Every component record, whether or not a model uses it.
pub fn all_components() -> Vec<Mechanic> {
    mechanic::COMPONENTS
        .iter()
        .map(|r| Mechanic::from_record(r, None))
        .collect()
}

/// The catalog of `model`, with its hooks `exts` (an empty registry for a
/// model without any).
pub fn catalog(model: &Model, exts: &Exts) -> Catalog {
    let mut used: BTreeMap<&'static str, Mechanic> = BTreeMap::new();
    let mut order: Vec<&'static str> = Vec::new();
    let mut problems = Vec::new();
    let mut opaque = Vec::new();
    let mut hook_records: BTreeMap<String, Vec<Record>> = BTreeMap::new();
    let mut add = |r: &Record, hook: Option<&str>, by: String| {
        let m = used.entry(r.name).or_insert_with(|| {
            order.push(r.name);
            Mechanic::from_record(r, hook)
        });
        if !m.used_by.contains(&by) {
            m.used_by.push(by);
        }
    };
    let comp = |name: &str| mechanic::component(name).expect("a component record");

    for (name, e) in &model.entities {
        let at = format!("entities.{name}");
        // Every platformer component the entity carries, in the
        // extension's order, its machine's states' included.
        let mut given: Vec<&str> = Vec::new();
        let mut look = |p: &gp_model::platformer::EntityComponents| {
            let Ok(toml::Value::Table(t)) = toml::Value::try_from(p) else {
                return;
            };
            for c in gp_model::platformer::EXTENSION.entity_components {
                if t.contains_key(*c) && !given.contains(c) {
                    given.push(c);
                }
            }
        };
        look(&e.platformer);
        if let Some(m) = &e.core.machine {
            for sname in m.states.keys() {
                if let Ok(se) = e.in_state(sname) {
                    look(&se.platformer);
                }
            }
        }
        given.sort_by_key(|c| {
            gp_model::platformer::EXTENSION
                .entity_components
                .iter()
                .position(|x| x == c)
        });
        let mut carried = Vec::new();
        for c in given {
            if let Some(r) = mechanic::component(&format!("platformer.{c}")) {
                add(r, None, format!("{at}.{c}"));
                carried.push(r);
            }
        }
        for r in &carried {
            for x in r.excludes {
                if carried.iter().any(|o| o.name == *x) && r.name < *x {
                    problems.push(Problem {
                        path: at.clone(),
                        message: format!(
                            "carries both {} and {x}, which exclude each other",
                            r.name
                        ),
                        severity: "error",
                    });
                }
            }
        }
        if e.core.tile_collision {
            add(
                comp("core.tile_collision"),
                None,
                format!("{at}.tile_collision"),
            );
        }
        if e.core.machine.is_some() {
            add(comp("core.modes"), None, format!("{at}.machine"));
        }
        if let Some(hook) = &e.core.ext {
            let records = exts.get(hook).map(|h| h.describe()).unwrap_or_default();
            if records.is_empty() {
                if !opaque.contains(hook) {
                    opaque.push(hook.clone());
                }
                problems.push(Problem {
                    path: format!("{at}.ext"),
                    message: if exts.contains_key(hook) {
                        format!("the hook `{hook}` describes no mechanic, so it is opaque")
                    } else {
                        format!("gp has no hook crate for `{hook}`, so it is opaque")
                    },
                    severity: "warning",
                });
            }
            for r in &records {
                add(r, Some(hook), format!("{at}.ext"));
            }
            hook_check(model, name, &records, &mut problems);
            hook_records.insert(hook.clone(), records);
        }
    }
    for (name, l) in &model.levels {
        let at = format!("levels.{name}");
        if l.platformer.kill_below.is_some() {
            add(comp("platformer.pit"), None, format!("{at}.kill_below"));
        }
        if l.platformer.goal.is_some() {
            add(comp("platformer.goal"), None, format!("{at}.goal"));
        }
        if !l.core.exit.is_empty() {
            add(comp("core.exits"), None, format!("{at}.exit"));
        }
        if l.core.spawn_ahead.is_some() || l.core.despawn_behind.is_some() {
            add(comp("core.spawn_by_view"), None, at.clone());
        }
    }
    for name in model.cameras.keys() {
        add(comp("core.camera"), None, format!("cameras.{name}"));
    }
    for name in model.rules.keys() {
        add(comp("core.flow"), None, format!("rules.{name}"));
    }
    for name in model.counters.keys() {
        add(comp("core.counters"), None, format!("counters.{name}"));
    }

    let systems = &model.model.systems;
    let mechanics: Vec<Mechanic> = order
        .iter()
        .map(|n| used.remove(n).expect("added above"))
        .collect();
    if !systems.is_empty() {
        for m in &mechanics {
            if !systems.iter().any(|s| s == m.system) {
                problems.push(Problem {
                    path: "model.systems".into(),
                    message: format!(
                        "{} runs in `{}`, which model.systems doesn't list",
                        m.name, m.system
                    ),
                    severity: "warning",
                });
            }
        }
    }

    // Classes, with each hook's paths named by its record, and each
    // ext_table by what its record says it is.
    let mut paths = classify::classify(model);
    for c in &mut paths {
        let Some(hook) = c.mechanic.as_deref().and_then(|m| m.strip_prefix("ext:")) else {
            continue;
        };
        let hook = hook.to_owned();
        let records = hook_records.get(&hook).map(Vec::as_slice).unwrap_or(&[]);
        c.mechanic = Some(match records {
            [] => format!("ext:{hook}"),
            [one] => one.name.to_owned(),
            many => {
                // The record that declares the parameter, if one does.
                let leaf = c.path.rsplit('.').next().unwrap_or_default();
                many.iter()
                    .find(|r| r.param(leaf).is_some() || r.table(leaf).is_some())
                    .unwrap_or(&many[0])
                    .name
                    .to_owned()
            }
        });
        if c.note == Some("ext table") {
            let leaf = c.path.rsplit('.').next().unwrap_or_default();
            if let Some(t) = records.iter().find_map(|r| r.table(leaf)) {
                c.class = match t.kind {
                    TableKind::Content => Class::Content,
                    TableKind::Mechanic => Class::Parameter,
                };
            }
        }
    }
    let mut debt = Vec::new();
    for (hook, records) in &hook_records {
        for r in records {
            let reason = if r.like.is_empty() {
                "names no general mechanic (`like`)".to_owned()
            } else if mechanic::component(&format!("platformer.{}", r.like)).is_some() {
                continue;
            } else {
                format!("instances `{}`, which no component runs yet", r.like)
            };
            debt.push(Debt {
                hook: hook.clone(),
                record: r.name,
                like: r.like,
                reason,
            });
        }
    }
    for hook in &opaque {
        debt.push(Debt {
            hook: hook.clone(),
            record: "",
            like: "",
            reason: "has no record at all (opaque)".into(),
        });
    }
    let mut counts = BTreeMap::new();
    for c in &paths {
        *counts.entry(c.class.name()).or_insert(0) += 1;
    }
    Catalog {
        catalog: CATALOG_VERSION,
        model: model.model.name.clone(),
        mechanics,
        opaque,
        problems,
        debt,
        counts,
        paths,
    }
}

/// An entity's `ext_params` and `ext_tables`, its states' `set` included,
/// against its hook's records.
fn hook_check(model: &Model, entity: &str, records: &[Record], out: &mut Vec<Problem>) {
    if records.is_empty() {
        return;
    }
    let e = &model.entities[entity];
    let mut params: Vec<String> = e.core.ext_params.keys().cloned().collect();
    if let Some(m) = &e.core.machine {
        for s in m.states.values() {
            for k in s.set.keys() {
                if let Some(p) = k.strip_prefix("ext_params.") {
                    params.push(p.to_owned());
                }
            }
        }
    }
    params.sort();
    params.dedup();
    for p in &params {
        if !records.iter().any(|r| r.param(p).is_some()) {
            out.push(Problem {
                path: format!("entities.{entity}.ext_params.{p}"),
                message: format!(
                    "the hook's record ({}) declares no parameter `{p}`",
                    names(records)
                ),
                severity: "error",
            });
        }
    }
    for r in records {
        for p in r.params.iter().filter(|p| !p.optional) {
            if !params.iter().any(|q| q == p.name) {
                out.push(Problem {
                    path: format!("entities.{entity}.ext_params"),
                    message: format!(
                        "{} needs the parameter `{}`, which the model doesn't give",
                        r.name, p.name
                    ),
                    severity: "error",
                });
            }
        }
    }
    for t in e.core.ext_tables.keys() {
        if !records.iter().any(|r| r.table(t).is_some()) {
            out.push(Problem {
                path: format!("entities.{entity}.ext_tables.{t}"),
                message: format!(
                    "the hook's record ({}) doesn't say whether table `{t}` is mechanic or content",
                    names(records)
                ),
                severity: "error",
            });
        }
    }
}

fn names(records: &[Record]) -> String {
    records
        .iter()
        .map(|r| r.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The records are for the extension and core versions gp-model has,
    /// and cover every component the extension declares, with every
    /// parameter the component's schema has.
    #[test]
    fn component_records_match_the_schema() {
        assert_eq!(mechanic::PLATFORMER_VERSION, gp_model::platformer::VERSION);
        assert_eq!(mechanic::CORE_VERSION, gp_model::CORE_VERSION);
        let schema: serde_json::Value =
            serde_json::from_str(&gp_model::schema::platformer()).unwrap();
        let defs = &schema["$defs"];
        for c in gp_model::platformer::EXTENSION.entity_components {
            let r = mechanic::component(&format!("platformer.{c}"))
                .unwrap_or_else(|| panic!("no record for platformer.{c}"));
            // The component's type, by its name in the schema.
            let ty: String = c
                .split('_')
                .map(|w| w[..1].to_uppercase() + &w[1..])
                .collect();
            // A plain list (`touch_points`) has no type of its own, so
            // its record has no parameters.
            if defs.get(&ty).is_none()
                && defs["EntityComponents"]["properties"][*c]["type"] == "array"
            {
                assert!(r.params.is_empty(), "{} has parameters", r.name);
                continue;
            }
            let props = defs[&ty]["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("no schema for {ty}"));
            for k in props.keys() {
                assert!(r.param(k).is_some(), "{} has no parameter `{k}`", r.name);
            }
            for p in r.params {
                assert!(
                    props.contains_key(p.name),
                    "{}: `{}` isn't in the schema",
                    r.name,
                    p.name
                );
            }
        }
    }

    #[test]
    fn a_hook_without_a_record_is_opaque_and_one_with_unknown_params_is_an_error() {
        let text = r#"
[model]
schema = "0.4.0"
name = "t"
title = "T"
view = "side"
frame_rate = "60/1"
extensions = { platformer = "0.4.0" }

[entities.a]
role = "enemy"
size = [8, 8]
hitbox = { x = 0, y = 0, w = 8, h = 8 }
ext = "nothing_known"

[entities.a.walk]
speed = 1

[entities.a.patrol]
step = 1
period = 2
"#;
        let m = Model::parse(text).unwrap();
        let c = catalog(&m, &Exts::new());
        assert_eq!(c.opaque, vec!["nothing_known".to_owned()]);
        assert_eq!(c.debt.len(), 1);
        assert!(
            c.problems
                .iter()
                .any(|p| p.message.contains("exclude each other"))
        );
        assert!(c.get("platformer.walk").is_some());
    }
}
