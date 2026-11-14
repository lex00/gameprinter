//! `gp labels`: look things up in a game's label file (docs/formats/labels.md)
//! and check that a state mapping's RAM addresses are all labelled.
//!
//! A game's labels file is `labels.toml` in its folder: Moss Hop's is
//! committed in this repo (`games/mosshop/labels.toml`), and a game in the
//! games checkout keeps its own beside its `roms.toml`. A game is named by
//! its folder or by a source ID its `roms.toml` lists, and a ROM hack (a
//! patched source) is read with its base's labels.
//!
//! Ported from game-printer's labels.rs and serve/labels.rs. The files the
//! old repo generated under `$GP_HOME/work` from the Super Mario Bros., Mega
//! Man 2 and Super Mario Bros. 2 disassemblies are those games' and are not
//! here.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Outcome;

/// A game's labels file, in its folder.
pub(crate) const FILE: &str = "labels.toml";

#[derive(Subcommand)]
pub enum LabelsCmd {
    /// List a game's labels, which cover RAM variables and routines and data
    /// tables.
    Show {
        #[command(flatten)]
        which: Which,
        /// Only this kind of label.
        #[arg(long, value_enum, default_value = "all")]
        kind: Kind,
    },
    /// Find labels by RAM or CPU address (`0x0400`, `$0400`) or by name.
    /// Exits 1 if nothing matches.
    Lookup {
        #[command(flatten)]
        which: Which,
        /// An address, or a label's name.
        query: String,
    },
    /// Check that every RAM address a state mapping reads has a label, and
    /// list the ones that don't. Exits 1 if any is missing.
    Check {
        /// A model's mapping.toml (docs/formats/state-mapping.md).
        mapping: PathBuf,
        /// The labels file, instead of the one for the mapping's `source`.
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Draft a labels file from a run of an arcade romset (its access log
    /// on the main Z80): the work RAM written, with its writers and
    /// readers, the routines that ran, with their callers, and the ROM
    /// read as data. Names come from addresses or the mapping; a person
    /// or agent renames and keeps what holds. Writes to --out (outside any
    /// git work tree) or standard output, never over a game's labels.toml.
    Propose {
        /// The romset (a MAME zip).
        rom: PathBuf,
        /// The input log to run from power-on.
        #[arg(long)]
        input: PathBuf,
        /// A state mapping whose fields name the RAM they read.
        #[arg(long)]
        mapping: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Args)]
pub struct Which {
    /// The game: a source ID a game's roms.toml lists (donkey-kong-oe), a
    /// game's folder name in the games checkout, or `mosshop`.
    pub source: String,
    /// The labels file, instead of the game's usual one.
    #[arg(long)]
    pub file: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    All,
    Ram,
    Routine,
    Data,
}

pub(crate) fn run(cmd: &LabelsCmd) -> Result<Outcome> {
    match cmd {
        LabelsCmd::Show { which, kind } => {
            let (file, _) = load(which)?;
            Ok(show(&file, *kind))
        }
        LabelsCmd::Lookup { which, query } => {
            let (file, _) = load(which)?;
            lookup(&file, query)
        }
        LabelsCmd::Check { mapping, file } => check(mapping, file.as_deref()),
        LabelsCmd::Propose {
            rom,
            input,
            mapping,
            out,
        } => crate::labels_propose::run(rom, input, mapping.as_deref(), out.as_deref()),
    }
}

// ---- The file ----

/// A labels file, schema 0.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelsFile {
    pub schema: u32,
    pub game: String,
    /// The pinned disassembly the file came from, for people.
    #[serde(default)]
    pub disassembly: String,
    /// Bytes per PRG bank, which `bank` in `routine` and `data` counts.
    #[serde(default)]
    pub bank_size: u32,
    #[serde(default)]
    pub label: Vec<Label>,
    #[serde(default)]
    pub routine: Vec<Rom>,
    #[serde(default)]
    pub data: Vec<Rom>,
}

/// A label in PRG ROM: a routine or a data table.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rom {
    pub name: String,
    pub bank: u32,
    /// The CPU address it is read or run at.
    pub addr: u16,
    /// Its offset in the PRG ROM.
    pub prg: u32,
    /// Bytes up to the next label in its bank.
    pub size: u32,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Label {
    pub name: String,
    pub addr: u16,
    pub size: u16,
    #[serde(default)]
    pub description: String,
}

pub(crate) fn parse(text: &str) -> Result<LabelsFile> {
    let file: LabelsFile = toml::from_str(text)?;
    anyhow::ensure!(file.schema == 0, "labels schema {} isn't 0", file.schema);
    Ok(file)
}

pub(crate) fn read(path: &Path) -> Result<LabelsFile> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text).with_context(|| path.display().to_string())
}

// ---- Where a game's file is ----

/// This repo, for Moss Hop's committed file: the working folder or one
/// above it holding `games/mosshop/labels.toml`, else the checkout gp was
/// built from.
fn repo_root() -> Option<PathBuf> {
    crate::repo::find_repo(|d| d.join("games/mosshop").join(FILE).is_file())
}

/// The folder of the game in the games checkout `games` that is named
/// `game` or whose roms.toml lists a source with that ID.
pub(crate) fn game_folder(games: &Path, game: &str) -> Option<PathBuf> {
    if game.is_empty() || game.contains('/') || game.contains("..") {
        return None;
    }
    let named = games.join(game);
    if named.join("roms.toml").is_file() {
        return Some(named);
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(games)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("roms.toml").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|d| {
        fs::read_to_string(d.join("roms.toml"))
            .ok()
            .and_then(|t| gp_evidence::registry::Registry::from_toml(&t, d).ok())
            .is_some_and(|r| r.by_id(game).is_some())
    })
}

/// Where `game`'s labels file is, or why it can't be found.
pub(crate) fn locate(game: &str) -> Result<PathBuf, String> {
    locate_in(
        game,
        repo_root().as_deref(),
        crate::roms_cmd::games_dir(None).as_deref(),
    )
}

pub(crate) fn locate_in(
    game: &str,
    repo: Option<&Path>,
    games: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(repo) = repo {
        let f = repo.join("games").join(game).join(FILE);
        if !game.contains('/') && !game.contains("..") && f.is_file() {
            return Ok(f);
        }
    }
    if let Some(dir) = games.and_then(|g| game_folder(g, game)) {
        let f = dir.join(FILE);
        if f.is_file() {
            return Ok(f);
        }
    }
    // A ROM hack (a patched source) is read with its base's labels: they
    // name the base's code and RAM, which a hack mostly keeps (`gp roms
    // diff --summary` says what it changed).
    let base = games
        .and_then(|g| gp_evidence::registry::Registry::load_games(g).ok())
        .and_then(|r| r.by_id(game).and_then(|k| k.patch.clone()))
        .map(|p| p.base);
    match base {
        Some(b) if b != game => locate_in(&b, repo, games),
        _ => Err(format!("no labels file is known for {game}")),
    }
}

/// A game's labels file, parsed, and where it was read from.
fn load(which: &Which) -> Result<(LabelsFile, PathBuf)> {
    let path = match &which.file {
        Some(f) => f.clone(),
        None => game_file(&which.source)?,
    };
    Ok((read(&path)?, path))
}

fn game_file(game: &str) -> Result<PathBuf> {
    match locate(game) {
        Err(why) => bail!("{why}"),
        Ok(path) => Ok(path),
    }
}

// ---- The commands ----

fn show(file: &LabelsFile, kind: Kind) -> Outcome {
    let mut text = String::new();
    let want = |k| kind == Kind::All || kind == k;
    if want(Kind::Ram) {
        for l in &file.label {
            let _ = writeln!(
                text,
                "ram      ${:04X} +{:<4} {}  {}",
                l.addr, l.size, l.name, l.description
            );
        }
    }
    for (k, word, rows) in [
        (Kind::Routine, "routine", &file.routine),
        (Kind::Data, "data", &file.data),
    ] {
        if want(k) {
            for r in rows {
                let _ = writeln!(
                    text,
                    "{word:<8} {:02X}:{:04X} +{:<5} {}  {}",
                    r.bank, r.addr, r.size, r.name, r.description
                );
            }
        }
    }
    let counts = format!(
        "{} RAM, {} routines, {} data labels for {}",
        file.label.len(),
        file.routine.len(),
        file.data.len(),
        file.game
    );
    let json = json!({
        "game": file.game,
        "disassembly": file.disassembly,
        "bank_size": file.bank_size,
        "ram": if want(Kind::Ram) { json!(file.label) } else { json!([]) },
        "routines": if want(Kind::Routine) { json!(file.routine) } else { json!([]) },
        "data": if want(Kind::Data) { json!(file.data) } else { json!([]) },
    });
    Outcome::ok(json, format!("{text}{counts}"))
}

/// An address as typed: `0x0400`, `$0400` or decimal digits.
fn address(q: &str) -> Option<u32> {
    let q = q.trim();
    if let Some(h) = q.strip_prefix('$').or_else(|| q.strip_prefix("0x")) {
        u32::from_str_radix(h, 16).ok()
    } else {
        q.parse().ok()
    }
}

fn lookup(file: &LabelsFile, query: &str) -> Result<Outcome> {
    let mut lines = Vec::new();
    let mut found = Vec::new();
    match address(query) {
        Some(a) => {
            for l in &file.label {
                if a >= u32::from(l.addr) && a < u32::from(l.addr) + u32::from(l.size.max(1)) {
                    let off = a - u32::from(l.addr);
                    let at = if off == 0 {
                        String::new()
                    } else {
                        format!("+{off}")
                    };
                    lines.push(format!("ram      {}{at}  {}", l.name, l.description));
                    found.push(json!({ "kind": "ram", "label": l, "offset": off }));
                }
            }
            for (word, rows) in [("routine", &file.routine), ("data", &file.data)] {
                for r in rows {
                    if a >= u32::from(r.addr) && a < u32::from(r.addr) + r.size.max(1) {
                        let off = a - u32::from(r.addr);
                        let at = if off == 0 {
                            String::new()
                        } else {
                            format!("+{off}")
                        };
                        lines.push(format!(
                            "{word:<8} {}{at}  (bank {:02X}, ${:04X})  {}",
                            r.name, r.bank, r.addr, r.description
                        ));
                        found.push(json!({ "kind": word, "label": r, "offset": off }));
                    }
                }
            }
        }
        None => {
            for l in file.label.iter().filter(|l| l.name == query) {
                lines.push(format!(
                    "ram      {} at ${:04X} +{}  {}",
                    l.name, l.addr, l.size, l.description
                ));
                found.push(json!({ "kind": "ram", "label": l }));
            }
            for (word, rows) in [("routine", &file.routine), ("data", &file.data)] {
                for r in rows.iter().filter(|r| r.name == query) {
                    lines.push(format!(
                        "{word:<8} {} at bank {:02X} ${:04X} +{}  {}",
                        r.name, r.bank, r.addr, r.size, r.description
                    ));
                    found.push(json!({ "kind": word, "label": r }));
                }
            }
        }
    }
    let text = if lines.is_empty() {
        format!("{query}: no label in {}", file.game)
    } else {
        lines.join("\n")
    };
    Ok(Outcome {
        json: json!({ "game": file.game, "query": query, "found": found }),
        text,
        exit: u8::from(found.is_empty()),
    })
}

/// Every RAM address a mapping reads (its `ram` lists and `frac` bytes), with
/// the paths that read it, in address order.
type Mapped = Vec<(u32, Vec<String>)>;

fn mapped_addresses(mapping: &str) -> Result<(String, Mapped)> {
    let v: toml::Value = toml::from_str(mapping).context("parsing the mapping")?;
    let source = v
        .get("source")
        .and_then(toml::Value::as_str)
        .context("the mapping has no `source`")?
        .to_owned();
    let mut by_addr: std::collections::BTreeMap<u32, Vec<String>> = Default::default();
    for f in v
        .get("field")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = f.get("path").and_then(toml::Value::as_str).unwrap_or("?");
        let ram = f.get("ram").and_then(toml::Value::as_array);
        let addrs = ram
            .into_iter()
            .flatten()
            .chain(f.get("frac"))
            .filter_map(toml::Value::as_integer);
        for a in addrs {
            let a = u32::try_from(a).context("a negative or huge address in the mapping")?;
            by_addr.entry(a).or_default().push(path.to_owned());
        }
    }
    Ok((source, by_addr.into_iter().collect()))
}

fn check(mapping: &Path, file: Option<&Path>) -> Result<Outcome> {
    let text =
        fs::read_to_string(mapping).with_context(|| format!("reading {}", mapping.display()))?;
    let (source, addrs) = mapped_addresses(&text)?;
    let path = match file {
        Some(f) => f.to_path_buf(),
        None => game_file(&source)?,
    };
    let labels = read(&path)?;
    let missing: Vec<&(u32, Vec<String>)> = addrs
        .iter()
        .filter(|(a, _)| {
            !labels.label.iter().any(|l| {
                *a >= u32::from(l.addr) && *a < u32::from(l.addr) + u32::from(l.size.max(1))
            })
        })
        .collect();
    let mut out = format!(
        "{}: {} mapped addresses, {} without a label in {}",
        mapping.display(),
        addrs.len(),
        missing.len(),
        path.display()
    );
    for (a, ps) in &missing {
        let _ = write!(out, "\n  ${a:04X} read by {}", ps.join(", "));
    }
    Ok(Outcome {
        json: json!({
            "mapping": mapping,
            "labels": path,
            "addresses": addrs.len(),
            "unlabelled": missing.iter().map(|(a, ps)| json!({ "addr": a, "paths": ps })).collect::<Vec<_>>(),
        }),
        text: out,
        exit: u8::from(!missing.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_TEXT: &str = r#"
schema = 0
game = "demo"
bank_size = 16384

[[label]]
name = "HP"
addr = 0x0010
size = 2
description = "hit points"

[[routine]]
name = "reset"
bank = 15
addr = 0xC000
prg = 0x3C000
size = 8

[[data]]
name = "tbl"
bank = 0
addr = 0x8000
prg = 0
size = 4
"#;

    fn file() -> LabelsFile {
        parse(FILE_TEXT).unwrap()
    }

    #[test]
    fn addresses_are_read_in_hex_or_decimal() {
        assert_eq!(address("$0400"), Some(0x400));
        assert_eq!(address("0x10"), Some(16));
        assert_eq!(address("16"), Some(16));
        assert_eq!(address("reset"), None);
    }

    #[test]
    fn lookup_by_address_covers_a_labels_size() {
        let r = lookup(&file(), "0x11").unwrap();
        assert_eq!(r.exit, 0);
        assert!(r.text.contains("HP+1"), "{}", r.text);
        let r = lookup(&file(), "$C004").unwrap();
        assert!(r.text.contains("reset+4"), "{}", r.text);
        assert_eq!(lookup(&file(), "$C008").unwrap().exit, 1);
    }

    #[test]
    fn lookup_by_name_is_exact() {
        assert_eq!(lookup(&file(), "tbl").unwrap().exit, 0);
        assert_eq!(lookup(&file(), "tb").unwrap().exit, 1);
    }

    #[test]
    fn a_mapping_reads_ram_and_fraction_bytes() {
        let m = "source = \"demo\"\n[[field]]\npath = \"a.x\"\nram = [0x10, 0x11]\nfrac = 0x12\n";
        let (source, addrs) = mapped_addresses(m).unwrap();
        assert_eq!(source, "demo");
        assert_eq!(
            addrs.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
            [0x10, 0x11, 0x12]
        );
    }

    #[test]
    fn check_names_the_unlabelled() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let labels = dir.join("labels.toml");
        fs::write(&labels, FILE_TEXT).unwrap();
        let mapping = dir.join("mapping.toml");
        fs::write(
            &mapping,
            "source = \"demo\"\n[[field]]\npath = \"a.x\"\nram = [0x10, 0x11]\nfrac = 0x12\n",
        )
        .unwrap();
        let r = check(&mapping, Some(&labels)).unwrap();
        assert_eq!(r.exit, 1);
        assert!(r.text.contains("$0012 read by a.x"), "{}", r.text);
        assert!(!r.text.contains("$0011"), "{}", r.text);
    }

    #[test]
    fn the_committed_mosshop_labels_parse() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let file = read(&repo.join("games/mosshop/labels.toml")).unwrap();
        assert_eq!(file.game, "mosshop");
        assert!(
            file.label
                .iter()
                .any(|l| l.name == "PLAYER_X" && l.addr == 0x10)
        );
        let found = locate_in("mosshop", Some(&repo), None).unwrap();
        assert!(found.ends_with("games/mosshop/labels.toml"));
    }

    #[test]
    fn unknown_fields_and_schemas_are_refused() {
        assert!(parse("schema = 1\ngame = \"x\"\n").is_err());
        assert!(parse("schema = 0\ngame = \"x\"\nextra = 1\n").is_err());
        let ok = parse("schema = 0\ngame = \"x\"\n[[label]]\nname = \"A\"\naddr = 1\nsize = 1\n")
            .unwrap();
        assert_eq!(ok.label[0].description, "");
    }

    /// A game in the games checkout is found by its folder or by a source
    /// ID its roms.toml lists; one with no file is named in the error.
    #[test]
    fn a_games_file_is_found_by_folder_or_source_id() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("demo-game");
        fs::create_dir_all(&g).unwrap();
        fs::write(
            g.join("roms.toml"),
            "[[source]]\nid = \"demo-usa\"\ntitle = \"Demo\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"00\"\nmapper = 0\nstatus = \"unverified\"\n",
        )
        .unwrap();
        assert_eq!(
            locate_in("demo-usa", None, Some(d.path())).unwrap_err(),
            "no labels file is known for demo-usa"
        );
        fs::write(g.join(FILE), FILE_TEXT).unwrap();
        for name in ["demo-usa", "demo-game"] {
            assert_eq!(locate_in(name, None, Some(d.path())).unwrap(), g.join(FILE));
        }
    }
}
