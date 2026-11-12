//! `gp package scan`: what of a package folder can ship in it, and whether
//! the rest is rebuilt from the person's own ROM.
//!
//! Every file, every field of its TOML and every function and const of its
//! hook code is put in one of four buckets:
//!
//! | Bucket | What |
//! |---|---|
//! | ours | facts, rules, behaviour, our hook code, original art, clip and probe recipes, docs |
//! | reference | code and RAM addresses, routine names, mapping rows: they point at the ROM and copy nothing |
//! | from the ROM | content that is the ROM's, present as a recipe (an asset map entry) checked by a hash |
//! | needs rewrite | ROM content typed out without a recipe, or hook code that transcribes a routine |
//!
//! The detectors are in `rom_match` (ROM content in text) and `transcribe`
//! (hook code against the routine it cites). A ROM is read locally to
//! compare and nothing of it is printed: a match is reported by where it is
//! and its length. With no ROM the report says plainly that nothing was
//! compared, and the only items it can find are ones that need no ROM.

mod fields;
mod rom_match;
mod transcribe;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use gp_assets::AssetMap;
use gp_assets::authored::Authored;
use gp_evidence::Paths;

use super::{OpenPart, Package, RomEntry};
use crate::Outcome;
use crate::roms::{self, Rom};
use crate::template::{self, lifted_digest, sha256_hex};

use rom_match::{Match, Num, RomIndex};

pub fn run(dir: &Path, given: &[PathBuf], wasm: &[PathBuf], paths: &Paths) -> Result<Outcome> {
    let pkg = Package::load(dir)?;
    let found = roms::find(&pkg.rom_hashes(), given, &paths.roms)?;
    let mut outcome = scan(&pkg, &found)?.outcome();
    let wasm_report = scan_wasm(dir, wasm, &found)?;
    outcome.text.push_str(&wasm_report.0);
    outcome.json["wasm"] = wasm_report.1;
    if wasm_report.2 {
        outcome.exit = outcome.exit.max(1);
    }
    Ok(outcome)
}

/// Every data segment and custom section of each `.wasm` of the folder, and
/// of `extra`, against the ROMs: the text report, the JSON and whether any
/// ROM bytes were found.
fn scan_wasm(
    dir: &Path,
    extra: &[PathBuf],
    found: &BTreeMap<String, Rom>,
) -> Result<(String, serde_json::Value, bool)> {
    let mut files: Vec<PathBuf> = extra.to_vec();
    // Any other file that isn't text is compared whole: no field of it can
    // be read, so a slice of a ROM in one would pass unseen.
    let mut binary: Vec<PathBuf> = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                if e.file_name() != "target" && e.file_name() != ".git" {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "wasm") {
                files.push(p);
            } else if fs::read_to_string(&p).is_err() {
                binary.push(p);
            }
        }
    }
    let mut text = String::new();
    let mut finds = Vec::new();
    let mut targets: Vec<(&PathBuf, bool)> = files.iter().map(|f| (f, true)).collect();
    targets.extend(binary.iter().map(|f| (f, false)));
    for (f, wasm) in targets {
        let bytes = fs::read(f).with_context(|| f.display().to_string())?;
        let parts = if wasm {
            crate::module::data_parts(&bytes).with_context(|| f.display().to_string())?
        } else {
            vec![("the file".to_owned(), &bytes[..])]
        };
        let total: usize = parts.iter().map(|p| p.1.len()).sum();
        for (id, rom) in found {
            for (name, data) in &parts {
                for (rom_offset, len) in crate::served::runs_of_rom(data, rom.data()) {
                    let line = format!(
                        "{}: {name} holds {len} bytes of ROM {} at ROM offset {rom_offset:#x}",
                        f.display(),
                        &id[..12]
                    );
                    finds.push(json!({ "file": f, "part": name, "rom": id, "rom_offset": rom_offset, "len": len }));
                    text.push_str(&format!("\nNEEDS REWRITE {line}"));
                }
            }
        }
        text.push_str(&format!(
            "\nwasm {}: {} data parts, {total} bytes, compared with {} ROM(s)",
            f.display(),
            parts.len(),
            found.len()
        ));
    }
    if !files.is_empty() && found.is_empty() {
        text.push_str(
            "\nNOT COMPARED: no ROM here, so nothing says these modules are free of ROM data",
        );
    }
    let bad = !finds.is_empty();
    Ok((text, json!({ "files": files, "finds": finds }), bad))
}

// ---- Buckets ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    Ours,
    Reference,
    FromRom,
    Rewrite,
}

impl Bucket {
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Ours => "ours",
            Bucket::Reference => "reference",
            Bucket::FromRom => "from the ROM",
            Bucket::Rewrite => "needs rewrite",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Bucket::Ours => "ours",
            Bucket::Reference => "reference",
            Bucket::FromRom => "from_rom",
            Bucket::Rewrite => "rewrite",
        }
    }
    const ALL: [Bucket; 4] = [
        Bucket::Ours,
        Bucket::Reference,
        Bucket::FromRom,
        Bucket::Rewrite,
    ];
}

/// What a file is, which decides its fields' buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Manifest,
    Model,
    Mapping,
    /// Other TOML: probes, clips, corpus, settings, a derived delta.
    Settings,
    /// An original's authored data: ours, never compared with its own image.
    Authored,
    Hook,
    /// Counted as one unit: clip presses, scores, themes, docs.
    Whole(&'static str),
    Recipe,
}

/// The last keys of fields that point into the ROM's code or RAM.
const REFERENCE_KEYS: &[&str] = &[
    "routine",
    "routines",
    "addr",
    "address",
    "addresses",
    "ram",
    "pc",
    "reads",
    "writes",
    "rom_offset",
    "entry_pc",
    "code",
];

/// A string that holds only addresses, ranges and routine names: `$C890`,
/// `$FA48-$FE58`, `0x0203`, `$C0E3, $C10B`.
fn only_addresses(s: &str) -> bool {
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    let mut any = false;
    for w in s.split(|c: char| c.is_whitespace() || matches!(c, ',' | '-' | '+' | '(' | ')')) {
        if w.is_empty() || w == "and" || w == "to" {
            continue;
        }
        let h = w
            .strip_prefix('$')
            .or_else(|| w.strip_prefix("0x"))
            .unwrap_or("");
        if h.is_empty() || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        any = true;
    }
    any
}

/// The parts of a package scanned on their own: the ROM game at the
/// package's root, and the open version in its folder.
struct Part<'a> {
    name: String,
    tier: String,
    dir: PathBuf,
    /// A folder under `dir` that is another part.
    skip: Option<String>,
    model: String,
    /// Asset maps, by path relative to `dir`, parsed, with the ROM entry
    /// each is for (None for an authored map).
    recipes: BTreeMap<String, (AssetMap, String, Option<&'a RomEntry>)>,
    /// The map the model's asset names are checked against.
    primary: Option<String>,
    authored: Option<&'a OpenPart>,
}

fn role_of(rel: &str, part: &Part) -> Role {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if part.recipes.contains_key(rel) {
        return Role::Recipe;
    }
    if matches!(rel, "template.toml" | "package.toml" | "roms.toml") {
        return Role::Manifest;
    }
    if rel == part.model || rel == "delta.toml" {
        return Role::Model;
    }
    if name == "mapping.toml" && !rel.contains('/') {
        return Role::Mapping;
    }
    if rel == "authored.toml" {
        return Role::Authored;
    }
    if rel.ends_with(".rs") {
        return Role::Hook;
    }
    if rel.starts_with("themes/") {
        return Role::Whole("theme");
    }
    if rel.starts_with("evidence/") || rel.starts_with("baselines/") {
        return Role::Whole("score");
    }
    if rel.starts_with("clips/") || rel.starts_with("inputs/") {
        return Role::Whole("clip recipe");
    }
    if rel.ends_with(".toml") {
        return Role::Settings;
    }
    if rel.ends_with(".md") || rel.ends_with(".txt") {
        return Role::Whole("doc");
    }
    Role::Whole("file")
}

// ---- The report ----

/// One needs-rewrite item.
#[derive(Debug, Clone)]
pub struct Rewrite {
    pub file: String,
    pub line: usize,
    /// The field path, function or item it is in.
    pub at: String,
    /// What it matched, in words, without ROM bytes.
    pub what: String,
    pub fix: String,
    /// Stable across edits that only move it.
    pub key: String,
    /// The bytes of the file it covers, when known.
    pub span: Option<(usize, usize)>,
}

/// One value taken from the ROM through the recipe.
#[derive(Debug, Clone)]
struct Recipe {
    asset: String,
    format: String,
    bytes: u64,
}

#[derive(Debug, Default)]
struct FileCounts {
    unit: &'static str,
    counts: [usize; 4],
}

/// One part's result.
struct PartReport {
    name: String,
    tier: String,
    compared: Vec<String>,
    files: BTreeMap<String, FileCounts>,
    rewrite: Vec<Rewrite>,
    /// Matches a person reviewed and kept, each with the reason they gave.
    reviewed: Vec<(Rewrite, String)>,
    /// Reviewed keys that matched nothing here.
    stale: Vec<String>,
    recipes: Vec<Recipe>,
    rederive: Vec<(String, Rederive)>,
    settings: Value,
}

/// The file beside a part's model that lists the matches a person reviewed
/// and kept. Only an exact match's key is excused; anything new still fails.
const REVIEWED_FILE: &str = "scan-reviewed.toml";

#[derive(serde::Deserialize)]
struct ReviewedFile {
    reviewed: Vec<ReviewedItem>,
}

#[derive(serde::Deserialize)]
struct ReviewedItem {
    key: String,
    why: String,
}

fn reviewed_items(dir: &Path) -> Result<Vec<ReviewedItem>> {
    let path = dir.join(REVIEWED_FILE);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let items = toml::from_str::<ReviewedFile>(&fs::read_to_string(&path)?)
        .with_context(|| path.display().to_string())?
        .reviewed;
    if let Some(i) = items.iter().find(|i| i.why.trim().is_empty()) {
        anyhow::bail!("{}: {} has no reason", path.display(), i.key);
    }
    Ok(items)
}

/// A package's result.
pub struct Report {
    name: String,
    version: String,
    tier: String,
    /// Each entry's id and whether its ROM was there to compare with.
    entries: Vec<(String, bool)>,
    parts: Vec<PartReport>,
}

impl Report {
    fn compared(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.1)
            .map(|e| e.0.as_str())
            .collect()
    }

    fn not_compared(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| !e.1)
            .map(|e| e.0.as_str())
            .collect()
    }

    fn rewrite_count(&self) -> usize {
        self.parts.iter().map(|p| p.rewrite.len()).sum()
    }

    fn rederive_problems(&self) -> Vec<String> {
        self.parts
            .iter()
            .flat_map(|p| {
                p.rederive
                    .iter()
                    .flat_map(|(id, d)| d.problems().into_iter().map(move |x| format!("{id}: {x}")))
            })
            .collect()
    }

    fn outcome(&self) -> Outcome {
        let mut text = format!("package scan {} {}", self.name, self.version);
        if !self.tier.is_empty() {
            let _ = write!(text, " ({})", self.tier);
        }
        let not = self.not_compared();
        if self.entries.is_empty() {
            text.push_str("\nROM comparison: this package names no ROM");
        } else if not.is_empty() {
            let _ = write!(
                text,
                "\nROM comparison: done against {}",
                self.compared().join(", ")
            );
        } else if not.len() == self.entries.len() {
            let _ = write!(
                text,
                "\nROM comparison: NOT RUN. No ROM with the hash of {} is under $GP_ROM_DIR, so nothing was compared with a ROM and this scan says nothing about ROM content in the package",
                not.join(", ")
            );
        } else {
            let _ = write!(
                text,
                "\nROM comparison: done against {}; NOT RUN against {}, whose ROM is not under $GP_ROM_DIR",
                self.compared().join(", "),
                not.join(", ")
            );
        }
        let mut parts_json = Vec::new();
        for p in &self.parts {
            part_text(p, &mut text);
            parts_json.push(part_json(p));
        }
        let problems = self.rederive_problems();
        let rewrite = self.rewrite_count();
        let mut why = Vec::new();
        if rewrite > 0 {
            why.push(format!("{rewrite} to rewrite"));
        }
        why.extend(problems.iter().cloned());
        // A reviewed match can only be seen missing when every ROM was read.
        if not.is_empty() {
            for k in self.parts.iter().flat_map(|p| &p.stale) {
                why.push(format!(
                    "{REVIEWED_FILE} lists {k}, which no longer matches"
                ));
            }
        }
        let failed = !why.is_empty();
        let complete = !failed && not.is_empty();
        let verdict = if failed {
            format!("no ({})", why.join("; "))
        } else if complete {
            "yes".to_owned()
        } else {
            "not known: not every ROM was compared".to_owned()
        };
        let _ = write!(text, "\ncomplete: {verdict}");
        Outcome {
            json: json!({
                "name": self.name,
                "version": self.version,
                "tier": self.tier,
                "rom_compared": !self.compared().is_empty(),
                "compared": self.compared(),
                "not_compared": not,
                "parts": parts_json,
                "rewrite": rewrite,
                "complete": complete,
            }),
            text,
            exit: u8::from(failed),
        }
    }
}

fn part_text(p: &PartReport, text: &mut String) {
    let _ = write!(
        text,
        "\npart {}{}, compared with {}",
        p.name,
        if p.tier.is_empty() {
            String::new()
        } else {
            format!(" ({})", p.tier)
        },
        if p.compared.is_empty() {
            "no ROM".to_owned()
        } else {
            p.compared.join(", ")
        }
    );
    let counts = counts(p);
    for b in Bucket::ALL {
        let mut by_unit: BTreeMap<&str, usize> = BTreeMap::new();
        for f in p.files.values() {
            *by_unit.entry(f.unit).or_default() += f.counts[b as usize];
        }
        let parts: Vec<String> = by_unit
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(u, n)| format!("{n} {u}"))
            .collect();
        let _ = write!(
            text,
            "\n  {:<14}{:>6}  {}",
            b.name(),
            counts[b as usize],
            parts.join(", ")
        );
    }
    if !p.rewrite.is_empty() {
        let _ = write!(text, "\nneeds rewrite ({}):", p.rewrite.len());
        for r in &p.rewrite {
            let _ = write!(
                text,
                "\n  {}:{} {}: {}\n      fix: {}",
                r.file, r.line, r.at, r.what, r.fix
            );
        }
    }
    if !p.reviewed.is_empty() {
        let _ = write!(text, "\nreviewed and kept ({}):", p.reviewed.len());
        for (r, why) in &p.reviewed {
            let _ = write!(
                text,
                "\n  {}:{} {}\n      why: {}",
                r.file, r.line, r.what, why
            );
        }
    }
    for (id, d) in &p.rederive {
        let _ = write!(text, "\n{}", d.text(id));
    }
}

fn counts(p: &PartReport) -> [usize; 4] {
    let mut c = [0; 4];
    for f in p.files.values() {
        for (k, n) in f.counts.iter().enumerate() {
            c[k] += n;
        }
    }
    c
}

fn part_json(p: &PartReport) -> Value {
    let c = counts(p);
    json!({
        "name": p.name,
        "tier": p.tier,
        "compared": p.compared,
        "counts": Bucket::ALL.iter().map(|b| (b.key().to_owned(), json!(c[*b as usize]))).collect::<serde_json::Map<String, Value>>(),
        "files": p.files.iter().map(|(f, c)| json!({
            "file": f,
            "unit": c.unit,
            "ours": c.counts[0], "reference": c.counts[1], "from_rom": c.counts[2], "rewrite": c.counts[3],
        })).collect::<Vec<_>>(),
        "rewrite": p.rewrite.iter().map(|r| json!({
            "file": r.file, "line": r.line, "at": r.at, "what": r.what, "fix": r.fix, "key": r.key,
        })).collect::<Vec<_>>(),
        "recipes": p.recipes.iter().map(|r| json!({
            "asset": r.asset, "format": r.format, "bytes": r.bytes,
        })).collect::<Vec<_>>(),
        "rederive": p.rederive.iter().map(|(id, d)| json!({ "id": id, "result": d.json() })).collect::<Vec<_>>(),
        "settings": p.settings,
    })
}

// ---- The rederive check ----

pub struct Rederive {
    map_pin_ok: bool,
    digest: String,
    pin: Option<String>,
    roundtrip_differences: usize,
    bytes_covered: u64,
    files: BTreeMap<String, String>,
    /// Assets the model names that the lift doesn't make.
    missing: Vec<String>,
}

impl Rederive {
    pub fn ok(&self) -> bool {
        self.problems().is_empty()
    }

    fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if !self.map_pin_ok {
            p.push("the asset map's hash isn't the one pinned".to_owned());
        }
        match &self.pin {
            None => p.push(format!(
                "no lifted_sha256 to check the rebuild against (the rebuild's is {})",
                self.digest
            )),
            Some(pin) if *pin != self.digest => p.push(format!(
                "the rebuild's digest {} isn't the pinned {pin}",
                self.digest
            )),
            _ => {}
        }
        if self.roundtrip_differences > 0 {
            p.push(format!(
                "{} bytes differ when the rebuild is encoded back",
                self.roundtrip_differences
            ));
        }
        if !self.missing.is_empty() {
            p.push(format!(
                "the model names assets the recipe doesn't make: {}",
                self.missing.join(", ")
            ));
        }
        p
    }

    fn text(&self, id: &str) -> String {
        let p = self.problems();
        format!(
            "rederive {id}: {} lifted files, {} ROM bytes, digest {} ({}), round trip {} differences: {}",
            self.files.len(),
            self.bytes_covered,
            self.digest,
            match &self.pin {
                None => "no pin".to_owned(),
                Some(x) if *x == self.digest => "matches the pin".to_owned(),
                Some(x) => format!("pin {x}"),
            },
            self.roundtrip_differences,
            if p.is_empty() {
                "ok".to_owned()
            } else {
                p.join("; ")
            }
        )
    }

    fn json(&self) -> Value {
        json!({
            "ok": self.ok(),
            "asset_map_pin_ok": self.map_pin_ok,
            "lifted_sha256": self.digest,
            "pin": self.pin,
            "roundtrip_differences": self.roundtrip_differences,
            "bytes_covered": self.bytes_covered,
            "files": self.files,
            "missing": self.missing,
            "problems": self.problems(),
        })
    }
}

/// The assets a model names: `assets.<slot>.from.asset` and the asset part
/// of each `ext_tables` file.
fn named_assets(model_text: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for f in fields::lex(model_text).fields {
        let v = model_text[f.start..f.end].trim_matches('"').to_owned();
        if f.path.starts_with("assets.") && f.path.ends_with(".from.asset") {
            out.insert((f.path.clone(), v));
        } else if f.path.contains(".ext_tables.") && f.value == fields::Value::Str {
            let asset = v.split(['.', '/']).next().unwrap_or("").to_owned();
            out.insert((f.path.clone(), asset));
        }
    }
    out
}

/// Rebuilds a part's ROM-derived files from an image with an asset map and
/// checks every hash. `model` is the model's text when the map must make
/// every asset the model names.
fn rederive(
    image: &[u8],
    map_text: &str,
    map_pin: Option<&str>,
    lifted_pin: Option<String>,
    model: Option<&str>,
) -> Result<Rederive> {
    let map_pin_ok = map_pin.is_some_and(|p| p == sha256_hex(map_text.as_bytes()));
    let map = AssetMap::parse(map_text).map_err(anyhow::Error::msg)?;
    let (lifted, _) =
        gp_assets::lift(image, &map, map_text).map_err(|e| anyhow::anyhow!("lifting: {e}"))?;
    let files: BTreeMap<String, Vec<u8>> = lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
    let digest = lifted_digest(&files);
    let encoded = gp_assets::encode(&map, &|name| {
        files
            .get(name)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name.to_owned()))
    })
    .map_err(|e| anyhow::anyhow!("encoding the rebuild: {e}"))?;
    let diffs = gp_assets::differences(image, &map, &encoded)
        .map_err(|e| anyhow::anyhow!("comparing the rebuild: {e}"))?;
    let names: BTreeSet<&str> = map.assets.iter().map(|a| a.name()).collect();
    let missing = model
        .map(|m| {
            named_assets(m)
                .into_iter()
                .filter(|(_, a)| !names.contains(a.as_str()))
                .map(|(p, a)| format!("{a} ({p})"))
                .collect()
        })
        .unwrap_or_default();
    Ok(Rederive {
        map_pin_ok,
        digest,
        pin: lifted_pin,
        roundtrip_differences: diffs.len(),
        bytes_covered: map.bytes_covered(),
        files: files
            .iter()
            .map(|(n, b)| (n.clone(), sha256_hex(b)))
            .collect(),
        missing,
    })
}

// ---- The scan ----

type Owned = Vec<(String, gp_assets::Region, u32, u32)>;

fn owned_by(map: &AssetMap) -> Owned {
    map.assets
        .iter()
        .flat_map(|a| {
            a.spans()
                .into_iter()
                .map(|s| (a.name().to_owned(), s.region, s.offset, s.len))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A ROM to compare with: its data after the header, and where PRG ends.
struct Source {
    label: String,
    data: Vec<u8>,
    prg_len: usize,
    /// The CPU's view of PRG ($8000 to $FFFF), for reading routines.
    image: Vec<u8>,
    owned: Owned,
}

impl Source {
    fn of(label: String, bytes: &[u8], owned: Owned) -> Self {
        let data = bytes.get(16..).unwrap_or(&[]).to_vec();
        let prg_len = (usize::from(bytes.get(4).copied().unwrap_or(0)) * 16384).min(data.len());
        let image = gp_nes::codemap::cpu_image(&data[..prg_len]);
        Self {
            label,
            data,
            prg_len,
            image,
            owned,
        }
    }

    /// A romset: its chips unzipped, one after another. It has no 6502
    /// program, so no hook is judged a transcription of one: that reading
    /// is the NES's only, and Z80 code has no counterpart yet.
    fn of_romset(label: String, chips: &[u8], owned: Owned) -> Self {
        Self {
            label,
            data: chips.to_vec(),
            prg_len: chips.len(),
            image: Vec::new(),
            owned,
        }
    }

    fn place(&self, offset: usize) -> String {
        if offset < self.prg_len {
            format!("PRG {offset:#06x}")
        } else {
            format!("CHR {:#06x}", offset - self.prg_len)
        }
    }
}

/// A text file to scan, with what the detectors need.
struct Text {
    rel: String,
    role: Role,
    text: String,
    nums: Vec<Num>,
    groups: Vec<rom_match::Group>,
    lexed: Option<fields::Lexed>,
}

/// Groups of numbers by byte span: every span's numbers in order.
fn groups_of(nums: &[Num], spans: &[(usize, usize)]) -> Vec<rom_match::Group> {
    spans
        .iter()
        .map(|&(s, e)| {
            let a = nums.partition_point(|n| n.start < s);
            let b = nums.partition_point(|n| n.start < e);
            (a..b).collect::<Vec<_>>()
        })
        .filter(|g| g.len() >= rom_match::MIN_GROUP)
        .collect()
}

/// Bracketed spans of a text that isn't TOML: each `[...]` and `{...}`
/// that holds no other bracket of its kind.
fn bracket_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut stack: Vec<(u8, usize, bool)> = Vec::new();
    for (i, c) in text.bytes().enumerate() {
        match c {
            b'[' | b'{' | b'(' => {
                if let Some(top) = stack.last_mut() {
                    top.2 = true;
                }
                stack.push((c, i, false));
            }
            b']' | b'}' | b')' => {
                if let Some((o, s, nested)) = stack.pop()
                    && !nested
                    && o != b'('
                {
                    out.push((s, i + 1));
                }
            }
            _ => {}
        }
    }
    out
}

fn line_at(text: &str, at: usize) -> usize {
    1 + text.as_bytes()[..at.min(text.len())]
        .iter()
        .filter(|&&c| c == b'\n')
        .count()
}

/// The field a byte offset is in, for TOML.
fn field_at(lexed: &fields::Lexed, at: usize) -> Option<&fields::Field> {
    lexed
        .fields
        .iter()
        .find(|f| f.start <= at && at < f.end.max(f.start + 1))
}

type FileList = Vec<(String, PathBuf)>;

/// Every file of a part: relative paths, plain files only, and the symlinks
/// found. `.git`, `target` and `sealed` folders and the skipped folder are
/// left out.
fn part_files(part: &Part) -> Result<(FileList, Vec<String>)> {
    fn walk(
        base: &Path,
        dir: &Path,
        skip: Option<&str>,
        out: &mut FileList,
        links: &mut Vec<String>,
    ) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let ft = e.file_type()?;
            if ft.is_symlink() {
                links.push(rel);
            } else if ft.is_dir() {
                let name = e.file_name();
                if name == ".git"
                    || name == "target"
                    || name.eq_ignore_ascii_case("sealed")
                    || skip == Some(rel.as_str())
                {
                    continue;
                }
                walk(base, &path, skip, out, links)?;
            } else if ft.is_file() {
                out.push((rel, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let mut links = Vec::new();
    walk(
        &part.dir,
        &part.dir,
        part.skip.as_deref(),
        &mut out,
        &mut links,
    )?;
    Ok((out, links))
}

fn entry_map(pkg: &Package, e: &RomEntry) -> Result<(String, AssetMap)> {
    let path = pkg.dir.join(&e.asset_map);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("{}: reading its asset map {}", e.id, path.display()))?;
    let map = AssetMap::parse(&text).map_err(anyhow::Error::msg)?;
    Ok((text, map))
}

fn root_part(pkg: &Package) -> Result<Part<'_>> {
    let mut recipes = BTreeMap::new();
    // An arcade entry has no asset map yet: the tools lift no arcade assets.
    let mapped = || pkg.entries.iter().filter(|e| !e.asset_map.is_empty());
    for e in mapped() {
        let (text, map) = entry_map(pkg, e)?;
        recipes.insert(e.asset_map.clone(), (map, text, Some(e)));
    }
    let primary = mapped()
        .find(|e| e.role == "scored")
        .or_else(|| mapped().next())
        .map(|e| e.asset_map.clone());
    Ok(Part {
        name: pkg.name.clone(),
        tier: pkg.tier.clone(),
        dir: pkg.dir.clone(),
        skip: pkg.open.as_ref().map(|o| o.rel.clone()),
        model: "model.toml".to_owned(),
        recipes,
        primary,
        authored: None,
    })
}

fn open_part(open: &OpenPart) -> Result<Part<'_>> {
    let path = open.dir.join(&open.asset_map);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("{}: reading its asset map {}", open.id, path.display()))?;
    let map = AssetMap::parse(&text).map_err(anyhow::Error::msg)?;
    let mut recipes = BTreeMap::new();
    recipes.insert(open.asset_map.clone(), (map, text, None));
    Ok(Part {
        name: if open.id.is_empty() {
            open.title.clone()
        } else {
            open.id.clone()
        },
        tier: "open".to_owned(),
        dir: open.dir.clone(),
        skip: None,
        model: open.model.clone(),
        recipes,
        primary: Some(open.asset_map.clone()),
        authored: Some(open),
    })
}

pub fn scan(pkg: &Package, found: &BTreeMap<String, Rom>) -> Result<Report> {
    let mut parts = vec![scan_part(pkg, &root_part(pkg)?, found)?];
    if let Some(open) = &pkg.open {
        parts.push(scan_part(pkg, &open_part(open)?, found)?);
    }
    for (id, what) in pkg.roms_toml_disagreements()? {
        parts[0].rewrite.push(Rewrite {
            file: "roms.toml".to_owned(),
            line: 0,
            at: format!("source {id}"),
            what,
            fix: "make roms.toml and package.toml name the same ROM".to_owned(),
            key: format!("roms.toml|{id}"),
            span: None,
        });
    }
    Ok(Report {
        name: pkg.name.clone(),
        version: pkg.version.clone(),
        tier: pkg.tier.clone(),
        entries: pkg
            .entries
            .iter()
            .map(|e| (e.id.clone(), found.contains_key(&e.sha256)))
            .collect(),
        parts,
    })
}

fn scan_part(pkg: &Package, part: &Part, found: &BTreeMap<String, Rom>) -> Result<PartReport> {
    let authored = part.authored.is_some();
    // The ROMs to compare with. An open version's image is built from its
    // own data, so its comparison is with the original's, as base ROMs.
    let mut sources = Vec::new();
    for e in &pkg.entries {
        let Some(r) = found.get(&e.sha256) else {
            continue;
        };
        let owned = match part.recipes.get(&e.asset_map) {
            Some((map, ..)) => owned_by(map),
            None => part
                .recipes
                .values()
                .next()
                .map_or_else(Vec::new, |(m, ..)| owned_by(m)),
        };
        let label = if authored {
            format!("base {}", r.label(&e.id))
        } else {
            r.label(&e.id)
        };
        sources.push(match r.platform {
            roms::Platform::Nes => Source::of(label, &r.bytes, owned),
            roms::Platform::Arcade => Source::of_romset(label, r.data(), owned),
        });
    }

    // Every file, read once.
    let (files_found, links) = part_files(part)?;
    let mut raw = Vec::new();
    let mut whole_binary = Vec::new();
    for (rel, path) in files_found {
        let role = role_of(&rel, part);
        match fs::read_to_string(&path) {
            Ok(text) => raw.push((rel, role, text)),
            Err(_) => whole_binary.push(rel),
        }
    }
    let texts: Vec<Text> = raw
        .into_iter()
        .map(|(rel, role, text)| {
            let nums = rom_match::numbers(&text);
            let toml = rel.ends_with(".toml") || role == Role::Recipe;
            let lexed = toml.then(|| fields::lex(&text));
            let spans = match &lexed {
                Some(l) => l.groups.clone(),
                None => bracket_spans(&text),
            };
            let groups = groups_of(&nums, &spans);
            Text {
                rel,
                role,
                text,
                nums,
                groups,
                lexed,
            }
        })
        .collect();

    // The chance bound counts every place tried: each number in two streams
    // and in the groups, against each ROM.
    let tries = (texts.iter().map(|x| x.nums.len()).sum::<usize>().max(1)
        * 3
        * sources.len().max(1)) as f64;

    let mut rewrite: Vec<Rewrite> = Vec::new();
    let indexes: Vec<RomIndex> = sources
        .iter()
        .map(|s| RomIndex::new(s.label.clone(), &s.data, s.prg_len))
        .collect();
    for x in &texts {
        // A recipe holds offsets and sizes, an original's authored data is
        // ours: neither is compared.
        if matches!(x.role, Role::Recipe | Role::Authored) {
            continue;
        }
        // Each ROM in order: a match another ROM also has is listed once.
        let mut taken: Vec<(usize, usize)> = Vec::new();
        for (si, idx) in indexes.iter().enumerate() {
            let src = &sources[si];
            for m in rom_match::scan_text(&x.nums, &x.groups, idx, tries) {
                if taken.iter().any(|&(a, b)| a <= m.last && m.first <= b) {
                    continue;
                }
                taken.push((m.first, m.last));
                rewrite.push(rom_item(x, &m, src));
            }
        }
    }

    // Lines of the lifted files copied into the package.
    if !authored {
        for e in &pkg.entries {
            let (Some(r), Some((map, map_text, _))) =
                (found.get(&e.sha256), part.recipes.get(&e.asset_map))
            else {
                continue;
            };
            let Ok((lifted, _)) = gp_assets::lift(&r.bytes, map, map_text) else {
                continue;
            };
            lifted_lines(&texts, lifted, &mut rewrite);
        }
    }

    // Hook code against the routines it cites.
    if let Some(src) = sources.first() {
        hook_items(&texts, src, &mut rewrite);
    }

    // Model slots naming assets the recipe doesn't make.
    let primary = part.primary.as_ref().and_then(|p| part.recipes.get(p));
    if let (Some(x), Some((map, ..))) = (
        texts
            .iter()
            .find(|x| x.role == Role::Model && x.rel == part.model),
        primary,
    ) {
        missing_assets(x, map, &mut rewrite);
    }
    for rel in &links {
        rewrite.push(Rewrite {
            file: rel.clone(),
            line: 0,
            at: "the file".to_owned(),
            what: "a symlink; a package holds only plain files".to_owned(),
            fix: "replace it with the file it points at, or remove it".to_owned(),
            key: format!("{rel}|symlink"),
            span: None,
        });
    }
    rewrite.sort_by(|a, b| (&a.file, a.line, &a.key).cmp(&(&b.file, b.line, &b.key)));
    rewrite.dedup_by(|a, b| a.key == b.key);

    // The matches a person reviewed and kept leave the list by exact key.
    let items = reviewed_items(&part.dir)?;
    let (kept, rewrite): (Vec<Rewrite>, Vec<Rewrite>) = rewrite
        .into_iter()
        .partition(|r| items.iter().any(|i| i.key == r.key));
    let stale: Vec<String> = items
        .iter()
        .filter(|i| !kept.iter().any(|r| r.key == i.key))
        .map(|i| i.key.clone())
        .collect();
    let reviewed: Vec<(Rewrite, String)> = kept
        .into_iter()
        .map(|r| {
            let why = items
                .iter()
                .find(|i| i.key == r.key)
                .map_or(String::new(), |i| i.why.clone());
            (r, why)
        })
        .collect();

    // Counts.
    let maps: BTreeMap<&str, &AssetMap> = part
        .recipes
        .iter()
        .map(|(rel, (map, ..))| (rel.as_str(), map))
        .collect();
    let mut files: BTreeMap<String, FileCounts> = BTreeMap::new();
    for x in &texts {
        let hits: Vec<&Rewrite> = rewrite.iter().filter(|r| r.file == x.rel).collect();
        files.insert(x.rel.clone(), count_file(x, &hits, &maps));
    }
    for rel in whole_binary.into_iter().chain(links) {
        let at = usize::from(rewrite.iter().any(|r| r.file == rel)) * Bucket::Rewrite as usize;
        let mut counts = [0; 4];
        counts[at] = 1;
        files.insert(
            rel,
            FileCounts {
                unit: "files",
                counts,
            },
        );
    }
    let recipes = maps
        .values()
        .flat_map(|m| {
            m.assets.iter().map(|a| Recipe {
                asset: a.name().to_owned(),
                format: a.format().to_owned(),
                bytes: a.spans().iter().map(|s| u64::from(s.len)).sum(),
            })
        })
        .collect();

    let rederive = rederive_part(pkg, part, found)?;
    Ok(PartReport {
        name: part.name.clone(),
        tier: part.tier.clone(),
        compared: sources.iter().map(|s| s.label.clone()).collect(),
        files,
        rewrite,
        reviewed,
        stale,
        recipes,
        rederive,
        settings: json!({
            "alpha": rom_match::ALPHA,
            "min_run": rom_match::MIN_RUN,
            "min_group": rom_match::MIN_GROUP,
            "max_window": rom_match::MAX_WINDOW,
            "max_skip": rom_match::MAX_SKIP,
            "transcription_min_lcs": transcribe::MIN_LCS,
            "transcription_min_cover": transcribe::MIN_COVER,
            "transcription_min_cycle_adds": transcribe::MIN_CYCLE_ADDS,
            "tries": tries,
        }),
    })
}

/// The rebuilds of a part: each of the ROMs found for the root; for an open
/// version, its own image, which needs no ROM.
fn rederive_part(
    pkg: &Package,
    part: &Part,
    found: &BTreeMap<String, Rom>,
) -> Result<Vec<(String, Rederive)>> {
    let model_text = fs::read_to_string(part.dir.join(&part.model)).ok();
    let mut out = Vec::new();
    if let Some(open) = part.authored {
        let built = Authored::load(&open.dir)
            .map_err(|e| anyhow::anyhow!("{}: {e}", open.dir.display()))?
            .build()
            .map_err(|e| anyhow::anyhow!("{}: {e}", open.dir.display()))?;
        if let Some((_, map_text, _)) = part.recipes.get(&open.asset_map) {
            let d = rederive(
                &built.image,
                map_text,
                open.asset_map_sha256.as_deref(),
                open.lifted_sha256.clone(),
                model_text.as_deref(),
            )?;
            out.push((part.name.clone(), d));
        }
        return Ok(out);
    }
    for e in &pkg.entries {
        let (Some(r), Some((_, map_text, _))) =
            (found.get(&e.sha256), part.recipes.get(&e.asset_map))
        else {
            continue;
        };
        let model = (part.primary.as_deref() == Some(e.asset_map.as_str()))
            .then_some(model_text.as_deref())
            .flatten();
        let d = rederive(
            &r.bytes,
            map_text,
            Some(e.asset_map_sha256.as_str()),
            e.lifted_sha256.clone(),
            model,
        )?;
        out.push((e.id.clone(), d));
    }
    Ok(out)
}

fn lifted_lines(texts: &[Text], lifted: Vec<gp_assets::File>, rewrite: &mut Vec<Rewrite>) {
    let theirs: BTreeMap<String, String> = lifted
        .into_iter()
        .filter_map(|f| String::from_utf8(f.bytes).ok().map(|s| (f.name, s)))
        .collect();
    let ours: BTreeMap<String, String> = texts
        .iter()
        .filter(|x| !matches!(x.role, Role::Recipe | Role::Authored))
        .map(|x| (x.rel.clone(), x.text.clone()))
        .collect();
    for (file, line, asset_file) in template::shared_lines(&ours, &theirs) {
        let text = &ours[&file];
        let at = text.find(&line).unwrap_or(0);
        let x = texts.iter().find(|x| x.rel == file);
        let field = x
            .and_then(|x| x.lexed.as_ref())
            .and_then(|l| field_at(l, at))
            .map(|f| f.path.clone())
            .unwrap_or_else(|| "a line".to_owned());
        rewrite.push(Rewrite {
            file: file.clone(),
            line: line_at(text, at),
            at: field,
            what: format!(
                "a line of the lifted file {asset_file} ({} characters)",
                line.len()
            ),
            fix: format!(
                "read it from the lifted {asset_file} (the asset map makes it) instead of a copy"
            ),
            key: format!(
                "{file}|lifted|{asset_file}|{}",
                &sha256_hex(line.as_bytes())[..16]
            ),
            span: Some((at, at + line.len())),
        });
    }
}

fn hook_items(texts: &[Text], src: &Source, rewrite: &mut Vec<Rewrite>) {
    for x in texts.iter().filter(|x| x.role == Role::Hook) {
        for item in transcribe::items(&x.text) {
            let Some(v) = transcribe::judge(&x.text, &item, &src.image) else {
                continue;
            };
            if v.signals.is_empty() {
                continue;
            }
            let mut what = Vec::new();
            if v.signals.contains(&"same-order") {
                what.push(format!(
                    "its operands follow ${:04X}'s in order: {} of its {} and the routine's {} in common order ({} with the routine reversed)",
                    v.routine, v.lcs, v.fn_ops, v.routine_ops, v.null
                ));
            }
            if v.signals.contains(&"cycle-count") {
                what.push(format!(
                    "it counts ${:04X}'s instruction cycles ({} literal adds to an accumulator)",
                    v.routine, v.cycle_adds
                ));
            }
            rewrite.push(Rewrite {
                file: x.rel.clone(),
                line: item.line,
                at: format!("fn {}", item.name),
                what: format!("transcribes ${:04X}: {}", v.routine, what.join("; ")),
                fix: format!(
                    "describe what ${:04X} does (the states it moves between, what it tests and what it changes) as model rules or ext_params, not its instructions{}",
                    v.routine,
                    if v.signals.contains(&"cycle-count") {
                        "; keep its cost as a measured fact per case, not counted per instruction"
                    } else {
                        ""
                    }
                ),
                key: format!("{}|fn {}|{:04X}", x.rel, item.name, v.routine),
                span: Some((item.start, item.end)),
            });
        }
    }
}

fn missing_assets(x: &Text, map: &AssetMap, rewrite: &mut Vec<Rewrite>) {
    let names: BTreeSet<&str> = map.assets.iter().map(|a| a.name()).collect();
    for (path, asset) in named_assets(&x.text) {
        if !names.contains(asset.as_str()) {
            let line = x
                .lexed
                .as_ref()
                .and_then(|l| l.fields.iter().find(|f| f.path == path))
                .map_or(0, |f| f.line);
            rewrite.push(Rewrite {
                file: x.rel.clone(),
                line,
                at: path.clone(),
                what: format!("names the asset {asset}, which the recipe doesn't make"),
                fix: format!("add {asset} to the asset map, or name an asset it has"),
                key: format!("{}|{path}|missing", x.rel),
                span: None,
            });
        }
    }
}

/// A ROM match as a needs-rewrite item.
fn rom_item(x: &Text, m: &Match, src: &Source) -> Rewrite {
    let start = x.nums[m.first].start;
    let line = line_at(&x.text, start);
    let last_line = line_at(&x.text, x.nums[m.last].start);
    let at = match &x.lexed {
        Some(l) => {
            let a = field_at(l, start).map(|f| f.path.clone());
            let b = field_at(l, x.nums[m.last].start).map(|f| f.path.clone());
            match (a, b) {
                (Some(a), Some(b)) if a != b => format!("{a} to {b}"),
                (Some(a), _) => a,
                (None, Some(b)) => format!("a comment, to {b}"),
                (None, None) => "a comment".to_owned(),
            }
        }
        None => transcribe::items(&x.text)
            .into_iter()
            .find(|i| i.start <= start && start < i.end)
            .map_or_else(
                || "a comment".to_owned(),
                |i| format!("{} {}", i.kind, i.name),
            ),
    };
    let values = m.last - m.first + 1;
    let how = match m.kind {
        rom_match::Kind::Exact => format!("the same order ({} stream)", m.stream),
        rom_match::Kind::Reordered => "another order".to_owned(),
        rom_match::Kind::Scaled => format!(
            "scaled: each value {} times the ROM's {} {}",
            m.scale,
            if m.offset < 0 { "less" } else { "plus" },
            m.offset.abs()
        ),
    };
    let spread = if last_line > line {
        format!(", over lines {line} to {last_line}")
    } else {
        String::new()
    };
    let place = src.place(m.rom_offset);
    let (region, off) = if m.rom_offset < src.prg_len {
        (gp_assets::Region::Prg, m.rom_offset as u32)
    } else {
        (gp_assets::Region::Chr, (m.rom_offset - src.prg_len) as u32)
    };
    let have = src
        .owned
        .iter()
        .find(|(_, r, o, l)| *r == region && *o <= off && off < o + l)
        .map(|(n, ..)| n.clone());
    let fix = match (&have, x.role) {
        (Some(a), Role::Hook) => format!(
            "the recipe already lifts these bytes as {a}: read them from it through ext_tables instead of the literal"
        ),
        (Some(a), _) => format!(
            "the recipe already lifts these bytes as {a}: point the field at it (assets.*.from or ext_tables) instead of the copy"
        ),
        (None, _) => format!(
            "add an asset map entry for {place}, {} bytes (format byte-rows, named for what the table is), and read the field from the lifted file{}",
            m.len,
            if m.kind == rom_match::Kind::Scaled {
                format!(
                    "; state the scale {} and offset {} as fields",
                    m.scale, m.offset
                )
            } else {
                String::new()
            }
        ),
    };
    Rewrite {
        file: x.rel.clone(),
        line,
        at,
        what: format!(
            "{values} values hold {} bytes of {} from {place} in {how}{spread} (chance {:.0e})",
            m.len,
            if src.label.starts_with("base ") {
                "the base's ROM"
            } else {
                "the ROM"
            },
            m.ln_expected.exp()
        ),
        fix,
        key: format!(
            "{}|{}|{}|{}|{}",
            x.rel,
            m.kind.name(),
            src.label.starts_with("base "),
            m.rom_offset,
            m.len
        ),
        span: Some((start, x.nums[m.last].end)),
    }
}

/// A file's units by bucket. A unit is needs rewrite when an item's span
/// overlaps it (or, for an item with no span, names it); an item that
/// overlaps no unit (a match in a comment) is a unit of its own.
fn count_file(x: &Text, hits: &[&Rewrite], maps: &BTreeMap<&str, &AssetMap>) -> FileCounts {
    let mut c = FileCounts::default();
    let overlaps = |s: usize, e: usize| {
        hits.iter()
            .filter(|r| r.span.is_some_and(|(a, b)| a < e.max(s + 1) && s < b))
            .count()
    };
    let mut placed = vec![false; hits.len()];
    let mark = |s: usize, e: usize, placed: &mut [bool]| {
        for (k, r) in hits.iter().enumerate() {
            if r.span.is_some_and(|(a, b)| a < e.max(s + 1) && s < b) {
                placed[k] = true;
            }
        }
    };
    match x.role {
        Role::Recipe => {
            c.unit = "recipes";
            c.counts[Bucket::FromRom as usize] =
                maps.get(x.rel.as_str()).map_or(0, |m| m.assets.len());
            placed.iter_mut().for_each(|p| *p = true);
        }
        Role::Hook => {
            c.unit = "items";
            let items = transcribe::items(&x.text);
            for i in &items {
                let at = format!("{} {}", i.kind, i.name);
                let bad = overlaps(i.start, i.end) > 0 || hits.iter().any(|r| r.at == at);
                mark(i.start, i.end, &mut placed);
                for (k, r) in hits.iter().enumerate() {
                    if r.at == at {
                        placed[k] = true;
                    }
                }
                c.counts[if bad { Bucket::Rewrite } else { Bucket::Ours } as usize] += 1;
            }
            if items.is_empty() && hits.is_empty() {
                c.counts[Bucket::Ours as usize] += 1;
            }
        }
        Role::Whole(_) => {
            c.unit = "files";
            c.counts[if hits.is_empty() {
                Bucket::Ours
            } else {
                Bucket::Rewrite
            } as usize] = 1;
            placed.iter_mut().for_each(|p| *p = true);
        }
        _ => {
            c.unit = "fields";
            if let Some(l) = &x.lexed {
                for f in &l.fields {
                    let b = if overlaps(f.start, f.end) > 0
                        || hits.iter().any(|r| r.span.is_none() && r.at == f.path)
                    {
                        Bucket::Rewrite
                    } else {
                        field_bucket(x.role, f, &x.text)
                    };
                    mark(f.start, f.end, &mut placed);
                    for (k, r) in hits.iter().enumerate() {
                        if r.span.is_none() && r.at == f.path {
                            placed[k] = true;
                        }
                    }
                    c.counts[b as usize] += 1;
                }
            }
        }
    }
    c.counts[Bucket::Rewrite as usize] += placed.iter().filter(|p| !**p).count();
    c
}

/// A TOML field's bucket, when no detector flagged it.
fn field_bucket(role: Role, f: &fields::Field, text: &str) -> Bucket {
    let v = &text[f.start..f.end];
    match role {
        Role::Mapping => Bucket::Reference,
        Role::Model
            if (f.path.starts_with("assets.") && f.path.ends_with(".from.asset"))
                || f.path.contains(".ext_tables.") =>
        {
            Bucket::FromRom
        }
        _ if REFERENCE_KEYS.contains(&f.key.as_str()) => Bucket::Reference,
        _ if f.value == fields::Value::Str && only_addresses(v) => Bucket::Reference,
        _ => Bucket::Ours,
    }
}
