//! `gp template cites`: the ROM addresses a template's model, state
//! mapping, probes, delta and hooks cite, each with the routine of the code
//! map it is in, so a planner can see which of a level's routines the
//! template already covers (`gp reach --window ... --cites TEMPLATE`
//! counts them per routine).
//!
//! Here are the index and its placing by routine, which `gp reach --cites`
//! reads; the `gp template cites` command itself comes with wave 9 of the
//! port from game-printer.
//!
//! A citation is an address of $8000 or more written `$C46B` or `0xC46B`
//! in one of those files. The hooks are the `[ext] crate` of the template,
//! or of the template it is derived from (`[derive] from`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gp_nes::codemap::Va;
use toml::Value as Toml;

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
pub(crate) fn addresses(text: &str) -> Vec<(u16, usize)> {
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
                && a >= 0x8000
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
            for (a, line) in addresses(&text) {
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
