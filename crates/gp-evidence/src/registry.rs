//! Known sources: the games and revisions Game Printer targets, identified by
//! hash. The registry lives in the repo (`sources/*.toml`) and holds hashes
//! and facts only, never game data. See docs/sources.md.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SourceStatus {
    /// Hash recorded from a file we were given, not yet checked against a reference.
    Unverified,
    /// Hash matches an independent reference: a byte-exact build, or checksums
    /// published by at least two independent sources (weaker; see `evidence`).
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KnownSource {
    /// Stable ID used in recordings and templates, e.g. `smb1-ntsc-prg0`.
    pub id: String,
    pub title: String,
    /// `nes` or `arcade`.
    pub platform: String,
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// An arcade source's MAME set name, such as `rbtapper`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// SHA-256 of PRG and CHR without header (see `source::identify_nes`);
    /// for an arcade romset, its set's identity (gp_arcade::romset).
    pub sha256: String,
    /// The NES mapper; an arcade source has none and leaves it out.
    #[serde(default)]
    pub mapper: u16,
    /// Board facts (docs/sources.md, "Board facts"): with them, the source
    /// is run and cleaned with a header built from the registry, not the
    /// file's. All of `mirroring`, `battery` and the four sizes are given,
    /// or none of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submapper: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirroring: Option<BoardMirroring>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prg_rom_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chr_rom_bytes: Option<u64>,
    /// PRG RAM on the board, battery-backed or not (`battery` says which).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prg_ram_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chr_ram_bytes: Option<u64>,
    /// The board's name, e.g. `NES-SGROM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    /// Where the board facts come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_evidence: Option<String>,
    /// MD5 of PRG and CHR without header, lowercase hex: what FCEUX writes
    /// as an `.fm2` movie's `romChecksum` (docs/movies.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    /// SHA-1 of PRG and CHR without header, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// SHA-1 of the file with a plain iNES 1.0 header and no trainer,
    /// lowercase hex: header bytes 0 to 6 as in the file with byte 6's
    /// trainer bit cleared, byte 7's high nibble (the mapper's) kept, the
    /// rest zero. What BizHawk writes as a `.bk2` movie's `SHA1` for a ROM
    /// not in its game database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ines_sha1: Option<String>,
    pub status: SourceStatus,
    /// How the hash was verified, or why it isn't yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// `dump` (the default): a release, read from a cartridge. `patched`: a
    /// base source with a ROM hack's patch applied (docs/sources.md,
    /// "Patched sources"), described by `patch`.
    #[serde(default, skip_serializing_if = "SourceKind::is_dump")]
    pub kind: SourceKind,
    /// For a patched source: the base and the patch. `sha256` and the other
    /// hashes are the patched image's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<PatchSource>,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    #[default]
    Dump,
    Patched,
}

impl SourceKind {
    fn is_dump(&self) -> bool {
        *self == SourceKind::Dump
    }
}

/// How a patched source is made: which base, which patch file, and who
/// made it. The repo holds these facts only, never the patch or the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchSource {
    /// The base source's ID, a `dump` entry in the registry.
    pub base: String,
    /// `ips` or `bps`.
    pub format: PatchFormat,
    /// SHA-256 and size of the patch file, measured on the author's file.
    pub sha256: String,
    pub bytes: u64,
    /// Where the patch counts offsets from: `ines` when they count the
    /// 16-byte header (most NES IPS patches), `headerless` otherwise.
    pub offsets: PatchOffsets,
    /// Where the author or a hub hosts the patch, and the hack's page.
    /// Never a copy of ours.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    pub author: String,
    /// How to reach the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// `none`, `asked <date>` or `granted <date> <where>`: whether the
    /// author has agreed to anything printed from the hack being shown.
    pub permission: String,
    /// Other people whose work is in the hack, such as the original
    /// levels' author.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credits: Vec<String>,
}

/// A patch file's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PatchFormat {
    Ips,
    Bps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PatchOffsets {
    Ines,
    Headerless,
}

/// How a board wires nametable mirroring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BoardMirroring {
    Horizontal,
    Vertical,
    FourScreen,
    /// The mapper chip sets it (MMC1, MMC3 without four-screen RAM). The
    /// header's mirroring bit is no board fact, so it isn't compared, and
    /// the registry's header writes 0. Emulators may still read it as the
    /// power-on mirroring (tetanes-core does for MMC3, not MMC1), which is
    /// why it's normalised.
    Mapper,
}

/// The facts about a source's cartridge that an iNES or NES 2.0 header
/// would otherwise give: the registry's answer, which wins over the file's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BoardFacts {
    pub mapper: u16,
    pub submapper: u8,
    pub mirroring: BoardMirroring,
    pub battery: bool,
    pub prg_rom_bytes: u64,
    pub chr_rom_bytes: u64,
    pub prg_ram_bytes: u64,
    pub chr_ram_bytes: u64,
}

impl KnownSource {
    /// The board facts, if the source has them.
    pub fn board_facts(&self) -> Option<BoardFacts> {
        Some(BoardFacts {
            mapper: self.mapper,
            submapper: self.submapper.unwrap_or(0),
            mirroring: self.mirroring?,
            battery: self.battery?,
            prg_rom_bytes: self.prg_rom_bytes?,
            chr_rom_bytes: self.chr_rom_bytes?,
            prg_ram_bytes: self.prg_ram_bytes?,
            chr_ram_bytes: self.chr_ram_bytes?,
        })
    }

    /// Whether some board facts are given and others left out.
    fn partial_board_facts(&self) -> bool {
        let given = [
            self.mirroring.is_some(),
            self.battery.is_some(),
            self.prg_rom_bytes.is_some(),
            self.chr_rom_bytes.is_some(),
            self.prg_ram_bytes.is_some(),
            self.chr_ram_bytes.is_some(),
        ];
        given.iter().any(|&g| g) && !given.iter().all(|&g| g)
    }
}

#[derive(Debug, Default, Clone, Deserialize, schemars::JsonSchema)]
struct RegistryFile {
    #[serde(default)]
    source: Vec<KnownSource>,
}

/// The JSON Schema of a games checkout's `roms.toml` (and of the older
/// `sources/*.toml` files), for `gp self schemas`: its `[[source]]` rows.
pub fn schema() -> String {
    let mut s = schemars::schema_for!(RegistryFile);
    s.insert("$id".into(), "game-printer:roms@1".into());
    s.insert("title".into(), "ROM sources, `roms.toml`".into());
    let mut text = serde_json::to_string_pretty(&s).expect("schemas serialize");
    text.push('\n');
    text
}

#[derive(Debug, Default, Clone)]
pub struct Registry {
    pub sources: Vec<KnownSource>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "{path}: source {id} gives some board facts but not all (mirroring, battery, prg_rom_bytes, chr_rom_bytes, prg_ram_bytes, chr_ram_bytes)"
    )]
    PartialBoard { path: PathBuf, id: String },
    #[error("{path}: source {id}: {msg}")]
    Patched {
        path: PathBuf,
        id: String,
        msg: String,
    },
    #[error("{path}: duplicate {what} {value:?}")]
    Duplicate {
        path: PathBuf,
        what: &'static str,
        value: String,
    },
}

impl Registry {
    /// Loads every `*.toml` file in `dir`.
    pub fn load_dir(dir: &Path) -> Result<Self, RegistryError> {
        let io = |source| RegistryError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();

        let mut reg = Registry::default();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|source| RegistryError::Io {
                path: path.clone(),
                source,
            })?;
            reg.add_toml(&text, &path)?;
        }
        reg.check_patched(dir)?;
        Ok(reg)
    }

    /// Every game's `roms.toml` in a games checkout (`dir/GAME/roms.toml`),
    /// as one registry: the games repo holds the sources now, one file a game.
    pub fn load_games(dir: &Path) -> Result<Self, RegistryError> {
        let io = |source| RegistryError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .filter_map(|e| e.ok().map(|e| e.path().join("roms.toml")))
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        let mut reg = Registry::default();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|source| RegistryError::Io {
                path: path.clone(),
                source,
            })?;
            reg.add_toml(&text, &path)?;
        }
        reg.check_patched(dir)?;
        Ok(reg)
    }

    /// A registry from one file's text; `path` names it in errors.
    pub fn from_toml(text: &str, path: &Path) -> Result<Self, RegistryError> {
        let mut reg = Registry::default();
        reg.add_toml(text, path)?;
        reg.check_patched(path)?;
        Ok(reg)
    }

    /// Every patched source names a base that is a registered dump, and has
    /// its patch facts; every other source has none.
    fn check_patched(&self, path: &Path) -> Result<(), RegistryError> {
        for s in &self.sources {
            let fail = |msg: String| RegistryError::Patched {
                path: path.to_path_buf(),
                id: s.id.clone(),
                msg,
            };
            match (s.kind, &s.patch) {
                (SourceKind::Dump, None) => {}
                (SourceKind::Dump, Some(_)) => {
                    return Err(fail(
                        "has a [patch] table but isn't kind = \"patched\"".into(),
                    ));
                }
                (SourceKind::Patched, None) => {
                    return Err(fail(
                        "is kind = \"patched\" but has no [patch] table".into(),
                    ));
                }
                (SourceKind::Patched, Some(p)) => match self.by_id(&p.base) {
                    None => return Err(fail(format!("its base {:?} isn't registered", p.base))),
                    Some(b) if b.kind != SourceKind::Dump => {
                        return Err(fail(format!("its base {:?} is itself patched", p.base)));
                    }
                    Some(_) => {}
                },
            }
        }
        Ok(())
    }

    /// The patched sources made with the patch whose SHA-256 is `sha256`,
    /// one per base it's registered for.
    pub fn by_patch_sha256(&self, sha256: &str) -> Vec<&KnownSource> {
        self.sources
            .iter()
            .filter(|s| s.patch.as_ref().is_some_and(|p| p.sha256 == sha256))
            .collect()
    }

    fn add_toml(&mut self, text: &str, path: &Path) -> Result<(), RegistryError> {
        let file: RegistryFile = toml::from_str(text).map_err(|source| RegistryError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        for s in file.source {
            for (what, value, clash) in [
                ("id", &s.id, self.sources.iter().any(|k| k.id == s.id)),
                ("sha256", &s.sha256, self.by_sha256(&s.sha256).is_some()),
            ] {
                if clash {
                    return Err(RegistryError::Duplicate {
                        path: path.to_path_buf(),
                        what,
                        value: value.clone(),
                    });
                }
            }
            if s.partial_board_facts() {
                return Err(RegistryError::PartialBoard {
                    path: path.to_path_buf(),
                    id: s.id.clone(),
                });
            }
            self.sources.push(s);
        }
        Ok(())
    }

    /// Finds `sources/` in `start` or the nearest ancestor that has one.
    pub fn find_dir(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .map(|d| d.join("sources"))
            .find(|d| d.is_dir())
    }

    pub fn by_sha256(&self, sha256: &str) -> Option<&KnownSource> {
        self.sources.iter().find(|s| s.sha256 == sha256)
    }

    pub fn by_id(&self, id: &str) -> Option<&KnownSource> {
        self.sources.iter().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[source]]
id = "homebrew-ntsc"
title = "Reference Homebrew"
platform = "nes"
region = "ntsc"
sha256 = "aa"
mapper = 0
status = "verified"
evidence = "built from homebrew/ in this repo"
"#;

    #[test]
    fn loads_and_looks_up() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("nes.toml"), SAMPLE).unwrap();
        std::fs::write(tmp.path().join("README.md"), "ignored").unwrap();
        let reg = Registry::load_dir(tmp.path()).unwrap();
        assert_eq!(reg.by_sha256("aa").unwrap().id, "homebrew-ntsc");
        assert_eq!(
            reg.by_id("homebrew-ntsc").unwrap().status,
            SourceStatus::Verified
        );
        assert!(reg.by_sha256("bb").is_none());
    }

    #[test]
    fn rejects_duplicate_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.toml"), SAMPLE).unwrap();
        std::fs::write(
            tmp.path().join("b.toml"),
            SAMPLE.replace("homebrew-ntsc", "other"),
        )
        .unwrap();
        let err = Registry::load_dir(tmp.path()).unwrap_err();
        assert!(matches!(
            err,
            RegistryError::Duplicate { what: "sha256", .. }
        ));
    }

    const PATCHED: &str = r#"
[[source]]
id = "homebrew-hack-1"
title = "Reference Homebrew Hack"
platform = "nes"
region = "ntsc"
sha256 = "cc"
mapper = 0
status = "unverified"
kind = "patched"

[source.patch]
base = "homebrew-ntsc"
format = "ips"
sha256 = "dd"
bytes = 10
offsets = "ines"
url = "https://example.org/hack.ips"
author = "someone"
permission = "none"
"#;

    #[test]
    fn loads_patched_sources_and_checks_their_base() {
        let reg = Registry::from_toml(&format!("{SAMPLE}{PATCHED}"), Path::new("x.toml")).unwrap();
        let hack = reg.by_sha256("cc").unwrap();
        assert_eq!(hack.kind, SourceKind::Patched);
        assert_eq!(hack.patch.as_ref().unwrap().base, "homebrew-ntsc");
        assert_eq!(reg.by_patch_sha256("dd").len(), 1);
        assert!(reg.by_patch_sha256("ee").is_empty());

        let orphan = Registry::from_toml(PATCHED, Path::new("x.toml")).unwrap_err();
        assert!(orphan.to_string().contains("isn't registered"), "{orphan}");
        let no_table = PATCHED.split("[source.patch]").next().unwrap();
        let err =
            Registry::from_toml(&format!("{SAMPLE}{no_table}"), Path::new("x.toml")).unwrap_err();
        assert!(err.to_string().contains("no [patch] table"), "{err}");
        let not_patched = PATCHED.replace("kind = \"patched\"\n", "");
        let err = Registry::from_toml(&format!("{SAMPLE}{not_patched}"), Path::new("x.toml"))
            .unwrap_err();
        assert!(err.to_string().contains("isn't kind"), "{err}");
    }

    #[test]
    fn finds_sources_dir_in_an_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sources")).unwrap();
        let deep = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(
            Registry::find_dir(&deep).unwrap(),
            tmp.path().join("sources")
        );
    }
}
