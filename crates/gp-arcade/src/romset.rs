//! Reading a romset: a MAME zip, every chip checked by size and SHA-1
//! against the set's list.
//!
//! The tools know no set of their own. A set's list (its chips' names,
//! regions, offsets, sizes and SHA-1s, from MAME's source) is in its
//! game's roms.toml, as `[[source.chip]]` rows under the arcade
//! `[[source]]` (docs/sources.md, "Arcade chips"), and comes in through
//! [`add_roms_toml`] or [`add_games_dir`], or from the games checkout
//! `$GP_GAMES_DIR` names ([`known`]). With none, every zip is refused with
//! [`NO_SETS`]. The SSIO board's PROM comes from MAME's
//! src/mame/midway/midway.cpp, `ROM_START( midway_ssio )`:
//! license:BSD-3-Clause, copyright-holders:Aaron Giles.
//!
//! # A set's identity
//!
//! A zip is known by its chips' contents, never by its file names: the
//! set is the known one all of whose chips are in the zip by SHA-1.
//! Its identity, the `sha256` a games checkout's roms.toml and package.toml
//! give it, is [`identity`] of the set's chips: the SHA-256 of their SHA-1s,
//! in lower-case hex, sorted, each followed by a newline. So it is the
//! same for any zip holding those chips under any names, a person can
//! work it out from MAME's list without the ROMs, and two sets that share
//! every chip would share it. The board's own PROM
//! ([`SSIO_PROM`]) isn't in the set's list, so it isn't in the identity:
//! a zip with or without it is the same set.
//!
//! ROM bytes stay in memory. Nothing here prints or writes them.

use std::io::Read;
use std::path::Path;

/// One chip of a set: its file name, the region and offset it loads at,
/// its size and its SHA-1.
#[derive(Debug, Clone, Copy)]
pub struct Chip {
    pub name: &'static str,
    pub region: Region,
    pub offset: usize,
    pub size: usize,
    pub sha1: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The main board's Z80 program, 0x10000 bytes.
    MainCpu,
    /// The SSIO board's Z80 program, 0x10000 bytes.
    SsioCpu,
    /// Background tiles, 0x8000 bytes.
    Gfx1,
    /// Sprites, 0x20000 bytes.
    Gfx2,
}

impl Region {
    /// MAME's name for the region.
    pub fn name(self) -> &'static str {
        match self {
            Self::MainCpu => "maincpu",
            Self::SsioCpu => "ssio:cpu",
            Self::Gfx1 => "gfx1",
            Self::Gfx2 => "gfx2",
        }
    }

    /// The region MAME calls `name` ([`Region::name`]).
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::MainCpu, Self::SsioCpu, Self::Gfx1, Self::Gfx2]
            .into_iter()
            .find(|r| r.name() == name)
    }

    pub fn size(self) -> usize {
        match self {
            Self::MainCpu | Self::SsioCpu => 0x10000,
            Self::Gfx1 => 0x8000,
            Self::Gfx2 => 0x20000,
        }
    }
}

#[cfg(test)]
const fn chip(
    name: &'static str,
    region: Region,
    offset: usize,
    size: usize,
    sha1: &'static str,
) -> Chip {
    Chip {
        name,
        region,
        offset,
        size,
        sha1,
    }
}

/// A set this board runs: MAME's name for it, its title, and its chips.
#[derive(Debug, Clone)]
pub struct SetInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub chips: &'static [Chip],
}

impl SetInfo {
    /// The set's identity ([`identity`] of its chips).
    pub fn identity(&self) -> String {
        identity(self.chips.iter().map(|c| c.sha1))
    }
}

/// The sets the tools know, from roms.toml files: none until some are
/// added ([`add_roms_toml`], [`add_games_dir`]), apart from those of the
/// games checkout `$GP_GAMES_DIR` names, which are read on first use. Each
/// set is kept for the life of the process (a few hundred bytes), once.
static KNOWN: std::sync::RwLock<Vec<&'static SetInfo>> = std::sync::RwLock::new(Vec::new());
static FROM_ENV: std::sync::Once = std::sync::Once::new();

/// The environment variable naming a games checkout, whose `*/roms.toml`
/// files list the sets the tools know.
pub const GAMES_DIR_ENV: &str = "GP_GAMES_DIR";

fn from_env() {
    FROM_ENV.call_once(|| {
        let Some(dir) = std::env::var_os(GAMES_DIR_ENV).filter(|v| !v.is_empty()) else {
            return;
        };
        match games_dir_sets(Path::new(&dir)) {
            Ok(sets) => {
                insert(sets);
            }
            Err(e) => eprintln!("gp-arcade: ${GAMES_DIR_ENV}: {e}"),
        }
    });
}

/// Every set known, in the order they were added.
pub fn known() -> Vec<&'static SetInfo> {
    from_env();
    KNOWN.read().map(|k| k.clone()).unwrap_or_default()
}

/// The known set MAME calls `name`.
pub fn set(name: &str) -> Option<&'static SetInfo> {
    known().into_iter().find(|s| s.name == name)
}

/// What to say when no set is known: where the sets come from.
pub const NO_SETS: &str = "no arcade romset is known: the chip lists come from a game's roms.toml ([[source.chip]] rows); point --games or $GP_GAMES_DIR at a games checkout, or work in the game's folder";

/// Adds sets to those known, each once (by name and identity). Returns how
/// many were new.
pub fn add_sets(sets: Vec<SetInfo>) -> usize {
    from_env();
    insert(sets)
}

/// [`add_sets`] without first reading `$GP_GAMES_DIR`: how [`from_env`]
/// adds that checkout's sets.
fn insert(sets: Vec<SetInfo>) -> usize {
    let Ok(mut k) = KNOWN.write() else {
        return 0;
    };
    let mut added = 0;
    for s in sets {
        let id = s.identity();
        if k.iter().any(|o| o.name == s.name && o.identity() == id) {
            continue;
        }
        k.push(Box::leak(Box::new(s)));
        added += 1;
    }
    added
}

#[derive(serde::Deserialize)]
struct RomsFile {
    #[serde(default)]
    source: Vec<SourceRow>,
}

#[derive(serde::Deserialize)]
struct SourceRow {
    #[serde(default)]
    id: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    chip: Vec<ChipRow>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ChipRow {
    name: String,
    region: String,
    offset: usize,
    size: usize,
    sha1: String,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// The sets a roms.toml lists: each arcade `[[source]]` with
/// `[[source.chip]]` rows, by its `system` (MAME's name; else its id),
/// titled by its `revision` (else its title). A source whose `sha256`
/// isn't its chips' identity is an error, as is a chip row whose region
/// isn't one of this board's or that doesn't fit in it.
pub fn sets_from_roms_toml(text: &str) -> Result<Vec<SetInfo>, String> {
    let file: RomsFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for s in file.source {
        if s.platform != "arcade" || s.chip.is_empty() {
            continue;
        }
        let name = s.system.clone().unwrap_or_else(|| s.id.clone());
        let mut chips = Vec::new();
        for c in s.chip {
            let region = Region::from_name(&c.region).ok_or_else(|| {
                format!(
                    "{name}: chip {}: region {:?} isn't one of maincpu, ssio:cpu, gfx1 or gfx2",
                    c.name, c.region
                )
            })?;
            if c.offset + c.size > region.size() {
                return Err(format!(
                    "{name}: chip {} at {:#x}, {:#x} bytes, is past the end of {} ({:#x} bytes)",
                    c.name,
                    c.offset,
                    c.size,
                    c.region,
                    region.size()
                ));
            }
            let sha1 = c.sha1.to_ascii_lowercase();
            if sha1.len() != 40 || !sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!(
                    "{name}: chip {}: {:?} isn't a SHA-1",
                    c.name, c.sha1
                ));
            }
            chips.push(Chip {
                name: leak(c.name),
                region,
                offset: c.offset,
                size: c.size,
                sha1: leak(sha1),
            });
        }
        let info = SetInfo {
            name: leak(name.clone()),
            title: leak(s.revision.unwrap_or(s.title)),
            chips: Box::leak(chips.into_boxed_slice()),
        };
        if let Some(want) = s.sha256
            && !want.eq_ignore_ascii_case(&info.identity())
        {
            return Err(format!(
                "{name}: sha256 {want} isn't its chips' identity {}",
                info.identity()
            ));
        }
        out.push(info);
    }
    Ok(out)
}

/// Adds the sets a roms.toml lists ([`sets_from_roms_toml`]); how many
/// were new.
pub fn add_roms_toml(text: &str) -> Result<usize, String> {
    Ok(add_sets(sets_from_roms_toml(text)?))
}

/// Adds the sets of a games checkout (each `DIR/*/roms.toml`) or of one
/// game's folder (`DIR/roms.toml`); how many were new. A folder with no
/// roms.toml adds none; a folder that isn't there is an error.
pub fn add_games_dir(dir: &Path) -> Result<usize, String> {
    Ok(add_sets(games_dir_sets(dir)?))
}

/// The sets [`add_games_dir`] adds, read.
fn games_dir_sets(dir: &Path) -> Result<Vec<SetInfo>, String> {
    let mut files = Vec::new();
    let own = dir.join("roms.toml");
    if own.is_file() {
        files.push(own);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut sub: Vec<_> = rd
            .flatten()
            .map(|e| e.path().join("roms.toml"))
            .filter(|p| p.is_file())
            .collect();
        sub.sort();
        files.extend(sub);
    }
    if !dir.is_dir() {
        return Err(format!("{}: no such folder", dir.display()));
    }
    let mut sets = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        sets.extend(sets_from_roms_toml(&text).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    Ok(sets)
}

/// A set's identity from its chips' SHA-1s (lower-case hex): the SHA-256 of
/// them sorted, each followed by a newline, in lower-case hex.
pub fn identity<'a>(sha1s: impl IntoIterator<Item = &'a str>) -> String {
    use sha2::Digest as _;
    let mut v: Vec<&str> = sha1s.into_iter().collect();
    v.sort_unstable();
    let mut h = sha2::Sha256::new();
    for s in v {
        h.update(s.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// The SSIO board's 82S123 PROM: the board's own ROM, which MAME's set lists
/// don't name. Only the sound volumes read it. Zips carry it as
/// `82s123.12d` or under a set's own chip name, so it is found by its hash.
pub const SSIO_PROM: Chip = Chip {
    name: "82s123.12d",
    region: Region::SsioCpu,
    offset: 0,
    size: 32,
    sha1: "9ac9b01d24affc0ee9227a4364c4fd8f8290343a",
};

/// Where a set's zip is on this machine, for the tests and checks that
/// play it and skip without it: `$GP_ARCADE_<NAME>` (upper case), else
/// `$GP_ROM_DIR/arcade/<name>.zip`. Nothing else is searched, so a check
/// finds a ROM only when it was pointed at one.
pub fn local_romset(name: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let file = format!("{name}.zip");
    [
        env(&format!("GP_ARCADE_{}", name.to_uppercase())),
        env("GP_ROM_DIR").map(|d| d.join("arcade").join(&file)),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

/// Every known set as a roms.toml lists it ([`sets_from_roms_toml`] reads
/// it back the same): for a server to hand the sets to a browser's board
/// (gp_arcade_play's `gp_sets_load`). Hashes and places only.
pub fn known_roms_toml() -> String {
    let q = |s: &str| format!("{s:?}");
    let mut t =
        String::from("# The arcade sets the tools know, from the games' roms.toml files.\n");
    for s in known() {
        t += &format!(
            "\n[[source]]\nid = {}\ntitle = {}\nplatform = \"arcade\"\nregion = \"us\"\nrevision = {}\nsystem = {}\nsha256 = {}\nstatus = \"unverified\"\n",
            q(s.name),
            q(s.title),
            q(s.title),
            q(s.name),
            q(&s.identity())
        );
        for c in s.chips {
            t += &format!(
                "\n[[source.chip]]\nname = {}\nregion = {}\noffset = {:#x}\nsize = {:#x}\nsha1 = {}\n",
                q(c.name),
                q(c.region.name()),
                c.offset,
                c.size,
                q(c.sha1)
            );
        }
    }
    t
}

/// The environment variable naming the game folder the tests and checks
/// that need a real romset play: a game of the games checkout, such as
/// `GAMES/NAME`, whose roms.toml lists its sets.
pub const TEST_GAME_ENV: &str = "GP_GAME_DIR";

/// For the tests and checks that need a real romset: the game folder
/// [`TEST_GAME_ENV`] names, its sets added to those known. Err says why
/// there is none, for a test to print as it skips.
pub fn test_game() -> Result<std::path::PathBuf, String> {
    let Some(dir) = std::env::var_os(TEST_GAME_ENV).filter(|v| !v.is_empty()) else {
        return Err(format!(
            "no game to play: set ${TEST_GAME_ENV} to an arcade game's folder in the games checkout"
        ));
    };
    let dir = std::path::PathBuf::from(dir);
    add_games_dir(&dir).map_err(|e| format!("${TEST_GAME_ENV}: {e}"))?;
    Ok(dir)
}

/// The first set of [`test_game`] (in its roms.toml's order) whose zip is
/// on this machine: its name and its zip's path. Err says why there is
/// none.
pub fn test_romset_path() -> Result<(&'static str, std::path::PathBuf), String> {
    let dir = test_game()?;
    let text = std::fs::read_to_string(dir.join("roms.toml"))
        .map_err(|e| format!("{}: {e}", dir.join("roms.toml").display()))?;
    let sets = sets_from_roms_toml(&text)?;
    for s in &sets {
        if let (Some(path), Some(known)) = (local_romset(s.name), set(s.name)) {
            return Ok((known.name, path));
        }
    }
    Err(format!(
        "no zip of {} on this machine ($GP_ROM_DIR/arcade)",
        sets.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
    ))
}

/// The set [`test_game`]'s first oracle input log was recorded on (its
/// `system` line), when its zip is on this machine: its name and its
/// zip's path. Err says why there is none.
pub fn test_oracle_set_path() -> Result<(&'static str, std::path::PathBuf), String> {
    let log = test_input_log()?;
    let system = log
        .lines()
        .find_map(|l| l.strip_prefix("system "))
        .map(str::trim)
        .ok_or("the game's first oracle input log names no system")?;
    let known = set(system).ok_or_else(|| format!("the game's roms.toml lists no set {system}"))?;
    let path = local_romset(system).ok_or_else(|| {
        format!(
            "no {system}.zip on this machine ($GP_ARCADE_{}, $GP_ROM_DIR/arcade)",
            system.to_uppercase()
        )
    })?;
    Ok((known.name, path))
}

/// [`test_romset_path`]'s set, loaded.
pub fn test_romset() -> Result<Romset, String> {
    let (_, path) = test_romset_path()?;
    Romset::load_zip(&path).map_err(|e| e.to_string())
}

/// Where a game keeps its MAME captures: `[[capture]]` (how to make each
/// one and its hash) and `[[replay]]` (the captures the arcade tests
/// replay, each by its name and input log), input paths from the game's
/// folder.
pub const CAPTURES_FILE: &str = "oracle/mame-captures.toml";

/// The captures [`test_game`]'s [`CAPTURES_FILE`] lists for the arcade
/// tests to replay (`[[replay]]`), as (name, input log path). Err says why
/// there are none.
pub fn test_replays() -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let dir = test_game()?;
    let path = dir.join(CAPTURES_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let list: toml::Table = toml::from_str(&text).map_err(|e| format!("{CAPTURES_FILE}: {e}"))?;
    let out: Vec<(String, std::path::PathBuf)> = list
        .get("replay")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some((
                r.get("name")?.as_str()?.to_owned(),
                dir.join(r.get("input")?.as_str()?),
            ))
        })
        .collect();
    if out.is_empty() {
        return Err(format!("{} lists no [[replay]]", path.display()));
    }
    Ok(out)
}

/// The first input log [`test_replays`] names: its text.
pub fn test_input_log() -> Result<String, String> {
    let (_, input) = test_replays()?.remove(0);
    std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))
}

/// Whether `data` starts as a zip file does.
pub fn looks_like_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06")
}

/// A loaded set: the four regions, and what else the zip held.
#[derive(Clone)]
pub struct Romset {
    /// MAME's name for the set.
    pub name: &'static str,
    /// Its identity ([`identity`]).
    pub identity: String,
    pub maincpu: Vec<u8>,
    pub ssiocpu: Vec<u8>,
    pub gfx1: Vec<u8>,
    pub gfx2: Vec<u8>,
    /// Files in the zip that aren't chips of the set, with what they are
    /// when MAME knows them. Reported, then ignored.
    pub extras: Vec<String>,
    /// The SSIO board's PROM ([`SSIO_PROM`]), found by its hash under
    /// whatever name: the sound's volumes come from it. Without it the
    /// board is silent.
    pub ssio_prom: Option<Vec<u8>>,
}

impl std::fmt::Debug for Romset {
    // Never the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Romset")
            .field("name", &self.name)
            .field("identity", &self.identity)
            .field("extras", &self.extras)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RomError {
    /// No set is known: no roms.toml was read ([`NO_SETS`]).
    #[error("{}", NO_SETS)]
    NoSets,
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    #[error("{0}: not a zip file I can read ({1})")]
    Zip(String, String),
    #[error("{set}: chip {chip} is missing")]
    Missing {
        set: &'static str,
        chip: &'static str,
    },
    #[error("{set}: chip {chip} is {found} bytes, the set's list says {want}")]
    Size {
        set: &'static str,
        chip: &'static str,
        found: usize,
        want: usize,
    },
    #[error("{set}: chip {chip} has SHA-1 {found}, the set's list says {want}")]
    Hash {
        set: &'static str,
        chip: &'static str,
        found: String,
        want: &'static str,
    },
    #[error(
        "the zip matches no set gp-arcade knows ({known}); the nearest is {nearest}, with {have} of its {of} chips"
    )]
    Unknown {
        known: String,
        nearest: &'static str,
        have: usize,
        of: usize,
    },
}

impl Romset {
    /// Reads a MAME zip and finds which known set ([`known`]) it is.
    pub fn load_zip(path: &Path) -> Result<Self, RomError> {
        let shown = path.display().to_string();
        let data = std::fs::read(path).map_err(|e| RomError::Io(shown.clone(), e))?;
        Self::from_zip(&data).map_err(|e| match e {
            RomError::Zip(_, why) => RomError::Zip(shown, why),
            e => e,
        })
    }

    /// [`Romset::load_zip`] on a zip's bytes.
    pub fn from_zip(data: &[u8]) -> Result<Self, RomError> {
        let files = read_zip(data).map_err(|e| RomError::Zip("the romset".into(), e))?;
        Self::identify(files)
    }

    /// The known set ([`known`]) whose every chip is among `files` by
    /// SHA-1, whatever the files are named, loaded from them.
    pub fn identify(files: Vec<(String, Vec<u8>)>) -> Result<Self, RomError> {
        Self::identify_in(&known(), files)
    }

    /// [`Romset::identify`] among `sets`.
    pub fn identify_in(
        sets: &[&'static SetInfo],
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, RomError> {
        let have: std::collections::HashSet<String> = files
            .iter()
            .map(|(_, d)| hex(&crate::sha1::sha1(d)))
            .collect();
        let count = |s: &SetInfo| s.chips.iter().filter(|c| have.contains(c.sha1)).count();
        if let Some(s) = sets.iter().find(|s| count(s) == s.chips.len()) {
            return Self::from_files(s.name, s.chips, files);
        }
        let Some(nearest) = sets.iter().max_by_key(|s| count(s)) else {
            return Err(RomError::NoSets);
        };
        Err(RomError::Unknown {
            known: sets.iter().map(|s| s.name).collect::<Vec<_>>().join(", "),
            nearest: nearest.name,
            have: count(nearest),
            of: nearest.chips.len(),
        })
    }

    /// Builds a set from (name, bytes) pairs. A chip is found by its name,
    /// or else by its SHA-1, as MAME does.
    pub fn from_files(
        name: &'static str,
        chips: &[Chip],
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, RomError> {
        let hashed: Vec<(String, String, Vec<u8>)> = files
            .into_iter()
            .map(|(n, d)| (n, hex(&crate::sha1::sha1(&d)), d))
            .collect();
        let mut used = vec![false; hashed.len()];
        let mut regions = [
            vec![0u8; Region::MainCpu.size()],
            vec![0u8; Region::SsioCpu.size()],
            vec![0u8; Region::Gfx1.size()],
            vec![0u8; Region::Gfx2.size()],
        ];
        for c in chips {
            let found = hashed
                .iter()
                .position(|(n, _, _)| n == c.name)
                .or_else(|| hashed.iter().position(|(_, h, _)| h == c.sha1));
            let Some(i) = found else {
                return Err(RomError::Missing {
                    set: name,
                    chip: c.name,
                });
            };
            let (_, hash, bytes) = &hashed[i];
            if bytes.len() != c.size {
                return Err(RomError::Size {
                    set: name,
                    chip: c.name,
                    found: bytes.len(),
                    want: c.size,
                });
            }
            if hash != c.sha1 {
                return Err(RomError::Hash {
                    set: name,
                    chip: c.name,
                    found: hash.clone(),
                    want: c.sha1,
                });
            }
            used[i] = true;
            let r = match c.region {
                Region::MainCpu => 0,
                Region::SsioCpu => 1,
                Region::Gfx1 => 2,
                Region::Gfx2 => 3,
            };
            regions[r][c.offset..c.offset + c.size].copy_from_slice(bytes);
        }
        let mut ssio_prom = None;
        let mut extras = Vec::new();
        for ((n, h, d), _) in hashed.iter().zip(&used).filter(|(_, u)| !**u) {
            if h == SSIO_PROM.sha1 {
                ssio_prom = Some(d.clone());
            } else {
                extras.push(format!(
                    "{n} ({} bytes, SHA-1 {h}): not in the set's list; ignored",
                    d.len()
                ));
            }
        }
        let [maincpu, ssiocpu, gfx1, gfx2] = regions;
        Ok(Self {
            name,
            identity: identity(chips.iter().map(|c| c.sha1)),
            maincpu,
            ssiocpu,
            gfx1,
            gfx2,
            extras,
            ssio_prom,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn le16(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| "truncated".to_string())
}

fn le32(d: &[u8], at: usize) -> Result<usize, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| "truncated".to_string())
}

/// The files of a zip: (name, contents). Stored and deflated entries only,
/// which is what MAME romsets use.
pub fn read_zip(d: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    // The end of central directory record, searched for from the end.
    let eocd = (0..=d.len().saturating_sub(22))
        .rev()
        .find(|&i| d[i..].starts_with(b"PK\x05\x06"))
        .ok_or("no end of central directory")?;
    let count = le16(d, eocd + 10)?;
    let mut at = le32(d, eocd + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !d.get(at..).is_some_and(|s| s.starts_with(b"PK\x01\x02")) {
            return Err("bad central directory entry".into());
        }
        let method = le16(d, at + 10)?;
        let csize = le32(d, at + 20)?;
        let usize_ = le32(d, at + 24)?;
        let nlen = le16(d, at + 28)?;
        let elen = le16(d, at + 30)?;
        let clen = le16(d, at + 32)?;
        let local = le32(d, at + 42)?;
        let name = String::from_utf8_lossy(d.get(at + 46..at + 46 + nlen).ok_or("truncated")?)
            .into_owned();
        at += 46 + nlen + elen + clen;
        if name.ends_with('/') {
            continue;
        }
        if !d.get(local..).is_some_and(|s| s.starts_with(b"PK\x03\x04")) {
            return Err(format!("{name}: bad local header"));
        }
        let start = local + 30 + le16(d, local + 26)? + le16(d, local + 28)?;
        let raw = d.get(start..start + csize).ok_or("truncated entry")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw)
                    .read_to_end(&mut v)
                    .map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: compression method {m}")),
        };
        if bytes.len() != usize_ {
            return Err(format!(
                "{name}: {} bytes, the directory says {usize_}",
                bytes.len()
            ));
        }
        out.push((name, bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(chips: &[Chip]) -> Vec<(String, Vec<u8>)> {
        chips
            .iter()
            .map(|c| (c.name.to_string(), vec![0u8; c.size]))
            .collect()
    }

    /// A made-up set's roms.toml: two chips of made-up bytes (1s and 2s).
    fn made_up_toml(system: &str, fill: u8) -> String {
        let a = vec![fill; 0x1000];
        let b = vec![fill + 1; 0x4000];
        let (ha, hb) = (hex(&crate::sha1::sha1(&a)), hex(&crate::sha1::sha1(&b)));
        let id = identity([ha.as_str(), hb.as_str()]);
        format!(
            "[[source]]\nid = \"{system}-x\"\ntitle = \"Made Up\"\nplatform = \"arcade\"\nregion = \"us\"\n\
             revision = \"Made Up ({system})\"\nsystem = \"{system}\"\nsha256 = \"{id}\"\nstatus = \"unverified\"\n\n\
             [[source.chip]]\nname = \"{system}.1a\"\nregion = \"maincpu\"\noffset = 0x0000\nsize = 0x1000\nsha1 = \"{ha}\"\n\n\
             [[source.chip]]\nname = \"{system}.bg\"\nregion = \"gfx1\"\noffset = 0x4000\nsize = 0x4000\nsha1 = \"{hb}\"\n\n\
             [[source]]\nid = \"nes-thing\"\ntitle = \"An NES game\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"00\"\nstatus = \"unverified\"\n"
        )
    }

    fn files_of(fill: u8) -> Vec<(String, Vec<u8>)> {
        vec![
            ("x".into(), vec![fill + 1; 0x4000]),
            ("y".into(), vec![fill; 0x1000]),
        ]
    }

    #[test]
    fn a_wrong_chip_is_named() {
        let sets = sets_from_roms_toml(&made_up_toml("madeupa", 1)).unwrap();
        let s = &sets[0];
        let err = Romset::from_files(s.name, s.chips, fake(s.chips)).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Hash {
                    chip: "madeupa.1a",
                    ..
                }
            ),
            "{err}"
        );
        let mut files = fake(s.chips);
        files.remove(0);
        let err = Romset::from_files(s.name, s.chips, files).unwrap_err();
        assert!(
            matches!(
                err,
                RomError::Missing {
                    chip: "madeupa.1a",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_roms_toml_lists_its_arcade_sets_and_a_zip_is_known_by_them() {
        let sets = sets_from_roms_toml(&made_up_toml("madeupb", 3)).unwrap();
        assert_eq!(sets.len(), 1, "the NES source lists no chips");
        assert_eq!(sets[0].name, "madeupb");
        assert_eq!(sets[0].title, "Made Up (madeupb)");
        let sets: Vec<&'static SetInfo> =
            sets.into_iter().map(|s| &*Box::leak(Box::new(s))).collect();
        let set = Romset::identify_in(&sets, files_of(3)).unwrap();
        assert_eq!(set.name, "madeupb");
        assert_eq!(set.maincpu[0], 3);
        assert_eq!(set.gfx1[0x4000], 4);
        assert_eq!(set.identity, sets[0].identity());
        // Added once, however often it is read.
        let text = made_up_toml("madeupc", 5);
        assert_eq!(add_roms_toml(&text).unwrap(), 1);
        assert_eq!(add_roms_toml(&text).unwrap(), 0);
        assert!(set_named_known("madeupc"));
        assert_eq!(Romset::identify(files_of(5)).unwrap().name, "madeupc");
    }

    fn set_named_known(name: &str) -> bool {
        set(name).is_some()
    }

    #[test]
    fn a_roms_toml_whose_rows_are_wrong_is_refused() {
        let good = made_up_toml("madeupd", 1);
        let bad_id = good.replacen("sha256 = \"", "sha256 = \"ff", 1);
        assert!(
            sets_from_roms_toml(&bad_id)
                .unwrap_err()
                .contains("isn't its chips' identity")
        );
        let bad_region = good.replace("region = \"gfx1\"", "region = \"gfx9\"");
        assert!(
            sets_from_roms_toml(&bad_region)
                .unwrap_err()
                .contains("gfx9")
        );
        let past = good.replace("offset = 0x4000", "offset = 0x6000");
        assert!(
            sets_from_roms_toml(&past)
                .unwrap_err()
                .contains("past the end")
        );
        let extra = good.replace("size = 0x1000", "size = 0x1000\ncrc = 1");
        assert!(sets_from_roms_toml(&extra).is_err());
    }

    #[test]
    fn the_known_sets_read_back_from_their_roms_toml() {
        add_roms_toml(&made_up_toml("madeupf", 9)).unwrap();
        let back = sets_from_roms_toml(&known_roms_toml()).unwrap();
        let f = back.iter().find(|s| s.name == "madeupf").unwrap();
        assert_eq!(f.identity(), set("madeupf").unwrap().identity());
        assert_eq!(f.chips.len(), 2);
    }

    #[test]
    fn with_no_set_known_a_zip_is_refused_in_words() {
        let err = Romset::identify_in(&[], files_of(1)).unwrap_err();
        assert!(matches!(err, RomError::NoSets), "{err}");
        assert!(err.to_string().contains("roms.toml"), "{err}");
    }

    #[test]
    fn checks_against_a_made_up_list_and_reports_extras() {
        // A two-chip "set" whose hashes are those of the made-up contents.
        let a = vec![1u8; 0x1000];
        let b = vec![2u8; 0x4000];
        let ha: &'static str = Box::leak(hex(&crate::sha1::sha1(&a)).into_boxed_str());
        let hb: &'static str = Box::leak(hex(&crate::sha1::sha1(&b)).into_boxed_str());
        let chips = [
            chip("a", Region::SsioCpu, 0x1000, 0x1000, ha),
            chip("b", Region::Gfx1, 0x4000, 0x4000, hb),
        ];
        let files = vec![
            ("renamed".to_string(), b.clone()),
            ("a".to_string(), a),
            ("junk".to_string(), vec![9u8; 7]),
        ];
        let set = Romset::from_files("test", &chips, files).unwrap();
        assert_eq!(set.ssiocpu[0x1000], 1);
        assert_eq!(set.ssiocpu[0x0fff], 0);
        assert_eq!(set.gfx1[0x4000], 2);
        assert_eq!(set.extras.len(), 1);
        assert!(set.extras[0].starts_with("junk (7 bytes"));
    }

    #[test]
    fn identity_is_the_sorted_sha1s_and_names_do_not_matter() {
        assert_eq!(identity(["b", "a"]), identity(["a", "b"]));
        // The SHA-256 of "a\nb\n".
        assert_eq!(
            identity(["b", "a"]),
            "911169ddaaf146aff539f58c26c489af3b892dff0fe283c1c264c65ae5aa59a2"
        );
    }

    #[test]
    fn a_zip_of_made_up_chips_matches_no_set_and_says_the_nearest() {
        let sets: Vec<&'static SetInfo> = sets_from_roms_toml(&made_up_toml("madeupe", 7))
            .unwrap()
            .into_iter()
            .map(|s| &*Box::leak(Box::new(s)))
            .collect();
        let mut files = files_of(7);
        files.remove(0);
        let err = Romset::identify_in(&sets, files).unwrap_err();
        assert!(
            matches!(err, RomError::Unknown { have: 1, of: 2, .. }),
            "{err}"
        );
        assert!(err.to_string().contains("madeupe"), "{err}");
    }

    #[test]
    fn reads_a_stored_zip() {
        // A zip with one stored file, built by hand.
        let body = b"hello";
        let mut z = Vec::new();
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes()); // crc, unchecked here
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.push(b'x');
        z.extend_from_slice(body);
        let cd = z.len();
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&5u32.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&[0; 12]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.push(b'x');
        let cdlen = z.len() - cd;
        z.extend_from_slice(b"PK\x05\x06");
        z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend_from_slice(&(cdlen as u32).to_le_bytes());
        z.extend_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        let files = read_zip(&z).unwrap();
        assert_eq!(files, vec![("x".to_string(), body.to_vec())]);
    }
}
