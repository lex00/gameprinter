//! Per-game coverage definitions: what a screen, a level and an object are
//! in one game, read from RAM at the end of each frame, and the thresholds a
//! corpus must reach. One TOML file per game in `coverage/`, named after its
//! source ID (docs/coverage.md has the format).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gp_evidence::Hash;
use gp_runtime::select::{Condition, Index, Selector};
use serde::{Deserialize, Serialize};

/// The definition format this code reads.
pub const DEFINITION_SCHEMA: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum DefinitionError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    source: String,
    title: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    screens: Option<ScreensFile>,
    #[serde(default)]
    objects: Option<ObjectsFile>,
    #[serde(default)]
    thresholds: Option<Thresholds>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreensFile {
    #[serde(default)]
    when: Option<String>,
    parts: Vec<PartFile>,
    screen: String,
    level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartFile {
    name: String,
    read: String,
    #[serde(default)]
    add: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectsFile {
    #[serde(default)]
    when: Option<String>,
    slots: Vec<SlotFile>,
    #[serde(default)]
    names: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotFile {
    id: IdFile,
    active: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum IdFile {
    Fixed(i64),
    Read(String),
}

/// What a corpus must reach. Every name listed must be seen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    /// Screen names, as the definition's `screen` template writes them.
    #[serde(default)]
    pub screens: Vec<String>,
    /// Level names, as the definition's `level` template writes them.
    #[serde(default)]
    pub levels: Vec<String>,
    /// Object names, from the definition's `names` table (or `0x..` for an
    /// unnamed ID).
    #[serde(default)]
    pub objects: Vec<String>,
    /// Controller 1 button combinations, as [`crate::combo_name`] writes
    /// them, each seen on at least one frame the game read the controller.
    #[serde(default)]
    pub inputs: Vec<String>,
}

/// A loaded definition.
#[derive(Debug, Clone)]
pub struct Definition {
    pub source: String,
    pub title: String,
    pub notes: Option<String>,
    /// Hash of the file's bytes: coverage stored with one definition isn't
    /// reused with another.
    pub hash: Hash,
    pub path: PathBuf,
    pub screens: Option<Screens>,
    pub objects: Option<Objects>,
    pub thresholds: Option<Thresholds>,
}

#[derive(Debug, Clone)]
pub struct Screens {
    when: Option<RamCondition>,
    parts: Vec<Part>,
    screen: Template,
    level: Template,
}

#[derive(Debug, Clone)]
struct Part {
    addr: u16,
    add: i64,
}

#[derive(Debug, Clone)]
pub struct Objects {
    when: Option<RamCondition>,
    slots: Vec<(Id, u16)>,
    names: BTreeMap<i64, String>,
}

#[derive(Debug, Clone, Copy)]
enum Id {
    Fixed(i64),
    Read(u16),
}

/// A condition over `ram[...]` selectors only, so it can be checked against
/// internal RAM without the rest of the console.
#[derive(Debug, Clone)]
struct RamCondition(Condition);

/// A name template: text with `{part}` placeholders.
#[derive(Debug, Clone)]
struct Template(Vec<Piece>);

#[derive(Debug, Clone)]
enum Piece {
    Text(String),
    Part(usize),
}

impl Definition {
    pub fn load(path: &Path) -> Result<Self, DefinitionError> {
        let bytes = std::fs::read(path).map_err(|source| DefinitionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&bytes, path)
    }

    pub fn parse(bytes: &[u8], path: &Path) -> Result<Self, DefinitionError> {
        let invalid = |message: String| DefinitionError::Invalid {
            path: path.to_path_buf(),
            message,
        };
        let text = std::str::from_utf8(bytes).map_err(|e| invalid(e.to_string()))?;
        let file: File = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
        if file.schema != DEFINITION_SCHEMA {
            return Err(invalid(format!(
                "schema {} isn't one this gp reads ({DEFINITION_SCHEMA})",
                file.schema
            )));
        }
        let screens = file
            .screens
            .map(|s| -> Result<Screens, String> {
                let names: Vec<&str> = s.parts.iter().map(|p| p.name.as_str()).collect();
                Ok(Screens {
                    when: s.when.as_deref().map(RamCondition::parse).transpose()?,
                    parts: s
                        .parts
                        .iter()
                        .map(|p| {
                            Ok(Part {
                                addr: ram_addr(&p.read)?,
                                add: p.add,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                    screen: Template::parse(&s.screen, &names)?,
                    level: Template::parse(&s.level, &names)?,
                })
            })
            .transpose()
            .map_err(invalid)?;
        let objects = file
            .objects
            .map(|o| -> Result<Objects, String> {
                let mut names = BTreeMap::new();
                for (key, name) in &o.names {
                    names.insert(parse_number(key)?, name.clone());
                }
                Ok(Objects {
                    when: o.when.as_deref().map(RamCondition::parse).transpose()?,
                    slots: o
                        .slots
                        .iter()
                        .map(|s| {
                            let id = match &s.id {
                                IdFile::Fixed(n) => Id::Fixed(*n),
                                IdFile::Read(sel) => Id::Read(ram_addr(sel)?),
                            };
                            Ok((id, ram_addr(&s.active)?))
                        })
                        .collect::<Result<_, String>>()?,
                    names,
                })
            })
            .transpose()
            .map_err(invalid)?;
        if let Some(t) = &file.thresholds {
            if !(t.levels.is_empty() && t.screens.is_empty()) && screens.is_none() {
                return Err(invalid(
                    "thresholds name screens or levels but there's no [screens]".into(),
                ));
            }
            if !t.objects.is_empty() && objects.is_none() {
                return Err(invalid(
                    "thresholds name objects but there's no [objects]".into(),
                ));
            }
            for name in &t.objects {
                if name.starts_with("0x") && name != &name.to_ascii_lowercase() {
                    return Err(invalid(format!(
                        "object {name:?}: write unnamed IDs in lowercase, as reports do ({})",
                        name.to_ascii_lowercase()
                    )));
                }
            }
            for combo in &t.inputs {
                crate::parse_combo(combo).map_err(invalid)?;
            }
        }
        Ok(Self {
            source: file.source,
            title: file.title,
            notes: file.notes,
            hash: Hash::of(bytes),
            path: path.to_path_buf(),
            screens,
            objects,
            thresholds: file.thresholds,
        })
    }

    /// Finds the definition for `source_id` in `dir`: `<dir>/<source_id>.toml`,
    /// if it exists. Its `source` must name the same ID.
    pub fn find(dir: &Path, source_id: &str) -> Result<Option<Self>, DefinitionError> {
        let path = dir.join(format!("{source_id}.toml"));
        if !path.is_file() {
            return Ok(None);
        }
        let def = Self::load(&path)?;
        if def.source != source_id {
            return Err(DefinitionError::Invalid {
                path,
                message: format!("its source is {:?}, not {source_id:?}", def.source),
            });
        }
        Ok(Some(def))
    }

    /// The `coverage/` directory at or above `start`, as the source registry
    /// finds `sources/`.
    pub fn find_dir(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .map(|d| d.join("coverage"))
            .find(|d| d.is_dir())
    }

    /// The name an object ID has in this definition.
    pub fn object_name(&self, id: i64) -> String {
        self.objects
            .as_ref()
            .and_then(|o| o.names.get(&id).cloned())
            .unwrap_or_else(|| format!("{id:#04x}"))
    }
}

impl Screens {
    /// The screen's part values at the end of a frame, or `None` when the
    /// `when` condition doesn't hold.
    pub fn key(&self, ram: &[u8]) -> Option<Vec<i64>> {
        if let Some(when) = &self.when
            && !when.holds(ram)
        {
            return None;
        }
        Some(
            self.parts
                .iter()
                .map(|p| i64::from(ram[usize::from(p.addr)]) + p.add)
                .collect(),
        )
    }

    pub fn screen_name(&self, key: &[i64]) -> String {
        self.screen.render(key)
    }

    pub fn level_name(&self, key: &[i64]) -> String {
        self.level.render(key)
    }
}

impl Objects {
    /// The IDs in active slots at the end of a frame.
    pub fn active(&self, ram: &[u8], out: &mut Vec<i64>) {
        out.clear();
        if let Some(when) = &self.when
            && !when.holds(ram)
        {
            return;
        }
        for (id, active) in &self.slots {
            if ram[usize::from(*active)] != 0 {
                out.push(match id {
                    Id::Fixed(n) => *n,
                    Id::Read(addr) => i64::from(ram[usize::from(*addr)]),
                });
            }
        }
    }
}

impl RamCondition {
    fn parse(text: &str) -> Result<Self, String> {
        let cond = Condition::parse(text).map_err(|e| format!("condition {text:?}: {e}"))?;
        for sel in cond.selectors() {
            ram_selector(sel).map_err(|e| format!("condition {text:?}: {e}"))?;
        }
        Ok(Self(cond))
    }

    fn holds(&self, ram: &[u8]) -> bool {
        // Every selector was checked when parsing, so reading can't fail.
        self.0
            .eval(&mut |sel| {
                let addr = ram_selector(sel)?;
                Ok(i64::from(ram[usize::from(addr)]))
            })
            .unwrap_or(false)
    }
}

impl Template {
    fn parse(text: &str, names: &[&str]) -> Result<Self, String> {
        let mut pieces = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            if open > 0 {
                pieces.push(Piece::Text(rest[..open].to_owned()));
            }
            let close = rest[open..]
                .find('}')
                .ok_or_else(|| format!("template {text:?} has an unclosed {{"))?
                + open;
            let name = &rest[open + 1..close];
            let i = names
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| format!("template {text:?} names {name:?}, which isn't a part"))?;
            pieces.push(Piece::Part(i));
            rest = &rest[close + 1..];
        }
        if !rest.is_empty() {
            pieces.push(Piece::Text(rest.to_owned()));
        }
        Ok(Self(pieces))
    }

    fn render(&self, key: &[i64]) -> String {
        let mut out = String::new();
        for piece in &self.0 {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Part(i) => out.push_str(&key[*i].to_string()),
            }
        }
        out
    }
}

/// The address of a `ram[N]` selector, with N inside internal RAM.
fn ram_selector(sel: &Selector) -> Result<u16, String> {
    match (sel.name().as_str(), sel.last_index()?) {
        ("ram", Some(Index::At(n))) if (0..0x800).contains(&n) => Ok(n as u16),
        _ => Err(format!(
            "{sel}: coverage definitions read only ram[N], with N from 0 to 0x7ff"
        )),
    }
}

fn ram_addr(text: &str) -> Result<u16, String> {
    ram_selector(&Selector::parse(text).map_err(|e| format!("{text:?}: {e}"))?)
}

fn parse_number(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let parsed = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix('$')) {
        i64::from_str_radix(hex, 16)
    } else {
        t.parse()
    };
    parsed.map_err(|_| format!("{text:?} isn't a number (decimal, or hex after 0x or $)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema = 1
source = "test"
title = "Test"

[screens]
when = "ram[0x05] == 0"
parts = [
  { name = "world", read = "ram[0x10]", add = 1 },
  { name = "area", read = "ram[0x11]" },
]
screen = "{world} area {area}"
level = "{world}"

[objects]
slots = [{ id = "ram[0x20]", active = "ram[0x21]" }, { id = 7, active = "ram[0x22]" }]
names = { "0x03" = "Slug", "7" = "Moth" }

[thresholds]
levels = ["1", "2"]
objects = ["Slug"]
inputs = ["right+a", "none"]
"#;

    #[test]
    fn reads_screens_and_objects_from_ram() {
        let def = Definition::parse(SAMPLE.as_bytes(), Path::new("test.toml")).unwrap();
        let screens = def.screens.as_ref().unwrap();
        let mut ram = vec![0u8; 0x800];
        ram[0x10] = 1;
        ram[0x11] = 4;
        let key = screens.key(&ram).unwrap();
        assert_eq!(screens.screen_name(&key), "2 area 4");
        assert_eq!(screens.level_name(&key), "2");
        ram[0x05] = 1;
        assert_eq!(screens.key(&ram), None);

        let objects = def.objects.as_ref().unwrap();
        let mut ids = Vec::new();
        ram[0x20] = 3;
        ram[0x21] = 1;
        objects.active(&ram, &mut ids);
        assert_eq!(ids, vec![3]);
        ram[0x22] = 1;
        objects.active(&ram, &mut ids);
        assert_eq!(ids, vec![3, 7]);
        assert_eq!(def.object_name(3), "Slug");
        assert_eq!(def.object_name(7), "Moth");
        assert_eq!(def.object_name(0x1b), "0x1b");
    }

    #[test]
    fn refuses_what_it_cant_read() {
        for (bad, why) in [
            (SAMPLE.replace("ram[0x10]", "cpu.pc"), "read only ram"),
            (SAMPLE.replace("ram[0x10]", "ram[0x800]"), "read only ram"),
            (
                SAMPLE.replace("{world} area", "{nope} area"),
                "isn't a part",
            ),
            (SAMPLE.replace("\"none\"", "\"jump\""), "jump"),
            (SAMPLE.replace("schema = 1", "schema = 9"), "schema 9"),
        ] {
            let e = Definition::parse(bad.as_bytes(), Path::new("t.toml")).unwrap_err();
            assert!(e.to_string().contains(why), "{e}");
        }
    }
}
