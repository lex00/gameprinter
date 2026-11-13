//! The check compiler: a model and its contract in, the checks to run out.
//!
//! The compiler finds every part of the model a check knows how to measure
//! (a [`Kind`]), and asks the contract which level governs each of the
//! part's quantities, using the quantity as the aspect. Rules with a `when`
//! become [`Conditional`]s, which replace that level on the frames their
//! condition holds. Facts no check measures are counted by level, so a
//! contract's gaps show in the report.

use std::collections::{BTreeMap, BTreeSet};

use gp_contract::{Bound, Contract, Level, Tolerance};
use gp_model::Model;
use gp_model::path::{Concrete, Step};
use gp_runtime::select::Condition;

/// What a check measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A jump's height and time in the air (`entities.<e>.jump`).
    Jump,
    /// Walking and running speed (`entities.<e>.run`).
    Run,
    /// A patrolling enemy's speed (`entities.<e>.patrol`).
    Patrol,
    /// A walking enemy's speed (`entities.<e>.walk`).
    Walk,
    /// A state machine's state and transitions (`rules.<m>`).
    Machine,
    /// An entity's own machine (`entities.<e>.machine`): each instance's
    /// state, its transitions and their timings.
    States,
    /// A counter's value (`counters.<c>`).
    Counter,
    /// A level's place in the level graph and the objects in it
    /// (`levels.<l>`).
    Level,
    /// An enemy in the original's reused object slots, its motion compared
    /// slot life by slot life with one body of the entity
    /// (`entities.<e>`; the state mapping's `[slots]`).
    Slots,
    /// Where a camera keeps the entity it follows while it scrolls
    /// (`cameras.<c>`).
    Camera,
    /// Whether a level's goal can be reached from the player's spawn by the
    /// player's own moves (`levels.<l>`, for a level with a goal;
    /// `gp_interp::reach`). It checks the model alone, once, not a
    /// recording ([`crate::reach`]).
    Reach,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Run => "run",
            Self::Patrol => "patrol",
            Self::Walk => "walk",
            Self::Machine => "machine",
            Self::States => "states",
            Self::Counter => "counter",
            Self::Level => "level",
            Self::Slots => "slots",
            Self::Camera => "camera",
            Self::Reach => "reach",
        }
    }

    /// The quantities it measures, which are also the contract aspects it
    /// asks about.
    pub fn quantities(self) -> &'static [&'static str] {
        match self {
            Self::Jump => &["height", "airtime"],
            Self::Run | Self::Patrol | Self::Walk => &["speed"],
            Self::Machine | Self::States => &["state"],
            Self::Counter => &["value"],
            Self::Level => &["graph"],
            Self::Slots => &["speed", "period"],
            Self::Camera => &["lead"],
            Self::Reach => &["reachable"],
        }
    }

    /// Whether a check of this kind does anything at `level`. Profiles are
    /// measured at Exact (no tolerance) and Metric; state and counters are
    /// compared at Exact; levels at Exact and Structural.
    pub fn checks_at(self, level: Level) -> bool {
        match self {
            Self::Jump
            | Self::Run
            | Self::Patrol
            | Self::Walk
            | Self::Slots
            | Self::Camera
            | Self::States => {
                matches!(level, Level::Exact | Level::Metric)
            }
            Self::Machine | Self::Counter => level == Level::Exact,
            Self::Level | Self::Reach => matches!(level, Level::Exact | Level::Structural),
        }
    }
}

/// A level and its tolerances, as the contract gives them for one quantity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Governing {
    pub level: Level,
    /// The rule it comes from, or `None` for the contract's default.
    pub rule: Option<usize>,
    pub tolerance: BTreeMap<String, Tolerance>,
}

/// A rule with a `when`, compiled.
#[derive(Debug, Clone)]
pub struct Conditional {
    pub rule: usize,
    pub when: Condition,
    pub governing: Governing,
}

/// One compiled check: a part of the model, what to measure there, and the
/// level of each quantity.
#[derive(Debug, Clone)]
pub struct Check {
    pub kind: Kind,
    /// The part's model path, such as `entities.player.jump`.
    pub part: String,
    /// The entity, machine, counter or level the part belongs to.
    pub name: String,
    /// By quantity: the level with no condition holding, then the
    /// conditional rules in file order.
    pub levels: BTreeMap<&'static str, (Governing, Vec<Conditional>)>,
}

/// What the level of a quantity is on one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum At {
    Level(Governing),
    /// Two conditional rules with different levels hold together.
    Conflict(usize, usize),
}

impl Check {
    pub fn id(&self, quantity: &str) -> String {
        format!("{} {quantity}", self.part)
    }

    /// The level of `quantity` on a frame, given whether each conditional
    /// rule's condition holds there.
    pub fn at(&self, quantity: &str, holds: &mut impl FnMut(&Condition) -> bool) -> At {
        let (base, conditional) = &self.levels[quantity];
        let mut found: Option<&Conditional> = None;
        for c in conditional {
            if holds(&c.when) {
                match found {
                    Some(f) if f.governing.level != c.governing.level => {
                        return At::Conflict(f.rule, c.rule);
                    }
                    Some(_) => {}
                    None => found = Some(c),
                }
            }
        }
        At::Level(found.map_or_else(|| base.clone(), |c| c.governing.clone()))
    }

    /// Every level the quantity can have.
    pub fn possible(&self, quantity: &str) -> BTreeSet<Level> {
        let (base, conditional) = &self.levels[quantity];
        std::iter::once(base.level)
            .chain(conditional.iter().map(|c| c.governing.level))
            .collect()
    }
}

/// Everything compiled from one model and contract.
#[derive(Debug, Clone)]
pub struct Plan {
    pub checks: Vec<Check>,
    /// Facts by the level that governs them, for the fact as a whole.
    pub facts: BTreeMap<Level, usize>,
    /// Facts no check measures, by level.
    pub unchecked: BTreeMap<Level, Vec<String>>,
    /// Counters an `ext` hook may write, by counter, with the hooks: one
    /// whose description writes `counters` or `counters.<c>`, and every
    /// hook that describes nothing, which may write any. The compiler
    /// can't know these; the caller, which has the hooks, fills them in.
    pub kept_by_hooks: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("fidelity.toml rule[{rule}]: `when` isn't a condition: {message}")]
    When { rule: usize, message: String },
}

fn key(name: &str) -> Step {
    Step::Key(name.to_owned())
}

/// Compiles a contract's rules against a model into checks.
pub fn compile(model: &Model, contract: &Contract) -> Result<Plan, CompileError> {
    compile_with_slots(model, contract, &BTreeSet::new())
}

/// The parts of an entity a [`Kind::Slots`] check measures: what moves it.
pub const SLOT_PARTS: [&str; 9] = [
    "walk",
    "patrol",
    "fly",
    "path",
    "hop",
    "fall",
    "ext",
    "ext_params",
    "ext_tables",
];

/// [`compile`], with a [`Kind::Slots`] check for each of `slotted`, the
/// entities the state mapping's `[slots]` names. It replaces their walk
/// check: an enemy in a slot is measured beside the original's slot, where
/// the model's own run soon leaves the recording's path.
pub fn compile_with_slots(
    model: &Model,
    contract: &Contract,
    slotted: &BTreeSet<String>,
) -> Result<Plan, CompileError> {
    let bound = contract.bind(model);
    let mut whens = BTreeMap::new();
    for (i, r) in contract.rules.iter().enumerate() {
        if let Some(w) = &r.when {
            let c =
                Condition::parse(w).map_err(|message| CompileError::When { rule: i, message })?;
            whens.insert(i, c);
        }
    }
    let mut parts: Vec<(Kind, Concrete, String)> = Vec::new();
    for (name, e) in &model.entities {
        let p = &e.platformer;
        for (present, kind, field) in [
            (p.jump.is_some(), Kind::Jump, "jump"),
            (p.run.is_some(), Kind::Run, "run"),
            (p.patrol.is_some(), Kind::Patrol, "patrol"),
            (p.walk.is_some(), Kind::Walk, "walk"),
        ] {
            if present && !(kind == Kind::Walk && slotted.contains(name)) {
                parts.push((
                    kind,
                    vec![key("entities"), key(name), key(field)],
                    name.clone(),
                ));
            }
        }
    }
    for name in model.entities.keys().filter(|n| slotted.contains(*n)) {
        parts.push((Kind::Slots, vec![key("entities"), key(name)], name.clone()));
    }
    for name in model.cameras.keys() {
        parts.push((Kind::Camera, vec![key("cameras"), key(name)], name.clone()));
    }
    for (name, e) in &model.entities {
        if e.core.machine.is_some() {
            parts.push((
                Kind::States,
                vec![key("entities"), key(name), key("machine")],
                name.clone(),
            ));
        }
    }
    for name in model.rules.keys() {
        parts.push((Kind::Machine, vec![key("rules"), key(name)], name.clone()));
    }
    for name in model.counters.keys() {
        parts.push((
            Kind::Counter,
            vec![key("counters"), key(name)],
            name.clone(),
        ));
    }
    for name in model.levels.keys() {
        parts.push((Kind::Level, vec![key("levels"), key(name)], name.clone()));
    }
    for (name, l) in &model.levels {
        if l.platformer.goal.is_some() {
            parts.push((Kind::Reach, vec![key("levels"), key(name)], name.clone()));
        }
    }

    let checks: Vec<Check> = parts
        .into_iter()
        .map(|(kind, part, name)| Check {
            kind,
            part: gp_model::path::display(&part),
            name,
            levels: kind
                .quantities()
                .iter()
                .map(|q| (*q, governing(contract, &bound, &part, q, &whens)))
                .collect(),
        })
        .collect();

    let tree = model.to_tree();
    let mut facts = BTreeMap::new();
    let mut unchecked: BTreeMap<Level, Vec<String>> = BTreeMap::new();
    for (fact, _) in gp_model::facts(&tree) {
        let level = bound.resolve(&fact, None).level;
        *facts.entry(level).or_insert(0) += 1;
        let text = gp_model::path::display(&fact);
        let measured = checks.iter().any(|c| {
            let under = if c.kind == Kind::Camera {
                text.starts_with(&format!("{}.lead", c.part))
            } else if c.kind == Kind::States {
                // What a state sets for the sprite is drawn, not run.
                text.starts_with(&format!("{}.", c.part)) && !text.contains(".set.sprite")
            } else if c.kind == Kind::Slots {
                SLOT_PARTS.iter().any(|p| {
                    let p = format!("{}.{p}", c.part);
                    text == p || text.starts_with(&format!("{p}."))
                })
            } else {
                text == c.part || text.starts_with(&format!("{}.", c.part))
            };
            under && c.levels.values().any(|(g, _)| c.kind.checks_at(g.level))
        });
        if !measured && level != Level::Free {
            unchecked.entry(level).or_default().push(text);
        }
    }
    Ok(Plan {
        checks,
        facts,
        unchecked,
        kept_by_hooks: BTreeMap::new(),
    })
}

fn governing(
    contract: &Contract,
    bound: &Bound<'_>,
    part: &[Step],
    quantity: &str,
    whens: &BTreeMap<usize, Condition>,
) -> (Governing, Vec<Conditional>) {
    let r = bound.resolve(part, Some(quantity));
    let base = Governing {
        level: r.level,
        rule: r.rule,
        tolerance: r.tolerance.clone(),
    };
    let conditional = r
        .conditional
        .iter()
        .map(|i| {
            let rule = &contract.rules[*i];
            // A conditional metric rule's own tolerances go over the ones
            // the part inherits.
            let mut tolerance = if rule.level == Level::Metric && r.level == Level::Metric {
                r.tolerance.clone()
            } else {
                BTreeMap::new()
            };
            tolerance.extend(rule.tolerance.iter().map(|(k, v)| (k.clone(), *v)));
            Conditional {
                rule: *i,
                when: whens[i].clone(),
                governing: Governing {
                    level: rule.level,
                    rule: Some(*i),
                    tolerance,
                },
            }
        })
        .collect();
    (base, conditional)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mosshop() -> (Model, Contract) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop");
        let m = Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        let c =
            Contract::parse(&std::fs::read_to_string(dir.join("fidelity.toml")).unwrap()).unwrap();
        (m, c)
    }

    #[test]
    fn moss_hops_contract_compiles_to_its_checks() {
        let (m, c) = mosshop();
        let plan = compile(&m, &c).unwrap();
        let got: Vec<String> = plan
            .checks
            .iter()
            .flat_map(|c| {
                c.levels.iter().map(move |(q, (g, _))| {
                    let tol: Vec<String> = g
                        .tolerance
                        .iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect();
                    format!("{} {q}: {} {}", c.part, g.level, tol.join(", "))
                        .trim_end()
                        .to_owned()
                })
            })
            .collect();
        assert_eq!(
            got,
            [
                "entities.player.jump airtime: metric airtime 2, height 5%",
                "entities.player.jump height: metric airtime 2, height 5%",
                "entities.player.run speed: metric",
                "entities.slug.patrol speed: metric speed 3%",
                "cameras.main lead: metric",
                "entities.player.machine state: metric",
                "entities.slug.machine state: metric speed 3%",
                "rules.game state: exact",
                "counters.coins value: exact",
                "counters.deaths value: exact",
                "counters.lives value: exact",
                "counters.stomps value: exact",
                "levels.main graph: structural",
                "levels.main reachable: structural",
            ]
        );
        // The hitbox, the camera's size and the contact rules have no
        // check yet;
        // style facts are left to later milestones.
        let unchecked = &plan.unchecked[&Level::Metric];
        assert!(unchecked.contains(&"entities.player.hitbox.w".to_owned()));
        assert!(unchecked.contains(&"entities.slug.contact.stomp_depth".to_owned()));
        assert!(
            !unchecked
                .iter()
                .any(|f| f.starts_with("entities.player.jump"))
        );
        // The machines are checked; what their states set for the sprite
        // isn't.
        assert!(!unchecked.contains(&"entities.slug.machine.initial".to_owned()));
        assert!(
            unchecked.contains(&"entities.slug.machine.states.flat.set.sprite.palette".to_owned())
        );
        assert!(
            plan.unchecked[&Level::Style].contains(&"entities.player.sprite.palette".to_owned())
        );
        assert!(!plan.unchecked.contains_key(&Level::Exact));
        assert!(!plan.unchecked.contains_key(&Level::Structural));
    }

    #[test]
    fn a_when_replaces_the_level_while_it_holds() {
        let (m, mut c) = mosshop();
        let extra = Contract::parse(
            r#"
[contract]
schema = "0.2.0"
model = "mosshop"
default = "metric"

[[rule]]
path = "entities.player.jump"
level = "exact"
when = "level_number == 2"

[[rule]]
path = "entities.player.jump"
level = "free"
aspect = "airtime"
when = "level_number >= 2"

[[rule]]
path = "entities.player.jump"
level = "metric"
when = "level_number == 3"

[rule.tolerance]
height = 1
"#,
        )
        .unwrap();
        c.rules.extend(extra.rules);
        let plan = compile(&m, &c).unwrap();
        let jump = plan.checks.iter().find(|c| c.kind == Kind::Jump).unwrap();
        let at = |q: &str, n: i64| {
            jump.at(q, &mut |cond: &Condition| {
                cond.eval(&mut |_| Ok(n)).unwrap()
            })
        };
        let level = |a: At| match a {
            At::Level(g) => g.level,
            At::Conflict(..) => panic!("conflict"),
        };
        assert_eq!(level(at("height", 1)), Level::Metric);
        assert_eq!(level(at("height", 2)), Level::Exact);
        // Different levels holding together are a conflict, reported where
        // it happens.
        assert_eq!(at("airtime", 2), At::Conflict(7, 8));
        assert_eq!(at("airtime", 3), At::Conflict(8, 9));
        let At::Level(g) = at("height", 3) else {
            panic!()
        };
        // Its own height tolerance over the inherited one; airtime inherited.
        let tol: Vec<String> = g
            .tolerance
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        assert_eq!(tol, ["airtime 2", "height 1"]);
        assert_eq!(
            jump.possible("height").into_iter().collect::<Vec<_>>(),
            [Level::Exact, Level::Metric]
        );

        c.rules[7].when = Some("level_number ==".into());
        let e = compile(&m, &c).unwrap_err();
        assert!(e.to_string().contains("rule[7]"), "{e}");
    }
}
