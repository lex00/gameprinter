//! `gp game fetch`: a game by its exact id into a working copy, and its
//! assets lifted from the person's own ROMs, each checked against its pins.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use gp_assets::RomLayout;
use gp_package::registry::Reader;
use serde_json::json;

use crate::Outcome;
use crate::template::{lifted_digest, sha256_hex};

/// Each ROM in the folder, by its headerless SHA-256.
fn roms(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).with_context(|| d.display().to_string())? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "nes") {
                let bytes = fs::read(&p)?;
                if let Ok(l) = RomLayout::of(&bytes) {
                    out.insert(l.sha256.clone(), bytes);
                }
            }
        }
    }
    Ok(out)
}

/// Version strings in order: dot- or dash-separated parts, numbers by value.
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<(u64, String)> {
        s.split(['.', '-'])
            .map(|p| (p.parse().unwrap_or(u64::MAX), p.to_owned()))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// The versions of `name` the registry's index can be read for: every
/// `index/NS/NAME/VERSION.toml` when the registry is a folder on this
/// machine, else the catalog's latest alone.
fn indexed_versions(reader: &Reader, from: &str, name: &str, now: &str) -> Result<Vec<String>> {
    let (ns, n) = name
        .split_once('/')
        .with_context(|| format!("{name:?} isn't namespace/name"))?;
    if let Some(dir) = crate::registry::local_folder(from) {
        let dir = dir.join("index").join(ns).join(n);
        let mut out = Vec::new();
        if dir.is_dir() {
            for e in fs::read_dir(&dir).with_context(|| dir.display().to_string())? {
                let p = e?.path();
                if p.extension().is_some_and(|x| x == "toml")
                    && let Some(v) = p.file_stem().and_then(|s| s.to_str())
                {
                    out.push(v.to_owned());
                }
            }
        }
        return Ok(out);
    }
    let c = reader.catalog(now).map_err(|e| anyhow!(e))?;
    Ok(c.package
        .into_iter()
        .filter(|p| p.name == name)
        .map(|p| p.latest)
        .collect())
}

/// The id to fetch: `--id` as given, or the one the registry's index lists
/// for the version (the newest not yanked when no version is given), with a
/// line saying where it came from.
fn resolve_id(
    reader: &Reader,
    from: &str,
    spec: &str,
    id: Option<&str>,
) -> Result<(String, String, String, Option<String>)> {
    if let Some(id) = id {
        let (name, version) = spec
            .rsplit_once('@')
            .with_context(|| format!("wanted NAME@VERSION, not {spec}"))?;
        return Ok((name.into(), version.into(), id.into(), None));
    }
    let now = crate::registry::utc(0)?;
    // Like the registry's reader, gp registry check and the page: with no
    // yank list there is no telling which versions were withdrawn.
    if reader
        .target
        .read("yanks.toml")
        .map_err(|e| anyhow!(e))?
        .is_none()
    {
        bail!(
            "the registry {from} has no yank list (yanks.toml), so gp can't tell which versions \
             were withdrawn: `gp registry reindex` writes it, or give NAME@VERSION with --id ID"
        );
    }
    let yanks = reader.yanks(&now).map_err(|e| anyhow!(e))?.yank;
    let yanked = |n: &str, v: &str| yanks.iter().find(|y| y.name == n && y.version == v);
    let (name, version, why) = match spec.rsplit_once('@') {
        Some((name, version)) => (name, version.to_owned(), String::new()),
        None => {
            let name = spec;
            let mut all = indexed_versions(reader, from, name, &now)?;
            if all.is_empty() {
                bail!(
                    "the registry's index lists no version of {name}: check the name, or give \
                     NAME@VERSION with the id as --id ID"
                );
            }
            all.sort_by(|a, b| version_order(a, b));
            let Some(v) = all.iter().rev().find(|v| yanked(name, v).is_none()) else {
                bail!(
                    "every version of {name} the registry's index lists is yanked ({}){}: to \
                     fetch one anyway, give NAME@VERSION and its id as --id ID",
                    all.join(", "),
                    if crate::registry::local_folder(from).is_some() {
                        ""
                    } else {
                        "; over s3:// or https only the catalog's latest can be read, so an \
                         older version not yanked must be named as NAME@VERSION"
                    }
                );
            };
            let why = if crate::registry::local_folder(from).is_some() {
                ", the newest version not yanked"
            } else {
                ", the catalog's latest"
            };
            (name, v.clone(), why.to_owned())
        }
    };
    let key = format!("index/{}/{version}.toml", name);
    let v = match reader.version(name, &version) {
        Ok(v) => v,
        Err(e) if reader.target.read(&key).ok().flatten().is_none() => bail!(
            "the registry's index has no {name}@{version} ({key}): check the name and version, \
             or give the id as --id ID ({e})"
        ),
        Err(e) => bail!(e),
    };
    let mut note = format!(
        "id {} for {name}@{version}, from the registry's index ({key}){why}",
        v.id
    );
    if let Some(y) = yanked(name, &version) {
        if y.severity == "security" {
            bail!(
                "{name}@{version} was yanked for security: {}. gp game fetch takes a yanked \
                 version only with its id as --id ID",
                y.reason
            );
        }
        note.push_str(&format!(
            "\nnote: {name}@{version} was yanked: {}",
            y.reason
        ));
    }
    Ok((name.into(), version, v.id, Some(note)))
}

pub fn fetch(
    spec: &str,
    id: Option<&str>,
    from: &str,
    root_file: Option<&Path>,
    into: &Path,
    rom_dir: &Path,
) -> Result<Outcome> {
    let into = std::path::absolute(into)?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&into) {
        bail!(
            "{} is inside the git work tree at {}",
            into.display(),
            repo.display()
        );
    }
    let root = root_file.map(crate::registry::read_root).transpose()?;
    let target = crate::registry::open(from);
    let reader = match &root {
        Some(root) => Reader::open(&*target, root),
        None => Reader::unsigned(&*target),
    }
    .map_err(|e| anyhow!(e))?;
    let (name, version, id, from_index) = resolve_id(&reader, from, spec, id)?;
    let spec = format!("{name}@{version}");
    let (_, files) = reader.fetch(&name, &version, &id).map_err(|e| anyhow!(e))?;
    for (path, bytes) in &files {
        let out = into.join(path);
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(out, bytes)?;
    }
    let text = |path: &str| -> Result<String> {
        let b = files
            .get(path)
            .ok_or_else(|| anyhow!("the package has no {path}"))?;
        Ok(String::from_utf8(b.clone())?)
    };
    let pkg: toml::Table = text("package.toml")?.parse().context("package.toml")?;
    let entries = pkg
        .get("rom")
        .and_then(|r| r.get("entry"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let have = roms(rom_dir)?;
    let mut lines: Vec<String> = from_index.iter().cloned().collect();
    lines.push(format!(
        "fetched {spec} ({} files), every {} checked",
        files.len(),
        if root.is_some() {
            "signature and hash"
        } else {
            "hash"
        }
    ));
    let (mut bad, mut results) = (0u32, Vec::new());
    for e in &entries {
        let field = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let (rom_id, sha, map_path) = (field("id"), field("sha256"), field("asset_map"));
        let Some(rom) = have.get(&sha) else {
            bad += 1;
            lines.push(format!(
                "MISSING {rom_id}: no ROM with its hash in the ROM folder"
            ));
            continue;
        };
        let map_text = text(&map_path)?;
        let map_sha = sha256_hex(map_text.as_bytes());
        let map_ok = map_sha == field("asset_map_sha256");
        let (lifted, _) =
            crate::assets::lift_files(rom, &map_text).with_context(|| map_path.clone())?;
        let out = into.join("lifted").join(&rom_id);
        let lifted: BTreeMap<String, Vec<u8>> =
            lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
        for (n, b) in &lifted {
            let p = out.join(n);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(p, b)?;
        }
        let digest = lifted_digest(&lifted);
        let pin = field("lifted_sha256");
        let ok = map_ok && (pin.is_empty() || pin == digest);
        bad += u32::from(!ok);
        lines.push(format!(
            "{} {rom_id}: {} files, lifted {digest}{}{}",
            if ok { "ok     " } else { "FAILED " },
            lifted.len(),
            if pin.is_empty() {
                " (no pin to compare)"
            } else {
                ""
            },
            if map_ok {
                ""
            } else {
                ", asset map isn't its pin"
            },
        ));
        results.push(json!({ "id": rom_id, "ok": ok, "digest": digest, "files": lifted.len() }));
    }
    Ok(Outcome {
        json: json!({
            "ok": bad == 0,
            "name": name,
            "version": version,
            "id": id,
            "id_from": if from_index.is_some() { "index" } else { "--id" },
            "files": files.len(),
            "roms": results,
        }),
        text: lines.join("\n"),
        exit: u8::from(bad > 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Writing, open, publish_cmd, yank_cmd};
    use gp_package::registry::{Signer, init};

    /// A made-up unsigned registry holding own/game at these versions, each a
    /// package.toml with no ROM entries, and an empty ROM folder.
    fn scratch(versions: &[&str]) -> (tempfile::TempDir, String, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let spec = d.path().join("reg").display().to_string();
        init(
            &*open(&spec),
            "scratch",
            &Signer::none(),
            &[],
            &[("own".to_owned(), Vec::new())],
            "2026-01-01",
        )
        .unwrap();
        for v in versions {
            let pkg = d.path().join(format!("pkg-{v}"));
            fs::create_dir_all(&pkg).unwrap();
            fs::write(pkg.join("package.toml"), format!("made_up = \"{v}\"\n")).unwrap();
            publish_cmd(
                &spec,
                &pkg,
                ("own/game", v),
                ("game", "Game"),
                &[],
                None,
                Writing::default(),
            )
            .unwrap();
        }
        // The yank list, as a registry's maintainer writes it.
        crate::registry::reindex_cmd(&spec, (&[], true, &[]), None, Writing::default()).unwrap();
        let roms = d.path().join("roms");
        fs::create_dir_all(&roms).unwrap();
        (d, spec, roms)
    }

    fn indexed_id(spec: &str, v: &str) -> String {
        let t =
            fs::read_to_string(Path::new(spec).join(format!("index/own/game/{v}.toml"))).unwrap();
        let t: toml::Table = t.parse().unwrap();
        t["id"].as_str().unwrap().to_owned()
    }

    fn yank(spec: &str, v: &str, severity: &str) {
        yank_cmd(
            spec,
            &format!("own/game@{v}"),
            Some(("made up", severity)),
            false,
            None,
            Writing::default(),
        )
        .unwrap();
    }

    #[test]
    fn no_id_takes_the_newest_version_not_yanked_from_the_index() {
        let (d, spec, roms) = scratch(&["0.2.0", "0.10.0", "0.9.1"]);
        yank(&spec, "0.10.0", "notice");
        let into = d.path().join("copy");
        let o = fetch("own/game", None, &spec, None, &into, &roms).unwrap();
        let id = indexed_id(&spec, "0.9.1");
        assert!(
            o.text.starts_with(&format!(
                "id {id} for own/game@0.9.1, from the registry's index \
                 (index/own/game/0.9.1.toml), the newest version not yanked\n\
                 fetched own/game@0.9.1 (1 files)"
            )),
            "{}",
            o.text
        );
        assert_eq!(o.json["id"], id.as_str());
        assert_eq!(o.json["id_from"], "index");
        assert_eq!(
            fs::read_to_string(into.join("package.toml")).unwrap(),
            "made_up = \"0.9.1\"\n"
        );
    }

    #[test]
    fn no_id_with_a_version_takes_that_versions_id_and_notes_a_notice_yank() {
        let (d, spec, roms) = scratch(&["0.1.0", "0.2.0"]);
        yank(&spec, "0.1.0", "notice");
        let o = fetch(
            "own/game@0.1.0",
            None,
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .unwrap();
        let id = indexed_id(&spec, "0.1.0");
        assert!(o.text.starts_with(&format!("id {id} for own/game@0.1.0, from the registry's index (index/own/game/0.1.0.toml)\nnote: own/game@0.1.0 was yanked: made up\n")), "{}", o.text);
        assert_eq!(o.exit, 0);
    }

    #[test]
    fn a_security_yank_is_fetched_only_with_its_id() {
        let (d, spec, roms) = scratch(&["0.1.0"]);
        yank(&spec, "0.1.0", "security");
        let e = fetch(
            "own/game@0.1.0",
            None,
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            e.contains("yanked for security") && e.contains("--id"),
            "{e}"
        );
        let e = fetch("own/game", None, &spec, None, &d.path().join("c"), &roms)
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains("every version of own/game") && e.contains("--id"),
            "{e}"
        );
        let id = indexed_id(&spec, "0.1.0");
        let o = fetch(
            "own/game@0.1.0",
            Some(&id),
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .unwrap();
        assert!(o.text.starts_with("fetched own/game@0.1.0"), "{}", o.text);
        assert_eq!(o.json["id_from"], "--id");
    }

    #[test]
    fn a_registry_with_no_yank_list_is_refused_without_an_id() {
        let (d, spec, roms) = scratch(&["0.1.0"]);
        let id = indexed_id(&spec, "0.1.0");
        fs::rename(
            Path::new(&spec).join("yanks.toml"),
            d.path().join("yanks.toml.moved"),
        )
        .unwrap();
        for wanted in ["own/game", "own/game@0.1.0"] {
            let e = fetch(wanted, None, &spec, None, &d.path().join("c"), &roms)
                .err()
                .unwrap()
                .to_string();
            assert!(
                e.contains("has no yank list (yanks.toml)")
                    && e.contains("`gp registry reindex` writes it")
                    && e.contains("--id"),
                "{e}"
            );
        }
        // With --id the yank list isn't read, as before.
        let o = fetch(
            "own/game@0.1.0",
            Some(&id),
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .unwrap();
        assert!(o.text.starts_with("fetched own/game@0.1.0"), "{}", o.text);
    }

    #[test]
    fn no_index_entry_says_so_and_names_the_flag() {
        let (d, spec, roms) = scratch(&["0.1.0"]);
        let e = fetch("own/other", None, &spec, None, &d.path().join("c"), &roms)
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains("lists no version of own/other") && e.contains("--id"),
            "{e}"
        );
        let e = fetch(
            "own/game@0.3.0",
            None,
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            e.contains("has no own/game@0.3.0 (index/own/game/0.3.0.toml)") && e.contains("--id"),
            "{e}"
        );
    }

    #[test]
    fn with_id_a_wrong_id_still_fails_and_a_bare_name_is_refused() {
        let (d, spec, roms) = scratch(&["0.1.0"]);
        let e = fetch(
            "own/game@0.1.0",
            Some("00"),
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(e.contains("not the id asked for"), "{e}");
        let e = fetch(
            "own/game",
            Some("00"),
            &spec,
            None,
            &d.path().join("c"),
            &roms,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(e.contains("wanted NAME@VERSION"), "{e}");
    }

    #[test]
    fn versions_order_by_number() {
        let mut v = vec!["1.0.10", "1.0.9", "1.0.0", "0.9"];
        v.sort_by(|a, b| version_order(a, b));
        assert_eq!(v, ["0.9", "1.0.0", "1.0.9", "1.0.10"]);
    }
}
