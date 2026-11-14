//! What a game's observe.toml [level] and mapping.toml [cells] say, as
//! `gp farm` and `gp localize` read them: the level key (the RAM bytes
//! that name the level, and their names) and the level grid. Each command
//! reads the file itself, with its own error context.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// The level key and its names, from observe.toml [level].
pub(crate) struct LevelKey {
    pub(crate) ram: Vec<u16>,
    pub(crate) when: Option<gp_runtime::select::Condition>,
    pub(crate) names: BTreeMap<Vec<u8>, String>,
}

impl LevelKey {
    /// The key from observe.toml's parsed text.
    pub(crate) fn parse(t: &toml::Value) -> Result<LevelKey> {
        let l = t
            .get("level")
            .context("the game's observe.toml has no [level]")?;
        let ram = l
            .get("ram")
            .and_then(|v| v.as_array())
            .context("[level] ram")?
            .iter()
            .filter_map(|v| v.as_integer().map(|n| n as u16))
            .collect();
        let when = l
            .get("when")
            .and_then(|w| w.as_str())
            .and_then(|w| gp_runtime::select::Condition::parse(w).ok());
        let mut names = BTreeMap::new();
        for n in l
            .get("names")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(a) = n.as_array() else { continue };
            let (Some(k), Some(v)) = (
                a.first().and_then(|k| k.as_str()),
                a.get(1).and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let key: Vec<u8> = k
                .split_whitespace()
                .filter_map(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
                .collect();
            names.insert(key, v.to_owned());
        }
        Ok(LevelKey { ram, when, names })
    }

    /// The level the RAM is in, or None between levels.
    pub(crate) fn of(&self, w: &[u8]) -> Option<String> {
        if let Some(c) = &self.when {
            let ok = c
                .eval(&mut |sel| match (sel.name().as_str(), sel.last_index()?) {
                    ("ram", Some(gp_runtime::select::Index::At(n))) if (0..0x800).contains(&n) => {
                        Ok(i64::from(w[n as usize]))
                    }
                    _ => Err("only ram[N]".into()),
                })
                .unwrap_or(false);
            if !ok {
                return None;
            }
        }
        let key: Vec<u8> = self.ram.iter().map(|a| w[usize::from(*a)]).collect();
        Some(self.names.get(&key).cloned().unwrap_or_else(|| {
            key.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("")
        }))
    }
}

/// The level grid mapping.toml [cells] reads: pages of columns of rows of
/// tile bytes.
pub(crate) struct Grid {
    pub(crate) pages: Vec<u16>,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) top: usize,
}

impl Grid {
    /// The grid in the game folder's mapping.toml, if it has one.
    pub(crate) fn load(game_dir: &Path) -> Option<Grid> {
        let t: toml::Value =
            toml::from_str(&fs::read_to_string(game_dir.join("mapping.toml")).ok()?).ok()?;
        let c = t.get("cells")?;
        Some(Grid {
            pages: c
                .get("pages")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_integer().map(|n| n as u16))
                .collect(),
            cols: c.get("page_columns")?.as_integer()? as usize,
            rows: c.get("rows")?.as_integer()? as usize,
            top: c.get("top").and_then(|v| v.as_integer()).unwrap_or(0) as usize,
        })
    }
}
