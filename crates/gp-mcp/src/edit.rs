//! Path-addressed edits to a model, and operations on a contract's rules.
//! An edit works on the model's TOML tree; the result is parsed again and
//! written in canonical form, so whatever the agent sends, the file stays
//! what `gp model fmt` writes.

use gp_contract::{Contract, Level, Rule};
use gp_model::Path;
use gp_model::path::Index;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as Json;
use toml::Value;

/// One change to a model, at a concrete model path.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    /// A concrete model path, such as `entities.player.jump.gravity` or
    /// `levels.main.spawn[1].x` (docs/formats/model-path.md). It may name
    /// a key that doesn't exist yet in a table that does, or one past the
    /// end of a list to append.
    pub path: String,
    /// The new value: an integer, a string (fractions are exact decimal
    /// strings, such as "0.25"), a boolean, a list or a table. No floats.
    #[serde(default)]
    pub value: Option<Json>,
    /// Remove the part instead of setting it.
    #[serde(default)]
    pub remove: bool,
}

/// Converts a JSON value to the TOML a model holds. Floats are refused,
/// as the model refuses them: fractions are strings.
pub fn to_toml(v: &Json) -> Result<Value, String> {
    Ok(match v {
        Json::Null => return Err("null isn't a model value; use remove".into()),
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => {
                return Err(format!(
                    "{n} isn't an integer; write a fraction as an exact decimal string, such as \"0.25\""
                ));
            }
        },
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::Array(items.iter().map(to_toml).collect::<Result<_, _>>()?),
        Json::Object(map) => {
            let mut t = toml::Table::new();
            for (k, v) in map {
                t.insert(k.clone(), to_toml(v)?);
            }
            Value::Table(t)
        }
    })
}

/// Applies one edit to a model tree.
pub fn apply(tree: &mut Value, edit: &Edit) -> Result<(), String> {
    let path = Path::parse(&edit.path).map_err(|e| format!("`{}`: {e}", edit.path))?;
    if !path.is_concrete() {
        return Err(format!(
            "`{}` is a pattern; an edit names one part, so give each part its own edit",
            edit.path
        ));
    }
    if path.segments[0].name == "model" {
        return Err("the `model` header isn't editable through model_edit: it names the model, its source and its schema".into());
    }
    if path.segments[0].name == "evidence" {
        return Err("model paths don't reach into the evidence table; give `evidence` with the edit instead".into());
    }
    let new = match (&edit.value, edit.remove) {
        (Some(_), true) => {
            return Err(format!("`{}`: give a value or remove, not both", edit.path));
        }
        (None, false) => return Err(format!("`{}`: give a value, or remove", edit.path)),
        (Some(v), false) => Some(to_toml(v)?),
        (None, true) => None,
    };
    let (last, init) = path.segments.split_last().expect("a path has a segment");
    let mut node = tree;
    let mut at = String::new();
    for seg in init {
        node = step_into(node, &seg.name, seg.index.as_ref(), &mut at)?;
    }
    let table = node
        .as_table_mut()
        .ok_or_else(|| format!("`{at}` isn't a table"))?;
    let shown = if at.is_empty() {
        last.name.clone()
    } else {
        format!("{at}.{}", last.name)
    };
    match &last.index {
        None => match new {
            Some(v) => {
                table.insert(last.name.clone(), v);
            }
            None => {
                table
                    .remove(&last.name)
                    .ok_or_else(|| format!("`{shown}` isn't in the model"))?;
            }
        },
        Some(Index::At(i)) => {
            let list = table
                .get_mut(&last.name)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| format!("`{shown}` isn't a list"))?;
            let i = usize::try_from(*i)
                .map_err(|_| format!("`{shown}[{i}]`: positions count from 0"))?;
            match new {
                Some(v) if i < list.len() => list[i] = v,
                Some(v) if i == list.len() => list.push(v),
                Some(_) => {
                    return Err(format!(
                        "`{shown}` has {} items; {i} is past the end",
                        list.len()
                    ));
                }
                None if i < list.len() => {
                    list.remove(i);
                }
                None => {
                    return Err(format!(
                        "`{shown}` has {} items, so there's no item {i}",
                        list.len()
                    ));
                }
            }
        }
        Some(_) => unreachable!("a concrete path"),
    }
    Ok(())
}

fn step_into<'a>(
    node: &'a mut Value,
    name: &str,
    index: Option<&Index>,
    at: &mut String,
) -> Result<&'a mut Value, String> {
    if !at.is_empty() {
        at.push('.');
    }
    at.push_str(name);
    let child = node
        .as_table_mut()
        .and_then(|t| t.get_mut(name))
        .ok_or_else(|| format!("`{at}` isn't in the model"))?;
    match index {
        None => Ok(child),
        Some(Index::At(i)) => {
            let shown = format!("{at}[{i}]");
            let item = usize::try_from(*i)
                .ok()
                .and_then(|i| child.as_array_mut().and_then(|l| l.get_mut(i)))
                .ok_or_else(|| format!("`{shown}` isn't in the model"))?;
            *at = shown;
            Ok(item)
        }
        Some(_) => unreachable!("a concrete path"),
    }
}

/// Removes the evidence entries that named a part of `before` and name
/// nothing in `after`, since the part they covered was removed, and returns
/// their keys. An edit can't reach the evidence table, so this is the only
/// way such an entry goes.
pub fn drop_dangling_evidence(before: &Value, after: &mut Value) -> Vec<String> {
    let resolves =
        |tree: &Value, key: &str| Path::parse(key).is_ok_and(|p| !p.resolve(tree).is_empty());
    let keys: Vec<String> = after
        .get("evidence")
        .and_then(Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    let dropped: Vec<String> = keys
        .into_iter()
        .filter(|k| resolves(before, k) && !resolves(after, k))
        .collect();
    if let Some(t) = after.get_mut("evidence").and_then(Value::as_table_mut) {
        for k in &dropped {
            t.remove(k);
        }
    }
    dropped
}

/// Where a contract operation applies: a rule's position, or its path when
/// exactly one rule has it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum At {
    Index(usize),
    Path(String),
}

/// One change to a contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum ContractOp {
    /// Append a rule.
    Add { rule: Rule },
    /// Replace a rule.
    Set { at: At, rule: Rule },
    /// Remove a rule.
    Remove { at: At },
    /// Change the level of facts no rule names.
    Default { level: Level },
}

fn find(contract: &Contract, at: &At) -> Result<usize, String> {
    match at {
        At::Index(i) if *i < contract.rules.len() => Ok(*i),
        At::Index(i) => Err(format!(
            "the contract has {} rules; there's no rule {i}",
            contract.rules.len()
        )),
        At::Path(p) => {
            let found: Vec<usize> = (0..contract.rules.len())
                .filter(|i| contract.rules[*i].path == *p)
                .collect();
            match found[..] {
                [i] => Ok(i),
                [] => Err(format!("no rule has the path `{p}`")),
                _ => Err(format!(
                    "rules {found:?} all have the path `{p}`; name one by its position"
                )),
            }
        }
    }
}

/// Applies contract operations in order.
pub fn apply_contract(contract: &mut Contract, ops: &[ContractOp]) -> Result<(), String> {
    for op in ops {
        match op {
            ContractOp::Add { rule } => contract.rules.push(rule.clone()),
            ContractOp::Set { at, rule } => {
                let i = find(contract, at)?;
                contract.rules[i] = rule.clone();
            }
            ContractOp::Remove { at } => {
                let i = find(contract, at)?;
                contract.rules.remove(i);
            }
            ContractOp::Default { level } => contract.contract.default = *level,
        }
    }
    Ok(())
}
