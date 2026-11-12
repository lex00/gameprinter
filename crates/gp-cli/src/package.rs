//! `gp package`: what is asked of a package folder.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use gp_package::Manifest;
use serde_json::json;

use crate::Outcome;

pub mod scan;

/// One `[[rom.entry]]` of package.toml.
pub struct RomEntry {
    pub id: String,
    pub role: String,
    pub sha256: String,
    /// `nes` (the default) or `arcade`.
    pub platform: String,
    /// An arcade entry's MAME set name.
    pub system: Option<String>,
    /// Relative to the package folder.
    pub asset_map: String,
    pub asset_map_sha256: String,
    pub lifted_sha256: Option<String>,
}

/// A package's open version, the folder under `[open]`.
pub struct OpenPart {
    pub id: String,
    pub title: String,
    pub dir: PathBuf,
    /// The folder relative to the package.
    pub rel: String,
    pub model: String,
    /// Asset map, relative to the open folder, and its pins, from the open
    /// version's template.toml.
    pub asset_map: String,
    pub asset_map_sha256: Option<String>,
    pub lifted_sha256: Option<String>,
}

/// What the checks need of a package folder's package.toml.
pub struct Package {
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    pub tier: String,
    pub entries: Vec<RomEntry>,
    pub open: Option<OpenPart>,
}

fn str_of(v: Option<&toml::Value>) -> String {
    v.and_then(toml::Value::as_str).unwrap_or("").to_owned()
}

impl Package {
    pub fn load(dir: &Path) -> Result<Self> {
        let file = dir.join("package.toml");
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let t: toml::Table = text
            .parse()
            .with_context(|| format!("parsing {}", file.display()))?;
        let pkg = t
            .get("package")
            .with_context(|| format!("{} has no [package]", file.display()))?;
        let entries = t
            .get("rom")
            .and_then(|r| r.get("entry"))
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|e| RomEntry {
                id: str_of(e.get("id")),
                role: str_of(e.get("role")),
                sha256: str_of(e.get("sha256")),
                platform: e
                    .get("platform")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("nes")
                    .to_owned(),
                system: e
                    .get("system")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                asset_map: str_of(e.get("asset_map")),
                asset_map_sha256: str_of(e.get("asset_map_sha256")),
                lifted_sha256: e
                    .get("lifted_sha256")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
            .collect();
        let open = match t.get("open") {
            Some(o) => Some(Self::load_open(dir, o)?),
            None => None,
        };
        Ok(Self {
            dir: dir.to_owned(),
            name: str_of(pkg.get("name")),
            version: str_of(pkg.get("version")),
            tier: str_of(pkg.get("tier")),
            entries,
            open,
        })
    }

    fn load_open(dir: &Path, o: &toml::Value) -> Result<OpenPart> {
        let model_path = str_of(o.get("model"));
        let (rel, model) = model_path
            .rsplit_once('/')
            .with_context(|| format!("[open] model {model_path} isn't in a folder"))?;
        let open_dir = dir.join(rel);
        let manifest = open_dir.join("template.toml");
        let m: toml::Table = fs::read_to_string(&manifest)
            .with_context(|| format!("reading {}", manifest.display()))?
            .parse()
            .with_context(|| format!("parsing {}", manifest.display()))?;
        let recipe = m.get("recipe");
        Ok(OpenPart {
            id: str_of(m.get("template").and_then(|t| t.get("id"))),
            title: str_of(o.get("title")),
            dir: open_dir,
            rel: rel.to_owned(),
            model: model.to_owned(),
            asset_map: str_of(recipe.and_then(|r| r.get("asset_map"))),
            asset_map_sha256: recipe
                .and_then(|r| r.get("asset_map_sha256"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            lifted_sha256: recipe
                .and_then(|r| r.get("lifted_sha256"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
        })
    }

    /// The platform of the scored entry (else the first), `nes` with none.
    pub fn platform(&self) -> Result<crate::roms::Platform> {
        let e = self
            .entries
            .iter()
            .find(|e| e.role == "scored")
            .or(self.entries.first());
        crate::roms::Platform::named(e.map(|e| e.platform.as_str()))
    }

    pub fn rom_hashes(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.sha256.clone()).collect()
    }

    /// The entries `roms.toml` doesn't agree with, each said in words.
    pub fn roms_toml_disagreements(&self) -> Result<Vec<(String, String)>> {
        let path = self.dir.join("roms.toml");
        let t: toml::Table = match fs::read_to_string(&path) {
            Ok(text) => text
                .parse()
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(_) if self.entries.is_empty() => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let sources = t
            .get("source")
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut out = Vec::new();
        for e in &self.entries {
            match sources.iter().find(|s| str_of(s.get("id")) == e.id) {
                None => out.push((e.id.clone(), "roms.toml has no source with this id".into())),
                Some(s) if str_of(s.get("sha256")) != e.sha256 => out.push((
                    e.id.clone(),
                    "roms.toml gives another sha256 than package.toml".into(),
                )),
                Some(s)
                    if s.get("platform")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("nes")
                        != e.platform =>
                {
                    out.push((
                        e.id.clone(),
                        "roms.toml gives another platform than package.toml".into(),
                    ))
                }
                Some(s) if s.get("system").and_then(toml::Value::as_str) != e.system.as_deref() => {
                    out.push((
                        e.id.clone(),
                        "roms.toml gives another system than package.toml".into(),
                    ))
                }
                Some(_) => {}
            }
        }
        Ok(out)
    }
}

/// Every file under a folder, by its path relative to it.
pub fn files_under(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
                files.insert(rel, fs::read(&p)?);
            }
        }
    }
    Ok(files)
}

/// The hash of a package folder's `ext/` hook code; `None` with none.
pub(crate) fn hook_code(dir: &Path) -> Result<Option<String>> {
    if !dir.join("ext").is_dir() {
        return Ok(None);
    }
    let mut m = Manifest::new("game", "x");
    for (path, bytes) in files_under(&dir.join("ext"))? {
        m.add(&format!("ext/{path}"), &bytes)
            .map_err(|e| anyhow!(e))?;
    }
    Ok(m.hook_hash())
}

/// The hash of a package folder's hook code, as a player's build names it.
pub fn hooks(dir: &Path) -> Result<Outcome> {
    let h = hook_code(dir)?.ok_or_else(|| anyhow!("{} has no hook code", dir.display()))?;
    Ok(Outcome {
        json: json!({ "hooks": h }),
        text: h,
        exit: 0,
    })
}
