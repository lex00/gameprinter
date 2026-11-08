//! The game model: an engine-neutral description of a game, written as
//! TOML, that engine code is generated from (ARCHITECTURE.md, docs/model.md).
//!
//! - [`model`]: the core schema's types, version [`CORE_VERSION`].
//! - [`platformer`]: the platformer genre extension.
//! - [`topdown`]: the top-down genre extension.
//! - [`fixed`]: fixed-point numbers, the only numbers game logic reads.
//! - [`path`]: the model path grammar (docs/formats/model-path.md).
//! - [`validate`]: checks beyond the types: references, roles, events,
//!   extensions, and that every fact has evidence.
//! - [`classify`]: every model path as mechanic, parameter, content or
//!   presentation.
//! - [`diff`]: the differences between two models, by model path.
//! - [`features`]: named optional features, which a model lists or which
//!   are derived from what it uses, and [`feature_list`], the ones this
//!   build knows.
//! - [`profile`]: named sets of overrides of mechanic parameters and
//!   options (`profiles.toml`), applied when a game is built or played.
//!
//! A model's canonical text is what [`Model::to_toml`] writes: tables and
//! keys in a fixed order, whole numbers as integers and fractions as exact
//! decimal strings, so two tools writing the same model write the same
//! bytes, and a change to one fact is a change to one line.

pub mod classify;
pub mod diff;
pub mod feature_list;
pub mod features;
pub mod fixed;
pub mod model;
pub mod path;
pub mod platformer;
pub mod profile;
pub mod schema;
pub mod topdown;
pub mod validate;

pub use fixed::Fixed;
pub use model::{CORE_SYSTEMS, CORE_VERSION, Model};
pub use path::Path;
pub use validate::Problem;

use path::{Concrete, Step};

/// The file a model directory keeps its model in.
pub const MODEL_FILE: &str = "model.toml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The text isn't TOML, or doesn't fit the schema's types.
    #[error("{0}")]
    Parse(String),
    /// The model lists features this build doesn't know.
    #[error("{message}")]
    Features { names: Vec<String>, message: String },
}

impl Model {
    /// Parses a model's text. This checks the types only; [`validate`]
    /// checks the rest. A model that lists a feature this build doesn't
    /// know is refused first, naming it ([`Error::Features`]).
    pub fn parse(text: &str) -> Result<Self, Error> {
        let listed = features::listed_in(text, Some("model"));
        let unknown = features::unknown(&listed, feature_list::FEATURES);
        if !unknown.is_empty() {
            return Err(Error::Features {
                message: features::refusal("the model", &unknown),
                names: unknown.into_iter().map(str::to_owned).collect(),
            });
        }
        let mut m: Model = toml::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        for e in m.entities.values_mut() {
            e.canonicalize_sets();
        }
        Ok(m)
    }

    /// The features the model uses, derived from the fields it sets.
    pub fn features_used(&self) -> std::collections::BTreeSet<&'static str> {
        features::used(&self.to_tree(), feature_list::FEATURES)
    }

    /// The canonical text.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a parsed model serializes")
    }

    /// The model as a TOML tree, for paths, evidence and diffs. Numbers are
    /// in canonical form, so two models that mean the same give the same
    /// tree.
    pub fn to_tree(&self) -> toml::Value {
        toml::Value::try_from(self).expect("a parsed model serializes")
    }
}

/// True for a key that holds notes rather than facts.
fn is_note(key: &str) -> bool {
    key == "description"
}

/// Every fact in a model tree: each leaf value, by concrete path. A list of
/// tables is walked into; a list of plain values is one fact. The header
/// (`model`), the evidence table and descriptions aren't facts.
pub fn facts(tree: &toml::Value) -> Vec<(Concrete, &toml::Value)> {
    let mut out = Vec::new();
    if let Some(t) = tree.as_table() {
        for (k, v) in t {
            if k != "model" && k != "evidence" {
                walk(v, vec![Step::Key(k.clone())], &mut out);
            }
        }
    }
    out
}

fn walk<'a>(node: &'a toml::Value, at: Concrete, out: &mut Vec<(Concrete, &'a toml::Value)>) {
    match node {
        toml::Value::Table(t) => {
            for (k, v) in t {
                if !is_note(k) {
                    let mut at = at.clone();
                    at.push(Step::Key(k.clone()));
                    walk(v, at, out);
                }
            }
        }
        toml::Value::Array(items) if items.iter().any(toml::Value::is_table) => {
            for (i, v) in items.iter().enumerate() {
                let mut at = at.clone();
                at.push(Step::Item(i));
                walk(v, at, out);
            }
        }
        _ => out.push((at, node)),
    }
}

#[cfg(test)]
mod tests;
