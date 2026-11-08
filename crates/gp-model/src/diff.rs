//! The differences between two models, by model path.
//!
//! Paths are model paths, except in the evidence table, whose keys are
//! themselves paths: a change there is labeled `evidence."<key>"` and then
//! the field, as `gp model check` labels its problems.
//!
//! Both models are compared in canonical form (see [`crate::Model::to_tree`]),
//! so `1` and `"1.0"`, or `0x10` and `16`, are the same value. Tables are
//! compared key by key; a list of tables item by item, by position; a list
//! of plain values, or any other value, as one value.

use serde::Serialize;

use crate::path::{self, Concrete, Step};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "lowercase")]
pub enum Change {
    Added {
        path: String,
        new: serde_json::Value,
    },
    Removed {
        path: String,
        old: serde_json::Value,
    },
    Changed {
        path: String,
        old: serde_json::Value,
        new: serde_json::Value,
    },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Self::Added { path, .. } | Self::Removed { path, .. } | Self::Changed { path, .. } => {
                path
            }
        }
    }
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added { path, new } => write!(f, "+ {path} = {new}"),
            Self::Removed { path, old } => write!(f, "- {path} = {old}"),
            Self::Changed { path, old, new } => write!(f, "~ {path}: {old} -> {new}"),
        }
    }
}

/// Every difference from `a` to `b`: tables key by key in key order, lists
/// in position order.
pub fn diff(a: &toml::Value, b: &toml::Value) -> Vec<Change> {
    let mut out = Vec::new();
    walk(Some(a), Some(b), Vec::new(), &mut out);
    out
}

fn json(v: &toml::Value) -> serde_json::Value {
    serde_json::to_value(v).expect("TOML values convert to JSON")
}

fn is_table_list(v: &toml::Value) -> bool {
    v.as_array()
        .is_some_and(|a| !a.is_empty() && a.iter().all(toml::Value::is_table))
}

fn walk(a: Option<&toml::Value>, b: Option<&toml::Value>, at: Concrete, out: &mut Vec<Change>) {
    let name = || label(&at);
    match (a, b) {
        (None, None) => {}
        (Some(a), None) => out.push(Change::Removed {
            path: name(),
            old: json(a),
        }),
        (None, Some(b)) => out.push(Change::Added {
            path: name(),
            new: json(b),
        }),
        (Some(toml::Value::Table(ta)), Some(toml::Value::Table(tb))) => {
            let mut keys: Vec<&String> = ta.keys().chain(tb.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let mut at = at.clone();
                at.push(Step::Key(k.clone()));
                walk(ta.get(k), tb.get(k), at, out);
            }
        }
        (Some(va), Some(vb)) if is_table_list(va) && is_table_list(vb) => {
            let (la, lb) = (va.as_array().unwrap(), vb.as_array().unwrap());
            for i in 0..la.len().max(lb.len()) {
                let mut at = at.clone();
                at.push(Step::Item(i));
                walk(la.get(i), lb.get(i), at, out);
            }
        }
        (Some(a), Some(b)) => {
            if a != b {
                out.push(Change::Changed {
                    path: name(),
                    old: json(a),
                    new: json(b),
                });
            }
        }
    }
}

/// A change's path: a model path, or `evidence."<key>"` and the rest.
fn label(at: &[Step]) -> String {
    match at {
        [Step::Key(e), Step::Key(key), rest @ ..] if e == "evidence" => {
            let rest = path::display(rest);
            let sep = if rest.is_empty() || rest.starts_with('[') {
                ""
            } else {
                "."
            };
            format!("evidence.\"{key}\"{sep}{rest}")
        }
        _ => path::display(at),
    }
}
