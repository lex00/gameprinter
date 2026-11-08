//! `gp template check`: a ROM held to a template's pins: the ROM's hash,
//! the asset map's hash and the digest of what the map lifts.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use gp_assets::{AssetMap, RomLayout};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::Outcome;

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The SHA-256 of `sha256sum`'s output for the files, sorted by name.
pub fn lifted_digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut lines = String::new();
    for (name, bytes) in files {
        let _ = writeln!(lines, "{}  {name}", sha256_hex(bytes));
    }
    sha256_hex(lines.as_bytes())
}

/// A lifted text line at least this long found in another file means a
/// lifted asset was copied into it.
pub const LIFTED_LINE: usize = 24;

/// Whether a line only names something (`asset = "palette_ground"`), as a
/// lifted file's header does and a model's slot recipe may too.
fn names_a_name(line: &str) -> bool {
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    let word = |w: &str| {
        !w.is_empty()
            && w.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    };
    let value = value.trim();
    word(key.trim())
        && (word(value)
            || value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .is_some_and(word))
}

/// Whether a trimmed line of a lifted file is one a copy can be known by:
/// long enough, not a comment and not a key naming a name.
pub fn is_data_line(line: &str) -> bool {
    line.len() >= LIFTED_LINE && !line.starts_with('#') && !names_a_name(line)
}

/// The lines of one set of files that a set of lifted files holds too, by
/// [`is_data_line`]. Each as (our file, the line, their file).
pub fn shared_lines(
    ours: &BTreeMap<String, String>,
    theirs: &BTreeMap<String, String>,
) -> Vec<(String, String, String)> {
    let mut index: HashMap<&str, &str> = HashMap::new();
    for (name, text) in theirs {
        if name == gp_assets::MAP_FILE {
            continue;
        }
        for line in text.lines().map(str::trim).filter(|l| is_data_line(l)) {
            index.entry(line).or_insert(name);
        }
    }
    let mut out = Vec::new();
    for (name, text) in ours {
        if name == gp_assets::MAP_FILE {
            continue;
        }
        for line in text.lines().map(str::trim) {
            if let Some(t) = index.get(line) {
                out.push((name.clone(), line.to_owned(), (*t).to_owned()));
            }
        }
    }
    out
}

pub fn check(dir: &Path, rom_path: &Path) -> Result<Outcome> {
    let manifest: toml::Table = fs::read_to_string(dir.join("template.toml"))
        .context("reading template.toml")?
        .parse()
        .context("parsing template.toml")?;
    let text = |sect: &str, key: &str| -> Result<String> {
        manifest
            .get(sect)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("template.toml has no {sect}.{key}"))
    };
    let accepts: Vec<String> = manifest
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let map_pin = text("recipe", "asset_map_sha256")?;
    let lifted_pin = text("recipe", "lifted_sha256")?;
    let map_path = dir.join(text("recipe", "asset_map")?);

    let rom = fs::read(rom_path).with_context(|| rom_path.display().to_string())?;
    let layout = RomLayout::of(&rom)?;
    let map_text = fs::read_to_string(&map_path).with_context(|| map_path.display().to_string())?;
    let map = AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?;
    let (files, _) = gp_assets::lift(&rom, &map, &map_text)?;
    let files: BTreeMap<String, Vec<u8>> = files.into_iter().map(|f| (f.name, f.bytes)).collect();

    let map_sha = sha256_hex(map_text.as_bytes());
    let lifted = lifted_digest(&files);
    let steps = [
        (
            "rom",
            accepts.contains(&layout.sha256),
            layout.sha256.clone(),
        ),
        ("asset map", map_sha == map_pin, map_sha.clone()),
        ("lifted", lifted == lifted_pin, lifted.clone()),
    ];
    let ok = steps.iter().all(|s| s.1);
    let mut out = format!("{}:", dir.display());
    for (name, pass, value) in &steps {
        let _ = write!(
            out,
            "\n  {name:<10} {}{value}",
            if *pass { "" } else { "FAILED: " }
        );
    }
    Ok(Outcome {
        json: json!({ "ok": ok, "rom_sha256": layout.sha256, "asset_map_sha256": map_sha, "lifted_sha256": lifted, "files": files.len() }),
        text: out,
        exit: u8::from(!ok),
    })
}

/// `gp template build`: an authored template's lifted files, and its pins.
pub fn build(dir: &Path, out: &Path) -> Result<Outcome> {
    let built = gp_assets::authored::Authored::load(dir)
        .map_err(|e| anyhow!("{e}"))?
        .build()
        .map_err(|e| anyhow!("{e}"))?;
    let out = std::path::absolute(out)?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&out) {
        anyhow::bail!(
            "{} is inside the git work tree at {}",
            out.display(),
            repo.display()
        );
    }
    let map = AssetMap::parse(&built.map_text).context("the built asset map")?;
    let (files, _) = gp_assets::lift(&built.image, &map, &built.map_text)?;
    let files: BTreeMap<String, Vec<u8>> = files.into_iter().map(|f| (f.name, f.bytes)).collect();
    for (name, bytes) in &files {
        let p = out.join(name);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(p, bytes)?;
    }
    let digest = lifted_digest(&files);
    let manifest: toml::Table = fs::read_to_string(dir.join("template.toml"))?.parse()?;
    let pin = manifest
        .get("recipe")
        .and_then(|r| r.get("lifted_sha256"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ok = pin == digest;
    Ok(Outcome {
        json: json!({ "ok": ok, "lifted_sha256": digest, "files": files.len() }),
        text: format!(
            "{} files, lifted {}{digest}",
            files.len(),
            if ok { "" } else { "FAILED: " }
        ),
        exit: u8::from(!ok),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_original_sharing_a_data_line_with_its_base_is_found() {
        let files = |t: &str| {
            std::collections::BTreeMap::from([
                ("shapes.toml".to_owned(), t.to_owned()),
                (
                    gp_assets::MAP_FILE.to_owned(),
                    "  [0x00, 0x00, 0x18, 0x00],".to_owned(),
                ),
            ])
        };
        // Made-up rows: the base's, ours with one of them, and ours apart.
        let base = files(
            "schema = 1\nasset = \"shapes\"\nrows = [\n  [0x00, 0x00, 0x2A, 0x00],\n  [0x00, 0x00, 0x07, 0x00],\n]\n",
        );
        let copied =
            files("schema = 1\nasset = \"shapes\"\nrows = [\n  [0x00, 0x00, 0x2A, 0x00],\n]\n");
        let own =
            files("schema = 1\nasset = \"shapes\"\nrows = [\n  [0x00, 0x03, 0x2A, 0x03],\n]\n");
        let found = super::shared_lines(&copied, &base);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, "[0x00, 0x00, 0x2A, 0x00],");
        // Format lines (schema, the asset's name, brackets) aren't data.
        assert!(super::shared_lines(&own, &base).is_empty());
    }
}
