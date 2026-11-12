//! Writing facts into a model: each value at its model path, with an
//! evidence entry for the path saying it was observed (or stated, for a
//! person's answer), and the model rewritten in canonical form. A value
//! that doesn't fit the schema is refused and nothing changes. An earlier
//! entry's refs are kept after the new ones; an entry whose evidence is
//! the game's code (stated, citing a line) is never replaced by an
//! observed one, and such a fact is refused.

use gp_model::Model;
use gp_model::model::Ref;
use gp_model::path::Step;

use crate::propose::steps;

/// One fact to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub path: String,
    /// A number in the model's fixed-point text (`1.5`, `-5`), or other
    /// text for a name.
    pub value: String,
    /// `observed` for a measurement, `stated` for a person's answer.
    pub source: &'static str,
    pub confidence: u8,
    pub note: String,
    /// The evidence refs: recordings, the question answered, and the
    /// sources the chosen candidate was read at.
    pub refs: Vec<Ref>,
}

/// A ref with only its `at`.
pub fn at(text: impl Into<String>) -> Ref {
    Ref {
        at: text.into(),
        within: None,
        quote: None,
    }
}

fn value_of(text: &str) -> toml::Value {
    if let Ok(i) = text.parse::<i64>() {
        return toml::Value::Integer(i);
    }
    toml::Value::String(text.to_owned())
}

/// The model text with `facts` written in, in canonical form.
pub fn apply(model_text: &str, facts: &[Fact]) -> Result<String, String> {
    let mut tree: toml::Value = toml::from_str(model_text).map_err(|e| e.to_string())?;
    for f in facts {
        let steps = steps(&f.path)?;
        let (last, parents) = steps.split_last().ok_or("an empty path")?;
        let mut node = &mut tree;
        for s in parents {
            node = match s {
                Step::Key(k) => node
                    .get_mut(k)
                    .ok_or_else(|| format!("{}: the model has no `{k}`", f.path))?,
                Step::Item(i) => node
                    .get_mut(*i)
                    .ok_or_else(|| format!("{}: the model has no item {i}", f.path))?,
            };
        }
        match last {
            Step::Key(k) => {
                let t = node
                    .as_table_mut()
                    .ok_or_else(|| format!("{}: its parent isn't a table", f.path))?;
                t.insert(k.clone(), value_of(&f.value));
            }
            Step::Item(_) => return Err(format!("{}: can't set a list item", f.path)),
        }
        let evidence = tree
            .as_table_mut()
            .expect("a model is a table")
            .entry("evidence")
            .or_insert_with(|| toml::Value::Table(Default::default()));
        let mut e = toml::map::Map::new();
        e.insert("source".into(), toml::Value::String(f.source.into()));
        e.insert(
            "confidence".into(),
            toml::Value::Integer(f.confidence.into()),
        );
        e.insert("note".into(), toml::Value::String(f.note.clone()));
        let refs = f
            .refs
            .iter()
            .map(|r| toml::Value::try_from(r).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let table = evidence
            .as_table_mut()
            .ok_or("the evidence isn't a table")?;
        // A measurement never replaces evidence from the game's code (a
        // person's statement that cites a line): the code outranks a fit.
        // Other earlier evidence is added to: its refs stay after the new.
        let old = table.get(&f.path).and_then(|v| v.as_table()).cloned();
        let mut refs = refs;
        if let Some(old) = &old {
            let old_refs: Vec<toml::Value> = old
                .get("refs")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let cites_code = old_refs.iter().any(|r| {
                r.get("in").is_some()
                    || r.get("at").and_then(|a| a.as_str()).is_some_and(|a| {
                        a.rsplit_once(':').is_some_and(|(file, n)| {
                            file.contains('.')
                                && !n.is_empty()
                                && n.bytes().all(|b| b.is_ascii_digit())
                        })
                    })
            });
            if f.source == "observed"
                && old.get("source").and_then(|v| v.as_str()) == Some("stated")
                && cites_code
            {
                return Err(format!(
                    "{}: its evidence is the game's code, which a measurement doesn't replace",
                    f.path
                ));
            }
            for r in old_refs {
                if !refs.contains(&r) {
                    refs.push(r);
                }
            }
        }
        e.insert("refs".into(), toml::Value::Array(refs));
        table.insert(f.path.clone(), toml::Value::Table(e));
    }
    let text = toml::to_string(&tree).map_err(|e| e.to_string())?;
    let model = Model::parse(&text).map_err(|e| e.to_string())?;
    Ok(model.to_toml())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOSSHOP: &str = include_str!("../../../templates/mosshop/model.toml");

    fn fit(path: &str, value: &str) -> Fact {
        Fact {
            path: path.into(),
            value: value.into(),
            source: "observed",
            confidence: 90,
            note: "a fit".into(),
            refs: vec![at("baseline new.toml")],
        }
    }

    #[test]
    fn a_fit_never_replaces_the_codes_evidence_and_adds_to_other_evidence() {
        let path = "entities.player.run.walk_speed";
        let coded = format!(
            "{MOSSHOP}\n[evidence.\"{path}\"]\nsource = \"stated\"\nconfidence = 100\n\n[[evidence.\"{path}\".refs]]\nat = \"games/mosshop/mosshop.s:31\"\n"
        );
        let e = apply(&coded, &[fit(path, "2")]).unwrap_err();
        assert!(e.contains("the game's code"), "{e}");

        let observed = format!(
            "{MOSSHOP}\n[evidence.\"{path}\"]\nsource = \"observed\"\nconfidence = 50\n\n[[evidence.\"{path}\".refs]]\nat = \"baseline old.toml\"\n"
        );
        let text = apply(&observed, &[fit(path, "2")]).unwrap();
        let m = Model::parse(&text).unwrap();
        let refs: Vec<&str> = m.evidence[path]
            .refs
            .iter()
            .map(|r| r.at.as_str())
            .collect();
        assert_eq!(refs, vec!["baseline new.toml", "baseline old.toml"]);
    }
}
