//! `gp template build`: an original template's assets, built from the
//! data the template holds (`authored.toml`) into a data image, its asset
//! map and a lifted directory the interpreter and every asset tool read.
//! With `--write`, a stale asset map and template.toml's pins are
//! rewritten.
//!
//! Ported from game-printer's template_build.rs. Changed by gameprinter's
//! layout: the built directory's slots.toml (the MCP slot catalog, wave
//! 7's gp-mcp) isn't written yet, and the run recorder is gone.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_assets::AssetMap;
use gp_assets::authored::Authored;
use gp_evidence::Paths;

use crate::Outcome;
use crate::template::sha256_hex;

/// The authored file's name in a template directory.
pub const AUTHORED: &str = gp_assets::authored::AUTHORED_FILE;

/// Where an original's data image is written: outside the repo, beside
/// the other work files.
pub(crate) fn image_path(paths: &Paths, source: &str) -> PathBuf {
    paths
        .home
        .join("work")
        .join("images")
        .join(format!("{source}.nes"))
}

/// Refuses a path inside a git work tree: built assets stay outside.
fn outside_repo(path: &Path) -> Result<PathBuf> {
    let path =
        std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&path) {
        bail!(
            "{} is inside the git work tree at {}; built assets stay outside any repo (by default in $GP_HOME/work/assets)",
            path.display(),
            repo.display()
        );
    }
    Ok(path)
}

/// Writes a file, refusing to follow a symlink there.
pub(crate) fn write_plain(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp template writes only plain files",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// Rewrites one `key = ...` line of a manifest's table, keeping the rest.
fn set_key(text: &str, table: &str, key: &str, value: &str) -> Result<String> {
    let mut out = String::new();
    let mut current = String::new();
    let mut done = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            current = t.trim_matches(|c| c == '[' || c == ']').to_owned();
        }
        if !done && current == table && t.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            out.push_str(&format!("{key} = {value}\n"));
            done = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !done {
        bail!("template.toml has no {table}.{key} line to update");
    }
    Ok(out)
}

pub(crate) fn build_cmd(
    dir: &Path,
    out: Option<&Path>,
    write: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let manifest_path = dir.join("template.toml");
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: toml::Table = manifest_text
        .parse()
        .with_context(|| manifest_path.display().to_string())?;
    let get = |table: &str, key: &str| {
        manifest
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    if get("source", "how").as_deref() != Some("authored") {
        bail!(
            "{} isn't an original template (source.how = \"authored\"); gp template build builds only those",
            dir.display()
        );
    }
    let id = get("template", "id").unwrap_or_default();
    let map_rel = get("recipe", "asset_map").context("template.toml has no recipe.asset_map")?;
    let authored = Authored::load(dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    let built = authored.build().map_err(|e| anyhow::anyhow!("{e}"))?;
    let map = AssetMap::parse(&built.map_text).context("the built asset map")?;
    let (files, lifted) = gp_assets::lift(&built.image, &map, &built.map_text)?;
    let by_name: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|f| (f.name.clone(), f.bytes.clone()))
        .collect();
    let lifted_sha256 = crate::template::lifted_digest(&by_name);
    let map_sha256 = sha256_hex(built.map_text.as_bytes());

    // The pins: the committed asset map and template.toml's hashes.
    let map_path = dir.join(&map_rel);
    let mut stale = Vec::new();
    if fs::read_to_string(&map_path).ok().as_deref() != Some(built.map_text.as_str()) {
        stale.push(map_rel.clone());
    }
    let accepts: Vec<String> = manifest
        .get("source")
        .and_then(|s| s.get("accepts"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if accepts != [built.sha256.clone()] {
        stale.push("source.accepts".into());
    }
    if get("recipe", "asset_map_sha256").as_deref() != Some(map_sha256.as_str()) {
        stale.push("recipe.asset_map_sha256".into());
    }
    if get("recipe", "lifted_sha256").as_deref() != Some(lifted_sha256.as_str()) {
        stale.push("recipe.lifted_sha256".into());
    }
    if write && !stale.is_empty() {
        fs::write(&map_path, &built.map_text)
            .with_context(|| format!("writing {}", map_path.display()))?;
        let mut t = set_key(
            &manifest_text,
            "source",
            "accepts",
            &format!("[\"{}\"]", built.sha256),
        )?;
        t = set_key(
            &t,
            "recipe",
            "asset_map_sha256",
            &format!("\"{map_sha256}\""),
        )?;
        t = set_key(
            &t,
            "recipe",
            "lifted_sha256",
            &format!("\"{lifted_sha256}\""),
        )?;
        fs::write(&manifest_path, &t)
            .with_context(|| format!("writing {}", manifest_path.display()))?;
    }

    // The lifted directory, outside the repo.
    let assets_dir: PathBuf = match out {
        Some(o) => o.to_path_buf(),
        None => paths
            .home
            .join("work")
            .join("assets")
            .join(&authored.source),
    };
    let assets_dir = outside_repo(&assets_dir)?;
    fs::create_dir_all(&assets_dir)
        .with_context(|| format!("creating {}", assets_dir.display()))?;
    for f in &files {
        write_plain(&assets_dir.join(&f.name), &f.bytes)?;
    }
    // The image, for the asset tools.
    let image_path = image_path(paths, &authored.source);
    write_plain(&image_path, &built.image)?;

    let ok = write || stale.is_empty();
    let mut text = format!(
        "built {id} from {AUTHORED}: {} assets, {} PRG and {} CHR bytes, image sha256 {}\n  image   {}\n  assets  {}",
        lifted.len(),
        built.prg_used,
        built.chr_used,
        built.sha256,
        image_path.display(),
        assets_dir.display()
    );
    match (stale.is_empty(), write) {
        (true, _) => text.push_str("\n  pins    current"),
        (false, true) => text.push_str(&format!("\n  updated {}", stale.join(", "))),
        (false, false) => text.push_str(&format!(
            "\n  stale   {} (run with --write to update them)",
            stale.join(", ")
        )),
    }
    Ok(Outcome {
        json: json!({
            "ok": ok,
            "id": id,
            "dir": dir,
            "sha256": built.sha256,
            "image": image_path,
            "assets": assets_dir,
            "asset_map_sha256": map_sha256,
            "lifted_sha256": lifted_sha256,
            "stale": stale,
            "written": write && !stale.is_empty(),
        }),
        text,
        exit: if ok { 0 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_key_rewrites_only_its_tables_key() {
        let t = "[source]\naccepts = [\"a\"]\n\n[recipe]\nasset_map_sha256 = \"x\"\naccepts = 1\n";
        let out = set_key(t, "source", "accepts", "[\"b\"]").unwrap();
        assert_eq!(
            out,
            "[source]\naccepts = [\"b\"]\n\n[recipe]\nasset_map_sha256 = \"x\"\naccepts = 1\n"
        );
        assert!(set_key(t, "template", "id", "\"z\"").is_err());
    }
}
