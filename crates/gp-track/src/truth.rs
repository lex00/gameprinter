//! Scoring a tracker's slots against a hand-written state mapping
//! (docs/formats/tracks.md, "Scoring").
//!
//! Both sides are read as sets of (role, address) pairs, one for every slot:
//! the slots' x, page, y, active and ID bytes, the frame counter, and the
//! player's x, page and y. Precision is the share of the proposed pairs the
//! mapping has; recall is the share of the mapping's pairs proposed.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::{Report, slots::Group};

/// A (role, address) pair.
pub type Item = (&'static str, u16);

#[derive(Debug, Serialize)]
pub struct Score {
    pub proposed: usize,
    pub truth: usize,
    pub correct: usize,
    pub precision: f64,
    pub recall: f64,
    /// Roles the mapping has that the tracker missed, and roles it proposed
    /// that the mapping doesn't have, as `role@addr`.
    pub missed: Vec<String>,
    pub wrong: Vec<String>,
    /// Groups the tracker found that none of the mapping's slots overlap.
    /// They're real objects the mapping leaves out, and aren't scored.
    pub other_groups: usize,
}

/// The pairs a mapping lists: `[slots]` and its fields when it has them,
/// otherwise `entities.<name>[i].x` and `.alive` fields grouped by entity.
pub fn items(mapping: &str) -> Result<BTreeSet<Item>, String> {
    let v: toml::Value = toml::from_str(mapping).map_err(|e| format!("mapping: {e}"))?;
    let mut out = BTreeSet::new();
    let addrs = |f: &toml::Value| -> Vec<u16> {
        f.get("ram")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_integer())
                    .map(|n| n as u16)
                    .collect()
            })
            .unwrap_or_default()
    };
    let fields = v
        .get("field")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let mut enemy_names: BTreeSet<String> = BTreeSet::new();
    for f in &fields {
        let path = f.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let a = addrs(f);
        if a.is_empty() {
            continue;
        }
        // entities.mario[0].x
        let Some(rest) = path.strip_prefix("entities.") else {
            continue;
        };
        let Some((name, field)) = rest.split_once('.') else {
            continue;
        };
        let name = name.split('[').next().unwrap_or(name);
        let is_player = name == "player" || name == "mario";
        match (field, is_player) {
            ("x", true) => {
                out.insert(("player_x", a[0]));
                if let Some(p) = a.get(1) {
                    out.insert(("player_page", *p));
                }
            }
            ("y", true) => {
                out.insert(("player_y", a[0]));
            }
            ("x", false) => {
                enemy_names.insert(name.to_string());
                out.insert(("x", a[0]));
                if let Some(p) = a.get(1) {
                    out.insert(("x_page", *p));
                }
            }
            ("y", false) => {
                out.insert(("y", a[0]));
            }
            ("alive" | "active", false) => {
                out.insert(("active", a[0]));
            }
            _ => {}
        }
    }
    if let Some(s) = v.get("slots") {
        let count = s.get("count").and_then(|c| c.as_integer()).unwrap_or(0) as u16;
        for (key, role) in [("active", "active"), ("id", "id")] {
            if let Some(a) = s.get(key).and_then(|a| a.as_integer()) {
                for i in 0..count {
                    out.insert((role, a as u16 + i));
                }
            }
        }
        if let Some(c) = s.get("clock").and_then(|a| a.as_integer()) {
            out.insert(("clock", c as u16));
        }
        for f in s
            .get("field")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
        {
            let a = addrs(f);
            if a.is_empty() {
                continue;
            }
            match f.get("path").and_then(|p| p.as_str()) {
                Some("x") => {
                    for i in 0..count {
                        out.insert(("x", a[0] + i));
                        if let Some(p) = a.get(1) {
                            out.insert(("x_page", *p + i));
                        }
                    }
                }
                Some("y") => {
                    for i in 0..count {
                        out.insert(("y", a[0] + i));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

fn group_items(g: &Group, out: &mut BTreeSet<Item>) {
    let at = |base: u16, i: usize| base + (i * g.stride) as u16;
    for i in 0..g.count {
        out.insert(("x", at(g.x.ram, i)));
        if let Some(p) = &g.x_page {
            out.insert(("x_page", at(p.ram, i)));
        }
        if let Some(y) = &g.y {
            out.insert(("y", at(y.ram, i)));
        }
        if let Some(a) = &g.active {
            out.insert(("active", at(a.ram, i)));
        }
        if let Some(id) = &g.id {
            out.insert(("id", at(id.ram, i)));
        }
    }
}

/// The pairs a report proposes: its player, its frame counter, and the
/// group that overlaps the truth's slots most (the others are counted), in
/// the roles the mapping lists.
pub fn proposed(report: &Report, truth: &BTreeSet<Item>) -> (BTreeSet<Item>, usize) {
    let mut out = BTreeSet::new();
    if let Some(p) = &report.player {
        out.insert(("player_x", p.x.ram));
        if let Some(pg) = &p.page {
            out.insert(("player_page", pg.ram));
        }
        if let Some(y) = &p.y {
            out.insert(("player_y", y.ram));
        }
    }
    if let Some(c) = &report.clock {
        out.insert(("clock", c.ram));
    }
    let overlap = |g: &Group| {
        let mut s = BTreeSet::new();
        group_items(g, &mut s);
        s.intersection(truth).count()
    };
    let best = report
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| (overlap(g), std::cmp::Reverse(i)))
        .max();
    let mut other = report.groups.len();
    if let Some((n, std::cmp::Reverse(i))) = best
        && n > 0
    {
        group_items(&report.groups[i], &mut out);
        other -= 1;
    }
    // Roles the mapping doesn't list at all (Moss Hop's has no frame
    // counter) say nothing either way.
    let roles: BTreeSet<&str> = truth.iter().map(|(r, _)| *r).collect();
    out.retain(|(r, _)| roles.contains(r));
    (out, other)
}

pub fn score(report: &Report, mapping: &str) -> Result<Score, String> {
    let truth = items(mapping)?;
    let (got, other_groups) = proposed(report, &truth);
    let correct = got.intersection(&truth).count();
    let show = |s: &BTreeSet<Item>, not: &BTreeSet<Item>| -> Vec<String> {
        s.difference(not)
            .map(|(r, a)| format!("{r}@0x{a:04X}"))
            .collect()
    };
    Ok(Score {
        proposed: got.len(),
        truth: truth.len(),
        correct,
        precision: if got.is_empty() {
            0.0
        } else {
            correct as f64 / got.len() as f64
        },
        recall: if truth.is_empty() {
            0.0
        } else {
            correct as f64 / truth.len() as f64
        },
        missed: show(&truth, &got),
        wrong: show(&got, &truth),
        other_groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapping_s_pairs_are_read_from_its_fields_and_slots() {
        let m = r#"
[[field]]
path = "entities.hero[0].x"
ram = [0x10, 0x11]
[[field]]
path = "entities.thing[0].x"
ram = [0x20, 0x22]
[[field]]
path = "entities.thing[1].x"
ram = [0x21, 0x23]
[[field]]
path = "entities.thing[0].alive"
ram = [0x26]
"#;
        let a = items(m).unwrap();
        // "hero" isn't a player name, so it's a slot entity here.
        assert!(a.contains(&("x", 0x20)) && a.contains(&("x_page", 0x23)));
        assert!(a.contains(&("active", 0x26)));
        let s = r#"
[slots]
count = 3
active = 0x0F
id = 0x16
clock = 0x09
[[slots.field]]
path = "x"
ram = [0x87, 0x6E]
"#;
        let b = items(s).unwrap();
        assert!(b.contains(&("x", 0x89)) && b.contains(&("x_page", 0x70)));
        assert!(b.contains(&("id", 0x18)) && b.contains(&("clock", 9)));
    }
}
