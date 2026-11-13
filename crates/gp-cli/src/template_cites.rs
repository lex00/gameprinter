//! `gp template cites`: the ROM addresses a template's model, state
//! mapping, probes, delta and hooks cite, each with the routine of the code
//! map it is in, so a planner can see which of a level's routines the
//! template already covers (`gp reach --window ... --cites TEMPLATE`
//! counts them per routine).
//!
//! A citation is an address of $8000 or more written `$C46B` or `0xC46B`
//! in one of those files. The hooks are the `[ext] crate` of the template,
//! or of the template it is derived from (`[derive] from`).
//!
//! On an arcade romset the program starts at $0000, so a citation there is
//! any such address inside the main CPU's program (below its RAM), placed
//! in the routines of gp-cli's Z80 code map (crate::code_z80), seeded from
//! an idle run of 1800 frames from power-on.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use std::fmt::Write as _;

use anyhow::{Context, Result};
use gp_evidence::Paths;
use gp_nes::codemap::{Va, va_hex};
use serde_json::{Value, json};
use toml::Value as Toml;

use crate::Outcome;

use crate::code::CodeMap;

/// The template's files a citation counts in, besides its hooks' sources:
/// its model, state mapping, probes and delta.
const FILES: &[&str] = &["model.toml", "mapping.toml", "probes.toml", "delta.toml"];

/// Every ROM address the template cites: by CPU address, each file (as
/// `TEMPLATE/FILE`) and line that cites it.
pub(crate) type Index = BTreeMap<u16, BTreeSet<(String, usize)>>;

/// The files of `dir` that count, the hooks' sources included.
pub(crate) fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = FILES
        .iter()
        .map(|f| dir.join(f))
        .filter(|p| p.is_file())
        .collect();
    let manifest: Toml = toml::from_str(
        &fs::read_to_string(dir.join("template.toml"))
            .with_context(|| format!("reading {}/template.toml", dir.display()))?,
    )?;
    let ext = |d: &Path, m: &Toml| -> Option<PathBuf> {
        let c = m.get("ext")?.get("crate")?.as_str()?;
        Some(d.join(c).join("src"))
    };
    let src = ext(dir, &manifest).or_else(|| {
        let from = manifest.get("derive")?.get("from")?.as_str()?;
        let base = dir.join(from);
        let m: Toml = toml::from_str(&fs::read_to_string(base.join("template.toml")).ok()?).ok()?;
        ext(&base, &m)
    });
    if let Some(src) = src
        && src.is_dir()
    {
        let mut rs = Vec::new();
        walk(&src, &mut rs)?;
        rs.retain(|p| p.extension().is_some_and(|e| e == "rs"));
        rs.sort();
        out.extend(rs);
    }
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// The addresses of $8000 or more a text writes as `$XXXX` or `0xXXXX`,
/// with their line numbers (from 1).
#[cfg(test)]
pub(crate) fn addresses(text: &str) -> Vec<(u16, usize)> {
    addresses_from(text, 0x8000)
}

/// The addresses of `lo` or more a text writes as `$XXXX` or `0xXXXX`.
fn addresses_from(text: &str, lo: u16) -> Vec<(u16, usize)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let skip = match b[i] {
                b'$' => 1,
                b'0' if b.get(i + 1).is_some_and(|c| *c == b'x' || *c == b'X') => 2,
                _ => 0,
            };
            // The prefix must start a token: `a$C000` and `10x8000` don't.
            let starts = i == 0 || !b[i - 1].is_ascii_alphanumeric();
            if skip == 0 || !starts {
                i += 1;
                continue;
            }
            let h = i + skip;
            let mut j = h;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            let ends = j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_');
            if j - h == 4
                && ends
                && let Ok(a) = u16::from_str_radix(&line[h..j], 16)
                && a >= lo
            {
                out.push((a, n + 1));
            }
            i = j.max(i + 1);
        }
    }
    out
}

/// The index of every template in `dirs`.
pub(crate) fn index(dirs: &[PathBuf]) -> Result<(Index, Vec<String>)> {
    index_from(dirs, 0x8000)
}

/// The index of every template in `dirs`, of addresses from `lo`.
fn index_from(dirs: &[PathBuf], lo: u16) -> Result<(Index, Vec<String>)> {
    let mut idx = Index::new();
    let mut names = Vec::new();
    for dir in dirs {
        // Each file is named from the folder that holds the template, so
        // a base's hooks read `base/ext/src/lib.rs` from either template.
        let real = fs::canonicalize(dir).with_context(|| format!("reading {}", dir.display()))?;
        let top = real.parent().unwrap_or(&real).to_path_buf();
        for f in files(dir)? {
            let rf = fs::canonicalize(&f).unwrap_or_else(|_| f.clone());
            let label = rf.strip_prefix(&top).unwrap_or(&rf).display().to_string();
            if names.contains(&label) {
                continue;
            }
            let text =
                fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
            for (a, line) in addresses_from(&text, lo) {
                idx.entry(a).or_default().insert((label.clone(), line));
            }
            names.push(label);
        }
    }
    Ok((idx, names))
}

/// For each routine of the map, the cited addresses in it; and the cited
/// addresses in no routine.
pub(crate) fn by_routine(idx: &Index, map: &CodeMap) -> (BTreeMap<usize, Vec<u16>>, Vec<u16>) {
    let mut by: BTreeMap<usize, Vec<u16>> = BTreeMap::new();
    let mut none = Vec::new();
    for &a in idx.keys() {
        match map.routine_of(Va::from(a)) {
            Some(i) => by.entry(i).or_default().push(a),
            None => none.push(a),
        }
    }
    (by, none)
}

/// The ROM the cites are placed in: the one given, else the first
/// template's accepted ROM in $GP_ROM_DIR (an NES image or a romset).
/// Named by its path, or by its romset or hash when found by hash.
fn template_rom(
    dirs: &[PathBuf],
    rom: Option<&Path>,
    paths: &Paths,
) -> Option<(String, crate::roms::Rom)> {
    if let Some(r) = rom {
        return Some((r.display().to_string(), crate::roms::Rom::read(r).ok()?));
    }
    let d = dirs.first()?;
    let t: Toml = toml::from_str(&fs::read_to_string(d.join("template.toml")).ok()?).ok()?;
    if let Ok(p) = crate::model_static::template_rom(d, &t, paths) {
        return Some((p.display().to_string(), crate::roms::Rom::read(&p).ok()?));
    }
    let accepts: Vec<String> = t
        .get("source")?
        .get("accepts")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect();
    let mut found = crate::roms::find(&accepts, &[], &paths.roms).ok()?;
    accepts.iter().find_map(|a| {
        let r = found.remove(a)?;
        Some((r.system.clone().unwrap_or_else(|| r.sha256.clone()), r))
    })
}

/// Frames of the idle run that seeds an arcade code map.
const IDLE_FRAMES: usize = 1800;

/// A routine as the report prints it: name, element, start and size.
struct Placed {
    name: String,
    element: String,
    at: String,
    size: u32,
}

/// `gp template cites`.
pub(crate) fn run(dirs: &[PathBuf], rom: Option<&Path>, paths: &Paths) -> Result<Outcome> {
    let found = template_rom(dirs, rom, paths);
    let arcade = found
        .as_ref()
        .is_some_and(|(_, r)| r.platform == crate::roms::Platform::Arcade);
    let set = match &found {
        Some((_, r)) if arcade => Some(gp_arcade::Romset::from_zip(&r.bytes)?),
        _ => None,
    };
    let (mut idx, files) = if arcade {
        index_from(dirs, 0)?
    } else {
        index(dirs)?
    };
    if let Some(set) = &set {
        // The main CPU's program; what is above it is RAM and the board.
        let end = crate::code_z80::program(set, 0).1;
        idx.retain(|a, _| u32::from(*a) < end);
    }
    let mut per_file: BTreeMap<&str, BTreeSet<u16>> = BTreeMap::new();
    for (a, cites) in &idx {
        for (f, _) in cites {
            per_file.entry(f.as_str()).or_default().insert(*a);
        }
    }
    let mut text = format!(
        "{} ROM addresses cited in {} files ({})\n",
        idx.len(),
        files.len(),
        per_file
            .iter()
            .map(|(f, s)| format!("{f} {}", s.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let cite_text = |a: u16| -> String {
        let mut by: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (f, l) in &idx[&a] {
            by.entry(f.as_str()).or_default().push(*l);
        }
        by.iter()
            .map(|(f, ls)| {
                let ls: Vec<String> = ls.iter().map(usize::to_string).collect();
                format!("{f}:{}", ls.join(","))
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut doc = json!({
        "templates": dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>(),
        "files": files,
        "addresses": idx.iter().map(|(a, c)| (format!("{a:04X}"), c.iter().map(|(f, l)| format!("{f}:{l}")).collect::<Vec<_>>())).collect::<BTreeMap<_, _>>(),
    });
    let Some((label, rom)) = found else {
        text.push_str("(no ROM found: give --rom to place them in routines)\n");
        for a in idx.keys() {
            let _ = writeln!(text, "{a:04X}  {}", cite_text(*a));
        }
        return Ok(Outcome::ok(doc, text.trim_end().to_owned()));
    };
    // Each routine of the code map, and the cited addresses in it.
    let (routines, by, none): (Vec<Placed>, BTreeMap<usize, Vec<u16>>, Vec<u16>) = match &set {
        None => {
            let map = crate::code::CodeMap::of(&rom.bytes, &[])?;
            let (by, none) = by_routine(&idx, &map);
            let rs = map
                .routines
                .iter()
                .map(|r| Placed {
                    name: r.name.clone(),
                    element: r.element.to_string(),
                    at: va_hex(r.at),
                    size: r.size,
                })
                .collect();
            (rs, by, none)
        }
        Some(set) => {
            // The descent from reset alone misses what the interrupt
            // reaches, so the map is seeded from an idle run from power-on
            // (the attract mode), as the NES map is from its vectors.
            let idle = vec![gp_evidence::input_log::InputFrame::default(); IDLE_FRAMES];
            let lv = crate::code_z80::run_input(set, &idle)?;
            z80_by_routine(
                &idx,
                &crate::code_z80::CodeMap::of(set, 0, Some(&lv.cpus[0])),
            )
        }
    };
    let _ = writeln!(
        text,
        "{} of {} routines of the code map hold a cited address; {} addresses are in no routine (tables and data)",
        by.len(),
        routines.len(),
        none.len()
    );
    let mut rows = Vec::new();
    for (&i, addrs) in &by {
        let r = &routines[i];
        let _ = writeln!(
            text,
            "\n{} ({}) {}, size {}",
            r.name, r.element, r.at, r.size
        );
        for a in addrs {
            let _ = writeln!(text, "  {a:04X}  {}", cite_text(*a));
        }
        rows.push(json!({
            "routine": r.name, "addr": r.at, "element": r.element,
            "cited": addrs.iter().map(|a| format!("{a:04X}")).collect::<Vec<_>>(),
        }));
    }
    if !none.is_empty() {
        let _ = writeln!(text, "\nIn no routine:");
        for a in &none {
            let _ = writeln!(text, "  {a:04X}  {}", cite_text(*a));
        }
    }
    doc["rom"] = Value::String(label);
    doc["routines_cited"] = json!(by.len());
    doc["routines"] = json!(routines.len());
    doc["by_routine"] = json!(rows);
    doc["in_no_routine"] = json!(none.iter().map(|a| format!("{a:04X}")).collect::<Vec<_>>());
    Ok(Outcome::ok(doc, text.trim_end().to_owned()))
}

/// The cites placed in the Z80 code map's routines: a routine runs from its
/// entry to the next one, and holds an address when one of its
/// instructions covers it.
fn z80_by_routine(
    idx: &Index,
    map: &crate::code_z80::CodeMap,
) -> (Vec<Placed>, BTreeMap<usize, Vec<u16>>, Vec<u16>) {
    let entries: Vec<(u16, &str)> = map.routines.iter().map(|(a, w)| (*a, *w)).collect();
    let end = |i: usize| -> u32 {
        entries
            .get(i + 1)
            .map_or(map.rom.len() as u32, |(a, _)| u32::from(*a))
    };
    let routines = entries
        .iter()
        .enumerate()
        .map(|(i, (at, why))| Placed {
            name: format!("Z80_{at:04X}"),
            element: (*why).to_owned(),
            at: format!("${at:04X}"),
            size: end(i) - u32::from(*at),
        })
        .collect();
    let covered = |a: u16| {
        map.insns
            .range(..=a)
            .next_back()
            .is_some_and(|(pc, i)| u32::from(a) < u32::from(*pc) + u32::from(i.len))
    };
    let mut by: BTreeMap<usize, Vec<u16>> = BTreeMap::new();
    let mut none = Vec::new();
    for &a in idx.keys() {
        let i = entries.partition_point(|(at, _)| *at <= a).checked_sub(1);
        match i {
            Some(i) if covered(a) => by.entry(i).or_default().push(a),
            _ => none.push(a),
        }
    }
    (routines, by, none)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_as_tokens_of_four_hex_digits_from_8000() {
        let text = "a = \"$C46B\" # $53, $0200\nlet t = 0xE7A3; // not 0x12, not $C46B0 or x$D000\n$8000-$80FF, 0xFFFF";
        let got = addresses(text);
        assert_eq!(
            got,
            vec![
                (0xC46B, 1),
                (0xE7A3, 2),
                (0x8000, 3),
                (0x80FF, 3),
                (0xFFFF, 3)
            ]
        );
    }

    #[test]
    fn a_derived_template_counts_its_bases_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let derived = tmp.path().join("derived");
        fs::create_dir_all(base.join("ext/src")).unwrap();
        fs::create_dir_all(&derived).unwrap();
        fs::write(base.join("template.toml"), "[ext]\ncrate = \"ext\"\n").unwrap();
        fs::write(base.join("model.toml"), "# reads $D2FD\n").unwrap();
        fs::write(base.join("ext/src/lib.rs"), "// the arc, $EF72\n").unwrap();
        fs::write(
            derived.join("template.toml"),
            "[derive]\nfrom = \"../base\"\n",
        )
        .unwrap();
        fs::write(derived.join("delta.toml"), "# $8A71 hooks the NMI\n").unwrap();
        fs::write(derived.join("clips.toml"), "# $9999 is not counted\n").unwrap();
        let (idx, files) = index(std::slice::from_ref(&derived)).unwrap();
        assert_eq!(
            idx.keys().copied().collect::<Vec<_>>(),
            vec![0x8A71, 0xEF72]
        );
        assert_eq!(files, vec!["derived/delta.toml", "base/ext/src/lib.rs"]);
        // Both templates together count the base's hooks once.
        let (_, both) = index(&[base, derived]).unwrap();
        assert_eq!(
            both,
            vec![
                "base/model.toml",
                "base/ext/src/lib.rs",
                "derived/delta.toml"
            ]
        );
    }
}
