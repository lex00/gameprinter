//! `gp template extract` and `gp template pack` (docs/templates.md): a
//! template turned into a working local copy from the person's own ROM,
//! and an asset-free copy of a template for distribution.
//!
//! Ported from game-printer's template.rs with the same flags, steps and
//! files (extract.toml, recipe/asset-map.toml, a zip with fixed times).
//! What gameprinter's layout changed: a template is a game's package folder
//! or a folder of templates/; gameprinter has no full manifest check (the
//! old `gp template check`), so extract holds the ROM to the template's
//! pins as gameprinter's `gp template check` does, and pack runs gameprinter's
//! asset-free checks instead: `gp package scan` on a package folder (every
//! file sorted into ours, reference, from the ROM and needs rewrite) and,
//! when an accepted ROM is found, the pins. Both work on an arcade game:
//! the romset is found by its identity in $GP_ROM_DIR and lifted through
//! the arcade map.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use gp_evidence::Paths;

use crate::Outcome;
use crate::template::{lifted_digest, sha256_hex};

/// The manifest's file name.
const MANIFEST_FILE: &str = "template.toml";

/// Where a packed template keeps its asset map.
const PACKED_MAP: &str = "recipe/asset-map.toml";

/// A template folder and its manifest, read as TOML.
pub(crate) struct Template {
    pub(crate) dir: PathBuf,
    text: String,
    toml: toml::Table,
}

impl Template {
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let toml: toml::Table = text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            text,
            toml,
        })
    }

    pub(crate) fn str(&self, sect: &str, key: &str) -> Option<&str> {
        self.toml.get(sect)?.get(key)?.as_str()
    }

    pub(crate) fn id(&self) -> String {
        self.str("template", "id")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                self.dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
    }

    fn accepts(&self) -> Vec<String> {
        self.toml
            .get("source")
            .and_then(|s| s.get("accepts"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }

    fn slots(&self) -> Vec<String> {
        self.toml
            .get("slots")
            .and_then(toml::Value::as_table)
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The `[ext]` hooks, and whether an `[ext]` crate implements them.
    fn hooks(&self) -> (usize, bool) {
        let ext = self.toml.get("ext");
        let hooks = ext
            .and_then(|e| e.get("hooks"))
            .map_or(0, |h| h.as_table().map_or(0, toml::Table::len));
        let krate = ext.and_then(|e| e.get("crate")).is_some();
        (hooks, krate)
    }

    /// A sentence for a refusal: the [[loads]] entry that accepts `sha`.
    fn loads_note(&self, sha: &str) -> String {
        self.toml
            .get("loads")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .find(|l| {
                l.get("accepts")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|a| a.iter().any(|h| h.as_str() == Some(sha)))
            })
            .map(|l| {
                format!(
                    ". It is a ROM the template loads only to play ([[loads]] {}): nothing scores, gates, records clips on or derives from it",
                    l.get("source").and_then(|v| v.as_str()).unwrap_or("")
                )
            })
            .unwrap_or_default()
    }
}

fn problem(file: &str, message: impl Into<String>) -> Value {
    json!({ "file": file, "message": message.into() })
}

fn finish(t: &Template, problems: Vec<Value>, mut json: Value, text: String) -> Outcome {
    let ok = problems.is_empty();
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
    json["ok"] = json!(ok);
    json["id"] = json!(t.id());
    json["dir"] = json!(t.dir);
    json["problems"] = json!(problems);
    Outcome {
        json,
        text: out.trim_end().to_owned(),
        exit: u8::from(!ok),
    }
}

/// One step of extract's report.
fn step(steps: &mut Vec<Value>, text: &mut String, name: &str, ok: bool, line: String) {
    let _ = write!(
        text,
        "\n  {name:<10} {}{line}",
        if ok { "" } else { "FAILED: " }
    );
    steps.push(json!({ "step": name, "ok": ok, "detail": line }));
}

/// Refuses a path inside a git work tree.
fn outside_repo(path: &Path, what: &str) -> Result<PathBuf> {
    let path =
        std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&path) {
        bail!(
            "{} is inside the git work tree at {}; {what} stay outside any repo (by default in $GP_HOME/work/templates)",
            path.display(),
            repo.display()
        );
    }
    Ok(path)
}

fn write_plain(path: &Path, bytes: &[u8]) -> Result<()> {
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

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The ROM: the one given, or the first file in $GP_ROM_DIR (an NES image
/// or a romset zip) whose hash the template accepts.
pub(crate) fn find_rom(
    t: &Template,
    given: Option<&Path>,
    paths: &Paths,
) -> Result<Option<(PathBuf, String)>> {
    if let Some(p) = given {
        let r = crate::roms::Rom::read(p)?;
        return Ok(Some((p.to_path_buf(), r.sha256)));
    }
    // An original's image is built from its own authored.toml, and
    // written where gp template build writes it.
    if t.str("source", "how") == Some("authored") {
        let built = gp_assets::authored::Authored::load(&t.dir)
            .and_then(|a| a.build())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let source = t.str("source", "id").map_or_else(|| t.id(), str::to_owned);
        let path = crate::template_build::image_path(paths, &source);
        write_plain(&path, &built.image)?;
        let sha = gp_evidence::source::identify_nes(&built.image)?.sha256;
        return Ok(Some((path, sha)));
    }
    if !paths.roms.is_dir() {
        return Ok(None);
    }
    let accepts = t.accepts();
    let mut folder = crate::open::RomFolder::new(paths);
    for a in &accepts {
        if let Some(p) = folder.find(a)? {
            return Ok(Some((p.to_path_buf(), a.clone())));
        }
    }
    // A romset's identity isn't an NES hash: read the zips.
    let arcade = paths.roms.join("arcade");
    let mut zips: Vec<PathBuf> = fs::read_dir(&arcade)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
                .collect()
        })
        .unwrap_or_default();
    zips.sort();
    for z in zips {
        if let Ok(r) = crate::roms::Rom::read(&z)
            && accepts.contains(&r.sha256)
        {
            return Ok(Some((z, r.sha256)));
        }
    }
    Ok(None)
}

/// Every file of a template folder, by its path inside it, sorted: no
/// `.git`, no build output (`target/`), and refusing symlinks.
fn template_files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok())
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
                bail!("{rel} is a symlink; a template holds only plain files");
            } else if ft.is_dir() {
                if e.file_name() == ".git" || e.file_name() == "target" {
                    continue;
                }
                walk(base, &path, out)?;
            } else if ft.is_file() {
                out.push((rel, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// The manifest text with `recipe.asset_map` pointing at the packed copy.
fn packed_manifest(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut done = false;
    for line in text.lines() {
        let l = line.trim_start();
        if !done && (l.starts_with("asset_map ") || l.starts_with("asset_map=")) {
            out.push_str(&format!("asset_map = {}", toml_str(PACKED_MAP)));
            done = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !done {
        bail!("{MANIFEST_FILE} has no asset_map line to point at the packed map");
    }
    Ok(out)
}

/// What a packed template holds: every template file, and the asset map
/// at PACKED_MAP with the manifest pointing there, so the copy stands alone.
fn packed_entries(t: &Template) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut entries = BTreeMap::new();
    for (rel, path) in template_files(&t.dir)? {
        entries.insert(rel, fs::read(&path)?);
    }
    if let Some(map) = t.str("recipe", "asset_map") {
        entries.insert(
            MANIFEST_FILE.to_owned(),
            packed_manifest(&t.text)?.into_bytes(),
        );
        entries.insert(PACKED_MAP.to_owned(), fs::read(t.dir.join(map))?);
    }
    Ok(entries)
}

/// `gp template extract DIR`.
pub(crate) fn extract(
    dir: &Path,
    rom_arg: Option<&Path>,
    out: Option<&Path>,
    root: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let t = Template::load(dir)?;
    let id = t.id();
    let mut problems = Vec::new();
    let mut steps = Vec::new();
    let mut text = format!(
        "template {id} ({}): {}",
        t.str("template", "tier").unwrap_or("?"),
        t.str("template", "title").unwrap_or("")
    );

    // 1. The manifest: the recipe's map and its pin.
    let format = t.str("template", "format").unwrap_or("?").to_owned();
    let map_rel = t.str("recipe", "asset_map").map(str::to_owned);
    let map_pin = t.str("recipe", "asset_map_sha256").map(str::to_owned);
    let map_text = match &map_rel {
        Some(rel) => {
            let p = t.dir.join(rel);
            Some(fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?)
        }
        None => None,
    };
    match (&map_text, &map_pin) {
        (None, _) => problems.push(problem(MANIFEST_FILE, "no recipe.asset_map to lift with")),
        (Some(m), Some(pin)) if sha256_hex(m.as_bytes()) != *pin => problems.push(problem(
            MANIFEST_FILE,
            format!(
                "recipe.asset_map_sha256 is {pin}, and {} has {}",
                map_rel.as_deref().unwrap_or(""),
                sha256_hex(m.as_bytes())
            ),
        )),
        (Some(_), None) => problems.push(problem(MANIFEST_FILE, "no recipe.asset_map_sha256")),
        _ => {}
    }
    let slots = t.slots();
    step(
        &mut steps,
        &mut text,
        "manifest",
        problems.is_empty(),
        format!("format {format}, {} slots", slots.len()),
    );
    let Some(map_text) = map_text.filter(|_| problems.is_empty()) else {
        return Ok(finish(&t, problems, json!({ "steps": steps }), text));
    };

    // 2. The ROM, by hash.
    let Some((rom_path, rom_sha)) = find_rom(&t, rom_arg, paths)? else {
        bail!(
            "no ROM the template accepts (sha256 {}); put your own dump of {} in {} (gp roms add, docs/sources.md) or give it with --rom",
            t.accepts().join(" or "),
            t.str("source", "id").unwrap_or(&id),
            paths.roms.display()
        );
    };
    let registry = crate::open::load_registry().ok();
    let which = registry
        .as_ref()
        .and_then(|r| r.by_sha256(&rom_sha))
        .map(|k| (k.id.clone(), format!("{:?}", k.status).to_lowercase()));
    if !t.accepts().contains(&rom_sha) {
        let line = format!(
            "{} has sha256 {rom_sha}{}, which the template doesn't accept ({}){}",
            rom_path.display(),
            which
                .as_ref()
                .map(|(i, _)| format!(" ({i})"))
                .unwrap_or_default(),
            t.accepts().join(", "),
            t.loads_note(&rom_sha)
        );
        problems.push(problem(MANIFEST_FILE, line.clone()));
        step(&mut steps, &mut text, "rom", false, line);
        return Ok(finish(&t, problems, json!({ "steps": steps }), text));
    }
    let note = match t.str("source", "how") {
        Some("authored") => ", built from the template's authored.toml".to_owned(),
        Some("build") => ", a build from source".to_owned(),
        _ => which
            .map(|(i, s)| format!(", {i} in the registry ({s})"))
            .unwrap_or_default(),
    };
    step(
        &mut steps,
        &mut text,
        "rom",
        true,
        format!("{} sha256 {rom_sha} accepted{note}", rom_path.display()),
    );

    // 3. Lift with the template's own asset map, into the work directory.
    // Extracting again replaces an earlier extraction's lift: the work
    // directory is the template's, rebuilt from the ROM each time, and
    // edits belong in a fork of it.
    let work = match out {
        Some(o) => o.to_path_buf(),
        None => paths.home.join("work").join("templates").join(&id),
    };
    let work = outside_repo(&work, "extracted templates")?;
    let assets_dir = work.join("assets");
    let rom = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    let (lifted, about) = crate::assets::lift_files(&rom, &map_text)
        .with_context(|| map_rel.clone().unwrap_or_default())?;
    let mut files = BTreeMap::new();
    for f in lifted {
        write_plain(&assets_dir.join(&f.name), &f.bytes)?;
        files.insert(f.name, f.bytes);
    }
    let digest = lifted_digest(&files);
    let lifted_pin = t.str("recipe", "lifted_sha256").map(str::to_owned);
    let pin_ok = lifted_pin.as_ref().is_none_or(|p| *p == digest);
    let covered = gp_assets::AssetMap::parse(&map_text)
        .map(|m| format!(", {} ROM bytes", m.bytes_covered()))
        .unwrap_or_default();
    let line = format!(
        "{} assets, {} files{covered} into {}; digest {digest}{}",
        about.len(),
        files.len(),
        assets_dir.display(),
        match &lifted_pin {
            None => " (no pin in the manifest yet)".to_owned(),
            Some(p) if *p == digest => ", matches the pin".to_owned(),
            Some(p) => format!(", but the manifest pins {p}"),
        },
    );
    if !pin_ok {
        problems.push(problem(
            MANIFEST_FILE,
            format!("recipe.lifted_sha256: {line}"),
        ));
    }
    step(&mut steps, &mut text, "lift", pin_ok, line);

    // 4. The round trip: the lifted files encode back to the ROM's bytes.
    let rt = crate::assets::check_dir(&assets_dir, &rom_path, None)?;
    let rt_ok = rt.exit == 0;
    if !rt_ok {
        problems.push(problem("assets", rt.text.clone()));
    }
    let diffs = rt.json["difference_count"]
        .as_u64()
        .or(rt.json["differences"].as_u64())
        .unwrap_or(0);
    step(
        &mut steps,
        &mut text,
        "roundtrip",
        rt_ok,
        match rt.json.get("bytes_covered") {
            Some(b) => format!("{b} bytes, {diffs} differences"),
            None => format!("{diffs} differences"),
        },
    );

    // 5. The local copy: the template's files beside the lifted assets.
    let entries = packed_entries(&t)?;
    for (rel, bytes) in &entries {
        write_plain(&work.join(rel), bytes)?;
    }
    step(
        &mut steps,
        &mut text,
        "copy",
        true,
        format!("{} template files into {}", entries.len(), work.display()),
    );

    // 6. The model, with its slots checked against the lifted assets.
    let root = match root {
        Some(r) => r.to_path_buf(),
        None if t.dir.join("package.toml").is_file() => std::path::absolute(&t.dir)?,
        None => gp_evidence::paths::enclosing_git_work_tree(&std::path::absolute(&t.dir)?)
            .unwrap_or_else(|| t.dir.clone()),
    };
    let mc = crate::model::check_in(&work, Some(&assets_dir), Some(&root), &paths.home)?;
    let checked: Vec<String> = mc.json["assets_checked"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let mc_ok = mc.exit == 0;
    for p in mc.json["problems"].as_array().into_iter().flatten() {
        problems.push(problem(
            p["file"].as_str().unwrap_or(""),
            format!(
                "{} {}",
                p["path"].as_str().unwrap_or(""),
                p["message"].as_str().unwrap_or("")
            ),
        ));
    }
    let unfilled: Vec<&String> = slots.iter().filter(|s| !checked.contains(s)).collect();
    step(
        &mut steps,
        &mut text,
        "model",
        mc_ok && unfilled.is_empty(),
        format!(
            "{} facts, {} of {} slots filled and checked{}{}",
            mc.json["facts"],
            checked.len(),
            slots.len(),
            if unfilled.is_empty() {
                String::new()
            } else {
                format!(
                    "; not filled: {}",
                    unfilled
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            match mc.json["refs_skipped"].as_u64() {
                Some(n) if n > 0 => format!(
                    "; {n} evidence refs into pinned downloads not checked (not in $GP_HOME/work)"
                ),
                _ => String::new(),
            }
        ),
    );
    if !unfilled.is_empty() {
        problems.push(problem(
            MANIFEST_FILE,
            format!(
                "slots not filled by the recipe: {}",
                unfilled
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    // 7. What's ready, written beside the copy with every hash.
    let (hooks, has_crate) = t.hooks();
    let ok = problems.is_empty();
    let mut record = String::new();
    let _ = writeln!(
        record,
        "# Written by gp template extract (docs/templates.md). Lifted from the person's own copy: never commit this directory."
    );
    let _ = writeln!(record, "template = {}", toml_str(&id));
    let _ = writeln!(record, "format = {}", toml_str(&format));
    let _ = writeln!(record, "ok = {ok}");
    let _ = writeln!(record, "rom_sha256 = {}", toml_str(&rom_sha));
    let _ = writeln!(
        record,
        "asset_map_sha256 = {}",
        toml_str(map_pin.as_deref().unwrap_or(""))
    );
    let _ = writeln!(record, "lifted_sha256 = {}", toml_str(&digest));
    let _ = writeln!(
        record,
        "slots_ready = [{}]",
        checked
            .iter()
            .map(|s| toml_str(s))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let _ = writeln!(
        record,
        "ext_hooks_unimplemented = {}",
        if has_crate { 0 } else { hooks }
    );
    write_plain(&work.join("extract.toml"), record.as_bytes())?;
    let _ = write!(
        text,
        "\n  {:<10} {}: model{}, {} of {} slots{}",
        if ok { "ready" } else { "not ready" },
        work.display(),
        if t.str("template", "contract").is_some() {
            ", contract"
        } else {
            ""
        },
        checked.len(),
        slots.len(),
        if hooks > 0 && !has_crate {
            format!("; {hooks} ext hooks have no implementation yet, so those entities don't run")
        } else {
            String::new()
        }
    );
    Ok(finish(
        &t,
        problems,
        json!({
            "steps": steps,
            "out": work,
            "rom": rom_path,
            "rom_sha256": rom_sha,
            "lifted_sha256": digest,
            "slots_ready": checked,
            "ext_hooks_unimplemented": if has_crate { 0 } else { hooks },
        }),
        text,
    ))
}

/// The checks pack runs first: `gp package scan` on a package folder, and
/// the pins against an accepted ROM when one is found.
fn pack_checks(t: &Template, rom: Option<&Path>, paths: &Paths) -> Result<(Vec<Value>, String)> {
    let mut problems = Vec::new();
    let mut text = String::new();
    if t.dir.join("package.toml").is_file() {
        let given: Vec<PathBuf> = rom.map(Path::to_path_buf).into_iter().collect();
        let scan = crate::package::scan::run(&t.dir, &given, &[], paths)?;
        let _ = writeln!(text, "{}", scan.text);
        if scan.exit != 0 {
            problems.push(problem("package.toml", "gp package scan found a problem"));
        }
    }
    match find_rom(t, rom, paths)? {
        Some((path, _)) if t.str("recipe", "asset_map").is_some() => {
            let c = crate::template::check(&t.dir, &path)?;
            let _ = writeln!(text, "{}", c.text);
            if c.exit != 0 {
                problems.push(problem(MANIFEST_FILE, "the pins aren't what the ROM gives"));
            }
        }
        Some(_) => {}
        None => {
            let _ = writeln!(
                text,
                "no ROM the template accepts in {}: the pins aren't checked",
                paths.roms.display()
            );
        }
    }
    Ok((problems, text.trim_end().to_owned()))
}

/// `gp template pack DIR -o OUT`.
pub(crate) fn pack(
    dir: &Path,
    out: &Path,
    rom: Option<&Path>,
    force: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let out = &outside_repo(out, "packed templates")?;
    let t = Template::load(dir)?;
    let (problems, check_text) = pack_checks(&t, rom, paths)?;
    if !problems.is_empty() {
        return Ok(finish(
            &t,
            problems,
            json!({}),
            format!("{check_text}\nnot packed: the template isn't asset-free"),
        ));
    }
    let entries = packed_entries(&t)?;
    let is_zip = out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let (kind, sha) = if is_zip {
        if out.exists() && !force {
            bail!("{} exists; give --force to replace it", out.display());
        }
        let bytes = zip_bytes(&entries)?;
        write_plain(out, &bytes)?;
        ("zip", Some(sha256_hex(&bytes)))
    } else {
        if out.exists()
            && fs::read_dir(out)
                .with_context(|| format!("reading {}", out.display()))?
                .next()
                .is_some()
        {
            bail!(
                "{} isn't empty; pack writes only into a new or empty directory",
                out.display()
            );
        }
        for (rel, bytes) in &entries {
            write_plain(&out.join(rel), bytes)?;
        }
        ("directory", None)
    };
    let digest = lifted_digest(&entries);
    let mut text = format!(
        "{check_text}\npacked {} files into {} ({kind}); content digest {digest}",
        entries.len(),
        out.display()
    );
    if let Some(s) = &sha {
        let _ = write!(text, ", archive sha256 {s}");
    }
    Ok(finish(
        &t,
        Vec::new(),
        json!({
            "out": out,
            "kind": kind,
            "files": entries.keys().collect::<Vec<_>>(),
            "content_sha256": digest,
            "archive_sha256": sha,
        }),
        text,
    ))
}

/// A zip archive of the entries, sorted, with fixed times and permissions,
/// so the same entries always give the same bytes.
fn zip_bytes(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cur);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::DEFAULT)
            .unix_permissions(0o644);
        for (name, bytes) in entries {
            zip.start_file(name.as_str(), opts)?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
    }
    Ok(cur.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_packed_manifest_points_at_the_packed_map_and_a_zip_is_the_same_bytes_twice() {
        let text = "[template]\nid = \"t\"\n\n[recipe]\nasset_map = \"assets/t.toml\"\nasset_map_sha256 = \"x\"\n";
        let packed = packed_manifest(text).unwrap();
        assert!(packed.contains("asset_map = \"recipe/asset-map.toml\""));
        assert!(packed.contains("asset_map_sha256 = \"x\""));
        assert!(packed_manifest("[template]\nid = \"t\"\n").is_err());
        let entries = BTreeMap::from([
            ("a.toml".to_owned(), b"a = 1\n".to_vec()),
            ("b/c.toml".to_owned(), b"c = 2\n".to_vec()),
        ]);
        assert_eq!(zip_bytes(&entries).unwrap(), zip_bytes(&entries).unwrap());
    }
}
