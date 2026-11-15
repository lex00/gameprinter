//! `gp model static --model`'s mapping.toml in the state mapping format
//! (docs/formats/state-mapping.md), beside the model.toml
//! crate::model_static_fit writes: the RAM addresses the harvest found for
//! the parts of the model it wrote, so `gp model propose` reads the
//! player's place from RAM where the sprite tracker finds no player.
//!
//! Only what the harvest names goes in, each with the fact it came from:
//! an entity's x and y (`entities.E.bytes.x`, one plain address), a
//! camera's, a counter's byte, and a game state machine's byte with the
//! values of its states. The controller's bytes, the frame counter and the
//! pointers it found go in the ignore list with their roles. A slot table's
//! bytes (`$20,x`) are left out: the mapping's `[slots]` needs a stride and
//! an active flag the code reading doesn't give. Nothing is made up: a
//! byte the harvest didn't name (a position's high byte, a fraction) isn't
//! there, and a part of the model the mapping can't reach is left out and
//! named.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::model_static_fit::{Console, LeftOut};

/// The first line of a mapping static writes: a mapping.toml without it is
/// someone else's and is kept.
pub(crate) const MARK: &str = "# Written by gp model static";

/// What `mapping` wrote: the text, the model paths it maps, the bytes it
/// lists as ignored, and the facts with an address it left out.
#[derive(Debug, Clone)]
pub(crate) struct Mapped {
    pub text: String,
    pub fields: Vec<String>,
    pub ignored: Vec<String>,
    pub left_out: Vec<LeftOut>,
}

/// A harvest address that is one plain byte: `$10` (the 6502 reader's) or
/// `E16C` (the Z80's). An indexed one (`$20,x`, `E3F3+1k`) or a word
/// (`E08Aw`) is not.
fn plain(v: &Value) -> Option<u16> {
    let s = v.as_str()?.trim();
    let s = s.strip_prefix('$').unwrap_or(s);
    (!s.is_empty() && s.len() <= 4 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u16::from_str_radix(s, 16).ok())
        .flatten()
}

/// A word the Z80 reader names, `E08Aw`: its first byte.
fn word(v: &Value) -> Option<u16> {
    let s = v.as_str()?.trim().strip_suffix('w')?;
    plain(&Value::String(s.to_owned()))
}

fn hex(a: u16) -> String {
    if a > 0xFF {
        format!("0x{a:04X}")
    } else {
        format!("0x{a:02X}")
    }
}

/// The mapping of the harvest's `facts` (the same list the model was
/// written from) and `machines` (the harvest's own), for the model whose
/// text is `model_text`, of the original `source`.
pub(crate) fn mapping(
    facts: &[(String, Value)],
    machines: &Value,
    model_text: &str,
    source: &str,
    console: Console,
) -> Result<Mapped> {
    let model = gp_model::Model::parse(model_text)
        .map_err(|e| anyhow::anyhow!("the model gp model static wrote is refused: {e}"))?;
    let fact = |p: &str| facts.iter().find(|(q, _)| q == p).map(|(_, v)| v);
    let mut fields: Vec<(String, Vec<u16>, String, String)> = Vec::new();
    let mut left = Vec::new();
    let mut leave = |path: &str, why: &str| {
        left.push(LeftOut {
            path: path.to_owned(),
            why: why.to_owned(),
        })
    };
    for (p, v) in facts {
        let parts: Vec<&str> = p.split('.').collect();
        match parts.as_slice() {
            ["entities", e, "bytes", axis @ ("x" | "y")] => {
                let slots = fact(&format!("entities.{e}.count")).is_some();
                match plain(v) {
                    _ if !model.entities.contains_key(*e) => {
                        leave(p, "the model has no such entity")
                    }
                    Some(a) if !slots => fields.push((
                        format!("entities.{e}[0].{axis}"),
                        vec![a],
                        String::new(),
                        p.clone(),
                    )),
                    _ => leave(
                        p,
                        "a slot table's byte: the mapping's [slots] needs the table's stride and active flag, which the code reading doesn't give",
                    ),
                }
            }
            ["entities", _, "bytes", _] => {
                leave(p, "the mapping has no field for this entity byte")
            }
            ["cameras", c, "bytes", axis @ ("x" | "y")] => match plain(v) {
                Some(a) if model.cameras.contains_key(*c) => fields.push((
                    format!("cameras.{c}.{axis}"),
                    vec![a],
                    String::new(),
                    p.clone(),
                )),
                Some(_) => leave(p, "the model has no such camera"),
                None => leave(p, "not one plain byte"),
            },
            ["counters", c, "bytes"] => match plain(v) {
                Some(a) if model.counters.contains_key(*c) => {
                    fields.push((format!("counters.{c}"), vec![a], String::new(), p.clone()))
                }
                Some(_) => leave(p, "the model has no such counter"),
                None => leave(p, "not one plain byte"),
            },
            _ => {}
        }
    }
    // A game state's byte, its states by value.
    for m in machines.as_array().into_iter().flatten() {
        let (Some(path), Some(byte)) = (m["path"].as_str(), m.get("byte")) else {
            continue;
        };
        let Some(rule) = path.strip_prefix("rules.") else {
            continue;
        };
        let at = format!("{path}.state");
        match plain(byte) {
            _ if !model.rules.contains_key(rule) => leave(&at, "the model has no such rule"),
            Some(a) => {
                let names: Vec<String> = m["states"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| {
                        Some(format!(
                            "[{}, \"{}\"]",
                            s["value"].as_u64()?,
                            s["name"].as_str()?
                        ))
                    })
                    .collect();
                if names.is_empty() {
                    leave(&at, "the harvest gives none of its states' values");
                    continue;
                }
                fields.push((
                    at,
                    vec![a],
                    format!("names = [{}]\n", names.join(", ")),
                    format!("the state machine on {}", byte.as_str().unwrap_or("")),
                ));
            }
            None => leave(&at, "the machine's byte isn't one plain byte"),
        }
    }
    let named: Vec<u16> = fields.iter().flat_map(|f| f.1.clone()).collect();
    // The ignore list: bytes the model needs no field for, by their role.
    let mut ignore: Vec<(u16, u16, &str, String)> = Vec::new();
    let mut ignored = |a: u16, len: u16, role: &'static str, why: String| {
        if (a..a + len).any(|b| named.contains(&b))
            || ignore.iter().any(|i| a < i.0 + i.1 && i.0 < a + len)
        {
            return;
        }
        ignore.push((a, len, role, why));
    };
    for (p, v) in facts {
        let parts: Vec<&str> = p.split('.').collect();
        match parts.as_slice() {
            ["hardware", "frame_counter"] => match plain(v) {
                Some(a) => ignored(a, 1, "counter", format!("the code's frame counter ({p})")),
                None => leave(p, "not one plain byte"),
            },
            ["input", "pad" | "pad_new"] => match plain(v) {
                Some(a) => ignored(
                    a,
                    1,
                    "input",
                    format!("the code reads the controller into it ({p})"),
                ),
                None => leave(p, "not one plain byte"),
            },
            ["input", "bytes", b] => match plain(&Value::String((*b).to_owned())) {
                Some(a) => ignored(
                    a,
                    1,
                    "input",
                    format!(
                        "the code reads an input port into it ({p}: {})",
                        v.as_str().unwrap_or("")
                    ),
                ),
                None => leave(p, "not one plain byte"),
            },
            ["pointers", _] => match word(v) {
                Some(a) => ignored(
                    a,
                    2,
                    "pointer",
                    format!("the code reads a pointer from it ({p})"),
                ),
                None => leave(p, "not one plain word"),
            },
            _ => {}
        }
    }
    ignore.sort_by_key(|i| i.0);
    let mut text = format!(
        "{MARK} from the ROM's code alone: the addresses its\n\
         # harvest names for the parts of model.toml it wrote. A byte the\n\
         # harvest didn't name (a high byte, a fraction) isn't here; check each\n\
         # against the code before trusting a comparison.\n"
    );
    if console == Console::Arcade {
        text.push_str("platform = \"arcade\"\n");
    }
    let _ = write!(
        text,
        "schema = {}\nmodel = {}\nsource = {}\nalign = \"polled\"\n",
        gp_interp::mapping::SCHEMA,
        toml_str(&model.model.name),
        toml_str(source),
    );
    if fields.is_empty() {
        // The format needs the list: the harvest named no byte of the model.
        text.push_str("field = []\n");
    }
    for (path, ram, extra, from) in &fields {
        let ram: Vec<String> = ram.iter().map(|&a| hex(a)).collect();
        let _ = write!(
            text,
            "\n[[field]]\npath = {}\nram = [{}]\n{extra}note = {}\n",
            toml_str(path),
            ram.join(", "),
            toml_str(&format!("from the code: {from}")),
        );
    }
    for (a, len, role, why) in &ignore {
        let _ = write!(text, "\n[[ignore]]\nram = {}\n", hex(*a));
        if *len > 1 {
            let _ = writeln!(text, "len = {len}");
        }
        let _ = write!(text, "role = \"{role}\"\nwhy = {}\n", toml_str(why));
    }
    if let Err(e) = gp_interp::mapping::Mapping::parse(&text) {
        bail!("the mapping gp model static wrote is refused: {e}");
    }
    Ok(Mapped {
        text,
        fields: fields.into_iter().map(|f| f.0).collect(),
        ignored: ignore.iter().map(|i| hex(i.0)).collect(),
        left_out: left,
    })
}

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Made-up facts in the harvest's shapes, over a made-up model.
    #[test]
    fn the_harvests_addresses_become_fields_and_ignores_and_slots_are_left_out() {
        let model = r#"
[model]
schema = "0.4"
name = "static"
title = "static"
description = "a test"
view = "side"
frame_rate = "60"

[entities.player]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.slots_30]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[cameras.main]
axis = "x"
follow = "player"
lead = [120, 0]
clamp_to_level = true
size = [256, 240]

[counters.c_07]
start = 0

[rules.state_05]
initial = "s0"
[rules.state_05.states.s0]
[rules.state_05.states.s2]
freeze = true
"#;
        let facts: Vec<(String, Value)> = [
            ("entities.player.bytes.x", json!("$40")),
            ("entities.player.bytes.y", json!("$41")),
            ("entities.slots_30.bytes.x", json!("$30,x")),
            ("entities.slots_30.count", json!(3)),
            ("cameras.main.bytes.x", json!("$44")),
            ("counters.c_07.bytes", json!("$07")),
            ("counters.c_09.bytes", json!("$09")),
            ("hardware.frame_counter", json!("$00")),
            ("input.pad", json!("$02")),
            ("input.pad_new", json!("$03")),
        ]
        .into_iter()
        .map(|(p, v)| (p.to_owned(), v))
        .collect();
        let machines = json!([
            {"byte": "$05", "path": "rules.state_05", "states": [{"name": "s0", "value": 0}, {"name": "s2", "value": 2}]},
            {"byte": "$26,x", "path": "entities.slots_30.machine", "states": []},
        ]);
        let m = mapping(&facts, &machines, model, "made-up", Console::Nes).unwrap();
        assert_eq!(
            m.fields,
            [
                "entities.player[0].x",
                "entities.player[0].y",
                "cameras.main.x",
                "counters.c_07",
                "rules.state_05.state"
            ]
        );
        assert_eq!(m.ignored, ["0x00", "0x02", "0x03"]);
        let left: Vec<&str> = m.left_out.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(left, ["entities.slots_30.bytes.x", "counters.c_09.bytes"]);
        let parsed = gp_interp::mapping::Mapping::parse(&m.text).unwrap();
        assert_eq!(parsed.model, "static");
        assert_eq!(parsed.source, "made-up");
        assert!(
            m.text.contains("names = [[0, \"s0\"], [2, \"s2\"]]"),
            "{}",
            m.text
        );
        assert!(m.text.starts_with(MARK));
        // An arcade harvest's words and port bytes.
        let facts: Vec<(String, Value)> = vec![
            ("hardware.frame_counter".into(), json!("E16C")),
            ("input.bytes.E090".into(), json!("port 00")),
            ("pointers.w_E08Aw".into(), json!("E08Aw")),
            ("counters.c_E3F3+1k".into(), json!("E3F3+1k")),
        ];
        let model = model.split("[entities.player]").next().unwrap();
        let m = mapping(&facts, &json!([]), model, "tapper", Console::Arcade).unwrap();
        assert!(m.fields.is_empty());
        assert_eq!(m.ignored, ["0xE08A", "0xE090", "0xE16C"]);
        assert!(m.text.contains("platform = \"arcade\""));
        assert!(m.text.contains("ram = 0xE08A\nlen = 2\nrole = \"pointer\""));
        gp_interp::mapping::Mapping::parse(&m.text).unwrap();
    }
}
