//! `gp coverage`: what recordings exercised (Milestone 1, step 9;
//! docs/coverage.md). The measuring is in `gp-coverage`; this module finds
//! the recordings, ROMs and definitions, prints the report and checks the
//! thresholds.
//!
//! It replays gp-nes recordings with the 6502 observer. Arcade recordings
//! (port wave 6) are replayed on gp-arcade with its levels instead, and
//! measured as both Z80s' code coverage: of each CPU's code map, the
//! instructions run and the conditional branches that went both ways
//! (crate::code_z80::coverage). gp-coverage's definitions (state regions,
//! mechanics) are the NES's and aren't read for them.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use gp_coverage::definition::Definition;
use gp_coverage::report::{self, Env, Outcome as Measured, Report};
use gp_evidence::manifest::SourceRef;
use gp_evidence::recordings::{self, IndexEntry};
use gp_evidence::source::identify_nes;
use gp_evidence::{Paths, Store};

use crate::Outcome;
use crate::open::RomFolder;
use crate::open::{load_registry, now_rfc3339};

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct CoverageArgs {
    #[command(subcommand)]
    pub cmd: Option<CoverageCmd>,
    #[command(flatten)]
    pub select: Select,
}

#[derive(Subcommand)]
pub enum CoverageCmd {
    /// Check coverage against the thresholds in each game's definition
    /// (every level visited, every object type seen, the main input
    /// combinations). Exits 1 if a threshold isn't reached, 2 if no
    /// definition has thresholds.
    Check(Select),
}

#[derive(Args)]
pub struct Select {
    /// Recordings, by ID or a unique prefix of at least 4 characters. Their
    /// coverage is combined per ROM.
    #[arg(required_unless_present = "store")]
    pub ids: Vec<String>,
    /// Every recording in the store, combined per ROM: the corpus.
    #[arg(long, conflicts_with = "ids")]
    pub store: bool,
    /// With `--store`, include retired recordings.
    #[arg(long, requires = "store")]
    pub retired: bool,
    /// The coverage definition to use, instead of coverage/<source-id>.toml.
    #[arg(long)]
    pub definition: Option<PathBuf>,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash. With
    /// `--store`, only recordings of this ROM are covered.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// Don't store newly measured coverage (leaves the store untouched).
    #[arg(long)]
    pub no_save: bool,
    /// Threads per recording (default: the machine's cores).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub threads: Option<u32>,
}

pub(crate) fn run(args: &CoverageArgs, paths: &Paths) -> Result<Outcome> {
    match &args.cmd {
        Some(CoverageCmd::Check(select)) => covered(select, true, paths),
        None => covered(&args.select, false, paths),
    }
}

/// One recording's coverage for gp serve's `/api/recordings/{id}/coverage`
/// (wave 8): `gp coverage ID --no-save`'s output, so the store is only read.
pub(crate) fn served(id: &str, rom: Option<&Path>, paths: &Paths) -> Result<Outcome> {
    let select = Select {
        ids: vec![id.to_owned()],
        store: false,
        retired: false,
        definition: None,
        rom: rom.map(Path::to_path_buf),
        no_save: true,
        threads: None,
    };
    covered(&select, false, paths)
}

/// What a report needs from the command line: the ROM folder, the
/// registry and the definitions.
struct CliEnv<'a> {
    roms: RomFolder<'a>,
    rom_file: Option<(String, Vec<u8>)>,
    registry: gp_evidence::registry::Registry,
    definition: Option<Definition>,
    dir: Option<PathBuf>,
}

impl Env for CliEnv<'_> {
    fn rom(&mut self, sha256: &str) -> Result<Option<Vec<u8>>, String> {
        if let Some((sha, bytes)) = &self.rom_file
            && sha == sha256
        {
            return Ok(Some(bytes.clone()));
        }
        self.roms.read(sha256).map_err(|e| format!("{e:#}"))
    }

    fn source(&mut self, sha256: &str) -> Option<(String, String)> {
        self.registry
            .by_sha256(sha256)
            .map(|s| (s.id.clone(), s.title.clone()))
    }

    fn definition(
        &mut self,
        sha256: &str,
        source_id: Option<&str>,
    ) -> Result<Option<Definition>, String> {
        if let Some(def) = &self.definition {
            return Ok(Some(def.clone()));
        }
        // A registered source by its ID; a game built from source, like
        // Moss Hop, by the `source` of a committed baseline that pins the
        // same ROM hash, as the inspector finds its labels.
        let id = source_id
            .map(str::to_owned)
            .or_else(|| game_of(repo_root().as_deref(), sha256));
        match (&self.dir, id) {
            (Some(dir), Some(id)) => Definition::find(dir, &id).map_err(|e| e.to_string()),
            _ => Ok(None),
        }
    }
}

/// The directory definitions are found in: $GP_COVERAGE, `coverage/` at
/// or above the working directory, or the gameprinter checkout's.
fn definitions_dir() -> Result<Option<PathBuf>> {
    if let Some(dir) = std::env::var_os("GP_COVERAGE").filter(|v| !v.is_empty()) {
        return Ok(Some(PathBuf::from(dir)));
    }
    Ok(Definition::find_dir(&std::env::current_dir()?).or_else(|| {
        repo_root()
            .map(|r| r.join("coverage"))
            .filter(|d| d.is_dir())
    }))
}

/// The gameprinter checkout: the working folder or one above it holding
/// `baselines/` and `coverage/`, else the checkout gp was built from. The
/// old tool found it beside its sources/ registry, which gameprinter keeps
/// with each game instead.
fn repo_root() -> Option<PathBuf> {
    let is_repo = |d: &Path| d.join("baselines").is_dir() && d.join("coverage").is_dir();
    if let Ok(here) = std::env::current_dir()
        && let Some(d) = here.ancestors().find(|d| is_repo(d))
    {
        return Some(d.to_path_buf());
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    is_repo(&built).then_some(built)
}

/// The game a ROM hash belongs to: its ROM entry's ID in a game's
/// roms.toml, else, for a game built from source like Moss Hop, the
/// `source` of a committed baseline that pins the same hash.
fn game_of(repo: Option<&Path>, sha256: &str) -> Option<String> {
    if let Ok(registry) = load_registry()
        && let Some(known) = registry.by_sha256(sha256)
    {
        return Some(known.id.clone());
    }
    let baselines = repo?.join("baselines");
    for dir in fs::read_dir(baselines).ok()?.flatten() {
        for file in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().is_some_and(|e| e == "toml")
                && let Ok(b) = gp_evidence::baseline::Baseline::load(&path)
                && b.sha256 == sha256
                && let Some(source) = b.source
            {
                return Some(source);
            }
        }
    }
    None
}

fn covered(select: &Select, check: bool, paths: &Paths) -> Result<Outcome> {
    let save = !select.no_save;
    // Reading never creates or changes anything in the store, and neither
    // does --no-save, so a store can be reported on read-only.
    let store = if save {
        Store::open(&paths.store)?
    } else {
        Store::open_existing(&paths.store)?
    };
    // The recordings by platform: an arcade one is measured on gp-arcade
    // (`arcade`), an NES one by gp-coverage's report; a selection of both
    // gets both reports.
    let rom_bytes = match &select.rom {
        Some(path) => Some(fs::read(path).with_context(|| format!("reading {}", path.display()))?),
        None => None,
    };
    let rom_arcade = rom_bytes
        .as_deref()
        .is_some_and(|b| crate::roms::Platform::of_file(b) == crate::roms::Platform::Arcade);
    let (arcade_entries, mut entries) = split_platforms(&store, entries(&store, select)?)?;
    let arcade_out = if rom_arcade || !arcade_entries.is_empty() {
        let rom = rom_bytes.as_deref().filter(|_| rom_arcade);
        arcade(select, rom, &arcade_entries, &entries, paths)?
    } else {
        None
    };
    if rom_arcade || entries.is_empty() {
        return arcade_out.context("no recording was chosen");
    }
    let rom_file = match rom_bytes {
        Some(bytes) => {
            let sha = identify_nes(&bytes)?.sha256;
            Some((sha, bytes))
        }
        None => None,
    };
    if let (true, Some((sha, _))) = (select.store, &rom_file) {
        entries.retain(|e| {
            recordings::load_manifest(&store, &e.id)
                .map(|m| matches!(&m.source, SourceRef::NesRom { sha256, .. } if sha256 == sha))
                .unwrap_or(true)
        });
    }
    let definition = select
        .definition
        .as_deref()
        .map(Definition::load)
        .transpose()?;
    let mut env = CliEnv {
        roms: RomFolder::new(paths),
        rom_file,
        registry: load_registry()?,
        definition,
        dir: definitions_dir()?,
    };
    let threads = select.threads.map_or_else(
        || std::thread::available_parallelism().map_or(1, |n| n.get()),
        |t| t as usize,
    );
    let report = report::build(
        &store,
        &entries,
        &mut env,
        &report::Options {
            save,
            threads,
            now: now_rfc3339(),
        },
    )
    .map_err(anyhow::Error::msg)?;

    let count = |o: Measured| report.recordings.iter().filter(|r| r.result == o).count();
    let (diverged, failed) = (count(Measured::Diverged), count(Measured::Error));

    let mut text = render(&report, &paths.roms);
    let checked: Vec<_> = report
        .groups
        .iter()
        .filter_map(|g| g.check.as_ref())
        .collect();
    let mut exit = if failed > 0 || report.groups.is_empty() {
        2
    } else {
        u8::from(diverged > 0)
    };
    if report.groups.is_empty() {
        text.push_str("\nNothing was measured, so there's no coverage to report.");
    }
    if check && exit != 2 {
        if checked.is_empty() {
            text.push_str("\nNo definition for these ROMs has thresholds, so nothing was checked.");
            exit = 2;
        } else if checked.iter().any(|c| !c.passed) {
            exit = 1;
        }
    }
    let mut json = serde_json::to_value(&report).expect("reports serialize");
    if let Some(a) = arcade_out {
        json["arcade"] = a.json["arcade"].clone();
        text = format!("{}\n{text}", a.text);
    }
    Ok(Outcome { json, text, exit })
}

/// The chosen recordings split by platform: the arcade ones with their
/// manifests, and the NES ones.
type ArcadeEntries = Vec<(IndexEntry, gp_evidence::manifest::RecordingManifest)>;

fn split_platforms(
    store: &Store,
    entries: Vec<IndexEntry>,
) -> Result<(ArcadeEntries, Vec<IndexEntry>)> {
    let mut arcade = Vec::new();
    let mut nes = Vec::new();
    for e in entries {
        let m = recordings::load_manifest(store, &e.id)?;
        if matches!(m.source, SourceRef::Romset { .. }) {
            arcade.push((e, m));
        } else {
            nes.push(e);
        }
    }
    Ok((arcade, nes))
}

/// The recordings to cover, each ID once.
/// Coverage of the arcade recordings chosen: each replayed on gp-arcade
/// with its levels. With `--rom` an arcade romset, only recordings of that
/// set are measured (`--store`), and an NES recording named is refused as
/// not that set's. `None` when there is nothing arcade to measure.
fn arcade(
    select: &Select,
    rom: Option<&[u8]>,
    chosen: &ArcadeEntries,
    nes: &[IndexEntry],
    paths: &Paths,
) -> Result<Option<Outcome>> {
    let mut arcade: Vec<&(IndexEntry, _)> = chosen.iter().collect();
    if let Some(bytes) = rom {
        let set = gp_arcade::Romset::from_zip(bytes)?;
        let identity = set.identity.clone();
        if !select.store
            && let Some(e) = nes.first()
        {
            anyhow::bail!(
                "recording {} plays an NES ROM, and --rom is the arcade romset {}",
                e.name,
                set.name
            );
        }
        arcade.retain(
            |(_, m)| matches!(&m.source, SourceRef::Romset { identity: i, .. } if *i == identity),
        );
        if arcade.is_empty() {
            anyhow::bail!("no recording chosen plays the romset {}", set.name);
        }
    }
    if arcade.is_empty() {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for (e, _) in arcade {
        let opened = crate::open::Opened::open(paths, &e.id.to_hex())?;
        let set = opened.romset(select.rom.as_deref(), paths)?;
        let input = String::from_utf8(opened.store.get(&opened.manifest.input_log)?)?;
        let log = gp_evidence::InputLog::parse(&input)?;
        let levels = crate::code_z80::run_input(&set, &log.frames)?;
        let mut v = crate::code_z80::coverage(&set, &levels);
        for c in v["cpus"].as_array().into_iter().flatten() {
            lines.push(format!(
                "{} ({})  {:<5} {} of {} instructions run, branches both ways {} and one way {} of {}",
                e.name,
                &e.id.to_hex()[..12],
                c["cpu"].as_str().unwrap_or("?"),
                c["instructions_run"],
                c["instructions"],
                c["branches_both_ways"],
                c["branches_one_way"],
                c["branches"],
            ));
        }
        v["id"] = serde_json::json!(e.id);
        v["name"] = serde_json::json!(e.name);
        rows.push(v);
    }
    Ok(Some(Outcome::ok(
        serde_json::json!({ "arcade": rows }),
        lines.join("\n"),
    )))
}

fn entries(store: &Store, select: &Select) -> Result<Vec<IndexEntry>> {
    if select.store {
        let listing = recordings::list_checked(store)?;
        for (line, message) in &listing.bad_lines {
            eprintln!("gp: skipping recordings index line {line}, which can't be read: {message}");
        }
        let retired = recordings::retired(&listing.marks);
        let mut sealed = 0usize;
        let out: Vec<IndexEntry> = recordings::unique(listing.entries)
            .into_iter()
            .filter(|e| select.retired || !retired.contains(&e.id))
            .filter(|e| match recordings::load_manifest(store, &e.id) {
                Err(gp_evidence::recordings::RecordingsError::Refused(_)) => {
                    sealed += 1;
                    false
                }
                _ => true,
            })
            .collect();
        if sealed > 0 {
            eprintln!("gp: left out {sealed} recording(s) that play a sealed run");
        }
        return Ok(out);
    }
    let mut out: Vec<IndexEntry> = Vec::new();
    for id in &select.ids {
        // The log, not the SQLite index, so a report never writes the store.
        let e = recordings::resolve(store, id)?;
        if !out.iter().any(|o| o.id == e.id) {
            out.push(e);
        }
    }
    Ok(out)
}

fn render(report: &Report, roms: &Path) -> String {
    let mut out = String::new();
    for g in &report.groups {
        let s = &g.summary;
        let name = match (&g.title, &g.source) {
            (Some(t), Some(id)) => format!("{t} ({id})"),
            (Some(t), None) => t.clone(),
            (None, Some(id)) => id.clone(),
            (None, None) => format!("ROM {}...", &g.sha256[..16]),
        };
        let _ = writeln!(
            out,
            "{name}: {} recordings, {} frames",
            s.recordings, s.frames
        );
        let banks: Vec<String> = s
            .banks
            .iter()
            .map(|b| format!("{}: {}", b.bank, b.executed))
            .collect();
        let _ = writeln!(
            out,
            "  executed  {} of {} PRG ROM bytes ({}%), {} instructions; by {}KB bank: {}",
            s.executed_bytes,
            s.prg_rom_bytes,
            s.executed_percent,
            s.instruction_starts,
            s.bank_size / 1024,
            banks.join(", ")
        );
        let prg_ram = if s.prg_ram_bytes > 0 {
            format!(
                "; PRG RAM {} of {}",
                s.prg_ram_written_bytes, s.prg_ram_bytes
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "  RAM       {} of {} bytes written{prg_ram}",
            s.ram_written_bytes, s.ram_bytes
        );
        if g.coverage.definition.is_some() {
            let levels: Vec<&str> = g.coverage.levels.keys().map(String::as_str).collect();
            let _ = writeln!(
                out,
                "  screens   {} visited, in {} levels: {}",
                s.screens,
                s.levels,
                levels.join(" ")
            );
            let objects: Vec<&str> = g.coverage.objects.keys().map(String::as_str).collect();
            let _ = writeln!(
                out,
                "  objects   {} seen: {}",
                s.objects,
                objects.join(", ")
            );
        } else {
            let _ = writeln!(
                out,
                "  screens   no coverage definition for this ROM (coverage/<source-id>.toml or --definition)"
            );
        }
        let mut inputs: Vec<(&String, &u64)> = g.coverage.inputs[0].iter().collect();
        inputs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let inputs: Vec<String> = inputs.iter().map(|(c, n)| format!("{c} {n}")).collect();
        let _ = writeln!(
            out,
            "  inputs    {} combinations on port 1, by frames: {}",
            s.input_combinations,
            inputs.join(", ")
        );
        if let Some(c) = &g.check {
            if c.passed {
                let parts: Vec<String> = [
                    ("screens", c.screens.required),
                    ("levels", c.levels.required),
                    ("objects", c.objects.required),
                    ("input combinations", c.inputs.required),
                ]
                .iter()
                .filter(|(_, n)| *n > 0)
                .map(|(what, n)| format!("{n} {what}"))
                .collect();
                let _ = writeln!(out, "  check     passed: {}", parts.join(", "));
            } else {
                let _ = writeln!(out, "  check     FAILED");
                for (what, r) in [
                    ("screens", &c.screens),
                    ("levels", &c.levels),
                    ("objects", &c.objects),
                    ("inputs", &c.inputs),
                ] {
                    if !r.missing.is_empty() {
                        let _ = writeln!(
                            out,
                            "            {} of {} {what} missing: {}",
                            r.missing.len(),
                            r.required,
                            r.missing.join(", ")
                        );
                    }
                }
            }
        }
    }
    for r in &report.recordings {
        let short = &r.id.to_hex()[..12];
        let word = match r.result {
            Measured::Measured => "measured",
            Measured::Stored => "stored  ",
            Measured::Skipped => "skip    ",
            Measured::Diverged => "DIVERGED",
            Measured::Error => "ERROR   ",
        };
        let detail = match (r.result, &r.message, r.ms) {
            (Measured::Skipped, Some(m), _) if m == "no ROM with its hash" => {
                format!("  (no ROM with its hash in {})", roms.display())
            }
            (_, Some(m), _) => format!("  ({m})"),
            (_, None, Some(ms)) => format!("  ({} frames in {:.1}s)", r.frames, ms as f64 / 1000.0),
            _ => String::new(),
        };
        let _ = writeln!(out, "{word} {short}  {}{detail}", r.name);
    }
    out.trim_end().to_owned()
}
