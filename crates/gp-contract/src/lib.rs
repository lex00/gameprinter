//! The fidelity contract, `fidelity.toml`: which level of fidelity each part
//! of a model is held to (ARCHITECTURE.md, docs/formats/contract.md).
//!
//! A contract is a default level and a list of rules. Each rule names parts
//! of the model by a model path, which may be a pattern, and gives them a
//! level, optionally for one aspect of them and with metric tolerances.
//! [`Contract::resolve`] works out which rule governs a fact; `gp-validate`
//! compiles the rules into checks.
//!
//! Resolution, for one fact and one aspect:
//!
//! 1. A rule applies when its path names the fact or a part the fact is in,
//!    and it has no aspect or the aspect asked about.
//! 2. Among the rules that apply without a `when`, the one whose named part
//!    is nearest the fact wins (the longest path), then the one with more
//!    exact steps (names and positions over filters and ranges, over
//!    `[*]`), then one for this aspect over one for every aspect.
//! 3. Rules still tied with different levels conflict; [`Contract::check`]
//!    reports them, and resolution takes the first in the file.
//! 4. With no rule, the contract's `default` applies.
//! 5. Rules with a `when` that apply and are at least as specific as the
//!    winner are listed as conditional: they replace the winner while their
//!    condition holds.
//!
//! Metric tolerances inherit: each one comes from the most specific
//! applying metric rule that sets it.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use gp_model::path::{self, Concrete, Path};
use gp_model::{Fixed, Model};

/// The contract format's version.
pub const VERSION: &str = "0.2.0";

/// Earlier format versions this build also reads. A 0.1 contract is a 0.2
/// contract that uses no `when`: 0.2.0 gave `when` its grammar.
pub const ALSO_READS: &[&str] = &["0.1.0"];

/// The file a model directory keeps its contract in.
pub const CONTRACT_FILE: &str = "fidelity.toml";

/// One `fidelity.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer fidelity contract 0.2.0")]
pub struct Contract {
    pub contract: Header,
    #[serde(default, rename = "rule", skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Features `corpus.together` and `corpus.longest_stretch`: what each
    /// clip of the scoring corpus the model is scored against must reach
    /// (docs/formats/scoring-corpus.md), checked by `gp corpus score`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus: Option<CorpusBars>,
}

/// The bars every clip of the model's scoring corpus is held to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorpusBars {
    /// Each clip's frames together, run straight through from its start
    /// (free mode, with the original's draws when the state mapping
    /// scores with them), as a percentage: "100%".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub together: Option<String>,
    /// Each long run's (a clip of more than 600 frames) longest exact
    /// stretch in resync mode, counting the one its end cuts off, in
    /// seconds: "10s".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longest_stretch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl CorpusBars {
    /// `together` in tenths of a percent.
    pub fn together_permille(&self) -> Result<Option<u64>, String> {
        let Some(t) = &self.together else {
            return Ok(None);
        };
        let n = t
            .strip_suffix('%')
            .and_then(|n| Fixed::parse(n.trim()).ok())
            .filter(|f| f.raw() >= 0 && f.raw() <= 100 << 16)
            .ok_or_else(|| format!("together `{t}` isn't a percentage from 0% to 100%"))?;
        Ok(Some((n.raw() as u64 * 10) >> 16))
    }

    /// `longest_stretch` in NTSC frames (60.0988 a second), rounded up.
    pub fn longest_frames(&self) -> Result<Option<u64>, String> {
        let Some(t) = &self.longest_stretch else {
            return Ok(None);
        };
        let n = t
            .strip_suffix('s')
            .and_then(|n| Fixed::parse(n.trim()).ok())
            .filter(|f| f.raw() >= 0)
            .ok_or_else(|| format!("longest_stretch `{t}` isn't seconds, such as \"10s\""))?;
        // Seconds times 60.0988, from 16.16 fixed point, rounded up.
        let milli = (n.raw() as u128 * 60_099) >> 16;
        Ok(Some(milli.div_ceil(1000) as u64))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// The contract format version, such as "0.2.0".
    pub schema: String,
    /// The `model.name` of the model it applies to.
    pub model: String,
    /// The level of any fact no rule names.
    pub default: Level,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// A model path or pattern (docs/formats/model-path.md).
    pub path: String,
    pub level: Level,
    /// One aspect of the parts named, such as "arc" or "reach", when the
    /// rule shouldn't cover all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// For metric rules: how far each measured quantity may be from the
    /// original's, as a percentage ("5%") or in the quantity's own units.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tolerance: BTreeMap<String, Tolerance>,
    /// A condition over runtime state while which the rule holds, such as
    /// `world == 1`: the run-until condition grammar of
    /// docs/runtime-protocol.md over state paths, evaluated on each frame
    /// of the original's run (docs/validation.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Why the rule is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// State equal to the original's every frame, through a state mapping.
    Exact,
    /// Measured behavior within tolerances.
    Metric,
    /// Same layout and connections, numbers free.
    Structural,
    /// Plays alike, judged by bots.
    Experiential,
    /// Looks and sounds alike, judged by embeddings and critics.
    Style,
    /// Nothing is checked.
    Free,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Self::Exact,
        Self::Metric,
        Self::Structural,
        Self::Experiential,
        Self::Style,
        Self::Free,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Metric => "metric",
            Self::Structural => "structural",
            Self::Experiential => "experiential",
            Self::Style => "style",
            Self::Free => "free",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How far a measured quantity may be from the original's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tolerance {
    /// A percentage of the original's value: "5%".
    Relative(Fixed),
    /// In the quantity's units (pixels, frames): 2 or "0.5".
    Absolute(Fixed),
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relative(p) => write!(f, "{p}%"),
            Self::Absolute(a) => write!(f, "{a}"),
        }
    }
}

impl Serialize for Tolerance {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Relative(_) => s.serialize_str(&self.to_string()),
            Self::Absolute(a) => a.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Tolerance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = toml::Value::deserialize(d)?;
        let t = match &v {
            toml::Value::String(s) if s.ends_with('%') => {
                Fixed::parse(&s[..s.len() - 1]).map(Tolerance::Relative)
            }
            _ => v
                .try_into::<Fixed>()
                .map(Tolerance::Absolute)
                .map_err(|e| e.message().to_owned()),
        }
        .map_err(D::Error::custom)?;
        let (Tolerance::Relative(n) | Tolerance::Absolute(n)) = t;
        if n < Fixed::ZERO {
            return Err(D::Error::custom("a tolerance can't be negative"));
        }
        Ok(t)
    }
}

impl JsonSchema for Tolerance {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Tolerance".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A percentage such as \"5%\", or an amount in the quantity's units: an integer or an exact decimal string.",
            "oneOf": [
                { "type": "integer", "minimum": 0 },
                { "type": "string", "pattern": "^[0-9]+(\\.[0-9]{1,16})?%?$" }
            ]
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Parse(String),
}

/// A problem with a contract, by the rule it's about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Problem {
    /// `rule[N]` (numbered from 0, in file order) or `contract.<field>`.
    pub at: String,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// Which rule governs a fact, and at what level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub level: Level,
    /// The winning rule's index, or `None` for the default.
    pub rule: Option<usize>,
    /// Tolerances for a metric level.
    pub tolerance: BTreeMap<String, Tolerance>,
    /// Rules with a `when` that replace the winner while it holds.
    pub conditional: Vec<usize>,
}

/// A contract with its rules' paths parsed and resolved against one model.
pub struct Bound<'a> {
    contract: &'a Contract,
    /// Each rule's parsed path and the parts it names; `None` when the path
    /// doesn't parse.
    rules: Vec<Option<(Path, Vec<Concrete>)>>,
}

impl Contract {
    pub fn parse(text: &str) -> Result<Self, Error> {
        toml::from_str(text).map_err(|e| Error::Parse(e.to_string()))
    }

    /// The canonical text.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a parsed contract serializes")
    }

    /// Resolves every rule's path against the model.
    pub fn bind<'a>(&'a self, model: &Model) -> Bound<'a> {
        let tree = model.to_tree();
        let rules = self
            .rules
            .iter()
            .map(|r| {
                Path::parse(&r.path).ok().map(|p| {
                    let found = p.resolve(&tree);
                    (p, found)
                })
            })
            .collect();
        Bound {
            contract: self,
            rules,
        }
    }

    /// Every problem with the contract for this model: version, model name,
    /// paths that don't parse or name nothing, tolerances on rules that
    /// aren't metric, and conflicting rules.
    pub fn check(&self, model: &Model) -> Vec<Problem> {
        let mut out = Vec::new();
        let mut add = |at: String, m: String| out.push(Problem { at, message: m });
        let supported = std::iter::once(VERSION)
            .chain(ALSO_READS.iter().copied())
            .any(|v| gp_model::validate::version_supported(&self.contract.schema, v));
        if !supported {
            add(
                "contract.schema".into(),
                format!(
                    "contract format {} isn't supported; this build reads {VERSION}",
                    self.contract.schema
                ),
            );
        }
        if self.contract.model != model.model.name {
            add(
                "contract.model".into(),
                format!(
                    "is for `{}`, but the model is `{}`",
                    self.contract.model, model.model.name
                ),
            );
        }
        if let Some(c) = &self.corpus {
            if let Err(e) = c.together_permille() {
                add("corpus.together".into(), e);
            }
            if let Err(e) = c.longest_frames() {
                add("corpus.longest_stretch".into(), e);
            }
        }
        let bound = self.bind(model);
        let facts: Vec<Concrete> = gp_model::facts(&model.to_tree())
            .into_iter()
            .map(|(f, _)| f)
            .collect();
        for (i, r) in self.rules.iter().enumerate() {
            let at = format!("rule[{i}]");
            match Path::parse(&r.path) {
                Err(e) => add(at.clone(), e),
                Ok(p) if p.segments[0].name == "model" || p.segments[0].name == "evidence" => add(
                    at.clone(),
                    format!(
                        "`{}`: the header and the evidence table have no fidelity",
                        r.path
                    ),
                ),
                Ok(_) if bound.rules[i].as_ref().is_some_and(|(_, f)| f.is_empty()) => add(
                    at.clone(),
                    format!("`{}` names no part of the model", r.path),
                ),
                Ok(_)
                    if bound.rules[i].as_ref().is_some_and(|(_, f)| {
                        !f.iter()
                            .any(|part| gp_model::validate::holds_facts(&facts, part))
                    }) =>
                {
                    add(
                        at.clone(),
                        format!(
                            "`{}` names no fact (descriptions and items of a plain list aren't facts)",
                            r.path
                        ),
                    )
                }
                Ok(_) => {}
            }
            if !r.tolerance.is_empty() && r.level != Level::Metric {
                add(
                    at.clone(),
                    format!(
                        "tolerances only apply to metric rules, and this one is {}",
                        r.level
                    ),
                );
            }
            for name in r.tolerance.keys() {
                if !path::is_name(name) {
                    add(at.clone(), format!("`{name}` isn't a quantity name"));
                }
            }
            if let Some(a) = &r.aspect
                && !path::is_name(a)
            {
                add(at.clone(), format!("`{a}` isn't an aspect name"));
            }
            match r.when.as_deref() {
                Some(w) if w.trim().is_empty() => add(at.clone(), "`when` is empty".into()),
                Some(w) => {
                    if let Err(e) = gp_runtime::select::Condition::parse(w) {
                        add(at.clone(), format!("`when` isn't a condition: {e}"));
                    }
                }
                None => {}
            }
        }
        // Conflicts: two rules without `when`, at the same specificity for
        // some fact and aspect, with different levels.
        let tree = model.to_tree();
        let mut seen = std::collections::BTreeSet::new();
        for (fact, _) in gp_model::facts(&tree) {
            let aspects: Vec<Option<&str>> = std::iter::once(None)
                .chain(
                    self.rules
                        .iter()
                        .filter_map(|r| r.aspect.as_deref().map(Some)),
                )
                .collect();
            for aspect in aspects {
                let tied = bound.tied(&fact, aspect);
                if let [first, rest @ ..] = &tied[..] {
                    for other in rest {
                        if self.rules[*first].level != self.rules[*other].level
                            && seen.insert((*first, *other))
                        {
                            add(
                                format!("rule[{other}]"),
                                format!(
                                    "conflicts with rule[{first}]: both are the most specific rule for `{}`{}, one {} and one {}",
                                    path::display(&fact),
                                    aspect.map(|a| format!(" (aspect {a})")).unwrap_or_default(),
                                    self.rules[*first].level,
                                    self.rules[*other].level
                                ),
                            );
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }
}

/// How a rule ranks for a fact: nearest part, exact steps, aspect-specific.
type Rank = ((usize, usize), bool);

impl Bound<'_> {
    /// The rules that apply to a fact for an aspect, with their rank, in
    /// file order.
    fn applying(&self, fact: &[path::Step], aspect: Option<&str>) -> Vec<(usize, Rank)> {
        let mut out = Vec::new();
        for (i, r) in self.contract.rules.iter().enumerate() {
            let Some((p, found)) = &self.rules[i] else {
                continue;
            };
            let aspect_ok = match (&r.aspect, aspect) {
                (None, _) => true,
                (Some(a), Some(b)) => a == b,
                (Some(_), None) => false,
            };
            if aspect_ok && found.iter().any(|part| fact.starts_with(part)) {
                out.push((i, (p.specificity(), r.aspect.is_some())));
            }
        }
        out
    }

    /// The unconditional rules tied for the top rank.
    fn tied(&self, fact: &[path::Step], aspect: Option<&str>) -> Vec<usize> {
        let plain: Vec<(usize, Rank)> = self
            .applying(fact, aspect)
            .into_iter()
            .filter(|(i, _)| self.contract.rules[*i].when.is_none())
            .collect();
        let Some(top) = plain.iter().map(|(_, r)| *r).max() else {
            return Vec::new();
        };
        plain
            .into_iter()
            .filter(|(_, r)| *r == top)
            .map(|(i, _)| i)
            .collect()
    }

    /// The level that governs a fact (a concrete path, as from
    /// [`gp_model::facts`]) for an aspect, or for the fact as a whole with
    /// `None`.
    pub fn resolve(&self, fact: &[path::Step], aspect: Option<&str>) -> Resolved {
        let applying = self.applying(fact, aspect);
        let rules = &self.contract.rules;
        let winner = self.tied(fact, aspect).first().copied();
        let top = winner.and_then(|w| applying.iter().find(|(i, _)| *i == w).map(|(_, r)| *r));
        let level = winner.map_or(self.contract.contract.default, |w| rules[w].level);

        let mut tolerance = BTreeMap::new();
        if level == Level::Metric {
            // Least specific first, so more specific rules overwrite.
            let mut metric: Vec<&(usize, Rank)> = applying
                .iter()
                .filter(|(i, _)| rules[*i].level == Level::Metric && rules[*i].when.is_none())
                .collect();
            // Among equally specific rules the first in the file wins, as
            // it does for the level, so it goes last.
            metric.sort_by_key(|(i, r)| (*r, std::cmp::Reverse(*i)));
            for (i, _) in metric {
                for (k, v) in &rules[*i].tolerance {
                    tolerance.insert(k.clone(), *v);
                }
            }
        }
        let conditional = applying
            .iter()
            .filter(|(i, r)| rules[*i].when.is_some() && top.is_none_or(|t| *r >= t))
            .map(|(i, _)| *i)
            .collect();
        Resolved {
            level,
            rule: winner,
            tolerance,
            conditional,
        }
    }

    /// How many facts of the model resolve to each level, for the fact as
    /// a whole.
    pub fn summary(&self, model: &Model) -> BTreeMap<Level, usize> {
        let tree = model.to_tree();
        let mut out = BTreeMap::new();
        for (fact, _) in gp_model::facts(&tree) {
            *out.entry(self.resolve(&fact, None).level).or_insert(0) += 1;
        }
        out
    }
}

/// The contract format's JSON Schema, for `gp model schema contract`.
pub fn schema() -> String {
    let mut s = schemars::schema_for!(Contract);
    s.insert(
        "$id".into(),
        format!("game-printer:contract@{VERSION}").into(),
    );
    let mut text = serde_json::to_string_pretty(&s).expect("schemas serialize");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
