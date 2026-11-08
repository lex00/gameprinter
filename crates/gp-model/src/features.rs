//! Named features: the optional parts of a format, so adding one needs no
//! version bump (docs/model.md, "Versions").
//!
//! A format's version (core `0.4`, platformer `0.4`, the state mapping's
//! `schema`) bumps only for a breaking change, once per release. Every
//! optional option or component added since is a feature with a name,
//! such as `launcher`, `leap.place_on_appear` or `contact.touch_edges`.
//! A model may list the features it uses in `model.features`; without the
//! list they're derived from the fields it sets ([`used`]). A reader
//! refuses a model that lists a feature it doesn't know, by name
//! ([`unknown`]), before it reads any of the model's fields.
//!
//! A feature is found in a model by its paths: dotted keys over the
//! model's TOML tree, where `*` is any one key or list item, `**` is any
//! number of them (so `entities.**.leap.place_on_appear` also finds a
//! machine state's override), a key on a list of tables looks in each
//! item, and a last step `key=value` matches a string or a list holding
//! it. A path is met when the value it reaches is set: not `false`, and
//! not an empty list or table.

use std::collections::BTreeSet;

/// Which format a feature belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Piece {
    Core,
    Platformer,
    /// The state mapping (`mapping.toml`, docs/formats/state-mapping.md).
    Mapping,
    /// The fidelity contract (`fidelity.toml`, docs/formats/contract.md).
    Contract,
    /// The asset map (`assets/<source-id>.toml`, docs/assets.md).
    AssetMap,
}

impl Piece {
    pub fn name(self) -> &'static str {
        match self {
            Piece::Core => "core",
            Piece::Platformer => "platformer",
            Piece::Mapping => "mapping",
            Piece::Contract => "contract",
            Piece::AssetMap => "asset-map",
        }
    }
}

/// One optional part of a format.
#[derive(Debug, Clone, Copy)]
pub struct Feature {
    /// Its name: lowercase words joined by `.` and `_`, unique across
    /// every piece.
    pub name: &'static str,
    pub piece: Piece,
    /// The version it first appeared in, kept as a note: "0.4.28", or
    /// "" for a feature added after versions stopped counting options.
    pub since: &'static str,
    /// What it is, in a sentence.
    pub about: &'static str,
    /// Where it shows in a file (see the module docs). A feature with no
    /// paths is found only by being listed.
    pub paths: &'static [&'static str],
}

/// True when `name` is a well-formed feature name.
pub fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|w| {
            !w.is_empty()
                && w.starts_with(|c: char| c.is_ascii_lowercase())
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// The features of `registry` that `tree` uses, by name.
pub fn used(tree: &toml::Value, registry: &[Feature]) -> BTreeSet<&'static str> {
    registry
        .iter()
        .filter(|f| {
            f.paths.iter().any(|p| {
                let steps: Vec<&str> = p.split('.').collect();
                meets(tree, &steps)
            })
        })
        .map(|f| f.name)
        .collect()
}

/// The names in `listed` that `registry` doesn't know, in order.
pub fn unknown<'a>(listed: &'a [String], registry: &[Feature]) -> Vec<&'a str> {
    listed
        .iter()
        .map(String::as_str)
        .filter(|n| !registry.iter().any(|f| f.name == *n))
        .collect()
}

/// The message a reader gives for features it doesn't know.
pub fn refusal(what: &str, names: &[&str]) -> String {
    let list = names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() == 1 {
        format!(
            "{what} uses the feature {list}, which this build doesn't know; update gp to read it"
        )
    } else {
        format!(
            "{what} uses the features {list}, which this build doesn't know; update gp to read them"
        )
    }
}

/// The `features` list at `table.features` of a TOML text, if it has one
/// that is a list of strings. Read before the typed parse, so a model
/// with an unknown feature is refused by that feature's name and not by
/// the first field the reader doesn't know.
pub fn listed_in(text: &str, table: Option<&str>) -> Vec<String> {
    let Ok(v) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let at = match table {
        Some(t) => v.get(t).and_then(toml::Value::as_table),
        None => Some(&v),
    };
    at.and_then(|t| t.get("features"))
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn set(v: &toml::Value) -> bool {
    match v {
        toml::Value::Boolean(b) => *b,
        toml::Value::Array(a) => !a.is_empty(),
        toml::Value::Table(t) => !t.is_empty(),
        _ => true,
    }
}

fn children(v: &toml::Value) -> Vec<&toml::Value> {
    match v {
        toml::Value::Table(t) => t.values().collect(),
        toml::Value::Array(a) => a.iter().collect(),
        _ => Vec::new(),
    }
}

fn meets(node: &toml::Value, steps: &[&str]) -> bool {
    let Some((step, rest)) = steps.split_first() else {
        return set(node);
    };
    if let toml::Value::Array(items) = node
        && *step != "*"
        && *step != "**"
    {
        // A key on a list of tables looks in each item.
        return items.iter().any(|i| i.is_table() && meets(i, steps));
    }
    match *step {
        "**" => meets(node, rest) || children(node).into_iter().any(|c| meets(c, steps)),
        "*" => children(node).into_iter().any(|c| meets(c, rest)),
        s => {
            if let Some((key, want)) = s.split_once('=') {
                let Some(v) = node.as_table().and_then(|t| t.get(key)) else {
                    return false;
                };
                // A value ending in `*` matches by its start (`do=emit:*`).
                let hit = |x: &toml::Value| match want.strip_suffix('*') {
                    Some(start) => x.as_str().is_some_and(|s| s.starts_with(start)),
                    None => x.as_str() == Some(want),
                };
                return match v {
                    toml::Value::Array(a) => a.iter().any(hit),
                    toml::Value::Table(t) => t.values().any(hit),
                    x => hit(x),
                };
            }
            node.as_table()
                .and_then(|t| t.get(s))
                .is_some_and(|c| meets(c, rest))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &[Feature] = &[
        Feature {
            name: "leap.place_on_appear",
            piece: Piece::Platformer,
            since: "0.4.26",
            about: "",
            paths: &["entities.**.leap.place_on_appear"],
        },
        Feature {
            name: "tile.coin",
            piece: Piece::Core,
            since: "0.4.16",
            about: "",
            paths: &["levels.*.tile_kinds=coin"],
        },
        Feature {
            name: "spawn.at_edge",
            piece: Piece::Core,
            since: "0.4.11",
            about: "",
            paths: &["levels.*.spawn.at_edge"],
        },
    ];

    fn tree(text: &str) -> toml::Value {
        toml::Value::Table(text.parse().unwrap())
    }

    #[test]
    fn paths_find_options_set_and_skip_ones_off() {
        let t = tree(
            "[entities.p.leap]\nplace_on_appear = true\n\
             [levels.a]\ntile_kinds = { 1 = \"solid\", 3 = \"coin\" }\n\
             [[levels.a.spawn]]\nat = 1\n[[levels.a.spawn]]\nat_edge = \"left\"\n",
        );
        let got: Vec<_> = used(&t, REG).into_iter().collect();
        assert_eq!(got, ["leap.place_on_appear", "spawn.at_edge", "tile.coin"]);
        let off = tree("[entities.p.leap]\nplace_on_appear = false\n");
        assert!(used(&off, REG).is_empty());
    }

    #[test]
    fn a_machine_state_override_is_found() {
        let t = tree("[entities.p.machine.states.s.leap]\nplace_on_appear = true\n");
        assert!(used(&t, REG).contains("leap.place_on_appear"));
    }

    #[test]
    fn an_unknown_feature_is_named_and_a_known_one_accepted() {
        let listed = vec!["tile.coin".to_owned(), "rope.swing".to_owned()];
        assert_eq!(unknown(&listed, REG), ["rope.swing"]);
        assert!(unknown(&listed[..1], REG).is_empty());
        let m = refusal("the model", &unknown(&listed, REG));
        assert!(m.contains("`rope.swing`"), "{m}");
    }

    #[test]
    fn names_are_checked() {
        assert!(is_name("contact.touch_edges"));
        assert!(is_name("launcher"));
        assert!(!is_name("Contact"));
        assert!(!is_name("a..b"));
        assert!(!is_name("1up"));
    }

    #[test]
    fn a_list_is_read_before_the_typed_parse() {
        let text = "[model]\nfeatures = [\"rope.swing\"]\nnot_a_field = 1\n";
        assert_eq!(listed_in(text, Some("model")), ["rope.swing"]);
        assert_eq!(listed_in("features = [\"x\"]\n", None), ["x"]);
    }
}
