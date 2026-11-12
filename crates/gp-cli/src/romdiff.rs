//! `gp roms diff`: the PRG and CHR of two NES images compared, either side
//! possibly a raw PRG binary (a disassembly build), by file offset or as
//! the CPU sees an address range through each side's board.
//!
//! What each range found is: the base's asset map names the assets it
//! falls in, and the base's labels file (docs/formats/labels.md) names the
//! routines and data tables. Together they sort a hack's changes into level
//! data, enemy data, graphics, text, music, tables and code, which is the
//! first thing to know before printing one (a change in a movement routine
//! is a mechanic to find before claiming exact). With `--summary`, names
//! only: nothing quotes the ROM's bytes.
//!
//! Two arcade romsets (MAME zips, port wave 6) are compared chip by chip
//! ([`romsets`]): each region of the first set (the main and sound
//! programs, the tiles, the sprites) by the first set's chips, each chip
//! paired with the second set's chip at the same place, said identical or
//! with its differing ranges, by offset in the chip and, for the programs,
//! by CPU address. `--summary` leaves the bytes out. The labels, asset
//! map and `--cpu` are the NES's and refused for romsets.
//!
//! Ported from game-printer's romdiff.rs and its `roms_diff` in main.rs.
//! The base's ROM is named, and its asset map and labels found, from the
//! games checkout (each game's roms.toml and package.toml) instead of the
//! old repo's `sources/` and asset map folder. The asset formats of other
//! games (Super Mario Bros., Mega Man 2, Super Mario Bros. 2) are cut.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_assets::AssetMap;
use gp_assets::map::{Asset, Region};
use gp_evidence::registry::Registry;
use gp_evidence::source::{ByteRange, NesImage, diff_ranges, looks_like_nes, nes_banks};
use serde::Serialize;
use serde_json::json;

use crate::Outcome;
use crate::labels::LabelsFile;

#[derive(Args)]
pub struct DiffArgs {
    /// First NES image or raw PRG binary.
    pub a: PathBuf,
    /// Second NES image or raw PRG binary.
    pub b: PathBuf,
    /// Merge differing runs separated by this many equal bytes or fewer.
    #[arg(long, default_value_t = 8)]
    pub gap: usize,
    /// Show at most this many ranges per section.
    #[arg(long, default_value_t = 40)]
    pub limit: usize,
    /// A labels file (docs/formats/labels.md) naming the first image's
    /// routines and data tables. Default: its game's labels.toml, when it
    /// has one.
    #[arg(long)]
    pub labels: Option<PathBuf>,
    /// Print each range's kind and names and the counts by kind, without
    /// the bytes, so the output can be quoted.
    #[arg(long)]
    pub summary: bool,
    /// Compare what the CPU sees over these addresses (`C000-FFFF`),
    /// each side through its own board, instead of PRG offsets: a 16K
    /// program against a 32K one's upper bank, or a bank of one banked
    /// board against another's. Ranges are named by CPU address.
    #[arg(long, value_name = "LO-HI")]
    pub cpu: Option<String>,
    /// With --cpu: the bank the first image's switched region holds
    /// (UxROM and MMC1 at $8000, MMC3 at $8000 and $A000). Default:
    /// what the CPU sees at power-on.
    #[arg(long, requires = "cpu")]
    pub bank_a: Option<u32>,
    /// With --cpu: the same for the second image.
    #[arg(long, requires = "cpu")]
    pub bank_b: Option<u32>,
    /// The games checkout whose roms.toml files name the ROMs (as for
    /// `scan`).
    #[arg(long)]
    pub games: Option<PathBuf>,
}

// ---- Naming what a range is ----

/// What a changed range is, by what it overlaps in the base. The old
/// classes for enemy data and music came from other games' asset formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Class {
    LevelData,
    Graphics,
    Text,
    Table,
    Code,
    Data,
    Unknown,
}

impl Class {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Class::LevelData => "level data",
            Class::Graphics => "graphics",
            Class::Text => "text",
            Class::Table => "table",
            Class::Code => "code",
            Class::Data => "data",
            Class::Unknown => "unknown",
        }
    }
}

/// The class of an asset, by its format.
fn asset_class(asset: &Asset) -> Class {
    match asset {
        Asset::Chr { .. }
        | Asset::Palette { .. }
        | Asset::Metatiles { .. }
        | Asset::SpriteSet(_) => Class::Graphics,
        Asset::Level { .. } => Class::LevelData,
        Asset::PpuStrings(_) => Class::Text,
        // gameprinter's pointer tables name no targets, so one is a table,
        // as the old code read a pointer table without them.
        _ => Class::Table,
    }
}

/// One range's annotation.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Note {
    pub class: Class,
    pub assets: Vec<String>,
    pub routines: Vec<String>,
    pub data: Vec<String>,
}

impl Note {
    pub(crate) fn line(&self) -> String {
        let mut parts = vec![self.class.name().to_owned()];
        if !self.assets.is_empty() {
            parts.push(format!("assets {}", self.assets.join(", ")));
        }
        if !self.routines.is_empty() {
            parts.push(format!("routines {}", self.routines.join(", ")));
        }
        if !self.data.is_empty() {
            parts.push(format!("data {}", self.data.join(", ")));
        }
        parts.join("; ")
    }
}

/// The base's map and labels, when they're found.
pub(crate) struct Annotator {
    pub map: Option<(String, AssetMap)>,
    pub labels: Option<(String, LabelsFile)>,
}

impl Annotator {
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_none() && self.labels.is_none()
    }

    /// The annotation of a range of `region` starting at `offset`.
    pub(crate) fn note(&self, region: Region, r: &ByteRange) -> Note {
        let (start, end) = (r.offset, r.offset + r.len);
        let overlaps = |o: u64, len: u64| o < end && o + len > start;
        let mut assets = Vec::new();
        let mut classes = Vec::new();
        if let Some((_, map)) = &self.map {
            for a in &map.assets {
                if a.spans()
                    .iter()
                    .any(|s| s.region == region && overlaps(u64::from(s.offset), u64::from(s.len)))
                {
                    assets.push(a.name().to_owned());
                    classes.push(asset_class(a));
                }
            }
        }
        let (mut routines, mut data) = (Vec::new(), Vec::new());
        if region == Region::Prg
            && let Some((_, labels)) = &self.labels
        {
            for l in &labels.routine {
                if overlaps(u64::from(l.prg), u64::from(l.size.max(1))) {
                    routines.push(l.name.clone());
                }
            }
            for l in &labels.data {
                if overlaps(u64::from(l.prg), u64::from(l.size.max(1))) {
                    data.push(l.name.clone());
                }
            }
        }
        let class = if region == Region::Chr {
            Class::Graphics
        } else if let Some(c) = classes.iter().min() {
            *c
        } else if !routines.is_empty() {
            Class::Code
        } else if !data.is_empty() {
            Class::Data
        } else {
            Class::Unknown
        };
        Note {
            class,
            assets,
            routines,
            data,
        }
    }
}

/// Counts by class, and the routines touched, for the summary.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Summary {
    /// Class name -> (ranges, differing bytes).
    pub classes: BTreeMap<&'static str, (u64, u64)>,
    pub routines: Vec<String>,
    pub assets: Vec<String>,
}

impl Summary {
    pub(crate) fn add(&mut self, note: &Note, r: &ByteRange) {
        let e = self.classes.entry(note.class.name()).or_default();
        e.0 += 1;
        e.1 += r.differing;
        for n in &note.routines {
            if !self.routines.contains(n) {
                self.routines.push(n.clone());
            }
        }
        for n in &note.assets {
            if !self.assets.contains(n) {
                self.assets.push(n.clone());
            }
        }
    }

    pub(crate) fn text(&self) -> String {
        let mut out = String::from("By kind (ranges, differing bytes):\n");
        for (class, (ranges, bytes)) in &self.classes {
            out.push_str(&format!("  {class:<12} {ranges:>4} {bytes:>6}\n"));
        }
        if !self.assets.is_empty() {
            out.push_str(&format!(
                "Assets touched ({}): {}\n",
                self.assets.len(),
                self.assets.join(", ")
            ));
        }
        if !self.routines.is_empty() {
            out.push_str(&format!(
                "Routines touched ({}): {}\n",
                self.routines.len(),
                self.routines.join(", ")
            ));
        }
        out
    }
}

// ---- The command ----

/// PRG and CHR of an NES image, or a raw binary treated as PRG only.
struct Banks {
    label: String,
    image: Option<NesImage>,
    prg: Vec<u8>,
    chr: Vec<u8>,
}

fn load_banks(path: &Path, registry: &Registry) -> Result<Banks> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.display().to_string();
    if bytes.starts_with(b"PK\x03\x04") {
        bail!(
            "{name} is an arcade romset (a zip): gp roms diff compares NES images and raw PRG binaries only, so far"
        );
    }
    if looks_like_nes(&bytes) {
        let (img, prg, chr) = nes_banks(&bytes)?;
        let id = registry
            .by_sha256(&img.sha256)
            .map(|k| k.id.clone())
            .unwrap_or_else(|| "unknown source".into());
        Ok(Banks {
            label: format!("{name} ({id}, sha256 {})", &img.sha256[..16]),
            prg: prg.to_vec(),
            chr: chr.to_vec(),
            image: Some(img),
        })
    } else {
        Ok(Banks {
            label: format!("{name} (raw PRG, {} bytes)", bytes.len()),
            image: None,
            prg: bytes,
            chr: Vec::new(),
        })
    }
}

/// A PRG offset as the CPU sees it, for mapper 0 (NROM) and mapper 3 (CNROM,
/// which banks CHR only). Other mappers bank PRG, so their offsets are shown
/// as bank and offset within a 16K bank.
fn prg_address(offset: u64, prg_len: usize, mapper: Option<u16>) -> String {
    match (mapper, prg_len) {
        (Some(0 | 3) | None, 32768) => format!("${:04X}", 0x8000 + offset),
        (Some(0) | None, 16384) => format!("${:04X}", 0xC000 + offset),
        _ => format!("bank {} +${:04X}", offset / 16384, offset % 16384),
    }
}

/// `C000-FFFF`, `$C000-$FFFF` or `C000..FFFF`: the first and last address.
fn cpu_range(s: &str) -> Result<(u16, u16)> {
    let (a, b) = s
        .split_once("..")
        .or_else(|| s.split_once('-'))
        .with_context(|| format!("--cpu {s}: give LO-HI, such as C000-FFFF"))?;
    let hex = |t: &str| {
        u16::from_str_radix(
            t.trim().trim_start_matches('$').trim_start_matches("0x"),
            16,
        )
        .with_context(|| format!("--cpu {s}: {t:?} isn't a hex address"))
    };
    let (lo, hi) = (hex(a)?, hex(b.trim_start_matches('='))?);
    if lo < 0x8000 || hi < lo {
        bail!("--cpu {s}: give a range within $8000 to $FFFF");
    }
    Ok((lo, hi))
}

/// The board a side's PRG sits on: its mapper, or mapper 0 for a raw PRG
/// binary.
fn diff_board(side: &Banks) -> gp_nes::codemap::Board {
    let mapper = side.image.as_ref().map_or(0, |i| i.header.mapper);
    gp_nes::codemap::Board::of(&side.prg, mapper)
}

/// What the CPU reads from `lo` to `hi` on a side's board, with `bank` in
/// its switched regions (else what it shows at power-on), and for each
/// byte its PRG offset.
fn cpu_view(
    side: &Banks,
    lo: u16,
    hi: u16,
    bank: Option<u32>,
) -> Result<(Vec<u8>, Vec<Option<usize>>)> {
    let board = diff_board(side);
    if bank.is_some() && !gp_nes::codemap::banked(&side.prg) {
        bail!(
            "{}: its PRG ({} KB) isn't banked, so it has no bank to choose",
            side.label,
            side.prg.len() / 1024
        );
    }
    let (mut bytes, mut offs) = (Vec::new(), Vec::new());
    for a in lo..=hi {
        let v = match (bank, board.region(a)) {
            (Some(b), Some(_)) => ((b + 1) << 16) | u32::from(a),
            _ => u32::from(a),
        };
        bytes.push(board.read(v));
        offs.push(board.prg_offset(v));
    }
    Ok((bytes, offs))
}

/// The game folder whose package.toml names a ROM entry with this hash,
/// and that entry's asset map.
fn package_map(games: &Path, sha256: &str) -> Option<(PathBuf, PathBuf)> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(games)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("package.toml").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter().find_map(|d| {
        let pkg: toml::Table = fs::read_to_string(d.join("package.toml"))
            .ok()?
            .parse()
            .ok()?;
        let entries = pkg.get("rom")?.get("entry")?.as_array()?;
        let e = entries
            .iter()
            .find(|e| e.get("sha256").and_then(|v| v.as_str()) == Some(sha256))?;
        let map = e.get("asset_map")?.as_str()?;
        Some((d.clone(), d.join(map)))
    })
}

/// The first image's asset map and labels, for naming what each range is.
fn diff_annotator(
    left: &Banks,
    registry: &Registry,
    labels: Option<&Path>,
    games: Option<&Path>,
) -> Result<Annotator> {
    let sha256 = left.image.as_ref().map(|i| i.sha256.clone());
    let map = sha256
        .as_deref()
        .and_then(|s| package_map(games?, s))
        .and_then(|(_, p)| {
            let text = fs::read_to_string(&p).ok()?;
            let map = AssetMap::parse(&text).ok()?;
            Some((p.display().to_string(), map))
        });
    let labels_path = match labels {
        Some(p) => Some(p.to_path_buf()),
        None => sha256
            .as_deref()
            .and_then(|s| registry.by_sha256(s))
            .and_then(|k| crate::labels::locate(&k.id).ok())
            .filter(|p| p.is_file()),
    };
    let labels = labels_path
        .map(|p| -> Result<_> { Ok((p.display().to_string(), crate::labels::read(&p)?)) })
        .transpose()?;
    Ok(Annotator { map, labels })
}

/// Whether a file is a zip, by its first bytes.
fn is_zip(path: &Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok()
        && gp_arcade::romset::looks_like_zip(&head)
}

/// `gp roms diff` of two arcade romsets, chip by chip.
fn romsets(opts: &DiffArgs, registry: &Registry) -> Result<Outcome> {
    use gp_arcade::romset::{self, Region as R};
    if opts.cpu.is_some() || opts.labels.is_some() {
        bail!(
            "--cpu and --labels read NES boards and labels; two romsets are compared chip by chip"
        );
    }
    let load = |p: &Path| -> Result<gp_arcade::Romset> {
        if !is_zip(p) {
            bail!(
                "{} isn't a romset zip: gp roms diff compares two NES images or two romsets",
                p.display()
            );
        }
        Ok(gp_arcade::Romset::load_zip(p)?)
    };
    let (a, b) = (load(&opts.a)?, load(&opts.b)?);
    let label = |p: &Path, s: &gp_arcade::Romset| {
        let id = registry
            .by_sha256(&s.identity)
            .map_or_else(|| "unknown source".to_owned(), |k| k.id.clone());
        format!(
            "{} ({}, {id}, identity {})",
            p.display(),
            s.name,
            &s.identity[..16]
        )
    };
    let chips = |s: &gp_arcade::Romset| -> Vec<romset::Chip> {
        romset::set(s.name).map_or_else(Vec::new, |i| i.chips.to_vec())
    };
    let (ca, cb) = (chips(&a), chips(&b));
    let region_bytes = |s: &gp_arcade::Romset, r: R| -> Vec<u8> {
        match r {
            R::MainCpu => s.maincpu.clone(),
            R::SsioCpu => s.ssiocpu.clone(),
            R::Gfx1 => s.gfx1.clone(),
            R::Gfx2 => s.gfx2.clone(),
        }
    };
    let mut lines = vec![
        format!("a: {}", label(&opts.a, &a)),
        format!("b: {}", label(&opts.b, &b)),
    ];
    let mut regions = Vec::new();
    let (mut same, mut differ) = (0usize, 0usize);
    for r in [R::MainCpu, R::SsioCpu, R::Gfx1, R::Gfx2] {
        let (ra, rb) = (region_bytes(&a, r), region_bytes(&b, r));
        let mut rows = Vec::new();
        for c in ca.iter().filter(|c| c.region == r) {
            let other = cb
                .iter()
                .find(|o| o.region == r && o.offset <= c.offset && c.offset < o.offset + o.size);
            let end = (c.offset + c.size).min(ra.len()).min(rb.len());
            let ranges = diff_ranges(
                &ra[c.offset.min(end)..end],
                &rb[c.offset.min(end)..end],
                opts.gap,
            );
            let differing: u64 = ranges.iter().map(|x| x.differing).sum();
            let same_chip = other.is_some_and(|o| o.sha1 == c.sha1);
            let code = matches!(r, R::MainCpu | R::SsioCpu);
            let head = format!(
                "{} {:<12} vs {:<12} {:05X}-{:05X}",
                r.name(),
                c.name,
                other.map_or("-", |o| o.name),
                c.offset,
                c.offset + c.size - 1
            );
            if ranges.is_empty() {
                same += 1;
                lines.push(format!(
                    "{head}  identical{}",
                    if same_chip { " (the same chip)" } else { "" }
                ));
            } else {
                differ += 1;
                lines.push(format!(
                    "{head}  {differing} bytes differ in {} range{}",
                    ranges.len(),
                    if ranges.len() == 1 { "" } else { "s" }
                ));
                for x in ranges.iter().take(opts.limit) {
                    let at = c.offset as u64 + x.offset;
                    let place = if code {
                        format!(
                            "{} ${at:04X}",
                            if r == R::MainCpu { "main" } else { "sound" }
                        )
                    } else {
                        format!("+{:05X}", x.offset)
                    };
                    if opts.summary {
                        lines.push(format!(
                            "  {place}, {} bytes ({} differ)",
                            x.len, x.differing
                        ));
                    } else {
                        lines.push(format!("  {place}, {} bytes: {} -> {}", x.len, x.a, x.b));
                    }
                }
                if ranges.len() > opts.limit {
                    lines.push(format!("  {} more ranges", ranges.len() - opts.limit));
                }
            }
            rows.push(json!({
                "chip": c.name,
                "other": other.map(|o| o.name),
                "offset": c.offset,
                "size": c.size,
                "identical": ranges.is_empty(),
                "same_chip": same_chip,
                "bytes_differ": differing,
                "ranges": ranges.iter().map(|x| {
                    let mut v = json!({"offset": x.offset, "len": x.len, "differing": x.differing});
                    if !opts.summary {
                        v["a"] = json!(x.a);
                        v["b"] = json!(x.b);
                    }
                    v
                }).collect::<Vec<_>>(),
            }));
        }
        regions.push(json!({"region": r.name(), "chips": rows}));
    }
    lines.push(format!("{same} chips identical, {differ} differ"));
    Ok(Outcome::ok(
        json!({
            "a": {"set": a.name, "identity": a.identity},
            "b": {"set": b.name, "identity": b.identity},
            "regions": regions,
            "chips_identical": same,
            "chips_differ": differ,
        }),
        lines.join("\n"),
    ))
}

pub(crate) fn run(opts: &DiffArgs) -> Result<Outcome> {
    let (gap, limit) = (opts.gap, opts.limit);
    if is_zip(&opts.a) || is_zip(&opts.b) {
        let (registry, _) = crate::roms_cmd::sources(opts.games.as_deref())?;
        return romsets(opts, &registry);
    }
    let games = crate::roms_cmd::games_dir(opts.games.as_deref());
    let (registry, _) = crate::roms_cmd::sources(opts.games.as_deref())?;
    let left = load_banks(&opts.a, &registry)?;
    let right = load_banks(&opts.b, &registry)?;
    let mapper = left
        .image
        .as_ref()
        .or(right.image.as_ref())
        .map(|i| i.header.mapper);

    let cpu = opts.cpu.as_deref().map(cpu_range).transpose()?;
    // In CPU mode each range's offset is from `lo`; the first image's PRG
    // offset of each byte names its assets and labels.
    let mut left_offsets: Vec<Option<usize>> = Vec::new();
    let prg = match cpu {
        Some((lo, hi)) => {
            let (va, oa) = cpu_view(&left, lo, hi, opts.bank_a)?;
            let (vb, _) = cpu_view(&right, lo, hi, opts.bank_b)?;
            left_offsets = oa;
            diff_ranges(&va, &vb, gap)
        }
        None => diff_ranges(&left.prg, &right.prg, gap),
    };
    let both_have_chr = !left.chr.is_empty() && !right.chr.is_empty() && cpu.is_none();
    let chr = if both_have_chr {
        diff_ranges(&left.chr, &right.chr, gap)
    } else {
        Vec::new()
    };
    let count = |r: &[ByteRange]| r.iter().map(|x| x.differing).sum::<u64>();

    let annotator = diff_annotator(&left, &registry, opts.labels.as_deref(), games.as_deref())?;
    let mut summary = Summary::default();
    let mut notes: Vec<serde_json::Value> = Vec::new();
    let mut text = format!("a: {}\nb: {}\n", left.label, right.label);
    match (cpu, (opts.bank_a, opts.bank_b)) {
        (Some((lo, hi)), (ba, bb)) => {
            let bank = |b: Option<u32>| b.map_or("power-on".to_owned(), |b| format!("bank {b}"));
            text.push_str(&format!(
                "compared as the CPU sees ${lo:04X}-${hi:04X} (a: {}, b: {})\n",
                bank(ba),
                bank(bb)
            ));
        }
        (None, _) if left.prg.len() != right.prg.len() => text.push_str(&format!(
            "PRG sizes differ ({} and {} bytes), so file offsets don't line up; --cpu C000-FFFF compares what the CPU sees\n",
            left.prg.len(),
            right.prg.len()
        )),
        _ => {}
    }
    if let Some((p, _)) = &annotator.map {
        text.push_str(&format!("asset map: {p}\n"));
    }
    if let Some((p, _)) = &annotator.labels {
        text.push_str(&format!("labels: {p}\n"));
    }
    let mut section =
        |title: &str, region: Region, ranges: &[ByteRange], addr: &dyn Fn(u64) -> String| {
            if ranges.is_empty() {
                text.push_str(&format!("{title}: identical\n"));
                return;
            }
            text.push_str(&format!(
                "{title}: {} bytes differ in {} ranges\n",
                count(ranges),
                ranges.len()
            ));
            for (i, r) in ranges.iter().enumerate() {
                let at_prg = |r: &ByteRange| match left_offsets.get(r.offset as usize) {
                    Some(Some(o)) if region == Region::Prg && cpu.is_some() => ByteRange {
                        offset: *o as u64,
                        ..r.clone()
                    },
                    _ => r.clone(),
                };
                let note = (!annotator.is_empty()).then(|| annotator.note(region, &at_prg(r)));
                if let Some(n) = &note {
                    summary.add(n, r);
                    notes.push(json!({ "section": title, "offset": r.offset, "note": n }));
                }
                if i >= limit {
                    continue;
                }
                if opts.summary {
                    text.push_str(&format!(
                        "  {:<14} len {:<5} differing {:<5} {}\n",
                        addr(r.offset),
                        r.len,
                        r.differing,
                        note.as_ref().map(Note::line).unwrap_or_default()
                    ));
                    continue;
                }
                text.push_str(&format!(
                    "  {:<14} len {:<5} differing {:<5} a {}\n  {:<14} {:<9} {:<15} b {}\n",
                    addr(r.offset),
                    r.len,
                    r.differing,
                    r.a,
                    "",
                    "",
                    "",
                    r.b
                ));
                if let Some(n) = &note {
                    text.push_str(&format!("  {:<14} {}\n", "", n.line()));
                }
            }
            if ranges.len() > limit {
                text.push_str(&format!(
                    "  ... {} more ranges (use --limit)\n",
                    ranges.len() - limit
                ));
            }
        };
    let prg_len = left.prg.len().max(right.prg.len());
    section("PRG", Region::Prg, &prg, &|o| match cpu {
        Some((lo, _)) => format!("${:04X}", u64::from(lo) + o),
        None => prg_address(o, prg_len, mapper),
    });
    if both_have_chr {
        section("CHR", Region::Chr, &chr, &|o| format!("CHR +${o:04X}"));
    } else if cpu.is_some() {
        text.push_str("CHR: not compared (--cpu compares PRG as the CPU sees it)\n");
    } else {
        text.push_str("CHR: not compared (one side is a raw PRG binary)\n");
    }

    if !annotator.is_empty() {
        text.push_str(&summary.text());
    }
    let identical =
        prg.is_empty() && chr.is_empty() && (cpu.is_some() || left.prg.len() == right.prg.len());
    let (prg, chr) = if opts.summary {
        (Vec::new(), Vec::new())
    } else {
        (prg, chr)
    };
    Ok(Outcome::ok(
        json!({
            "a": left.label,
            "b": right.label,
            "cpu": cpu.map(|(lo, hi)| format!("{lo:04X}-{hi:04X}")),
            "identical": identical,
            "prg": prg,
            "chr": chr,
            "chr_compared": both_have_chr,
            "notes": notes,
            "summary": summary,
        }),
        text.trim_end().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(offset: u64, len: u64) -> ByteRange {
        ByteRange {
            offset,
            len,
            differing: len,
            a: String::new(),
            b: String::new(),
        }
    }

    #[test]
    fn classes_come_from_the_map_then_the_labels() {
        let map = AssetMap::parse(
            r#"schema = 1
source = "made-up"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
[[asset]]
name = "tiles"
format = "chr-2bpp"
region = "chr"
offset = 0
tiles = 2
[[asset]]
name = "pal"
format = "palette"
region = "prg"
offset = 0x100
count = 1
"#,
        )
        .unwrap();
        let labels = crate::labels::parse(
            r#"schema = 0
game = "made-up"
[[routine]]
name = "Move"
bank = 0
addr = 0x8010
prg = 0x10
size = 16
[[data]]
name = "Speeds"
bank = 0
addr = 0x8040
prg = 0x40
size = 8
"#,
        )
        .unwrap();
        let a = Annotator {
            map: Some(("m".into(), map)),
            labels: Some(("l".into(), labels)),
        };
        assert_eq!(a.note(Region::Prg, &range(0x102, 1)).class, Class::Graphics);
        let code = a.note(Region::Prg, &range(0x18, 2));
        assert_eq!(code.class, Class::Code);
        assert_eq!(code.routines, ["Move"]);
        assert_eq!(a.note(Region::Prg, &range(0x41, 1)).class, Class::Data);
        assert_eq!(a.note(Region::Prg, &range(0x80, 1)).class, Class::Unknown);
        assert_eq!(a.note(Region::Chr, &range(0x5, 1)).class, Class::Graphics);
        let mut s = Summary::default();
        s.add(&code, &range(0x18, 2));
        assert!(s.text().contains("Routines touched (1): Move"));
    }

    #[test]
    fn cpu_ranges_and_prg_addresses_read_as_before() {
        assert_eq!(cpu_range("C000-FFFF").unwrap(), (0xC000, 0xFFFF));
        assert_eq!(cpu_range("$8000..$BFFF").unwrap(), (0x8000, 0xBFFF));
        assert!(cpu_range("0000-1000").is_err());
        assert_eq!(prg_address(0x10, 16384, Some(0)), "$C010");
        assert_eq!(prg_address(0x10, 32768, Some(3)), "$8010");
        assert_eq!(prg_address(0x4010, 65536, Some(2)), "bank 1 +$0010");
    }

    /// Two raw binaries: the differing runs, merged across short gaps, and
    /// no CHR compared.
    #[test]
    fn two_raw_binaries_are_compared_by_offset() {
        let d = tempfile::tempdir().unwrap();
        let a = vec![0u8; 16384];
        let mut b = a.clone();
        b[0x10] = 1;
        b[0x14] = 1;
        b[0x100] = 2;
        fs::write(d.path().join("a.bin"), &a).unwrap();
        fs::write(d.path().join("b.bin"), &b).unwrap();
        let o = run(&DiffArgs {
            a: d.path().join("a.bin"),
            b: d.path().join("b.bin"),
            gap: 8,
            limit: 40,
            labels: None,
            summary: false,
            cpu: None,
            bank_a: None,
            bank_b: None,
            games: Some(d.path().to_owned()),
        })
        .unwrap();
        assert_eq!(o.json["identical"], false);
        assert_eq!(o.json["prg"].as_array().unwrap().len(), 2);
        assert_eq!(o.json["prg"][0]["differing"], 2);
        assert!(o.text.contains("$C010"), "{}", o.text);
        assert!(
            o.text
                .contains("CHR: not compared (one side is a raw PRG binary)")
        );
        fs::write(d.path().join("set.zip"), b"PK\x03\x04rest").unwrap();
        let e = load_banks(&d.path().join("set.zip"), &Registry::default())
            .err()
            .unwrap();
        assert!(e.to_string().contains("arcade romset"), "{e}");
    }

    fn args(a: &Path, b: &Path, games: &Path) -> DiffArgs {
        DiffArgs {
            a: a.to_owned(),
            b: b.to_owned(),
            gap: 8,
            limit: 4,
            labels: None,
            summary: true,
            cpu: None,
            bank_a: None,
            bank_b: None,
            games: Some(games.to_owned()),
        }
    }

    /// A zip against an NES image is refused in words, and a zip that's
    /// no set gp-arcade knows says which set it is nearest.
    #[test]
    fn a_romset_is_compared_only_with_a_romset() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.nes"), gp_nes::testrom::build()).unwrap();
        fs::write(
            d.path().join("set.zip"),
            b"PK\x05\x06"
                .iter()
                .chain(&[0u8; 18])
                .copied()
                .collect::<Vec<u8>>(),
        )
        .unwrap();
        let e = run(&args(
            &d.path().join("set.zip"),
            &d.path().join("a.nes"),
            d.path(),
        ))
        .err()
        .unwrap();
        assert!(
            e.to_string().contains("no set gp-arcade knows")
                || e.to_string().contains("isn't a romset zip"),
            "{e}"
        );
        let e = run(&args(
            &d.path().join("a.nes"),
            &d.path().join("set.zip"),
            d.path(),
        ))
        .err()
        .unwrap();
        assert!(e.to_string().contains("isn't a romset zip"), "{e}");
    }

    /// With tapper.zip and tapperg.zip on this machine: the alternate
    /// graphics set has the same programs and other graphics, chip by chip.
    /// Skipped without them.
    #[test]
    fn tapper_and_tapperg_differ_in_their_graphics_only() {
        let (Some(a), Some(b)) = (
            gp_arcade::romset::local_romset("tapper"),
            gp_arcade::romset::local_romset("tapperg"),
        ) else {
            eprintln!("skipped: no tapper and tapperg romsets");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let o = run(&args(&a, &b, d.path())).unwrap();
        for r in o.json["regions"].as_array().unwrap() {
            let differs = r["chips"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["identical"] == false);
            let name = r["region"].as_str().unwrap();
            if name.ends_with("cpu") {
                assert!(!differs, "{name}: {}", o.text);
            }
        }
        assert!(o.json["chips_differ"].as_u64().unwrap() > 0);
    }
}
