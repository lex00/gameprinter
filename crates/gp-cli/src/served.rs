//! `gp template served`: a package's open version, and any folder to be
//! served, hold none of the original's game data. Every file is compared
//! with each ROM the package names: as a whole with its lifted files, line
//! by line with their data lines, and for runs of the ROM's bytes, raw or
//! written as numbers. Nothing of a ROM or a lifted file is printed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_assets::AssetMap;
use gp_assets::authored::Authored;
use gp_evidence::Paths;
use gp_evidence::source::looks_like_nes;
use serde_json::{Value, json};

use crate::Outcome;
use crate::package::Package;
use crate::template::{is_data_line, sha256_hex};

/// The shortest run of a ROM's bytes that counts: shorter runs turn up by
/// chance in text; 32 bytes of the same sequence don't.
const MATCH_BYTES: usize = 32;

/// A window must hold at least this many distinct byte values to count, so
/// fill never matches.
const MIN_DISTINCT: usize = 4;

/// The length of the windows that seed a match.
const SEED_BYTES: usize = 8;

/// Between two matched bytes, a file may hold up to this many others (a
/// line's offset column, a directive, a label) and the run goes on, as long
/// as the next `SYNC_BYTES` match again.
const MAX_SKIP: usize = 8;
const SYNC_BYTES: usize = 4;

/// Seeds kept per window, for windows a ROM holds more than once.
const SEEDS_PER_WINDOW: usize = 8;

/// A run of bytes shared with a ROM.
#[derive(Debug, PartialEq, Eq)]
struct Shared {
    /// How the bytes were read from the file: raw, numbers or hex.
    how: &'static str,
    /// Where the run starts in the file.
    at: usize,
    /// Where it starts in the ROM data, after the header.
    rom_offset: usize,
    len: usize,
}

/// A ROM's windows of `SEED_BYTES`, each with the offsets it's at, without
/// windows of fill and steady counts.
fn rom_windows(rom: &[u8]) -> HashMap<&[u8], Vec<usize>> {
    let mut map: HashMap<&[u8], Vec<usize>> = HashMap::new();
    if rom.len() < SEED_BYTES {
        return map;
    }
    for i in 0..=rom.len() - SEED_BYTES {
        let w = &rom[i..i + SEED_BYTES];
        if distinct(w) >= MIN_DISTINCT && !counts(w) {
            let v = map.entry(w).or_default();
            if v.len() < SEEDS_PER_WINDOW {
                v.push(i);
            }
        }
    }
    map
}

/// Where `data` holds a run of `rom`'s bytes, as (ROM offset, length).
pub fn runs_of_rom(data: &[u8], rom: &[u8]) -> Vec<(usize, usize)> {
    let windows = rom_windows(rom);
    let pos: Vec<usize> = (0..data.len()).collect();
    let mut shared = Vec::new();
    shared_runs(data, &pos, rom, &windows, "raw", &mut shared);
    shared.into_iter().map(|s| (s.rom_offset, s.len)).collect()
}

/// Whether every byte of `w` is the one before it plus the same step.
fn counts(w: &[u8]) -> bool {
    let step = w[1].wrapping_sub(w[0]);
    w.windows(2).all(|p| p[1].wrapping_sub(p[0]) == step)
}

fn distinct(w: &[u8]) -> usize {
    let mut seen = [false; 256];
    let mut n = 0;
    for &b in w {
        if !seen[b as usize] {
            seen[b as usize] = true;
            n += 1;
        }
    }
    n
}

/// How far `seq` from `i` follows `rom` from `r`, letting the file skip up
/// to `MAX_SKIP` bytes between matched ones. Returns the ROM bytes matched
/// and where the match ends in `seq`.
fn follow(seq: &[u8], i: usize, rom: &[u8], r: usize) -> (usize, usize) {
    let (mut si, mut ri) = (i, r);
    loop {
        if si < seq.len() && ri < rom.len() && seq[si] == rom[ri] {
            si += 1;
            ri += 1;
            continue;
        }
        let resync = (1..=MAX_SKIP).find(|k| {
            let a = si + k;
            a + SYNC_BYTES <= seq.len()
                && ri + SYNC_BYTES <= rom.len()
                && seq[a..a + SYNC_BYTES] == rom[ri..ri + SYNC_BYTES]
        });
        match resync {
            Some(k) => si += k,
            None => return (ri - r, si),
        }
    }
}

/// Runs of `MATCH_BYTES` or more of the ROM's bytes, in order, in `seq`.
/// `pos` maps an index into `seq` to an offset in the file.
fn shared_runs(
    seq: &[u8],
    pos: &[usize],
    rom: &[u8],
    windows: &HashMap<&[u8], Vec<usize>>,
    how: &'static str,
    out: &mut Vec<Shared>,
) {
    let mut i = 0;
    while i + SEED_BYTES <= seq.len() {
        let best = windows
            .get(&seq[i..i + SEED_BYTES])
            .into_iter()
            .flatten()
            .map(|&r| (follow(seq, i, rom, r), r))
            .max_by_key(|((n, _), _)| *n);
        match best {
            Some(((n, end), r)) if n >= MATCH_BYTES => {
                out.push(Shared {
                    how,
                    at: pos[i],
                    rom_offset: r,
                    len: n,
                });
                i = end;
            }
            _ => i += 1,
        }
    }
}

fn hex_pair(a: u8, b: u8) -> u8 {
    let d = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
    d(a) << 4 | d(b)
}

/// A byte-sized integer literal: decimal, 0x or $ hex, or 0b binary.
fn parse_byte(w: &str) -> Option<u8> {
    let v = if let Some(h) = w.strip_prefix("0x").or_else(|| w.strip_prefix('$')) {
        u32::from_str_radix(&h.replace('_', ""), 16).ok()?
    } else if let Some(bits) = w.strip_prefix("0b") {
        u32::from_str_radix(&bits.replace('_', ""), 2).ok()?
    } else if w.bytes().all(|c| c.is_ascii_digit()) {
        w.parse::<u32>().ok()?
    } else {
        return None;
    };
    u8::try_from(v).ok()
}

/// Two byte streams read from a text with everything between byte tokens
/// ignored, so a table split over lines, quoted, or written with directives
/// between its rows still reads as one run. `numbers` reads a word as a
/// byte-sized literal where it can and otherwise as hex digit pairs; `hex`
/// reads every word of hex digits as pairs, as in a hex dump. Each byte comes
/// with its offset in the file.
fn loose_streams(text: &str) -> [(&'static str, Vec<u8>, Vec<usize>); 2] {
    let b = text.as_bytes();
    let mut nums = (Vec::new(), Vec::new());
    let mut hex = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if !(c.is_ascii_alphanumeric() || c == b'$' || c == b'_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'$' || b[i] == b'_') {
            i += 1;
        }
        let word = &text[start..i];
        let bare = word
            .strip_suffix("_u8")
            .or_else(|| word.strip_suffix("u8"))
            .or_else(|| word.strip_suffix("_i8"))
            .or_else(|| word.strip_suffix("i8"))
            .unwrap_or(word);
        let digits_at = if bare.starts_with("0x") || bare.starts_with("0X") {
            2
        } else {
            usize::from(bare.starts_with('$'))
        };
        let digits = &bare[digits_at..];
        let pairs = !digits.is_empty()
            && digits.len().is_multiple_of(2)
            && digits.bytes().all(|d| d.is_ascii_hexdigit());
        let push_pairs = |out: &mut (Vec<u8>, Vec<usize>)| {
            let d = digits.as_bytes();
            for k in (0..d.len()).step_by(2) {
                out.0.push(hex_pair(d[k], d[k + 1]));
                out.1.push(start + digits_at + k);
            }
        };
        if pairs {
            push_pairs(&mut hex);
        }
        match parse_byte(bare) {
            Some(v) => {
                nums.0.push(v);
                nums.1.push(start);
            }
            None if pairs => push_pairs(&mut nums),
            None => {}
        }
    }
    [("numbers", nums.0, nums.1), ("hex", hex.0, hex.1)]
}

/// Runs shared with the ROM in a text's loose streams, each reported once.
fn shared_in_text(text: &str, rom: &[u8], windows: &HashMap<&[u8], Vec<usize>>) -> Vec<Shared> {
    let mut out: Vec<Shared> = Vec::new();
    for (how, seq, pos) in loose_streams(text) {
        let mut found = Vec::new();
        shared_runs(&seq, &pos, rom, windows, how, &mut found);
        for f in found {
            if !out
                .iter()
                .any(|o| o.rom_offset == f.rom_offset && o.len == f.len)
            {
                out.push(f);
            }
        }
    }
    out
}

/// Every file of a folder, by its path relative to the folder's parent, with
/// symlinks and anything under a `sealed` folder left out.
fn folder_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let base = dir.parent().unwrap_or(dir);
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let e = e?;
            let path = e.path();
            let ft = e.file_type()?;
            if ft.is_dir() {
                if !e.file_name().eq_ignore_ascii_case("sealed") {
                    stack.push(path);
                }
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes =
                    fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                out.push((rel, bytes));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The open version's built files, by `built NAME`.
fn built_open(pkg: &Package) -> Result<Vec<(String, Vec<u8>)>> {
    let Some(open) = &pkg.open else {
        return Ok(Vec::new());
    };
    let built = Authored::load(&open.dir)
        .map_err(|e| anyhow::anyhow!("{}: {e}", open.dir.display()))?
        .build()
        .map_err(|e| anyhow::anyhow!("{}: {e}", open.dir.display()))?;
    let map = AssetMap::parse(&built.map_text).context("the built asset map")?;
    let (files, _) = gp_assets::lift(&built.image, &map, &built.map_text)?;
    Ok(files
        .into_iter()
        .map(|f| (format!("built {}", f.name), f.bytes))
        .collect())
}

fn problem(file: &str, message: impl Into<String>) -> Value {
    json!({ "file": file, "message": message.into() })
}

/// One ROM's lifted files and what a copy of them is found by.
struct Original {
    id: String,
    lifted: usize,
    made: BTreeMap<String, String>,
    lines: HashMap<String, String>,
}

impl Original {
    fn check(
        &self,
        rel: &str,
        bytes: &[u8],
        data: &[u8],
        windows: &HashMap<&[u8], Vec<usize>>,
        problems: &mut Vec<Value>,
    ) {
        if bytes.len() >= 4 && looks_like_nes(&bytes[..4]) {
            problems.push(problem(rel, "starts with an iNES header: a game image"));
        }
        if let Some(what) = self.made.get(&sha256_hex(bytes)) {
            problems.push(problem(rel, format!("identical to {what} of {}", self.id)));
        }
        let mut shared = Vec::new();
        let pos: Vec<usize> = (0..bytes.len()).collect();
        shared_runs(bytes, &pos, data, windows, "raw", &mut shared);
        if let Ok(t) = std::str::from_utf8(bytes) {
            shared.extend(shared_in_text(t, data, windows));
            let mut held: BTreeSet<&str> = BTreeSet::new();
            let mut count = 0usize;
            for line in t.lines().map(str::trim) {
                if let Some(n) = self.lines.get(line) {
                    held.insert(n);
                    count += 1;
                }
            }
            if !held.is_empty() {
                problems.push(problem(
                    rel,
                    format!(
                        "holds {count} lines of {}'s lifted files {}",
                        self.id,
                        held.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                ));
            }
        }
        for s in shared {
            problems.push(problem(
                rel,
                format!(
                    "{} bytes of {} from offset {:#x} are here in order ({}), from byte {}",
                    s.len, self.id, s.rom_offset, s.how, s.at
                ),
            ));
        }
    }
}

pub fn served(
    dir: &Path,
    folders: &[PathBuf],
    given: &[PathBuf],
    paths: &Paths,
) -> Result<Outcome> {
    let pkg = Package::load(dir)?;
    let mut files = built_open(&pkg)?;
    for f in folders {
        files.extend(folder_files(f)?);
    }
    if files.is_empty() {
        bail!(
            "{} has no open version and no --folder was given, so there is nothing to check",
            dir.display()
        );
    }
    let bytes_total: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
    let found = crate::roms::find(&pkg.rom_hashes(), given, &paths.roms)?;

    let mut problems = Vec::new();
    let mut text = String::new();
    let mut missing = Vec::new();
    for e in &pkg.entries {
        let Some(rom) = found.get(&e.sha256) else {
            missing.push(e.id.clone());
            continue;
        };
        let map_text = fs::read_to_string(pkg.dir.join(&e.asset_map))
            .with_context(|| format!("{}: reading its asset map", e.id))?;
        let map = AssetMap::parse(&map_text)?;
        let (lifted, _) = gp_assets::lift(&rom.bytes, &map, &map_text)?;
        let mut original = Original {
            id: e.id.clone(),
            lifted: lifted.len(),
            made: BTreeMap::new(),
            lines: HashMap::new(),
        };
        for f in &lifted {
            // The map's copy is the recipe, which a page may carry.
            if f.name == gp_assets::MAP_FILE {
                continue;
            }
            original
                .made
                .insert(sha256_hex(&f.bytes), format!("the lifted file {}", f.name));
            if let Ok(t) = std::str::from_utf8(&f.bytes) {
                for line in t.lines().map(str::trim).filter(|l| is_data_line(l)) {
                    original
                        .lines
                        .entry(line.to_owned())
                        .or_insert_with(|| f.name.clone());
                }
            }
        }
        let data = rom.data();
        let windows = rom_windows(data);
        for (rel, bytes) in &files {
            original.check(rel, bytes, data, &windows, &mut problems);
        }
        let _ = writeln!(
            text,
            "{}: {} files, {bytes_total} bytes, against {} ({} lifted files)",
            e.id,
            files.len(),
            rom.label(&e.id),
            original.lifted
        );
    }
    if !missing.is_empty() {
        let _ = writeln!(
            text,
            "NOT COMPARED with {}: no ROM with its hash under $GP_ROM_DIR, so nothing here says these files are free of its data",
            missing.join(", ")
        );
    }
    let mut out = String::new();
    for p in &problems {
        let _ = writeln!(
            out,
            "{}: {}",
            p["file"].as_str().unwrap_or(""),
            p["message"].as_str().unwrap_or("")
        );
    }
    out.push_str(&text);
    let verdict = if !problems.is_empty() {
        "holds game data"
    } else if !missing.is_empty() {
        "not checked against every ROM"
    } else {
        "holds no game data"
    };
    let _ = write!(out, "{}: {verdict}", dir.display());
    let exit = if !problems.is_empty() {
        1
    } else {
        u8::from(!missing.is_empty()) * 2
    };
    Ok(Outcome {
        json: json!({
            "ok": exit == 0,
            "files": files.len(),
            "problems": problems,
            "not_compared": missing,
        }),
        text: out,
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up ROM data with no repeated windows.
    fn rom() -> Vec<u8> {
        let mut x: u32 = 0x1234_5678;
        (0..8192)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn a_run_of_rom_bytes_is_found_raw_and_as_numbers() {
        let data = rom();
        let windows = rom_windows(&data);
        let mut shared = Vec::new();
        let run: Vec<u8> = data[100..140].to_vec();
        let pos: Vec<usize> = (0..run.len()).collect();
        shared_runs(&run, &pos, &data, &windows, "raw", &mut shared);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].rom_offset, 100);
        let text: Vec<String> = run.iter().map(|b| format!("0x{b:02X}")).collect();
        let found = shared_in_text(&text.join(", "), &data, &windows);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rom_offset, 100);
    }

    #[test]
    fn a_short_run_counts_for_nothing() {
        let data = rom();
        let windows = rom_windows(&data);
        let text: Vec<String> = data[100..131]
            .iter()
            .map(|b| format!("0x{b:02X}"))
            .collect();
        assert!(shared_in_text(&text.join(", "), &data, &windows).is_empty());
    }
}
