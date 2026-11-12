//! `gp roms add --patch`: a patched source made from the person's own base
//! ROM and the patch file its author hands out. The patch is found in the
//! games' roms.toml files by its SHA-256, the base in $GP_ROM_DIR by hash,
//! the result checked against the registered hashes before anything is
//! written. The repo never holds the patch or the result.
//!
//! Ported from game-printer's rompatch.rs. Changed by gameprinter's layout:
//! the registry is the games checkout's roms.toml files (`kind =
//! "patched"` with `[source.patch]`, as sources/nes.toml had them), and
//! the run recorder is gone.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use gp_evidence::Paths;
use gp_evidence::patch::{self, PatchFormat};
use gp_evidence::registry::{KnownSource, PatchOffsets, Registry, SourceKind};
use gp_evidence::source::{identify_nes, index_nes_roms};

use crate::Outcome;
use crate::movie::RomHashes;
use crate::romclean;

/// What `<id>.patch.toml` beside a patched image records.
#[derive(Serialize)]
struct Sidecar<'a> {
    schema: u32,
    tool: &'static str,
    source: &'a str,
    base: &'a str,
    base_sha256: &'a str,
    base_file: &'a Path,
    patch_file: &'a Path,
    patch_sha256: &'a str,
    patch_bytes: u64,
    patch_format: &'static str,
    records: usize,
    bytes_changed: usize,
    sha256: &'a str,
    md5: &'a str,
    sha1: &'a str,
    ines_sha1: &'a str,
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The base a `--base` argument names: a registry ID, or a headerless
/// SHA-256 or a prefix of one at least 8 characters long.
fn resolve_base<'r>(registry: &'r Registry, arg: &str) -> Result<&'r KnownSource> {
    if let Some(k) = registry.by_id(arg) {
        return Ok(k);
    }
    let arg = arg.to_ascii_lowercase();
    if arg.len() < 8 || !arg.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(
            "--base {arg:?} is neither a registered source ID nor a SHA-256 (8 hex digits or more)"
        );
    }
    let hits: Vec<_> = registry
        .sources
        .iter()
        .filter(|s| s.sha256.starts_with(&arg))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => bail!("--base {arg}: no registered source has that hash"),
        _ => bail!("--base {arg}: more than one registered source starts with that hash"),
    }
}

/// The base image as the patch expects it: the registry's header (for a
/// source with board facts) and the game data, junk dropped, so a junk
/// header can't move an `ines` patch's offsets.
fn base_image(file: &Path, registry: &Registry, offsets: PatchOffsets) -> Result<Vec<u8>> {
    let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let job = romclean::prepare(&bytes, registry)
        .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    let image = job.cleaned.bytes;
    Ok(match offsets {
        PatchOffsets::Ines => image,
        PatchOffsets::Headerless => image[16..].to_vec(),
    })
}

/// A draft registry entry for a patch nobody has registered, for `--new`.
fn draft(
    patch_sha256: &str,
    patch_bytes: u64,
    format: PatchFormat,
    base: &KnownSource,
    hashes: &RomHashes,
    mapper: u16,
) -> String {
    format!(
        r#"[[source]]
id = "HACK-VERSION"
title = "TITLE"
platform = "{platform}"
region = "{region}"
sha256 = "{sha256}"
md5 = "{md5}"
sha1 = "{sha1}"
ines_sha1 = "{ines_sha1}"
mapper = {mapper}
status = "unverified"
evidence = "Measured by gp roms add --patch --new on DATE: the patch applied to {base_id}."
kind = "patched"

[source.patch]
base = "{base_id}"
format = "{format}"
sha256 = "{patch_sha256}"
bytes = {patch_bytes}
offsets = "ines"
url = "URL OF THE AUTHOR'S PATCH FILE"
page = "URL OF THE HACK'S PAGE"
author = "AUTHOR"
permission = "none"
"#,
        platform = base.platform,
        region = base.region,
        sha256 = hashes.sha256,
        md5 = hashes.md5,
        sha1 = hashes.sha1,
        ines_sha1 = hashes.ines_sha1,
        base_id = base.id,
        format = format.name(),
    )
}

pub(crate) fn add(
    patch_file: &Path,
    base_arg: Option<&str>,
    new: bool,
    registry: &Registry,
    paths: &Paths,
) -> Result<Outcome> {
    let patch_bytes =
        fs::read(patch_file).with_context(|| format!("reading {}", patch_file.display()))?;
    let format = PatchFormat::detect(&patch_bytes).with_context(|| {
        format!(
            "{} is not an IPS or BPS patch (it starts with neither \"PATCH\" nor \"BPS1\")",
            patch_file.display()
        )
    })?;
    let patch_sha256 = sha256_hex(&patch_bytes);
    let entries = registry.by_patch_sha256(&patch_sha256);
    let base = base_arg.map(|b| resolve_base(registry, b)).transpose()?;
    if let Some(b) = base
        && b.kind != SourceKind::Dump
    {
        bail!(
            "--base {} is itself a patched source; name the release it was made from",
            b.id
        );
    }

    // Which (entry, base) pairs to try: the entry for the named base, or
    // every registered one; with --new and no entry, just the named base.
    let wanted: Vec<(Option<&KnownSource>, &KnownSource)> = if entries.is_empty() {
        if !new {
            bail!(
                "{} (sha256 {patch_sha256}) is not a registered patch, so the result can't be checked; add its entry to the game's roms.toml first (`--new --base ID` prints a draft)",
                patch_file.display()
            );
        }
        let base = base.context("--new needs --base: the release the patch is for")?;
        vec![(None, base)]
    } else {
        let mut out = Vec::new();
        for e in &entries {
            let p = e.patch.as_ref().expect("a patched source has a patch");
            if base.is_some_and(|b| b.id != p.base) {
                continue;
            }
            let b = registry
                .by_id(&p.base)
                .expect("the registry checks every base is registered");
            out.push((Some(*e), b));
        }
        if out.is_empty() {
            let bases: Vec<_> = entries
                .iter()
                .filter_map(|e| e.patch.as_ref().map(|p| p.base.as_str()))
                .collect();
            bail!(
                "this patch is registered for {} only, not {}; a patch made for one revision can apply cleanly to another and still make a different game, so add an entry for that base once you've checked it runs",
                bases.join(", "),
                base.map(|b| b.id.as_str()).unwrap_or("?")
            );
        }
        out
    };

    let index = if paths.roms.is_dir() {
        index_nes_roms(&paths.roms).with_context(|| format!("reading {}", paths.roms.display()))?
    } else {
        Default::default()
    };
    let mut results = Vec::new();
    let mut text = Vec::new();
    let mut missing = Vec::new();
    for (entry, base) in wanted {
        let Some(base_file) = index.get(&base.sha256) else {
            missing.push(base.id.clone());
            continue;
        };
        let offsets = entry
            .and_then(|e| e.patch.as_ref())
            .map_or(PatchOffsets::Ines, |p| p.offsets);
        if let Some(p) = entry.and_then(|e| e.patch.as_ref())
            && p.format != format
        {
            bail!(
                "the registry says {} is {}, but the file is {}",
                entry.map(|e| e.id.as_str()).unwrap_or_default(),
                p.format.name(),
                format.name()
            );
        }
        let image = base_image(base_file, registry, offsets)?;
        let (mut out, applied) = patch::apply(&image, &patch_bytes).map_err(|e| {
            anyhow::anyhow!("applying {} to {}: {e}", patch_file.display(), base.id)
        })?;
        if offsets == PatchOffsets::Headerless {
            let mut with_header = fs::read(base_file)?[..16].to_vec();
            with_header.append(&mut out);
            out = with_header;
        }
        let img = identify_nes(&out)
            .map_err(|e| anyhow::anyhow!("the patched image isn't a usable NES image: {e}"))?;
        let hashes = RomHashes::of(&out)?;

        let Some(entry) = entry else {
            let known = registry.by_sha256(&img.sha256);
            text.push(format!(
                "applied {} ({} records, {} bytes changed) to {}; nothing written. The result is {}. A draft entry to check and paste into the game's roms.toml:\n\n{}",
                format.name(),
                applied.records,
                applied.bytes_changed,
                base.id,
                known.map_or_else(|| format!("sha256 {}", img.sha256), |k| format!("already registered as {}", k.id)),
                draft(&patch_sha256, patch_bytes.len() as u64, format, base, &hashes, img.header.mapper)
            ));
            results.push(json!({
                "base": base.id,
                "written": false,
                "sha256": img.sha256,
                "md5": hashes.md5,
                "sha1": hashes.sha1,
                "ines_sha1": hashes.ines_sha1,
                "applied": applied,
            }));
            continue;
        };

        // Every registered hash of the result must match, or nothing is
        // written: IPS has no checksum of its own.
        let checks = [
            ("sha256", Some(&entry.sha256), &img.sha256),
            ("md5", entry.md5.as_ref(), &hashes.md5),
            ("sha1", entry.sha1.as_ref(), &hashes.sha1),
            ("ines_sha1", entry.ines_sha1.as_ref(), &hashes.ines_sha1),
        ];
        let wrong: Vec<String> = checks
            .iter()
            .filter_map(|(name, want, got)| {
                want.filter(|w| w != got)
                    .map(|w| format!("{name} is {got}, the registry says {w}"))
            })
            .collect();
        if !wrong.is_empty() {
            bail!(
                "{} applied to {} doesn't make {}: {}. Nothing was written",
                patch_file.display(),
                base.id,
                entry.id,
                wrong.join("; ")
            );
        }

        let dir = paths.roms.join(&base.platform).join("patched");
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let dest = dir.join(format!("{}.nes", entry.id));
        let action = write_checked(&dest, &out, |back| {
            identify_nes(back).is_ok_and(|i| i.sha256 == entry.sha256)
        })?;
        let sidecar_path = dir.join(format!("{}.patch.toml", entry.id));
        let sidecar = Sidecar {
            schema: 1,
            tool: "gp roms add --patch",
            source: &entry.id,
            base: &base.id,
            base_sha256: &base.sha256,
            base_file,
            patch_file,
            patch_sha256: &patch_sha256,
            patch_bytes: patch_bytes.len() as u64,
            patch_format: format.name(),
            records: applied.records,
            bytes_changed: applied.bytes_changed,
            sha256: &img.sha256,
            md5: &hashes.md5,
            sha1: &hashes.sha1,
            ines_sha1: &hashes.ines_sha1,
        };
        let body = format!(
            "# How {} was made from the person's own {} and the author's patch.\n# Hashes only.\n{}",
            entry.id,
            base.id,
            toml::to_string(&sidecar)?
        );
        write_checked(&sidecar_path, body.as_bytes(), |b| b == body.as_bytes())?;

        // The patch itself, by hash, so the result can be made again.
        let patches = paths.roms.join("patches");
        fs::create_dir_all(&patches).with_context(|| format!("creating {}", patches.display()))?;
        let kept = patches.join(format!("{patch_sha256}.{}", format.name()));
        write_checked(&kept, &patch_bytes, |b| sha256_hex(b) == patch_sha256)?;

        text.push(format!(
            "{action:<15} {}  {}  {}\n      from {} and {} ({}, {} records, {} bytes changed); every registered hash matches",
            &img.sha256[..16],
            entry.id,
            dest.display(),
            base.id,
            patch_file.display(),
            format.name(),
            applied.records,
            applied.bytes_changed
        ));
        results.push(json!({
            "source": entry.id,
            "base": base.id,
            "written": true,
            "action": action,
            "dest": dest,
            "sidecar": sidecar_path,
            "patch_kept": kept,
            "sha256": img.sha256,
            "applied": applied,
        }));
    }
    if results.is_empty() {
        bail!(
            "no base for this patch is in {}: it needs {} (find yours with `gp roms scan`)",
            paths.roms.display(),
            missing.join(" or ")
        );
    }
    for m in &missing {
        text.push(format!(
            "skipped         {m} is not in {}",
            paths.roms.display()
        ));
    }
    Ok(Outcome::ok(
        json!({ "patch_sha256": patch_sha256, "format": format.name(), "results": results, "missing_bases": missing }),
        text.join("\n"),
    ))
}

/// Writes `bytes` to `dest` through a temporary file in the same folder,
/// read back and checked by `ok`. A file already there with the same bytes
/// is left; one with other bytes is refused.
fn write_checked(dest: &Path, bytes: &[u8], ok: impl Fn(&[u8]) -> bool) -> Result<&'static str> {
    if dest.exists() {
        if fs::read(dest)? == bytes {
            return Ok("already present");
        }
        bail!(
            "{} already exists with different contents; move it away first",
            dest.display()
        );
    }
    let dir: PathBuf = dest
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let tmp = tempfile::Builder::new()
        .prefix(".gp-patch-")
        .suffix(".partial")
        .tempfile_in(&dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    fs::write(tmp.path(), bytes).with_context(|| format!("writing {}", tmp.path().display()))?;
    let back = fs::read(tmp.path())?;
    if back != bytes || !ok(&back) {
        bail!("{} did not read back identically", dest.display());
    }
    tmp.persist(dest)
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok("patched")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up IPS patch: two records.
    fn ips() -> Vec<u8> {
        let mut p = b"PATCH".to_vec();
        p.extend([0, 0x01, 0x10, 0, 4, 1, 2, 3, 4]);
        p.extend([0, 0x02, 0x10, 0, 2, 9, 9]);
        p.extend(b"EOF");
        p
    }

    fn registry(base: &str, patched: Option<(&str, &str, &str)>) -> Registry {
        let mut text = format!(
            "[[source]]\nid = \"test-base\"\ntitle = \"Test\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"{base}\"\nmapper = 0\nstatus = \"unverified\"\nevidence = \"made up\"\n"
        );
        if let Some((sha, md5, patch)) = patched {
            text.push_str(&format!(
                "\n[[source]]\nid = \"test-hack\"\ntitle = \"Hack\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"{sha}\"\nmd5 = \"{md5}\"\nmapper = 0\nstatus = \"unverified\"\nevidence = \"made up\"\nkind = \"patched\"\n\n[source.patch]\nbase = \"test-base\"\nformat = \"ips\"\nsha256 = \"{patch}\"\nbytes = 24\noffsets = \"ines\"\nurl = \"https://example.invalid/p.ips\"\npage = \"https://example.invalid/\"\nauthor = \"nobody\"\npermission = \"none\"\n"
            ));
        }
        Registry::from_toml(&text, Path::new("roms.toml")).unwrap()
    }

    /// `--new` prints a draft with the result's hashes and writes nothing;
    /// a registered patch makes its source in nes/patched/ with a sidecar
    /// and the patch kept by hash, once; a registered hash that doesn't
    /// match writes nothing. On the built-in test ROM: no game's bytes.
    #[test]
    fn a_registered_patch_makes_its_source_and_a_wrong_hash_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: d.path().join("home"),
            store: d.path().join("store"),
            roms: d.path().join("roms"),
        };
        let base = gp_nes::testrom::build();
        fs::create_dir_all(paths.roms.join("nes")).unwrap();
        fs::write(paths.roms.join("nes/base.nes"), &base).unwrap();
        let patch = d.path().join("p.ips");
        fs::write(&patch, ips()).unwrap();
        let base_sha = identify_nes(&base).unwrap().sha256;
        let draft = add(
            &patch,
            Some("test-base"),
            true,
            &registry(&base_sha, None),
            &paths,
        )
        .unwrap();
        assert!(draft.text.contains("[source.patch]"), "{}", draft.text);
        assert!(!paths.roms.join("nes/patched").exists());
        let sha = draft.json["results"][0]["sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let md5 = draft.json["results"][0]["md5"].as_str().unwrap().to_owned();
        let patch_sha = sha256_hex(&ips());
        let wrong = add(
            &patch,
            None,
            false,
            &registry(&base_sha, Some((&sha, &"0".repeat(32), &patch_sha))),
            &paths,
        );
        assert!(matches!(wrong, Err(e) if e.to_string().contains("Nothing was written")));
        assert!(!paths.roms.join("nes/patched").exists());
        let reg = registry(&base_sha, Some((&sha, &md5, &patch_sha)));
        let made = add(&patch, None, false, &reg, &paths).unwrap();
        assert_eq!(made.json["results"][0]["action"], "patched");
        let out = fs::read(paths.roms.join("nes/patched/test-hack.nes")).unwrap();
        assert_eq!(identify_nes(&out).unwrap().sha256, sha);
        assert!(
            paths
                .roms
                .join("nes/patched/test-hack.patch.toml")
                .is_file()
        );
        assert!(
            paths
                .roms
                .join(format!("patches/{patch_sha}.ips"))
                .is_file()
        );
        let again = add(&patch, None, false, &reg, &paths).unwrap();
        assert_eq!(again.json["results"][0]["action"], "already present");
        // Not registered and no --new: refused.
        assert!(add(&patch, None, false, &registry(&base_sha, None), &paths).is_err());
    }
}
