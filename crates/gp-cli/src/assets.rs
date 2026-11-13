//! `gp assets`: lift a ROM's assets into files and pack them back, with a
//! byte-for-byte check, or into a copy of a base ROM. The formats are in
//! `gp-assets`; this module finds the asset map, reads and writes the files
//! and reports.
//!
//! Ported from game-printer's assets.rs. Changed by gameprinter's layout:
//! a map is found by the ROM's hash in $GP_ASSETS, else in the tool repo's
//! assets/ (Moss Hop) and every game's assets/ in the games checkout; the
//! run recorder's metrics are in the JSON (`metrics`).
//!
//! An arcade romset (a MAME zip) lifts through an arcade map
//! (gp_assets::arcade), given with --map: its pictures by graphics region
//! and index, from whichever listed set the zip is.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_assets::{AssetMap, Difference, MAP_FILE, RomLayout};
use serde_json::json;

use crate::Outcome;

/// Lifted assets must never be committed, so an output inside a git work
/// tree is refused.
pub(crate) fn outside_repo(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path).with_context(|| path.display().to_string())?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&path) {
        bail!(
            "{} is inside the git work tree at {}; lifted assets and packed ROMs stay outside any repo (by default in $GP_HOME/work/assets)",
            path.display(),
            repo.display()
        );
    }
    Ok(path)
}

/// Writes a file, refusing to follow a symlink there: one could point into
/// a git work tree, or anywhere else, past the check on its directory.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp assets writes only plain files",
            path.display()
        );
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// Where maps are looked for: $GP_ASSETS; else the tool repo's assets/
/// and each game's assets/ in the games checkout.
pub(crate) fn maps_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("GP_ASSETS").filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(dir)];
    }
    let mut dirs = Vec::new();
    let here = std::env::current_dir().ok();
    let tool = here
        .iter()
        .flat_map(|h| h.ancestors())
        .find(|d| d.join("assets/mosshop.toml").is_file())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    dirs.push(tool.join("assets"));
    if let Some(games) = crate::roms_cmd::games_dir(None)
        && let Ok(rd) = fs::read_dir(&games)
    {
        let mut games: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("assets")).collect();
        games.sort();
        dirs.extend(games);
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The map for a ROM: the one given, or the one in the maps directories
/// whose sha256 is the ROM's. Every NES map there is parsed, so a broken
/// one is an error rather than skipped; an arcade map isn't for an image's
/// hash and is passed over.
pub(crate) fn find_map(given: Option<&Path>, sha256: &str) -> Result<(PathBuf, String, AssetMap)> {
    let parse = |path: &Path| -> Result<(String, AssetMap)> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let map = AssetMap::parse(&text).with_context(|| path.display().to_string())?;
        Ok((text, map))
    };
    if let Some(path) = given {
        let (text, map) = parse(path)?;
        return Ok((path.to_path_buf(), text, map));
    }
    let dirs = maps_dirs();
    if dirs.is_empty() {
        bail!(
            "no assets/ directory found (run inside the repo, set GP_ASSETS or GP_GAMES_DIR, or give --map)"
        );
    }
    for dir in &dirs {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        entries.sort();
        for path in entries {
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            if gp_assets::arcade::is_arcade(&text) {
                continue;
            }
            let (text, map) = parse(&path)?;
            if map.sha256.eq_ignore_ascii_case(sha256) {
                return Ok((path, text, map));
            }
        }
    }
    bail!(
        "no asset map in {} is for ROM sha256 {sha256}; write one or give --map",
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Lifts `rom` (an NES image's or a romset zip's bytes) with the map's
/// text, whichever platform the map is for: the files and what each asset
/// gave.
pub(crate) fn lift_files(
    rom: &[u8],
    map_text: &str,
) -> Result<(Vec<gp_assets::File>, Vec<gp_assets::Lifted>)> {
    if gp_assets::arcade::is_arcade(map_text) {
        let map = gp_assets::arcade::ArcadeMap::parse(map_text)?;
        if !gp_arcade::romset::looks_like_zip(rom) {
            bail!("an arcade asset map lifts a romset zip, and this ROM isn't one");
        }
        let set = gp_arcade::Romset::from_zip(rom).map_err(anyhow::Error::msg)?;
        return Ok(gp_assets::arcade::lift(&decoded(&set), &map, map_text)?);
    }
    let map = AssetMap::parse(map_text)?;
    Ok(gp_assets::lift(rom, &map, map_text)?)
}

pub fn lift(
    rom_path: &Path,
    out: Option<&Path>,
    map_path: Option<&Path>,
    force: bool,
    home: &Path,
) -> Result<Outcome> {
    let rom = read(rom_path)?;
    if gp_arcade::romset::looks_like_zip(&rom) {
        return lift_arcade(&rom, out, map_path, force, home);
    }
    let layout = RomLayout::of(&rom).with_context(|| rom_path.display().to_string())?;
    let (map_path, map_text, map) = find_map(map_path, &layout.sha256)?;
    let out = match out {
        Some(p) => p.to_path_buf(),
        None => home.join("work").join("assets").join(&map.source),
    };
    let out = outside_repo(&out)?;
    // A directory with a map in it holds an earlier lift, whose files
    // someone may have edited: lifting again would replace them.
    refuse_earlier_lift(&out, force)?;
    let (files, lifted) = gp_assets::lift(&rom, &map, &map_text)?;
    write_files(&out, &files)?;
    let covered = map.bytes_covered();
    let title = map.title.clone().unwrap_or_else(|| map.source.clone());
    let mut text = format!(
        "lifted {title} ({}) into {}: {} assets, {covered} ROM bytes\n",
        map.source,
        out.display(),
        lifted.len()
    );
    let name_width = lifted.iter().map(|l| l.name.len()).max().unwrap_or(0);
    let format_width = lifted.iter().map(|l| l.format.len()).max().unwrap_or(0);
    for l in &lifted {
        let _ = writeln!(
            text,
            "  {:<name_width$} {:<format_width$} {:>6} bytes  {}",
            l.name, l.format, l.bytes, l.file
        );
    }
    let _ = write!(text, "  map: {} (copied to {MAP_FILE})", map_path.display());
    Ok(Outcome::ok(
        json!({
            "source": map.source,
            "title": map.title,
            "sha256": layout.sha256,
            "map": map_path,
            "out": out,
            "bytes_covered": covered,
            "assets": lifted,
            "files": files.iter().map(|f| &f.name).collect::<Vec<_>>(),
        }),
        text,
    ))
}

/// Refuses to lift over an earlier lift unless forced: its files may have
/// been edited. symlink_metadata, so a dangling link to a map counts as a
/// lift too.
fn refuse_earlier_lift(out: &Path, force: bool) -> Result<()> {
    if !force && fs::symlink_metadata(out.join(MAP_FILE)).is_ok() {
        bail!(
            "{} already holds lifted assets ({MAP_FILE}), which may have been edited; pass --force to replace them",
            out.display()
        );
    }
    Ok(())
}

/// Writes a lift's files under `out`, plain files and directories only.
fn write_files(out: &Path, files: &[gp_assets::File]) -> Result<()> {
    if fs::symlink_metadata(out).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp assets writes only plain files and directories",
            out.display()
        );
    }
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    for f in files {
        // An object-list asset lifts to a directory of files, one per
        // entry. The directory must be a real one, for the same reason a
        // file mustn't be a symlink.
        let path = out.join(&f.name);
        if let Some(parent) = path.parent().filter(|p| *p != out) {
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                bail!(
                    "{} is a symlink; gp assets writes only plain files and directories",
                    parent.display()
                );
            }
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        write_file(&path, &f.bytes)?;
    }
    Ok(())
}

/// [`lift`] for an arcade romset: the map must be given (an arcade map is
/// for a set's name, not an image's hash), and the output defaults to
/// $GP_HOME/work/assets/SET.
pub(crate) fn lift_arcade(
    rom: &[u8],
    out: Option<&Path>,
    map_path: Option<&Path>,
    force: bool,
    home: &Path,
) -> Result<Outcome> {
    let Some(map_path) = map_path else {
        bail!("an arcade romset lifts through an arcade asset map; give it with --map");
    };
    let map_text =
        fs::read_to_string(map_path).with_context(|| format!("reading {}", map_path.display()))?;
    if !gp_assets::arcade::is_arcade(&map_text) {
        bail!(
            "{} is an NES asset map, and this ROM is an arcade romset",
            map_path.display()
        );
    }
    let set = gp_arcade::Romset::from_zip(rom).map_err(anyhow::Error::msg)?;
    let out = match out {
        Some(p) => p.to_path_buf(),
        None => home.join("work").join("assets").join(set.name),
    };
    let out = outside_repo(&out)?;
    refuse_earlier_lift(&out, force)?;
    let (files, lifted) =
        lift_files(rom, &map_text).with_context(|| map_path.display().to_string())?;
    write_files(&out, &files)?;
    Ok(Outcome::ok(
        json!({ "set": set.name, "map": map_path, "out": out, "assets": lifted.len(), "files": files.len() }),
        format!(
            "lifted {} assets of {} into {}
  map: {} (copied to {MAP_FILE})",
            lifted.len(),
            set.name,
            out.display(),
            map_path.display()
        ),
    ))
}

/// The map in a lifted directory and the encoded bytes of its files.
fn encode_dir(dir: &Path) -> Result<(AssetMap, Vec<gp_assets::Encoded>)> {
    let map_path = dir.join(MAP_FILE);
    let text = fs::read_to_string(&map_path).with_context(|| {
        format!(
            "reading {} (is {} a lifted directory?)",
            map_path.display(),
            dir.display()
        )
    })?;
    let map = AssetMap::parse(&text).with_context(|| map_path.display().to_string())?;
    let encoded = gp_assets::encode(&map, &|name| fs::read(dir.join(name)))
        .with_context(|| format!("packing {}", dir.display()))?;
    Ok((map, encoded))
}

/// How many differences the text output lists before summing up the rest.
const LISTED: usize = 20;

fn list_differences(text: &mut String, diffs: &[Difference], rom: &str, encoded: &str) {
    for d in diffs.iter().take(LISTED) {
        let _ = writeln!(
            text,
            "  {}: {} offset {:#06x} (file offset {:#06x}): {rom} {:#04x}, {encoded} {:#04x}",
            d.part, d.region, d.offset, d.file_offset, d.rom, d.encoded
        );
    }
    if diffs.len() > LISTED {
        let _ = writeln!(text, "  and {} more", diffs.len() - LISTED);
    }
}

/// `gp assets pack --check`: the round trip. With `metrics`, the JSON
/// carries PREFIX.bytes_covered and PREFIX.differences.
pub fn check_dir(dir: &Path, rom_path: &Path, metrics: Option<&str>) -> Result<Outcome> {
    let rom = read(rom_path)?;
    let map_path = dir.join(MAP_FILE);
    if let Ok(text) = fs::read_to_string(&map_path)
        && gp_assets::arcade::is_arcade(&text)
    {
        return check_arcade(dir, &rom, &text);
    }
    let (map, encoded) = encode_dir(dir)?;
    let diffs = gp_assets::differences(&rom, &map, &encoded)
        .with_context(|| rom_path.display().to_string())?;
    let covered = map.bytes_covered();
    let ok = diffs.is_empty();
    let mut text = if ok {
        format!(
            "round trip ok: {} assets, {covered} bytes, identical to {}",
            map.assets.len(),
            rom_path.display()
        )
    } else {
        let mut assets: Vec<&str> = diffs.iter().map(|d| d.asset.as_str()).collect();
        assets.dedup();
        format!(
            "round trip failed: {} of {covered} bytes differ from {}, in {}\n",
            diffs.len(),
            rom_path.display(),
            assets.join(", ")
        )
    };
    list_differences(&mut text, &diffs, "ROM", "lifted files");
    // Another ROM (an edited one, say) is compared all the same: comparing
    // can only find real differences. The note says it isn't the map's.
    let layout = RomLayout::of(&rom)?;
    let is_maps = layout.is_for(&map);
    if !is_maps {
        let _ = write!(
            text,
            "\nnote: this ROM (sha256 {}) isn't the one {MAP_FILE} describes ({}); only the bytes the assets cover were compared",
            layout.sha256, map.sha256
        );
    }
    let mut j = json!({
        "ok": ok,
        "rom_is_maps": is_maps,
        "source": map.source,
        "rom": rom_path,
        "dir": dir,
        "bytes_covered": covered,
        "difference_count": diffs.len(),
        "differences": diffs,
    });
    if let Some(prefix) = metrics {
        j["metrics"] = json!({
            format!("{prefix}.bytes_covered"): covered,
            format!("{prefix}.differences"): diffs.len(),
        });
    }
    Ok(Outcome {
        json: j,
        text: text.trim_end().to_owned(),
        exit: u8::from(!ok),
    })
}

/// `gp assets pack --base ROM -o FILE`: a copy of the base ROM with the
/// lifted directory's assets encoded in place, and the bytes that changed.
pub fn pack_into(dir: &Path, base_path: &Path, out: &Path, force: bool) -> Result<Outcome> {
    let out = outside_repo(out)?;
    if fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symlink; gp assets writes only plain files",
            out.display()
        );
    }
    if out.exists() && !force {
        bail!(
            "{} already exists; pass --force to replace it",
            out.display()
        );
    }
    let base = read(base_path)?;
    let (map, encoded) = encode_dir(dir)?;
    let packed =
        gp_assets::apply(&base, &map, &encoded).with_context(|| base_path.display().to_string())?;
    let changed = gp_assets::differences(&base, &map, &encoded)?;
    write_file(&out, &packed)?;
    let mut text = format!(
        "packed {} into {}: {} bytes changed from {}\n",
        dir.display(),
        out.display(),
        changed.len(),
        base_path.display()
    );
    list_differences(&mut text, &changed, "was", "now");
    Ok(Outcome::ok(
        json!({
            "source": map.source,
            "base": base_path,
            "out": out,
            "sha256": RomLayout::of(&packed)?.sha256,
            "bytes_changed": changed.len(),
            "changes": changed,
        }),
        text.trim_end().to_owned(),
    ))
}

/// A set's graphics regions decoded, for the arcade lift.
fn decoded(set: &gp_arcade::Romset) -> gp_assets::arcade::Decoded {
    let g = gp_arcade::video::Gfx::decode(&set.gfx1, &set.gfx2);
    gp_assets::arcade::Decoded {
        set: set.name.to_owned(),
        identity: set.identity.clone(),
        tiles: g.tiles,
        sprites: g.sprites,
    }
}

/// [`check_dir`] for an arcade lift: its pictures packed back into the
/// graphics chips must be the romset's, byte for byte.
fn check_arcade(dir: &Path, rom: &[u8], text: &str) -> Result<Outcome> {
    let map = gp_assets::arcade::ArcadeMap::parse(text)?;
    let set = gp_arcade::Romset::from_zip(rom).map_err(anyhow::Error::msg)?;
    map.fits(set.name)?;
    let mut d = decoded(&set);
    gp_assets::arcade::unpack(&mut d, &map, &|name| fs::read(dir.join(name)))?;
    let (gfx1, gfx2) = gp_arcade::video::Gfx {
        tiles: d.tiles,
        sprites: d.sprites,
    }
    .encode();
    let mut diffs: Vec<(&str, usize)> = Vec::new();
    for (name, ours, theirs) in [("gfx1", &gfx1, &set.gfx1), ("gfx2", &gfx2, &set.gfx2)] {
        diffs.extend(
            ours.iter()
                .zip(theirs.iter())
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, _)| (name, i)),
        );
    }
    let ok = diffs.is_empty();
    Ok(Outcome {
        json: json!({ "ok": ok, "differences": diffs.len(), "set": set.name }),
        text: if ok {
            format!(
                "round trip ok: {} assets identical to {}'s graphics chips",
                map.assets.len(),
                set.name
            )
        } else {
            format!(
                "round trip FAILED: {} bytes of {}'s graphics chips differ, the first in {} at {:#x}",
                diffs.len(),
                set.name,
                diffs[0].0,
                diffs[0].1
            )
        },
        exit: u8::from(!ok),
    })
}

#[cfg(test)]
mod symlink_tests {
    use super::write_file;

    #[test]
    fn write_file_refuses_a_symlink() {
        let dir = std::env::temp_dir().join(format!("gp-assets-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.bin");
        std::fs::write(&target, b"old").unwrap();
        let link = dir.join("link.bin");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = write_file(&link, b"new").unwrap_err().to_string();
        assert!(err.contains("is a symlink"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        write_file(&dir.join("plain.bin"), b"new").unwrap();
        assert_eq!(std::fs::read(dir.join("plain.bin")).unwrap(), b"new");
    }
}
