//! Slot names (docs/formats/theme.md, "Slot names"): each restyle's slot
//! and entry against the models a theme is for. A slot or entry no model
//! has is refused with the nearest real name, since a runtime passes over
//! it with no word and a typo would hide. One only some models have is a
//! note naming which: a theme for two versions of a game may fill a slot
//! only one of them has.

use std::collections::{BTreeMap, BTreeSet};

use gp_model::Model;
use serde::Serialize;

use crate::Theme;

/// A model's asset slots and, for each, the entry names known for it.
/// An empty entry set is unknown: entries of that slot aren't checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSlots {
    /// The model's `model.name`.
    pub model: String,
    pub slots: BTreeMap<String, BTreeSet<String>>,
}

impl ModelSlots {
    /// The model's slots, with the entries the model itself names (every
    /// `{ slot, entry }` in it, such as a level's `layout`).
    pub fn of(model: &Model) -> Self {
        let mut s = ModelSlots {
            model: model.model.name.clone(),
            slots: model
                .assets
                .keys()
                .map(|k| (k.clone(), BTreeSet::new()))
                .collect(),
        };
        if let Ok(v) = serde_json::to_value(model) {
            let mut refs = Vec::new();
            entry_refs(&v, &mut refs);
            for (slot, entry) in refs {
                s.add_entries(&slot, [entry]);
            }
        }
        s
    }

    /// Adds entry names to a slot the model has (an asset map's entries
    /// of the slot's source asset); a slot it lacks is left alone.
    pub fn add_entries(&mut self, slot: &str, names: impl IntoIterator<Item = String>) {
        if let Some(e) = self.slots.get_mut(slot) {
            e.extend(names);
        }
    }
}

fn entry_refs(v: &serde_json::Value, out: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::Object(o) => {
            if let (Some(serde_json::Value::String(s)), Some(serde_json::Value::String(e))) =
                (o.get("slot"), o.get("entry"))
            {
                out.push((s.clone(), e.clone()));
            }
            for x in o.values() {
                entry_refs(x, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| entry_refs(x, out)),
        _ => {}
    }
}

/// One restyle whose slot or entry not every model has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The restyle's place in theme.toml, from 1.
    pub restyle: usize,
    pub slot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// No model has it.
    pub refused: bool,
    /// The models that have it (empty when refused).
    pub models: Vec<String>,
    /// The nearest real name, when refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest: Option<String>,
    /// One line for a person.
    pub message: String,
}

/// Checks each restyle naming a slot against `models`, the models the
/// theme is checked for. Returns the restyles not every model has, in
/// theme.toml's order.
pub fn check(theme: &Theme, models: &[ModelSlots]) -> Vec<Finding> {
    let names: Vec<&str> = models.iter().map(|m| m.model.as_str()).collect();
    let all = names.join(", ");
    let mut out = Vec::new();
    for (i, r) in theme.restyle.iter().enumerate() {
        let Some(slot) = &r.slot else { continue };
        let at = i + 1;
        let with: Vec<&ModelSlots> = models
            .iter()
            .filter(|m| m.slots.contains_key(slot))
            .collect();
        if with.is_empty() {
            let nearest = nearest(slot, models.iter().flat_map(|m| m.slots.keys()));
            out.push(Finding {
                restyle: at,
                slot: slot.clone(),
                entry: r.entry.clone(),
                refused: true,
                models: Vec::new(),
                message: format!(
                    "restyle {at} ({}): no slot `{slot}` in {all}{}",
                    r.file,
                    hint(&nearest)
                ),
                nearest,
            });
            continue;
        }
        if let Some(entry) = &r.entry {
            // Models whose entries of the slot are known.
            let known: Vec<&&ModelSlots> =
                with.iter().filter(|m| !m.slots[slot].is_empty()).collect();
            if !known.is_empty() {
                let has: Vec<String> = known
                    .iter()
                    .filter(|m| m.slots[slot].contains(entry))
                    .map(|m| m.model.clone())
                    .collect();
                if has.is_empty() {
                    let nearest = nearest(entry, known.iter().flat_map(|m| m.slots[slot].iter()));
                    out.push(Finding {
                        restyle: at,
                        slot: slot.clone(),
                        entry: Some(entry.clone()),
                        refused: true,
                        models: Vec::new(),
                        message: format!(
                            "restyle {at} ({}): slot `{slot}` has no entry `{entry}` in {all}{}",
                            r.file,
                            hint(&nearest)
                        ),
                        nearest,
                    });
                    continue;
                }
                if has.len() < models.len() {
                    out.push(note(at, &r.file, slot, Some(entry), has, &names));
                }
                continue;
            }
        }
        if with.len() < models.len() {
            let has = with.iter().map(|m| m.model.clone()).collect();
            out.push(note(at, &r.file, slot, r.entry.as_ref(), has, &names));
        }
    }
    out
}

fn note(
    at: usize,
    file: &str,
    slot: &str,
    entry: Option<&String>,
    has: Vec<String>,
    names: &[&str],
) -> Finding {
    let lacks: Vec<&str> = names
        .iter()
        .filter(|n| !has.iter().any(|h| h == *n))
        .copied()
        .collect();
    let what = match entry {
        Some(e) => format!("slot `{slot}` entry `{e}`"),
        None => format!("slot `{slot}`"),
    };
    Finding {
        restyle: at,
        slot: slot.to_owned(),
        entry: entry.cloned(),
        refused: false,
        message: format!(
            "restyle {at} ({file}): {what} is only in {}; {} passes over it",
            has.join(", "),
            lacks.join(", ")
        ),
        models: has,
        nearest: None,
    }
}

fn hint(nearest: &Option<String>) -> String {
    nearest
        .as_ref()
        .map(|n| format!("; the nearest is `{n}`"))
        .unwrap_or_default()
}

/// The candidate with the fewest edits from `name`, the first in name
/// order on a tie. `gp theme look check` says it beside a picture name
/// the game hasn't got.
pub fn nearest<'a>(name: &str, candidates: impl Iterator<Item = &'a String>) -> Option<String> {
    let set: BTreeSet<&String> = candidates.collect();
    set.into_iter()
        .min_by_key(|c| edits(name, c))
        .map(|c| c.to_string())
}

/// The Levenshtein distance between two names, by characters.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up pair: a base version and a later one with every slot of
    /// the base plus `bonus_tiles` and a third screen.
    fn pair() -> Vec<ModelSlots> {
        let base = |name: &str| ModelSlots {
            model: name.into(),
            slots: [
                ("tiles".to_string(), BTreeSet::new()),
                ("sprite_tiles".to_string(), BTreeSet::new()),
                (
                    "screens".to_string(),
                    ["screen_a", "screen_b"].map(String::from).into(),
                ),
            ]
            .into(),
        };
        let mut later = base("made_up_plus");
        later.slots.insert("bonus_tiles".into(), BTreeSet::new());
        later.add_entries("screens", ["screen_c".to_string()]);
        vec![base("made_up"), later]
    }

    fn theme(restyles: &str, for_models: &str) -> Theme {
        let text = format!(
            "format = \"0.1.3\"\nid = \"t\"\ntitle = \"T\"\nlicense = \"CC0-1.0\"\nfor = [{for_models}]\n{restyles}"
        );
        toml::from_str(&text).unwrap()
    }

    fn restyle(slot: &str, entry: Option<&str>) -> String {
        format!(
            "[[restyle]]\nslot = \"{slot}\"\n{}file = \"x.pix\"\norigin = \"drawn\"\nlicense = \"CC0-1.0\"\nsha256 = \"00\"\n",
            entry
                .map(|e| format!("entry = \"{e}\"\n"))
                .unwrap_or_default()
        )
    }

    #[test]
    fn a_misspelt_slot_is_refused_with_the_nearest_name() {
        let t = theme(
            &restyle("sprite_tile", None),
            "\"made_up\", \"made_up_plus\"",
        );
        let f = check(&t, &pair());
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].refused);
        assert_eq!(f[0].nearest.as_deref(), Some("sprite_tiles"));
        assert_eq!(
            f[0].message,
            "restyle 1 (x.pix): no slot `sprite_tile` in made_up, made_up_plus; the nearest is `sprite_tiles`"
        );
    }

    #[test]
    fn a_misspelt_entry_is_refused_with_the_nearest_entry() {
        let t = theme(&restyle("screens", Some("screen_bb")), "");
        let f = check(&t, &pair());
        assert!(f[0].refused, "{f:?}");
        assert_eq!(f[0].nearest.as_deref(), Some("screen_b"));
    }

    #[test]
    fn a_slot_only_the_later_version_has_is_a_note() {
        let r = restyle("tiles", None)
            + &restyle("bonus_tiles", None)
            + &restyle("screens", Some("screen_c"));
        let t = theme(&r, "\"made_up\", \"made_up_plus\"");
        let f = check(&t, &pair());
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f.iter().all(|x| !x.refused));
        assert_eq!(f[0].models, ["made_up_plus"]);
        assert_eq!(
            f[0].message,
            "restyle 2 (x.pix): slot `bonus_tiles` is only in made_up_plus; made_up passes over it"
        );
        assert_eq!(f[1].entry.as_deref(), Some("screen_c"));
    }

    #[test]
    fn checked_against_one_model_a_slot_it_lacks_is_refused() {
        // A theme with no `for` is checked against the model it is given.
        let t = theme(&restyle("bonus_tiles", None), "");
        let f = check(&t, &pair()[..1]);
        assert!(f[0].refused, "{f:?}");
        let t = theme(&restyle("tiles", None), "");
        assert!(check(&t, &pair()[..1]).is_empty());
    }

    #[test]
    fn unknown_entries_are_not_checked() {
        let t = theme(&restyle("tiles", Some("anything")), "");
        assert!(check(&t, &pair()).is_empty());
    }

    #[test]
    fn edits_counts_changes() {
        assert_eq!(edits("sprite_tile", "sprite_tiles"), 1);
        assert_eq!(edits("tiles", "tiles"), 0);
        assert_eq!(edits("", "abc"), 3);
        assert_eq!(edits("kitten", "sitting"), 3);
    }
}
