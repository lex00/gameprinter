//! Profiles (profiles format 0.2.0, docs/model.md "Profiles"): named
//! switches, each a set of overrides of a model's mechanic parameters and
//! options, kept beside the model in `profiles.toml` and shown in groups,
//! such as Super Mario Bros.' physics fixes ("smooth steps", "turn in the
//! air") and its assist ("more lives", "no timer").
//!
//! The model as written is the profile [`BASE`], "retro": every switch off.
//! It is the default and what every comparison, score and refinement runs.
//! Switches combine freely, since no two may change the same part. A preset
//! (Super Mario Bros.' "normal") names a set of switches. Switches are
//! applied only when a game is built or played: [`Profiles::apply`] writes
//! the overrides of every switch on into a copy of the model and names the
//! set in `model.profile` (the switch names in order, joined by `+`), which
//! runtimes put in the state hash, so runs with two sets never mix. The
//! model's canonical text, and so its hash, is the base's alone.
//!
//! A switch can only override a path the base model has, and only one of
//! a mechanic's parameters or options: a path classed `parameter` or
//! `mechanic` that names its mechanic ([`crate::classify`]), such as
//! `entities.mario.run.move_step`, `counters.lives.start` or, since
//! `switch.rules`, `rules.game.states.dying.transition[0].after`. Content,
//! presentation, the header and a part with no mechanic (an entity's
//! `role`) are refused.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path as FsPath;

use serde::{Deserialize, Serialize};

use crate::classify::{self, Class};
use crate::model::Model;
use crate::path::{self, Concrete, Path, Step};
use crate::validate::{self, Problem};

/// The file a model directory keeps its profiles in.
pub const PROFILES_FILE: &str = "profiles.toml";

/// The format version of `profiles.toml`.
pub const FORMAT: &str = "0.2.0";

/// The base profile: the model as written, every switch off.
pub const BASE: &str = "retro";

/// What joins switch names in a profile's name, such as
/// `more_lives+no_timer`.
pub const JOIN: char = '+';

/// The preset every template has (docs/settings.md, "Modes"): the game
/// with no lag and none of the console's quirks. It may turn on no switch
/// (feature `preset.empty_normal`) when the model has nothing to turn off.
pub const NORMAL: &str = "normal";

/// The standard group for the game's frame timing (docs/settings.md,
/// "The timing group"): lag, slowdown, logic frames the game skips when
/// busy, and anything the spare time of a frame decides. `normal` turns on
/// every switch in it.
pub const TIMING: &str = "timing";

/// A `profiles.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub format: String,
    /// The switches, in groups, in the order an options screen shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// Named sets of switches, by name, besides [`BASE`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub presets: BTreeMap<String, Preset>,
}

/// A group of switches, such as "physics fixes" or "assist".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    /// The group's heading on an options screen; the name without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub switches: Vec<Switch>,
}

impl Group {
    /// The heading to show: the title, or the name.
    pub fn heading(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// One switch: values to set and options to turn off while it's on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Switch {
    pub name: String,
    /// What turning it on does, in plain words, for the options screen.
    pub description: String,
    /// New values by model path, of the type the base has there.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub set: BTreeMap<String, toml::Value>,
    /// Model paths to remove, so each goes back to its default: an option
    /// off, a list empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<String>,
}

impl Switch {
    /// Every path it names, `set`'s then `off`'s.
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.set.keys().chain(self.off.iter())
    }
}

/// A named set of switches, such as "normal".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub switches: Vec<String>,
}

impl Profiles {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The profiles beside a model: `dir/profiles.toml`, or none when the
    /// file isn't there.
    pub fn read_dir(dir: &FsPath) -> Result<Self, String> {
        let file = dir.join(PROFILES_FILE);
        if !file.is_file() {
            return Ok(Self {
                format: FORMAT.into(),
                ..Self::default()
            });
        }
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Whether the file names no switch.
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.switches.is_empty())
    }

    /// Every switch with its group, in the file's order.
    pub fn switches(&self) -> impl Iterator<Item = (&Group, &Switch)> {
        self.groups
            .iter()
            .flat_map(|g| g.switches.iter().map(move |s| (g, s)))
    }

    pub fn switch(&self, name: &str) -> Option<&Switch> {
        self.switches().map(|(_, s)| s).find(|s| s.name == name)
    }

    /// The profiles a person picks from a list: the base, then each
    /// preset. Any set of switches is a profile too ([`Self::resolve`]).
    pub fn names(&self) -> Vec<String> {
        std::iter::once(BASE.to_owned())
            .chain(self.presets.keys().cloned())
            .collect()
    }

    /// The switches a profile's name turns on, sorted: none for the base,
    /// a preset's, one switch's own name, or names of switches and presets
    /// joined by `+`, such as `normal+more_lives`.
    pub fn resolve(&self, name: &str) -> Result<Vec<String>, String> {
        let mut on = BTreeSet::new();
        if name == BASE {
            return Ok(Vec::new());
        }
        for part in name.split(JOIN) {
            if let Some(p) = self.presets.get(part) {
                on.extend(p.switches.iter().cloned());
            } else if self.switch(part).is_some() {
                on.insert(part.to_owned());
            } else {
                let mut known = self.names();
                known.extend(self.switches().map(|(_, s)| s.name.clone()));
                return Err(format!(
                    "no profile or switch named `{part}`; the model has {}",
                    known.join(", ")
                ));
            }
        }
        Ok(on.into_iter().collect())
    }

    /// The name of a set of switches: their names sorted and joined by
    /// `+`, what `model.profile` holds and the state hash covers. None for
    /// no switches, the base.
    pub fn name_of<S: AsRef<str>>(switches: &[S]) -> Option<String> {
        let set: BTreeSet<&str> = switches.iter().map(AsRef::as_ref).collect();
        (!set.is_empty()).then(|| set.into_iter().collect::<Vec<_>>().join("+"))
    }

    /// The model in profile `name` ([`Self::resolve`]). The base is the
    /// model as it is.
    pub fn apply(&self, model: &Model, name: &str) -> Result<Model, String> {
        let on = self.resolve(name)?;
        self.apply_switches(model, &on)
    }

    /// The model with these switches on and every other off.
    pub fn apply_switches<S: AsRef<str>>(&self, model: &Model, on: &[S]) -> Result<Model, String> {
        let Some(name) = Self::name_of(on) else {
            return Ok(model.clone());
        };
        let mut chosen = Vec::new();
        for n in name.split(JOIN) {
            let s = self
                .switch(n)
                .ok_or_else(|| format!("no switch named `{n}`"))?;
            if let Some(first) = check_switch(model, s).first() {
                return Err(first.to_string());
            }
            chosen.push(s);
        }
        if let Some((a, b, p)) = overlap(&chosen) {
            return Err(format!(
                "switches `{a}` and `{b}` both change `{p}`, so they can't be on together"
            ));
        }
        let m = apply_unchecked(model, &name, &chosen)?;
        if let Some(v) = validate::validate(&m)
            .into_iter()
            .find(|v| !v.path.starts_with("evidence"))
        {
            return Err(format!("switches {name} make {v}"));
        }
        Ok(m)
    }

    /// What the settings standard asks of a template's file
    /// (docs/settings.md): a `normal` preset, which turns on every switch
    /// of the `timing` group. Empty when it keeps to it.
    pub fn standard_problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        let Some(normal) = self.presets.get(NORMAL) else {
            out.push(problem(
                format!("presets.{NORMAL}"),
                format!(
                    "is missing: every template has `{NORMAL}` beside `{BASE}`, the game with no lag and no console quirks. Write `[presets.{NORMAL}]` with `switches = []` when the model has nothing to turn off"
                ),
            ));
            return out;
        };
        for g in self.groups.iter().filter(|g| g.name == TIMING) {
            for s in &g.switches {
                if !normal.switches.contains(&s.name) {
                    out.push(problem(
                        format!("presets.{NORMAL}"),
                        format!(
                            "leaves out `{}`, a switch of the `{TIMING}` group: `{NORMAL}` models no lag, so it turns on every timing switch",
                            s.name
                        ),
                    ));
                }
            }
        }
        out
    }

    /// Every problem with the profiles against `model`, by path
    /// (`switches.<name>.set."<path>"`, `presets.<name>`). Empty when
    /// they're good.
    pub fn check(&self, model: &Model) -> Vec<Problem> {
        let mut out = Vec::new();
        if !validate::version_supported(&self.format, FORMAT) {
            out.push(Problem {
                path: "format".into(),
                message: format!(
                    "profiles format {} isn't supported; this build reads {FORMAT}",
                    self.format
                ),
            });
        }
        if model.model.profile.is_some() {
            out.push(Problem {
                path: "model.profile".into(),
                message: "is set only by applying a profile, never in model.toml".into(),
            });
        }
        let name_rule = "a name is letters, digits and `_`, not starting with a digit";
        let mut groups = BTreeSet::new();
        for g in &self.groups {
            let at = format!("groups.{}", g.name);
            if !path::is_name(&g.name) {
                out.push(problem(at.clone(), name_rule));
            }
            if !groups.insert(g.name.as_str()) {
                out.push(problem(at.clone(), "is named twice"));
            }
            if g.switches.is_empty() {
                out.push(problem(at, "has no switches"));
            }
        }
        let mut seen = BTreeSet::new();
        for (_, s) in self.switches() {
            let at = format!("switches.{}", s.name);
            if !seen.insert(s.name.as_str()) {
                out.push(problem(at, "is named twice"));
                continue;
            }
            if self.presets.contains_key(&s.name) {
                out.push(problem(at.clone(), "is also a preset's name"));
            }
            out.extend(check_switch(model, s));
        }
        let all: Vec<&Switch> = self.switches().map(|(_, s)| s).collect();
        for (a, b, p) in overlaps(&all) {
            out.push(problem(
                format!("switches.{b}"),
                format!("changes `{p}`, as switch `{a}` does: switches must change different parts, so any of them combine"),
            ));
        }
        for (name, p) in &self.presets {
            let at = format!("presets.{name}");
            if name == BASE {
                out.push(problem(
                    at,
                    format!(
                        "`{BASE}` is the model as written, every switch off, and can't be redefined"
                    ),
                ));
                continue;
            }
            if !path::is_name(name) {
                out.push(problem(at.clone(), name_rule));
            }
            if p.switches.is_empty() && name != NORMAL {
                out.push(problem(at.clone(), "turns no switch on: give `switches`"));
            }
            let mut listed = BTreeSet::new();
            for s in &p.switches {
                if !listed.insert(s) {
                    out.push(problem(at.clone(), format!("names `{s}` twice")));
                } else if self.switch(s).is_none() {
                    out.push(problem(
                        at.clone(),
                        format!("names `{s}`, which isn't a switch"),
                    ));
                }
            }
        }
        if out.is_empty() {
            // Each preset, and every switch at once, makes a valid model.
            let every: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
            let sets = self
                .presets
                .iter()
                .map(|(n, p)| (format!("presets.{n}"), p.switches.clone()))
                .chain(std::iter::once((
                    "groups".to_owned(),
                    every.iter().map(|s| (*s).to_owned()).collect(),
                )));
            for (at, on) in sets {
                if let Err(e) = self.apply_switches(model, &on) {
                    out.push(problem(at, e));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

fn problem(at: String, message: impl Into<String>) -> Problem {
    Problem {
        path: at,
        message: message.into(),
    }
}

/// The first two switches, in order, that change the same part
/// ([`overlaps`]).
fn overlap<'a>(switches: &[&'a Switch]) -> Option<(&'a str, &'a str, String)> {
    overlaps(switches).into_iter().next()
}

/// Each two switches, in order, that name the same part, or a part and a
/// part inside it, or two items of one list (removing one moves the
/// other), with the later one's path.
fn overlaps<'a>(switches: &[&'a Switch]) -> Vec<(&'a str, &'a str, String)> {
    let claims = |s: &Switch| -> Vec<(Concrete, String)> {
        s.paths()
            .filter_map(|p| {
                let mut steps = concrete(p).ok()?;
                // An item claims its whole list.
                while matches!(steps.last(), Some(Step::Item(_))) {
                    steps.pop();
                }
                Some((steps, p.clone()))
            })
            .collect()
    };
    let all: Vec<_> = switches
        .iter()
        .map(|s| (s.name.as_str(), claims(s)))
        .collect();
    let mut out = Vec::new();
    for (j, (b, bs)) in all.iter().enumerate() {
        for (a, as_) in &all[..j] {
            if let Some((_, text)) = bs.iter().find(|(x, _)| {
                as_.iter()
                    .any(|(y, _)| x.starts_with(y) || y.starts_with(x))
            }) {
                out.push((*a, *b, text.clone()));
            }
        }
    }
    out
}

/// The model's top-level parts a switch may change. `rules` is the
/// profiles format's `switch.rules` (no version bump): a game rule's
/// timing or flow, such as Donkey Kong's death hold, is an engine
/// behaviour a switch can turn off like any mechanic's option. Under
/// `input` only `keep_direction` is a mechanic; the bindings aren't.
const SWITCHABLE: [&str; 6] = [
    "entities", "cameras", "levels", "counters", "rules", "input",
];

/// The problems with one switch: its name and description, each path, and
/// the model it makes on its own.
fn check_switch(model: &Model, s: &Switch) -> Vec<Problem> {
    let mut out = Vec::new();
    let here = format!("switches.{}", s.name);
    if s.name == BASE {
        out.push(problem(
            here.clone(),
            format!("`{BASE}` is the model as written and can't be a switch"),
        ));
        return out;
    }
    if !path::is_name(&s.name) {
        out.push(problem(
            here.clone(),
            "a switch's name is letters, digits and `_`, not starting with a digit",
        ));
    }
    if s.description.trim().is_empty() {
        out.push(problem(
            here.clone(),
            "needs a description: the options screen shows it",
        ));
    }
    if s.set.is_empty() && s.off.is_empty() {
        out.push(problem(
            here.clone(),
            "changes nothing: give `set` or `off`",
        ));
    }
    let tree = model.to_tree();
    let sim = classify::simulation_slots(model);
    let mut seen = BTreeSet::new();
    let paths = s
        .set
        .iter()
        .map(|(k, v)| (k, Some(v), "set"))
        .chain(s.off.iter().map(|k| (k, None, "off")));
    for (text, value, list) in paths {
        let at = format!("{here}.{list}.\"{text}\"");
        if !seen.insert(text.clone()) {
            out.push(problem(at, "is named twice"));
            continue;
        }
        let steps = match concrete(text) {
            Ok(s) => s,
            Err(e) => {
                out.push(problem(at, e));
                continue;
            }
        };
        let Some(base) = get(&tree, &steps) else {
            out.push(problem(
                at,
                "isn't in the model: a switch can only change what the model has",
            ));
            continue;
        };
        let c = classify::classify_path(model, &steps, &sim);
        let allowed = matches!(c.class, Class::Parameter | Class::Mechanic)
            && c.mechanic.is_some()
            && matches!(steps.first(), Some(Step::Key(k)) if SWITCHABLE.contains(&k.as_str()));
        if !allowed {
            out.push(problem(
                at,
                format!(
                    "is {}{}: a switch can only change a mechanic's parameters and options",
                    c.class.name(),
                    c.mechanic.map(|m| format!(" of {m}")).unwrap_or_default()
                ),
            ));
            continue;
        }
        if let Some(v) = value
            && !same_kind(v, base)
        {
            out.push(problem(
                at,
                format!(
                    "is a {} in the model, not a {}",
                    base.type_str(),
                    v.type_str()
                ),
            ));
        }
    }
    if out.is_empty() {
        match apply_unchecked(model, &s.name, &[s]) {
            Err(e) => out.push(problem(here, e)),
            Ok(m) => {
                // The evidence covers the base model's facts; a switch that
                // turns one off leaves its entry covering nothing, which is
                // fine here.
                for v in validate::validate(&m) {
                    if !v.path.starts_with("evidence") {
                        out.push(problem(here.clone(), format!("makes {v}")));
                    }
                }
            }
        }
    }
    out
}

/// Whether a value can stand where the base has `base`: the same TOML type,
/// or a number for a number, since a fixed-point number is an integer when
/// it's whole and a decimal string when it isn't.
fn same_kind(v: &toml::Value, base: &toml::Value) -> bool {
    use toml::Value::{Float, Integer, String};
    std::mem::discriminant(v) == std::mem::discriminant(base)
        || (matches!(v, Integer(_) | String(_) | Float(_))
            && matches!(base, Integer(_) | String(_)))
}

/// A path that names exactly one part, as steps.
fn concrete(text: &str) -> Result<Concrete, String> {
    let p = Path::parse(text)?;
    if !p.is_concrete() {
        return Err("must name one part: no `[*]`, range or filter".into());
    }
    let mut out = Vec::new();
    for s in p.segments {
        out.push(Step::Key(s.name));
        if let Some(path::Index::At(i)) = s.index {
            out.push(Step::Item(
                usize::try_from(i).map_err(|_| "a negative position")?,
            ));
        }
    }
    Ok(out)
}

fn get<'a>(node: &'a toml::Value, steps: &[Step]) -> Option<&'a toml::Value> {
    steps.iter().try_fold(node, |n, s| match s {
        Step::Key(k) => n.as_table()?.get(k),
        Step::Item(i) => n.as_array()?.get(*i),
    })
}

fn get_mut<'a>(node: &'a mut toml::Value, steps: &[Step]) -> Option<&'a mut toml::Value> {
    steps.iter().try_fold(node, |n, s| match s {
        Step::Key(k) => n.as_table_mut()?.get_mut(k),
        Step::Item(i) => n.as_array_mut()?.get_mut(*i),
    })
}

fn apply_unchecked(model: &Model, name: &str, switches: &[&Switch]) -> Result<Model, String> {
    let mut tree = model.to_tree();
    for s in switches {
        for (text, value) in &s.set {
            let steps = concrete(text)?;
            let slot =
                get_mut(&mut tree, &steps).ok_or_else(|| format!("`{text}` isn't in the model"))?;
            *slot = value.clone();
        }
        for text in &s.off {
            let steps = concrete(text)?;
            let (last, parent) = steps.split_last().ok_or("an empty path")?;
            let parent =
                get_mut(&mut tree, parent).ok_or_else(|| format!("`{text}` isn't in the model"))?;
            match (last, parent) {
                (Step::Key(k), toml::Value::Table(t)) => {
                    t.remove(k);
                }
                (Step::Item(i), toml::Value::Array(a)) if *i < a.len() => {
                    a.remove(*i);
                }
                _ => return Err(format!("`{text}` isn't in the model")),
            }
        }
    }
    let text = toml::to_string(&tree).map_err(|e| e.to_string())?;
    let mut m =
        Model::parse(&text).map_err(|e| format!("the model it makes doesn't parse: {e}"))?;
    m.model.profile = Some(name.to_owned());
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str) -> (Model, std::path::PathBuf) {
        let dir = FsPath::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates")
            .join(name);
        let m = Model::parse(&std::fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        (m, dir)
    }

    /// A made-up file for Moss Hop: a slower walk, no running and a lower
    /// fall limit, in two groups, and a preset of two of them.
    const SLOW: &str = r#"
format = "0.2.0"

[[groups]]
name = "feel"
title = "Feel"

[[groups.switches]]
name = "slow_walk"
description = "Walk at three quarters of the speed"

[groups.switches.set]
"entities.player.run.walk_speed" = "0.75"

[[groups.switches]]
name = "no_run"
description = "No running"
off = ["entities.player.run.run_speed", "entities.player.run.run_action"]

[[groups]]
name = "assist"

[[groups.switches]]
name = "soft_fall"
description = "Fall at most 3 pixels a frame"

[groups.switches.set]
"entities.player.jump.max_fall" = 3

[presets.slow]
description = "Slow, with no run"
switches = ["slow_walk", "no_run"]
"#;

    #[test]
    fn switches_combine_and_name_the_set_they_make() {
        let (m, _) = template("mosshop");
        let p = Profiles::parse(SLOW).unwrap();
        assert_eq!(p.check(&m), Vec::new());
        assert_eq!(p.names(), ["retro", "slow"]);
        let order: Vec<(&str, &str)> = p
            .switches()
            .map(|(g, s)| (g.heading(), s.name.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("Feel", "slow_walk"),
                ("Feel", "no_run"),
                ("assist", "soft_fall")
            ]
        );
        // A preset, a switch, and switches and presets joined by `+`; the
        // name is the switches sorted.
        assert_eq!(p.resolve("slow").unwrap(), ["no_run", "slow_walk"]);
        assert_eq!(p.resolve("retro").unwrap(), Vec::<String>::new());
        assert_eq!(
            p.resolve("soft_fall+slow").unwrap(),
            ["no_run", "slow_walk", "soft_fall"]
        );
        assert!(
            p.resolve("slow+fast")
                .unwrap_err()
                .contains("no profile or switch named `fast`")
        );
        let slow = p.apply(&m, "slow").unwrap();
        let run = slow.entities["player"].platformer.run.as_ref().unwrap();
        assert_eq!(run.walk_speed.to_string(), "0.75");
        assert!(run.run_speed.is_none() && run.run_action.is_none());
        assert_eq!(slow.model.profile.as_deref(), Some("no_run+slow_walk"));
        // The same set by its switches is the same model.
        assert_eq!(p.apply(&m, "slow_walk+no_run").unwrap(), slow);
        let all = p.apply(&m, "slow+soft_fall").unwrap();
        assert_eq!(
            all.entities["player"]
                .platformer
                .jump
                .as_ref()
                .unwrap()
                .max_fall
                .to_string(),
            "3"
        );
        assert_eq!(
            all.model.profile.as_deref(),
            Some("no_run+slow_walk+soft_fall")
        );
        // It reads back as it is, and validates.
        assert_eq!(Model::parse(&all.to_toml()).unwrap(), all);
        assert_eq!(
            validate::validate(&all)
                .iter()
                .filter(|p| !p.path.starts_with("evidence"))
                .count(),
            0
        );
        // The base is the model as written.
        assert_eq!(p.apply(&m, BASE).unwrap(), m);
        assert_eq!(Profiles::name_of::<&str>(&[]), None);
    }

    #[test]
    fn the_base_models_text_and_hash_are_the_same_with_switches() {
        let (m, _) = template("mosshop");
        let before = m.to_toml();
        let p = Profiles::parse(SLOW).unwrap();
        let slow = p.apply(&m, "slow+soft_fall").unwrap();
        assert_eq!(m.to_toml(), before);
        assert_ne!(slow.to_toml(), before);
        // Only the overridden facts and the header's profile differ.
        let changes: Vec<String> = crate::diff::diff(&m.to_tree(), &slow.to_tree())
            .iter()
            .map(|c| c.path().to_owned())
            .collect();
        assert_eq!(
            changes,
            [
                "entities.player.jump.max_fall",
                "entities.player.run.run_action",
                "entities.player.run.run_speed",
                "entities.player.run.walk_speed",
                "model.profile",
            ]
        );
    }

    fn one(body: &str) -> String {
        format!(
            "format = \"0.2.0\"\n[[groups]]\nname = \"g\"\n[[groups.switches]]\nname = \"a\"\ndescription = \"A\"\n{body}"
        )
    }

    /// `switch.rules`: a switch changes a game rule's timing, and the model
    /// it makes runs the rule with the new value.
    #[test]
    fn a_switch_changes_a_rules_timing() {
        let (m, _) = template("mosshop");
        let p = Profiles::parse(&one(
            "[groups.switches.set]\n\"rules.game.states.dying.transition[0].after\" = 30",
        ))
        .unwrap();
        assert!(p.check(&m).is_empty(), "{:?}", p.check(&m));
        let after = |m: &Model| m.rules["game"].states["dying"].transition[0].after;
        assert_eq!(after(&m), Some(90));
        let quick = p.apply(&m, "a").unwrap();
        assert_eq!(after(&quick), Some(30));
        assert_eq!(quick.model.profile.as_deref(), Some("a"));
    }

    #[test]
    fn bad_paths_names_and_overlaps_are_refused() {
        let (m, _) = template("mosshop");
        let bad = |text: &str, want: &str| {
            let p = Profiles::parse(text).unwrap();
            let problems = p.check(&m);
            assert!(
                problems.iter().any(|x| x.to_string().contains(want)),
                "{want}: {problems:?}"
            );
        };
        bad(
            &one("off = [\"entities.player.run.skid_by\"]"),
            "isn't in the model",
        );
        bad(
            &one("off = [\"entities.ghost.run.walk_speed\"]"),
            "isn't in the model",
        );
        bad(
            &one("[groups.switches.set]\n\"levels.main.goal.x\" = 3"),
            "is content",
        );
        bad(
            &one("[groups.switches.set]\n\"entities.player.sprite.palette\" = 3"),
            "is presentation",
        );
        bad(
            &one("[groups.switches.set]\n\"model.systems\" = []"),
            "a mechanic's parameters and options",
        );
        bad(
            &one("[groups.switches.set]\n\"entities.player.run.air_control\" = 1"),
            "is a boolean in the model, not a integer",
        );
        bad(
            &one("off = [\"entities[*].run.walk_speed\"]"),
            "must name one part",
        );
        bad(
            &one("off = [\"entities.player.run.air_control\"]"),
            "doesn't parse",
        );
        bad(&one(""), "changes nothing");
        bad(
            &one("off = [\"entities.player.run.run_speed\"]").replace("\"A\"", "\"\""),
            "needs a description",
        );
        bad(
            &one("off = [\"entities.player.run.run_speed\"]").replace("\"a\"", "\"retro\""),
            "can't be a switch",
        );
        // Two switches on the same part, or a part and one inside it,
        // can't combine.
        let two = |a: &str, b: &str| {
            one(&format!(
                "off = [\"{a}\"]\n[[groups.switches]]\nname = \"b\"\ndescription = \"B\"\noff = [\"{b}\"]\n"
            ))
        };
        bad(
            &two(
                "entities.player.run.run_speed",
                "entities.player.run.run_speed",
            ),
            "as switch `a` does",
        );
        bad(
            &two("entities.player.jump", "entities.player.jump.max_fall"),
            "as switch `a` does",
        );
        let p = Profiles::parse(&one(
            "[groups.switches.set]\n\"entities.player.jump.max_fall\" = 3\n\
             [[groups.switches]]\nname = \"b\"\ndescription = \"B\"\n\
             [groups.switches.set]\n\"entities.player.jump.max_fall\" = 2\n",
        ))
        .unwrap();
        let e = p.apply(&m, "a+b").unwrap_err();
        assert!(e.contains("both change"), "{e}");
        // Presets.
        let ok = one("[groups.switches.set]\n\"entities.player.jump.max_fall\" = 3\n");
        bad(
            &format!("{ok}[presets.retro]\nswitches = [\"a\"]\n"),
            "can't be redefined",
        );
        bad(
            &format!("{ok}[presets.p]\nswitches = [\"z\"]\n"),
            "which isn't a switch",
        );
        bad(
            &format!("{ok}[presets.p]\nswitches = []\n"),
            "turns no switch on",
        );
        bad(
            &format!("{ok}[presets.a]\nswitches = [\"a\"]\n"),
            "is also a preset's name",
        );
    }

    #[test]
    fn the_old_format_is_refused() {
        assert!(Profiles::parse("format = \"0.1.0\"\n[profiles.normal]\noff = []\n").is_err());
        let (m, _) = template("mosshop");
        let p = Profiles::parse("format = \"0.1.0\"\n").unwrap();
        assert!(p.check(&m)[0].message.contains("isn't supported"));
    }

    #[test]
    fn moss_hops_assist_checks() {
        let (m, dir) = template("mosshop");
        let p = Profiles::read_dir(&dir).unwrap();
        assert_eq!(p.check(&m), Vec::new());
        assert_eq!(p.names(), ["retro", "assist", "normal"]);
        let a = p.apply(&m, "assist").unwrap();
        assert_eq!(a.model.profile.as_deref(), Some("easy_stomp+slow_slugs"));
        let slug = &a.entities["slug"].platformer;
        assert_eq!(slug.patrol.as_ref().unwrap().period, 4);
        assert_eq!(
            slug.contact
                .as_ref()
                .unwrap()
                .stomp_depth
                .unwrap()
                .to_string(),
            "12"
        );
    }

    #[test]
    fn the_standard_asks_for_normal_with_every_timing_switch() {
        let (m, _) = template("mosshop");
        // No `normal`: the standard's one problem.
        let p = Profiles::parse(SLOW).unwrap();
        let got = p.standard_problems();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].path, "presets.normal");
        // An empty `normal` is allowed, and only `normal` may be empty.
        let empty = format!("{SLOW}\n[presets.normal]\nswitches = []\n");
        let p = Profiles::parse(&empty).unwrap();
        assert!(p.check(&m).is_empty(), "{:?}", p.check(&m));
        assert!(p.standard_problems().is_empty());
        assert!(p.resolve(NORMAL).unwrap().is_empty());
        let other = format!("{SLOW}\n[presets.none]\nswitches = []\n");
        let p = Profiles::parse(&other).unwrap();
        assert!(p.check(&m).iter().any(|x| x.path == "presets.none"));
        // A timing switch `normal` leaves out is refused.
        let timing = SLOW.replace("name = \"assist\"", "name = \"timing\"");
        let p = Profiles::parse(&format!(
            "{timing}\n[presets.normal]\nswitches = [\"slow_walk\"]\n"
        ))
        .unwrap();
        let got = p.standard_problems();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].message.contains("soft_fall"), "{got:?}");
    }

    #[test]
    fn every_template_keeps_to_the_settings_standard() {
        let root = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let mut seen = 0;
        for e in std::fs::read_dir(&root).unwrap() {
            let dir = e.unwrap().path();
            if !dir.join("model.toml").is_file() {
                continue;
            }
            seen += 1;
            assert!(
                dir.join(PROFILES_FILE).is_file(),
                "{} has no profiles.toml (docs/settings.md)",
                dir.display()
            );
            let p = Profiles::read_dir(&dir).unwrap();
            let got = p.standard_problems();
            assert!(got.is_empty(), "{}: {got:?}", dir.display());
        }
        assert!(seen >= 1);
    }
}
