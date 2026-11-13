//! `gp assets hash` and `gp assets find` (docs/assets.md, "Content" and
//! "Finding assets in other dumps"): an asset map's content written from
//! the ROM it describes, and a game's assets found in any other dump of it
//! by that content, with the model picked by the code found.
//!
//! Ported from game-printer's assets_find.rs. Changed by gameprinter's
//! layout: a game's templates are the game folders of the games checkout
//! (their template.toml) and the tool repo's templates/, the ROM entries
//! are every game's roms.toml, and a draft entry is for roms.toml.
//!
//! An arcade romset (a MAME zip) has no content section to write: its
//! chips are known by their SHA-1s in gp-arcade's set lists (romset.rs),
//! so `hash` says so and `find` compares a zip's files with the chips of
//! every set the game's arcade maps list, by SHA-1, whatever they are
//! named.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_assets::find::{self, Found, Match, Report};
use gp_assets::{AssetMap, MAP_FILE, RomLayout, content};

use crate::Outcome;
use crate::assets::{find_map, outside_repo, write_file};

/// `gp assets hash ROM`: the content section of the ROM's map, written into
/// the map (or, with `check`, compared with it: exit 1 when it differs).
pub(crate) fn hash(
    rom_path: &Path,
    map_path: Option<&Path>,
    code: bool,
    check: bool,
) -> Result<Outcome> {
    let rom = fs::read(rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    if gp_arcade::romset::looks_like_zip(&rom) {
        return hash_arcade(rom_path, &rom);
    }
    let layout = RomLayout::of(&rom).with_context(|| rom_path.display().to_string())?;
    let (map_path, text, map) = find_map(map_path, &layout.sha256)?;
    layout.check(&map)?;
    let img = gp_evidence::source::identify_nes(&rom)?;
    let start = 16 + if img.header.trainer { 512 } else { 0 };
    let prg = &rom[start..start + img.header.prg_rom_bytes as usize];
    let chr_start = start + img.header.prg_rom_bytes as usize;
    let chr = &rom[chr_start..chr_start + img.header.chr_rom_bytes as usize];
    let with_code = code || map.code.is_some();
    let (entries, code) = content::compute(&map, prg, chr, with_code)?;
    let new = content::write_section(&text, &entries, code.as_ref());
    AssetMap::parse(&new).context("the map with its content section")?;
    let same = new == text;
    let summary = format!(
        "{}: {} spans hashed{}",
        map_path.display(),
        entries.len(),
        if code.is_some() {
            ", and the PRG's program"
        } else {
            ""
        }
    );
    let json = json!({
        "map": map_path,
        "source": map.source,
        "spans": entries.len(),
        "code": code.is_some(),
        "current": same,
    });
    if check {
        let mut o = Outcome::ok(
            json,
            if same {
                format!("{summary}: current")
            } else {
                format!("{summary}: the map's content section is stale; run gp assets hash")
            },
        );
        o.exit = u8::from(!same);
        return Ok(o);
    }
    if !same {
        write_file(&map_path, new.as_bytes())?;
    }
    Ok(Outcome::ok(
        json,
        format!(
            "{summary}{}",
            if same { " (unchanged)" } else { ", written" }
        ),
    ))
}

/// `gp assets hash` on a romset: an arcade map holds no content section,
/// since every set's chips are listed by SHA-1 in gp-arcade. Nothing is
/// written and nothing is stale.
fn hash_arcade(rom_path: &Path, rom: &[u8]) -> Result<Outcome> {
    let set = gp_arcade::Romset::from_zip(rom).map_err(anyhow::Error::msg)?;
    let info = gp_arcade::romset::set(set.name).expect("a set from SETS");
    Ok(Outcome::ok(
        json!({
            "rom": rom_path,
            "set": set.name,
            "identity": set.identity,
            "chips": info.chips.len(),
            "current": true,
        }),
        format!(
            "{}: MAME's {}, {} chips. An arcade map has no content section: each set's chips are listed by SHA-1 in gp-arcade's romset.rs, and gp assets find compares a zip's files with them. Nothing to write",
            rom_path.display(),
            set.name,
            info.chips.len()
        ),
    ))
}

/// One template of the game: its id, folder and asset map.
pub(crate) struct Candidate {
    pub id: String,
    pub map_path: PathBuf,
    pub map_text: String,
    /// The NES map; None for an arcade map.
    pub map: Option<AssetMap>,
}

/// The folders that may hold templates: each game of the games checkout
/// and each folder of the tool repo's templates/.
fn template_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut add = |parent: &Path| {
        if let Ok(rd) = fs::read_dir(parent) {
            let mut v: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join("template.toml").is_file())
                .collect();
            v.sort();
            dirs.extend(v);
        }
    };
    if let Some(games) = crate::roms_cmd::games_dir(None) {
        add(&games);
    }
    add(&crate::serve::repo_root().join("templates"));
    dirs
}

/// The templates whose `template.game` (or id) is `game`, with their maps.
pub(crate) fn candidates(game: &str) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for dir in template_dirs() {
        let t: toml::Table = toml::from_str(&fs::read_to_string(dir.join("template.toml"))?)
            .with_context(|| dir.join("template.toml").display().to_string())?;
        let s = |a: &str, b: &str| t.get(a).and_then(|v| v.get(b)).and_then(|v| v.as_str());
        let id = s("template", "id").unwrap_or_default().to_owned();
        if s("template", "game") != Some(game) && id != game {
            continue;
        }
        let Some(rel) = s("recipe", "asset_map") else {
            continue;
        };
        if s("source", "how") == Some("authored") {
            continue;
        }
        let map_path = dir.join(rel);
        let map_text = fs::read_to_string(&map_path)
            .with_context(|| format!("reading {}", map_path.display()))?;
        let map = if gp_assets::arcade::is_arcade(&map_text) {
            gp_assets::arcade::ArcadeMap::parse(&map_text)
                .with_context(|| map_path.display().to_string())?;
            None
        } else {
            Some(AssetMap::parse(&map_text).with_context(|| map_path.display().to_string())?)
        };
        out.push(Candidate {
            id,
            map_path,
            map_text,
            map,
        });
    }
    if out.is_empty() {
        bail!(
            "no template's template.game or id is {game:?} (looked in the games checkout and the tool repo's templates/)"
        );
    }
    Ok(out)
}

fn state_line(b: &find::Block) -> String {
    match &b.found {
        Found::Same { at } => format!("same     {}", at.text()),
        Found::Changed {
            at,
            same_bytes,
            differ,
        } => {
            let ranges: Vec<String> = differ
                .iter()
                .take(4)
                .map(|[a, n]| format!("+{a:#x} ({n})"))
                .collect();
            let more = if differ.len() > 4 {
                format!(" and {} more", differ.len() - 4)
            } else {
                String::new()
            };
            format!(
                "changed  {}: {same_bytes} of {} bytes agree; differ at {}{more}",
                at.text(),
                b.len,
                ranges.join(", ")
            )
        }
        Found::Missing { best_same_bytes } => {
            if *best_same_bytes > 0 {
                format!(
                    "missing  (at best {best_same_bytes} of {} bytes agree)",
                    b.len
                )
            } else {
                "missing".to_owned()
            }
        }
    }
}

/// What a match means, in words.
pub(crate) fn match_words(m: Match, id: &str, share: f64) -> String {
    match m {
        Match::Exact => format!("{id}, the ROM its template is for"),
        Match::Code => {
            format!("{id}: all of its program is in this ROM, so the game plays as that model")
        }
        Match::Close => format!(
            "{id}, the closest: {:.1}% of its program is in this ROM. The code is changed in places, so the game may not match this ROM's behaviour",
            share * 100.0
        ),
        Match::None => format!(
            "none: at most {:.1}% of a template's program ({id}) is in this ROM",
            share * 100.0
        ),
    }
}

/// A draft roms.toml entry for an NES image no game's roms.toml knows.
fn source_entry(
    id: &str,
    title: &str,
    bytes: &[u8],
    image: &gp_evidence::source::NesImage,
    date: &str,
    by: &str,
) -> Result<String> {
    let h = crate::movie::RomHashes::of(bytes)?;
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let k = |n: u64| n / 1024;
    let mut s = String::from("\n[[source]]\n");
    let _ = writeln!(s, "id = {}", q(id));
    let _ = writeln!(s, "title = {}", q(title));
    s += "platform = \"nes\"\nregion = \"ntsc\"\n";
    let _ = writeln!(s, "sha256 = {}", q(&image.sha256));
    let _ = writeln!(s, "md5 = {}", q(&h.md5));
    let _ = writeln!(s, "sha1 = {}", q(&h.sha1));
    let _ = writeln!(s, "ines_sha1 = {}", q(&h.ines_sha1));
    let _ = writeln!(s, "mapper = {}", image.header.mapper);
    s += "status = \"unverified\"\n";
    let _ = writeln!(
        s,
        "evidence = {}",
        q(&format!(
            "Measured {date} by {by} on the user's dump. Not yet checked against a published checksum or a cartridge database."
        ))
    );
    let _ = writeln!(
        s,
        "notes = {}",
        q(&format!(
            "Its header says mapper {}, {}K PRG and {}K CHR. The region is a guess; no board facts until a cartridge database confirms them.",
            image.header.mapper,
            k(image.header.prg_rom_bytes),
            k(image.header.chr_rom_bytes)
        ))
    );
    Ok(s)
}

/// `gp assets find GAME --rom ROM`.
pub(crate) fn find_cmd(
    game: &str,
    rom_path: &Path,
    lift_out: Option<&Path>,
    force: bool,
    home: &Path,
) -> Result<Outcome> {
    let rom = fs::read(rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    if gp_arcade::romset::looks_like_zip(&rom) {
        return find_arcade(game, rom_path, &rom, lift_out, force, home);
    }
    let image =
        gp_evidence::source::identify_nes(&rom).with_context(|| rom_path.display().to_string())?;
    let cands: Vec<Candidate> = candidates(game)?
        .into_iter()
        .filter(|c| c.map.is_some())
        .collect();
    if cands.is_empty() {
        bail!(
            "{game}'s templates are arcade games, and {} is an NES image",
            rom_path.display()
        );
    }
    let map_of = |c: &Candidate| c.map.clone().expect("NES candidates only");
    let t0 = std::time::Instant::now();
    let reports: Vec<Report> = cands
        .iter()
        .map(|c| find::find(&rom, c.map.as_ref().expect("NES candidates only")))
        .collect::<Result<_, _>>()?;
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let refs: Vec<&Report> = reports.iter().collect();
    let (pick, m) = find::choose(&refs).expect("at least one template");
    let chosen = &cands[pick];
    let chosen_map = map_of(chosen);
    let rep = &reports[pick];
    let share = |r: &Report| r.code.as_ref().map_or(0.0, |c| c.share());
    let registry = crate::open::load_registry()?;
    let known = registry.by_sha256(&image.sha256);

    let mut text = String::new();
    let _ = writeln!(
        text,
        "{}: sha256 {}, mapper {}, {}K PRG, {}K CHR, {}",
        rom_path.display(),
        image.sha256,
        image.header.mapper,
        image.header.prg_rom_bytes / 1024,
        image.header.chr_rom_bytes / 1024,
        match known {
            Some(k) => format!("registered as {}", k.id),
            None => "not registered".to_owned(),
        }
    );
    let _ = writeln!(text, "model: {}", match_words(m, &chosen.id, share(rep)));
    let _ = writeln!(text, "templates of {game} compared ({ms:.0} ms):");
    for (c, r) in cands.iter().zip(&reports) {
        let _ = writeln!(
            text,
            "  {:<20} program {:>5.1}%  assets: {} same, {} changed, {} missing{}",
            c.id,
            share(r) * 100.0,
            r.count("same"),
            r.count("changed"),
            r.count("missing"),
            if r.blocks.is_empty() {
                " (its map has no content section)"
            } else {
                ""
            }
        );
    }
    let _ = writeln!(
        text,
        "assets of {} ({}):",
        chosen.id,
        chosen.map_path.display()
    );
    let width = rep.blocks.iter().map(|b| b.part.len()).max().unwrap_or(0);
    for b in &rep.blocks {
        let _ = writeln!(text, "  {:<width$} {}", b.part, state_line(b));
    }
    let mut lifted_json = serde_json::Value::Null;
    if let Some(out) = lift_out {
        let out = outside_repo(out)?;
        if !force && fs::symlink_metadata(out.join(MAP_FILE)).is_ok() {
            bail!(
                "{} already holds lifted assets ({MAP_FILE}); pass --force to replace them",
                out.display()
            );
        }
        let (img, names) = find::assemble(&rom, &chosen_map, rep)?;
        let sub = AssetMap {
            assets: chosen_map
                .assets
                .iter()
                .filter(|a| names.iter().any(|n| n == a.name()))
                .cloned()
                .collect(),
            content: Vec::new(),
            code: None,
            ..chosen_map.clone()
        };
        let (files, lifted) = gp_assets::lift_any(&img, &sub, &chosen.map_text)?;
        fs::create_dir_all(&out)?;
        for f in &files {
            let p = out.join(&f.name);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            write_file(&p, &f.bytes)?;
        }
        let _ = writeln!(
            text,
            "lifted {} of {} assets into {} with {}'s map",
            lifted.len(),
            chosen_map.assets.len(),
            out.display(),
            chosen.id
        );
        lifted_json = json!({ "out": out, "assets": lifted });
    }
    let mut draft = None;
    if known.is_none() {
        let title = chosen_map
            .title
            .clone()
            .unwrap_or_else(|| chosen_map.source.clone());
        let d = source_entry(
            &format!("{game}-REGION-REVISION"),
            &title,
            &rom,
            &image,
            &crate::game_new::today(),
            &format!(
                "gp assets find, which found {} of {}'s {} assets unchanged, {} changed and {} missing",
                rep.count("same"),
                chosen.id,
                rep.assets.len(),
                rep.count("changed"),
                rep.count("missing")
            ),
        )?;
        let _ = write!(
            text,
            "a draft entry for the game's roms.toml (check each field):\n{d}"
        );
        draft = Some(d);
    }
    let json = json!({
        "rom": rom_path,
        "sha256": image.sha256,
        "mapper": image.header.mapper,
        "registered": known.map(|k| &k.id),
        "model": { "template": chosen.id, "match": m, "program_share": share(rep) },
        "ms": ms,
        "templates": cands.iter().zip(&reports).map(|(c, r)| json!({
            "template": c.id,
            "map": c.map_path,
            "program": r.code,
            "same": r.count("same"),
            "changed": r.count("changed"),
            "missing": r.count("missing"),
        })).collect::<Vec<_>>(),
        "report": rep,
        "lifted": lifted_json,
        "draft_entry": draft,
    });
    Ok(Outcome::ok(json, text.trim_end().to_owned()))
}

/// One chip of a set looked for in a zip.
#[derive(serde::Serialize)]
struct ChipFound {
    chip: &'static str,
    region: &'static str,
    size: usize,
    /// same, changed or missing.
    state: &'static str,
    /// The zip's file it was found as: by SHA-1 when same, by name when
    /// changed (the chip's own bytes aren't known here, only its hash).
    file: Option<String>,
}

/// One set's chips in a zip.
struct SetFound {
    set: &'static gp_arcade::romset::SetInfo,
    chips: Vec<ChipFound>,
}

impl SetFound {
    fn count(&self, state: &str) -> usize {
        self.chips.iter().filter(|c| c.state == state).count()
    }

    /// The share of the main CPU's program bytes found unchanged.
    fn program_share(&self) -> f64 {
        let (mut have, mut all) = (0, 0);
        for c in &self.chips {
            if c.region == "maincpu" {
                all += c.size;
                if c.state == "same" {
                    have += c.size;
                }
            }
        }
        if all == 0 {
            0.0
        } else {
            have as f64 / all as f64
        }
    }

    fn region_state(&self, region: &str) -> &'static str {
        let v: Vec<&ChipFound> = self.chips.iter().filter(|c| c.region == region).collect();
        if !v.is_empty() && v.iter().all(|c| c.state == "same") {
            "same"
        } else if v.iter().all(|c| c.state == "missing") {
            "missing"
        } else {
            "changed"
        }
    }
}

fn sha1_hex(d: &[u8]) -> String {
    use sha1::Digest as _;
    hex::encode(sha1::Sha1::digest(d))
}

/// Each chip of `set` in the zip's files: by SHA-1 under any name, else a
/// file of the chip's name and size with other bytes.
fn look_for(
    set: &'static gp_arcade::romset::SetInfo,
    files: &[(String, Vec<u8>, String)],
) -> SetFound {
    let by_hash: HashMap<&str, &str> = files
        .iter()
        .map(|(n, _, h)| (h.as_str(), n.as_str()))
        .collect();
    let chips = set
        .chips
        .iter()
        .map(|c| {
            let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_owned();
            if let Some(n) = by_hash.get(c.sha1) {
                return ChipFound {
                    chip: c.name,
                    region: c.region.name(),
                    size: c.size,
                    state: "same",
                    file: Some(base(n)),
                };
            }
            let named = files
                .iter()
                .find(|(n, d, _)| base(n) == c.name && d.len() == c.size);
            match named {
                // The chip's own bytes aren't here to compare with, only
                // its hash, so a changed chip has no count of bytes agreeing.
                Some((n, _, _)) => ChipFound {
                    chip: c.name,
                    region: c.region.name(),
                    size: c.size,
                    state: "changed",
                    file: Some(base(n)),
                },
                None => ChipFound {
                    chip: c.name,
                    region: c.region.name(),
                    size: c.size,
                    state: "missing",
                    file: None,
                },
            }
        })
        .collect();
    SetFound { set, chips }
}

/// `gp assets find GAME --rom ZIP` for an arcade game.
fn find_arcade(
    game: &str,
    rom_path: &Path,
    rom: &[u8],
    lift_out: Option<&Path>,
    force: bool,
    home: &Path,
) -> Result<Outcome> {
    let cands: Vec<Candidate> = candidates(game)?
        .into_iter()
        .filter(|c| c.map.is_none())
        .collect();
    if cands.is_empty() {
        bail!(
            "{game}'s templates are NES games, and {} is a romset",
            rom_path.display()
        );
    }
    let files: Vec<(String, Vec<u8>, String)> = gp_arcade::romset::read_zip(rom)
        .map_err(|e| anyhow::anyhow!("{}: {e}", rom_path.display()))?
        .into_iter()
        .map(|(n, d)| {
            let h = sha1_hex(&d);
            (n, d, h)
        })
        .collect();
    let t0 = std::time::Instant::now();
    // Each template's map lists the sets it fits; every one is compared.
    let mut rows = Vec::new();
    for c in &cands {
        let map = gp_assets::arcade::ArcadeMap::parse(&c.map_text)
            .with_context(|| c.map_path.display().to_string())?;
        for name in &map.sets {
            if let Some(set) = gp_arcade::romset::set(name) {
                rows.push((c, map.clone(), look_for(set, &files)));
            }
        }
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if rows.is_empty() {
        bail!("no set {game}'s arcade maps list is one gp-arcade knows");
    }
    // The set all of whose chips are here, else the one with the most of
    // its program, then the most chips.
    let pick = rows
        .iter()
        .position(|(_, _, f)| f.count("same") == f.chips.len())
        .unwrap_or_else(|| {
            let mut best = 0;
            for (i, (_, _, f)) in rows.iter().enumerate() {
                let b = &rows[best].2;
                if (f.program_share(), f.count("same")) > (b.program_share(), b.count("same")) {
                    best = i;
                }
            }
            best
        });
    let (chosen, map, found) = &rows[pick];
    let share = found.program_share();
    let m = if found.count("same") == found.chips.len() {
        Match::Exact
    } else if share >= 1.0 {
        Match::Code
    } else if share >= find::CLOSE_SHARE {
        Match::Close
    } else {
        Match::None
    };
    let identity = match m {
        Match::Exact => found.set.identity(),
        _ => gp_arcade::romset::identity(
            files
                .iter()
                .filter(|(_, _, h)| h != gp_arcade::romset::SSIO_PROM.sha1)
                .map(|(_, _, h)| h.as_str()),
        ),
    };
    let registry = crate::open::load_registry()?;
    let known = registry.by_sha256(&identity);

    let mut text = String::new();
    let _ = writeln!(
        text,
        "{}: a romset of {} files, identity {identity}, {}",
        rom_path.display(),
        files.len(),
        match known {
            Some(k) => format!("registered as {}", k.id),
            None => "not registered".to_owned(),
        }
    );
    let label = format!("{} ({})", chosen.id, found.set.name);
    let words = match m {
        Match::Exact => format!(
            "{}: the zip is MAME's {}, a set its asset map lists",
            chosen.id, found.set.name
        ),
        _ => match_words(m, &label, share),
    };
    let _ = writeln!(text, "model: {words}");
    let _ = writeln!(text, "sets of {game} compared ({ms:.0} ms):");
    for (c, _, f) in &rows {
        let _ = writeln!(
            text,
            "  {:<20} program {:>5.1}%  chips: {} same, {} changed, {} missing",
            format!("{} {}", c.id, f.set.name),
            f.program_share() * 100.0,
            f.count("same"),
            f.count("changed"),
            f.count("missing"),
        );
    }
    let _ = writeln!(
        text,
        "assets of {} ({}), by the {} set's chips:",
        chosen.id,
        chosen.map_path.display(),
        found.set.name
    );
    let assets: Vec<serde_json::Value> = map
        .assets
        .iter()
        .filter(|a| a.format.lifted())
        .map(|a| {
            let region = a.region.as_str();
            json!({ "name": a.name, "region": region, "state": found.region_state(region) })
        })
        .collect();
    for a in &assets {
        let _ = writeln!(
            text,
            "  {:<10} {:<8} {}",
            a["name"].as_str().unwrap_or(""),
            a["state"].as_str().unwrap_or(""),
            a["region"].as_str().unwrap_or("")
        );
    }
    let width = found.chips.iter().map(|c| c.chip.len()).max().unwrap_or(0);
    for c in &found.chips {
        let _ = writeln!(
            text,
            "    {:<width$} {:<8} {:<8} {}",
            c.chip,
            c.region,
            c.state,
            c.file.as_deref().unwrap_or("")
        );
    }
    let mut lifted_json = serde_json::Value::Null;
    if let Some(out) = lift_out {
        if m != Match::Exact {
            bail!(
                "the zip isn't a whole set ({} of {}'s {} chips found), so nothing is lifted: an arcade lift reads every graphics chip of one set",
                found.count("same"),
                found.set.name,
                found.chips.len()
            );
        }
        let o = crate::assets::lift_arcade(rom, Some(out), Some(&chosen.map_path), force, home)?;
        let _ = writeln!(text, "{}", o.text);
        lifted_json = o.json;
    }
    let mut draft = None;
    if known.is_none() {
        let title = map.title.clone().unwrap_or_else(|| map.source.clone());
        let mut d = String::from("\n[[source]]\n");
        let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
        let _ = writeln!(d, "id = {}", q(&format!("{game}-SET")));
        let _ = writeln!(d, "title = {}", q(&title));
        d += "platform = \"arcade\"\nregion = \"us\"\n";
        let _ = writeln!(d, "system = {}", q(found.set.name));
        let _ = writeln!(d, "sha256 = {}", q(&identity));
        d += "status = \"unverified\"\n";
        let _ = writeln!(
            d,
            "evidence = {}",
            q(&format!(
                "Measured {} by gp assets find on the user's romset, which found {} of {}'s {} chips unchanged, {} changed and {} missing. The sha256 is the SHA-256 of the zip's chips' SHA-1s sorted (gp_arcade::romset).",
                crate::game_new::today(),
                found.count("same"),
                found.set.name,
                found.chips.len(),
                found.count("changed"),
                found.count("missing")
            ))
        );
        let _ = write!(
            text,
            "a draft entry for the game's roms.toml (check each field):\n{d}"
        );
        draft = Some(d);
    }
    let json = json!({
        "rom": rom_path,
        "identity": identity,
        "registered": known.map(|k| &k.id),
        "model": { "template": chosen.id, "set": found.set.name, "match": m, "program_share": share },
        "ms": ms,
        "sets": rows.iter().map(|(c, _, f)| json!({
            "template": c.id,
            "map": c.map_path,
            "set": f.set.name,
            "program_share": f.program_share(),
            "same": f.count("same"),
            "changed": f.count("changed"),
            "missing": f.count("missing"),
        })).collect::<Vec<_>>(),
        "assets": assets,
        "chips": found.chips,
        "lifted": lifted_json,
        "draft_entry": draft,
    });
    Ok(Outcome::ok(json, text.trim_end().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_by_its_name_with_other_bytes_is_changed_and_one_not_there_is_missing() {
        let set = gp_arcade::romset::set("tapper").unwrap();
        let first = &set.chips[0];
        // Made-up bytes under the first chip's name, at its size.
        let bytes = vec![0x5A; first.size];
        let h = sha1_hex(&bytes);
        let files = vec![(format!("dir/{}", first.name), bytes, h)];
        let f = look_for(set, &files);
        assert_eq!(f.chips[0].state, "changed");
        assert_eq!(f.chips[0].file.as_deref(), Some(first.name));
        assert_eq!(f.count("missing"), set.chips.len() - 1);
        assert_eq!(f.program_share(), 0.0);
        assert_eq!(f.region_state("gfx2"), "missing");
        assert_eq!(f.region_state("maincpu"), "changed");
    }
}
