//! `gp template derive`: a template made as a small delta on another, such
//! as a ROM hack's on the game it was made from (docs/templates.md,
//! "Derived templates"). The manifest's `[derive]` names the base template
//! and a delta file; the derived model, contract, state mapping and the
//! other per-game files are the base's with every `source` naming the
//! base's source renamed to this template's, and the delta merged over
//! them. The files are committed so every tool reads them as usual, and
//! `gp template check` fails while one is stale, so a fix to the base
//! reaches the hack by running `derive --write`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use gp_contract::Contract;
use gp_model::Model;

use crate::Outcome;

/// The files a derived template takes from its base, when the base has
/// them. template.toml, the ext crate and notes for agents stay the
/// template's own.
pub(crate) const DERIVED_FILES: &[&str] = &[
    "model.toml",
    "fidelity.toml",
    "mapping.toml",
    "observe.toml",
    "profiles.toml",
    "sprites.toml",
];

/// The manifest's `[derive]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Derive {
    /// The base template's directory, relative to this one.
    pub from: String,
    /// The delta file in this directory: a table per derived file, keyed
    /// by its name, merged over the base's.
    pub delta: String,
    /// The template's own generator (format 0.3.2): shell commands, run in
    /// order from the repo's root by `just derive` (scripts/template/derive.sh)
    /// after the derive, with $GP naming the gp to run. They remake every
    /// file the template's generator writes (its authored data, delta and
    /// art), re-derive, rebuild and repaint, so nothing is left from before
    /// the base changed.
    #[serde(default)]
    #[allow(dead_code)]
    pub generate: Vec<String>,
}

/// One derived file: its name, what it should hold, and what it holds.
pub(crate) struct Derived {
    pub name: &'static str,
    pub want: String,
    pub have: Option<String>,
}

impl Derived {
    pub(crate) fn stale(&self) -> bool {
        self.have.as_deref() != Some(self.want.as_str())
    }
}

/// The source ID a template declares, read from its template.toml.
fn source_id(dir: &Path) -> Result<String> {
    let path = dir.join("template.toml");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let t: toml::Table = toml::from_str(&text).with_context(|| path.display().to_string())?;
    t.get("source")
        .and_then(|s| s.get("id"))
        .and_then(|s| s.as_str())
        .map(str::to_owned)
        .with_context(|| format!("{} has no source.id", path.display()))
}

/// Every string value of a key named `source` that is `from` becomes `to`.
fn rename_source(v: &mut toml::Value, from: &str, to: &str) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t.iter_mut() {
                match v {
                    toml::Value::String(s) if k == "source" && s == from => *s = to.to_owned(),
                    _ => rename_source(v, from, to),
                }
            }
        }
        toml::Value::Array(a) => a.iter_mut().for_each(|v| rename_source(v, from, to)),
        _ => {}
    }
}

/// The keys an array's tables are merged by, in order: an array whose
/// items all have a `name` (a profile's groups and switches) merges by it,
/// else one whose items all have a `path` (a state mapping's fields).
const ITEM_KEYS: &[&str] = &["name", "path"];

/// The `key` of an array item that is a table with one.
fn item_key<'a>(v: &'a toml::Value, key: &str) -> Option<&'a str> {
    v.as_table()?.get(key)?.as_str()
}

/// The key both arrays' items all have, if any.
fn shared_key(b: &[toml::Value], d: &[toml::Value]) -> Option<&'static str> {
    ITEM_KEYS.iter().copied().find(|k| {
        !b.is_empty() && !d.is_empty() && b.iter().chain(d).all(|v| item_key(v, k).is_some())
    })
}

/// Merges `delta` over `base`: tables key by key; arrays of tables that
/// all have a `name` item by item by name, new names appended (a profile's
/// groups and switches), and likewise by `path` (a state mapping's
/// fields); everything else replaced whole.
pub(crate) fn merge(base: &mut toml::Value, delta: &toml::Value) {
    match (base, delta) {
        (toml::Value::Table(b), toml::Value::Table(d)) => {
            for (k, dv) in d {
                match b.get_mut(k) {
                    Some(bv) if (bv.is_table() && dv.is_table()) || named(bv, dv) => merge(bv, dv),
                    _ => {
                        b.insert(k.clone(), dv.clone());
                    }
                }
            }
        }
        (toml::Value::Array(b), toml::Value::Array(d)) if shared_key(b, d).is_some() => {
            let key = shared_key(b, d).expect("checked");
            for dv in d {
                let name = item_key(dv, key);
                match b.iter_mut().find(|bv| item_key(bv, key) == name) {
                    Some(bv) => merge(bv, dv),
                    None => b.push(dv.clone()),
                }
            }
        }
        (b, d) => *b = d.clone(),
    }
}

/// Two arrays of tables keyed alike, which merge item by item.
fn named(b: &toml::Value, d: &toml::Value) -> bool {
    matches!((b, d), (toml::Value::Array(b), toml::Value::Array(d)) if shared_key(b, d).is_some())
}

/// What each derived file of `dir` should hold.
pub(crate) fn derive(dir: &Path, d: &Derive) -> Result<Vec<Derived>> {
    let base_dir: PathBuf = dir.join(&d.from);
    if !base_dir.join("template.toml").is_file() {
        bail!("derive.from {} isn't a template directory", d.from);
    }
    let (from_id, to_id) = (source_id(&base_dir)?, source_id(dir)?);
    let delta_path = dir.join(&d.delta);
    let delta_text = fs::read_to_string(&delta_path)
        .with_context(|| format!("reading {}", delta_path.display()))?;
    let delta: toml::Table =
        toml::from_str(&delta_text).with_context(|| delta_path.display().to_string())?;
    for k in delta.keys() {
        if !DERIVED_FILES.contains(&k.as_str()) {
            bail!(
                "{}: {k:?} isn't a derived file ({})",
                delta_path.display(),
                DERIVED_FILES.join(", ")
            );
        }
    }
    let mut out = Vec::new();
    for &name in DERIVED_FILES {
        let path = base_dir.join(name);
        if !path.is_file() {
            if delta.contains_key(name) {
                bail!("{}: the base has no {name} to change", delta_path.display());
            }
            continue;
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut value: toml::Value =
            toml::from_str(&text).with_context(|| path.display().to_string())?;
        rename_source(&mut value, &from_id, &to_id);
        if let Some(dv) = delta.get(name) {
            merge(&mut value, dv);
        }
        let plain = toml::to_string(&value)?;
        let want = match name {
            "model.toml" => Model::parse(&plain)
                .with_context(|| format!("the derived {name}"))?
                .to_toml(),
            "fidelity.toml" => Contract::parse(&plain)
                .with_context(|| format!("the derived {name}"))?
                .to_toml(),
            _ => format!(
                "# Derived by `gp template derive` from {}/{name} and {}: edit those, not this.\n{plain}",
                d.from, d.delta
            ),
        };
        let have = fs::read_to_string(dir.join(name)).ok();
        out.push(Derived { name, want, have });
    }
    Ok(out)
}

/// The manifest's `[derive]`, if it has one. gameprinter's template.toml
/// has no typed manifest, so the section is read on its own.
pub(crate) fn of_manifest(dir: &Path) -> Result<Option<Derive>> {
    let path = dir.join("template.toml");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let t: toml::Table = toml::from_str(&text).with_context(|| path.display().to_string())?;
    match t.get("derive") {
        None => Ok(None),
        Some(v) => {
            Ok(Some(v.clone().try_into().with_context(|| {
                format!("{}: [derive]", path.display())
            })?))
        }
    }
}

/// `gp template derive DIR [--write]`.
pub(crate) fn derive_cmd(dir: &Path, write: bool) -> Result<Outcome> {
    let Some(d) = of_manifest(dir)? else {
        bail!("{}/template.toml has no [derive]", dir.display());
    };
    let d = &d;
    let files = derive(dir, d)?;
    let stale: Vec<&str> = files.iter().filter(|f| f.stale()).map(|f| f.name).collect();
    if write {
        for f in files.iter().filter(|f| f.stale()) {
            let path = dir.join(f.name);
            fs::write(&path, &f.want).with_context(|| format!("writing {}", path.display()))?;
        }
    }
    let text = match (stale.is_empty(), write) {
        (true, _) => format!(
            "{} files derived from {} and {}, all current",
            files.len(),
            d.from,
            d.delta
        ),
        (false, true) => format!("wrote {}", stale.join(", ")),
        (false, false) => format!(
            "stale (run gp template derive {} --write): {}",
            dir.display(),
            stale.join(", ")
        ),
    };
    Ok(Outcome {
        json: json!({
            "dir": dir,
            "from": d.from,
            "delta": d.delta,
            "files": files.iter().map(|f| f.name).collect::<Vec<_>>(),
            "stale": stale,
            "written": write,
        }),
        text,
        exit: u8::from(!write && !stale.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_replaces_values_and_arrays_and_recurses_into_tables() {
        let mut base: toml::Value =
            toml::from_str("a = 1\nlist = [1, 2]\n[t]\nx = 1\ny = 2\n[t.deep]\nz = 3\n").unwrap();
        let delta: toml::Value =
            toml::from_str("list = [9]\n[t]\ny = 20\nnew = true\n[t.deep]\nz = 30\n").unwrap();
        merge(&mut base, &delta);
        let want: toml::Value =
            toml::from_str("a = 1\nlist = [9]\n[t]\nx = 1\ny = 20\nnew = true\n[t.deep]\nz = 30\n")
                .unwrap();
        assert_eq!(base, want);
    }

    #[test]
    fn arrays_of_named_tables_merge_by_name() {
        let mut base: toml::Value =
            toml::from_str("[[g]]\nname = \"a\"\nx = 1\n[[g]]\nname = \"b\"\nx = 2\n").unwrap();
        let delta: toml::Value =
            toml::from_str("[[g]]\nname = \"b\"\nx = 20\n[[g]]\nname = \"c\"\nx = 3\n").unwrap();
        merge(&mut base, &delta);
        let got: Vec<(String, i64)> = base["g"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap().into(),
                    v["x"].as_integer().unwrap(),
                )
            })
            .collect();
        assert_eq!(got, [("a".into(), 1), ("b".into(), 20), ("c".into(), 3)]);
    }

    #[test]
    fn mapping_fields_merge_by_path_and_keep_their_other_keys() {
        let mut base: toml::Value = toml::from_str(
            "[[field]]\npath = \"a\"\nram = [1]\n[[field]]\npath = \"b\"\nram = [2]\nlookup = [[1, 0]]\n",
        )
        .unwrap();
        let delta: toml::Value =
            toml::from_str("[[field]]\npath = \"b\"\nlookup = [[1, 0], [2, 1]]\n").unwrap();
        merge(&mut base, &delta);
        let f = base["field"].as_array().unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[1]["ram"].as_array().unwrap().len(), 1, "ram kept");
        assert_eq!(f[1]["lookup"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn only_source_keys_naming_the_base_are_renamed() {
        let mut v: toml::Value = toml::from_str(
            "source = \"base\"\nother = \"base\"\n[a.from]\nsource = \"base\"\n[b]\nsource = \"else\"\n",
        )
        .unwrap();
        rename_source(&mut v, "base", "hack");
        assert_eq!(v["source"].as_str(), Some("hack"));
        assert_eq!(v["other"].as_str(), Some("base"));
        assert_eq!(v["a"]["from"]["source"].as_str(), Some("hack"));
        assert_eq!(v["b"]["source"].as_str(), Some("else"));
    }
}
