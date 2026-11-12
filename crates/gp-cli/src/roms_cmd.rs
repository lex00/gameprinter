//! `gp roms`: the ROMs a person owns, in `$GP_ROM_DIR`. `add` copies files
//! there, `scan` lists what's there, `clean` writes a copy without the junk
//! around a dump. Each file is named by the game whose `roms.toml` holds its
//! headerless hash (`sources`). No ROM byte is ever printed or written
//! anywhere but `$GP_ROM_DIR` or the path asked for, never in a repo.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_evidence::registry::Registry;
use gp_evidence::source::{identify_nes, looks_like_nes};
use serde_json::{Value, json};

use crate::{Outcome, romclean};

#[derive(Subcommand)]
pub enum RomsCmd {
    /// Identify every NES image and arcade romset (a MAME zip) in
    /// $GP_ROM_DIR, whatever it's named, by the game whose roms.toml holds
    /// its hash, and report junk around an NES image's game data.
    Scan {
        /// The games checkout whose roms.toml files name the ROMs (default:
        /// $GP_GAMES_DIR, else a `games` folder at or beside the current
        /// folder or one above it).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// Copy ROM files you own into $GP_ROM_DIR/nes/, or a romset (a MAME
    /// zip) into $GP_ROM_DIR/arcade/, identifying each one. A ROM no game
    /// names yet is added too, as unknown: `gp game new` starts a game for
    /// it.
    Add {
        /// The ROM files to add.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Move the files instead of copying them.
        #[arg(long = "move", conflicts_with = "clean")]
        move_files: bool,
        /// Add a cleaned copy instead, as `gp roms clean` writes it, with its
        /// sidecar. The original is left where it is. Exits 1 if a cleaned
        /// file matches no game's roms.toml.
        #[arg(long)]
        clean: bool,
        /// The games checkout (as for `scan`).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// Write a cleaned copy of a NES image: header from the game's roms.toml
    /// (or the file's, by the header rules), no trainer, no trailing bytes.
    /// Never changes the input. Exits 1 when the cleaned data matches no
    /// game's roms.toml.
    Clean {
        /// The NES image to clean. It's only read.
        file: PathBuf,
        /// Where to write the cleaned copy, outside any git work tree. A
        /// sidecar, `<out>.clean.toml`, records what was removed.
        #[arg(short, long)]
        out: PathBuf,
        /// Replace the output and its sidecar if they exist.
        #[arg(long)]
        force: bool,
        /// The games checkout (as for `scan`).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// Compare the PRG and CHR data of two NES images. Either side may be a
    /// raw PRG binary, such as a disassembly build.
    Diff(crate::romdiff::DiffArgs),
}

pub fn run(cmd: &RomsCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        RomsCmd::Scan { games } => {
            let (reg, from) = sources(games.as_deref())?;
            scan(paths, &reg, &from)
        }
        RomsCmd::Add {
            files,
            move_files,
            clean,
            games,
        } => {
            let (reg, from) = sources(games.as_deref())?;
            add(files, *move_files, *clean, paths, &reg, &from)
        }
        RomsCmd::Clean {
            file,
            out,
            force,
            games,
        } => {
            let (reg, _) = sources(games.as_deref())?;
            romclean::clean(file, out, *force, &reg)
        }
        RomsCmd::Diff(a) => crate::romdiff::run(a),
    }
}

/// The games checkout, if there is one: `--games`, $GP_GAMES_DIR, the
/// current folder or one above it when it holds a game (`GAME/roms.toml`),
/// else a `games` folder beside one of those.
pub fn games_dir(given: Option<&Path>) -> Option<PathBuf> {
    if let Some(g) = given {
        return Some(g.to_path_buf());
    }
    if let Some(g) = std::env::var_os("GP_GAMES_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(g));
    }
    let holds_games = |d: &Path| {
        fs::read_dir(d).is_ok_and(|rd| {
            rd.filter_map(Result::ok)
                .any(|e| e.path().join("roms.toml").is_file())
        })
    };
    let cwd = std::env::current_dir().ok()?;
    for d in cwd.ancestors() {
        if holds_games(d) {
            return Some(d.to_path_buf());
        }
        let beside = d.join("games");
        if holds_games(&beside) {
            return Some(beside);
        }
    }
    None
}

/// Every game's roms.toml in the games checkout, as one registry, and what
/// it was read from in words. With no checkout the registry is empty and
/// every ROM is reported as unknown.
pub fn sources(given: Option<&Path>) -> Result<(Registry, String)> {
    match games_dir(given) {
        Some(dir) => {
            let reg = Registry::load_games(&dir)
                .with_context(|| format!("reading the roms.toml files in {}", dir.display()))?;
            let from = format!("the roms.toml of each game in {}", dir.display());
            Ok((reg, from))
        }
        None => Ok((
            Registry::default(),
            "no games checkout (pass --games or set GP_GAMES_DIR), so no ROM is named".into(),
        )),
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let e = e?;
        let path = e.path();
        if e.file_type()?.is_dir() {
            // A folder named `sealed` is never opened (roms.rs).
            if !e.file_name().eq_ignore_ascii_case("sealed") {
                walk(&path, out)?;
            }
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn known_json(reg: &Registry, sha256: &str) -> Value {
    reg.by_sha256(sha256)
        .map_or(Value::Null, |k| json!({ "id": k.id, "title": k.title }))
}

/// What `scan` and `add` say of a romset: its set, its identity and what
/// else the zip held. Never its bytes.
fn romset_json(set: &gp_arcade::Romset, rel: &Path, reg: &Registry) -> Value {
    let title = gp_arcade::romset::set(set.name).map_or("", |s| s.title);
    let mut notes: Vec<String> = vec![format!("{} ({title})", set.name)];
    if !set.ssio_prom {
        notes.push(
            "no SSIO PROM (82s123.12d) in the zip: not needed while the sound is silent".into(),
        );
    }
    notes.extend(set.extras.iter().cloned());
    json!({
        "path": rel,
        "romset": {"system": set.name, "title": title, "identity": set.identity, "ssio_prom": set.ssio_prom, "extras": set.extras},
        "known": known_json(reg, &set.identity),
        "notes": notes,
    })
}

fn scan(paths: &Paths, reg: &Registry, from: &str) -> Result<Outcome> {
    if !paths.roms.is_dir() {
        return Ok(Outcome::ok(
            json!({ "roms_dir": paths.roms, "exists": false, "roms": [] }),
            format!(
                "No ROM folder at {}. `gp roms add FILE` makes it and copies a ROM you own there; ROMs never go in a repo.",
                paths.roms.display()
            ),
        ));
    }
    let mut files = Vec::new();
    walk(&paths.roms, &mut files)?;
    files.sort();
    let (mut roms, mut romsets, mut problems, mut skipped) =
        (Vec::new(), Vec::new(), Vec::new(), 0u64);
    for path in &files {
        let rel = path.strip_prefix(&paths.roms).unwrap_or(path).to_path_buf();
        let mut head = [0u8; 4];
        let read = fs::File::open(path)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
            .is_ok();
        if read && gp_arcade::romset::looks_like_zip(&head) {
            let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            match gp_arcade::Romset::from_zip(&bytes) {
                Ok(set) => romsets.push(romset_json(&set, &rel, reg)),
                Err(e) => problems.push(
                    json!({ "path": rel, "error": e.to_string(), "line": format!("a zip that is no romset gp knows: {e}") }),
                ),
            }
            continue;
        }
        if !(read && looks_like_nes(&head)) {
            skipped += 1;
            continue;
        }
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match identify_nes(&bytes) {
            Ok(img) => {
                let known = known_json(reg, &img.sha256);
                let (report, notes) = romclean::scan_findings(&bytes, &name, reg)?;
                roms.push(json!({ "path": rel, "image": img, "known": known, "report": report, "notes": notes }));
            }
            Err(e) => problems.push(
                json!({ "path": rel, "error": e.to_string(), "line": romclean::problem_line(&e) }),
            ),
        }
    }
    let mut text = format!(
        "{} NES images and {} arcade romsets in {} ({} other files skipped), named from {from}",
        roms.len(),
        romsets.len(),
        paths.roms.display(),
        skipped
    );
    for r in &roms {
        let img = &r["image"];
        let h = &img["header"];
        text.push_str(&format!(
            "\n  {}  mapper {:<3} PRG {:>4}K CHR {:>4}K  {:<22} {}",
            &img["sha256"].as_str().unwrap_or_default()[..16],
            h["mapper"],
            h["prg_rom_bytes"].as_u64().unwrap_or(0) / 1024,
            h["chr_rom_bytes"].as_u64().unwrap_or(0) / 1024,
            r["known"]["id"].as_str().unwrap_or("unknown"),
            r["path"].as_str().unwrap_or_default(),
        ));
        for line in r["notes"].as_array().into_iter().flatten() {
            text.push_str(&format!("\n      {}", line.as_str().unwrap_or_default()));
        }
    }
    for r in &romsets {
        text.push_str(&format!(
            "\n  {}  arcade {:<9} {:<22} {}",
            &r["romset"]["identity"].as_str().unwrap_or_default()[..16],
            r["romset"]["system"].as_str().unwrap_or_default(),
            r["known"]["id"].as_str().unwrap_or("unknown"),
            r["path"].as_str().unwrap_or_default(),
        ));
        for line in r["notes"].as_array().into_iter().flatten() {
            text.push_str(&format!("\n      {}", line.as_str().unwrap_or_default()));
        }
    }
    for p in &problems {
        text.push_str(&format!(
            "\n  problem: {}: {}",
            p["path"].as_str().unwrap_or_default(),
            p["line"].as_str().unwrap_or_default()
        ));
    }
    Ok(Outcome::ok(
        json!({ "roms_dir": paths.roms, "exists": true, "roms": roms, "romsets": romsets, "problems": problems, "skipped": skipped, "named_from": from }),
        text,
    ))
}

fn add(
    files: &[PathBuf],
    move_files: bool,
    clean: bool,
    paths: &Paths,
    reg: &Registry,
    from: &str,
) -> Result<Outcome> {
    let dir = paths.roms.join("nes");
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&paths.roms) {
        bail!(
            "$GP_ROM_DIR ({}) is inside the git work tree at {}; ROMs never go in a repo",
            paths.roms.display(),
            repo.display()
        );
    }
    let (mut results, mut text) = (Vec::new(), vec![format!("named from {from}")]);
    let mut unknown_cleaned = 0u64;
    for file in files {
        let original = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        if gp_arcade::romset::looks_like_zip(&original) {
            if clean {
                bail!(
                    "{} is an arcade romset: --clean is for NES images only",
                    file.display()
                );
            }
            let (r, line) = add_romset(file, &original, move_files, paths, reg)?;
            text.extend(line);
            results.push(r);
            continue;
        }
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let img = identify_nes(&original).map_err(|e| {
            anyhow::anyhow!(
                "{} is not a usable NES image: {}",
                file.display(),
                romclean::problem_line(&e)
            )
        })?;
        let name = file
            .file_name()
            .with_context(|| format!("{} has no file name", file.display()))?;
        let dest = dir.join(name);
        // With --clean, the cleaned copy goes in, through the same code as
        // `gp roms clean`; the original stays where it is.
        let job = if clean {
            Some(
                romclean::prepare(&original, reg)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?,
            )
        } else {
            None
        };
        let bytes = job.as_ref().map_or(&original[..], |j| &j.cleaned.bytes[..]);
        let mut sidecar = None;
        let action = if dest.exists() {
            if fs::read(&dest)? == bytes {
                "already there"
            } else {
                bail!(
                    "{} already exists with other contents; rename one of them",
                    dest.display()
                );
            }
        } else if let Some(job) = &job {
            if romclean::sidecar_path(&dest).exists() {
                bail!(
                    "{} already exists; move it away first",
                    romclean::sidecar_path(&dest).display()
                );
            }
            sidecar = Some(romclean::write(job, file, &dest, reg)?);
            "cleaned"
        } else {
            let tmp = dir.join(format!(".{}.partial", name.to_string_lossy()));
            fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
            if fs::read(&tmp)? != bytes {
                let _ = fs::remove_file(&tmp);
                bail!("the copy of {} didn't read back the same", file.display());
            }
            fs::rename(&tmp, &dest).with_context(|| format!("writing {}", dest.display()))?;
            if move_files { "moved" } else { "copied" }
        };
        if move_files && gp_evidence::paths::real_path(file) != gp_evidence::paths::real_path(&dest)
        {
            fs::remove_file(file).with_context(|| format!("removing {}", file.display()))?;
        }
        let known = reg.by_sha256(&img.sha256);
        if clean && known.is_none() {
            unknown_cleaned += 1;
        }
        text.push(format!(
            "{action:<14} {}  {}  {}",
            &img.sha256[..16],
            known.map_or("unknown: `gp game new` starts a game for it", |k| k
                .id
                .as_str()),
            dest.display()
        ));
        let (report, notes) = romclean::scan_findings(&original, &name.to_string_lossy(), reg)?;
        text.extend(notes.iter().map(|l| format!("      {l}")));
        results.push(json!({
            "file": file,
            "dest": dest,
            "action": action,
            "image": img,
            "known": known_json(reg, &img.sha256),
            "report": report,
            "removed": job.as_ref().map(|j| j.cleaned.removed()),
            "sidecar": sidecar,
        }));
    }
    if unknown_cleaned > 0 {
        text.push(format!(
            "{unknown_cleaned} cleaned file(s) match no game's roms.toml (exit 1)"
        ));
    }
    Ok(Outcome {
        json: json!({ "added": results, "named_from": from }),
        text: text.join("\n"),
        exit: u8::from(unknown_cleaned > 0),
    })
}

/// Copies (or moves) a file into `dir` under its own name: nothing is
/// written over, and the copy must read back the same. Returns the
/// destination and what was done.
fn put_in(
    dir: &Path,
    file: &Path,
    bytes: &[u8],
    move_files: bool,
) -> Result<(PathBuf, &'static str)> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = file
        .file_name()
        .with_context(|| format!("{} has no file name", file.display()))?;
    let dest = dir.join(name);
    let action = if dest.exists() {
        if fs::read(&dest)? != bytes {
            bail!(
                "{} already exists with other contents; rename one of them",
                dest.display()
            );
        }
        "already there"
    } else {
        let tmp = dir.join(format!(".{}.partial", name.to_string_lossy()));
        fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
        if fs::read(&tmp)? != bytes {
            let _ = fs::remove_file(&tmp);
            bail!("the copy of {} didn't read back the same", file.display());
        }
        fs::rename(&tmp, &dest).with_context(|| format!("writing {}", dest.display()))?;
        if move_files { "moved" } else { "copied" }
    };
    if move_files && gp_evidence::paths::real_path(file) != gp_evidence::paths::real_path(&dest) {
        fs::remove_file(file).with_context(|| format!("removing {}", file.display()))?;
    }
    Ok((dest, action))
}

/// `add` for an arcade romset: checked against the sets gp-arcade knows,
/// then copied whole into $GP_ROM_DIR/arcade/.
fn add_romset(
    file: &Path,
    bytes: &[u8],
    move_files: bool,
    paths: &Paths,
    reg: &Registry,
) -> Result<(Value, Vec<String>)> {
    let set = gp_arcade::Romset::from_zip(bytes)
        .map_err(|e| anyhow::anyhow!("{} is not a romset gp knows: {e}", file.display()))?;
    let (dest, action) = put_in(&paths.roms.join("arcade"), file, bytes, move_files)?;
    let known = reg.by_sha256(&set.identity);
    let rel = dest
        .strip_prefix(&paths.roms)
        .unwrap_or(&dest)
        .to_path_buf();
    let mut r = romset_json(&set, &rel, reg);
    let mut lines = vec![format!(
        "{action:<14} {}  {}  {}",
        &set.identity[..16],
        known.map_or("unknown: `gp game new` starts a game for it", |k| k
            .id
            .as_str()),
        dest.display()
    )];
    lines.extend(
        r["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| format!("      {}", l.as_str().unwrap_or_default())),
    );
    r["file"] = json!(file);
    r["dest"] = json!(dest);
    r["action"] = json!(action);
    Ok((r, lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up NROM image: a header and filler, never a real game's bytes.
    fn image(fill: u8) -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend(vec![fill; 16384 + 8192]);
        rom
    }

    fn paths(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            store: home.join("store"),
            roms: home.join("roms"),
        }
    }

    fn games_with(dir: &Path, id: &str, rom: &[u8]) {
        let sha = identify_nes(rom).unwrap().sha256;
        fs::create_dir_all(dir.join(id)).unwrap();
        fs::write(
            dir.join(id).join("roms.toml"),
            format!("[[source]]\nid = \"{id}\"\ntitle = \"A Test\"\nplatform = \"nes\"\nregion = \"ntsc\"\nsha256 = \"{sha}\"\nmapper = 0\nstatus = \"unverified\"\nevidence = \"made in a test\"\n"),
        )
        .unwrap();
    }

    #[test]
    fn add_copies_a_rom_in_and_names_it_by_the_games_roms_toml() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (image(1), image(2));
        let games = d.path().join("games");
        games_with(&games, "a-test", &a);
        fs::write(d.path().join("a.nes"), &a).unwrap();
        fs::write(d.path().join("b.nes"), &b).unwrap();
        let p = paths(d.path());
        let (reg, from) = sources(Some(&games)).unwrap();
        let out = add(
            &[d.path().join("a.nes"), d.path().join("b.nes")],
            false,
            false,
            &p,
            &reg,
            &from,
        )
        .unwrap();
        assert_eq!(out.exit, 0);
        assert_eq!(fs::read(p.roms.join("nes/a.nes")).unwrap(), a);
        assert_eq!(out.json["added"][0]["known"]["id"], "a-test");
        assert!(out.json["added"][1]["known"].is_null());
        assert!(out.text.contains("unknown: `gp game new`"));
        // Again: already there, and the originals are untouched.
        let again = add(&[d.path().join("a.nes")], false, false, &p, &reg, &from).unwrap();
        assert_eq!(again.json["added"][0]["action"], "already there");
        assert!(d.path().join("a.nes").is_file());
        // Scan finds both, one named.
        let s = scan(&p, &reg, &from).unwrap();
        assert_eq!(s.json["roms"].as_array().unwrap().len(), 2);
        assert!(s.text.contains("a-test"));
    }

    #[test]
    fn add_refuses_a_name_that_holds_another_rom_and_move_removes_the_original() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let (reg, from) = (Registry::default(), String::new());
        fs::write(d.path().join("a.nes"), image(1)).unwrap();
        add(&[d.path().join("a.nes")], true, false, &p, &reg, &from).unwrap();
        assert!(!d.path().join("a.nes").exists());
        fs::write(d.path().join("a.nes"), image(2)).unwrap();
        assert!(add(&[d.path().join("a.nes")], false, false, &p, &reg, &from).is_err());
    }

    #[test]
    fn a_rom_folder_inside_a_repo_is_refused() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".git")).unwrap();
        let p = paths(d.path());
        fs::write(d.path().join("a.nes"), image(1)).unwrap();
        let err = add(
            &[d.path().join("a.nes")],
            false,
            false,
            &p,
            &Registry::default(),
            "",
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("never go in a repo"), "{err}");
    }

    /// A zip that is no set gp-arcade knows (an empty one, made here) is a
    /// problem for scan and refused by add; NES files beside it still read.
    #[test]
    fn a_zip_that_is_no_known_romset_is_a_problem_and_is_not_added() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let p = paths(&home);
        fs::create_dir_all(p.roms.join("arcade")).unwrap();
        let mut empty_zip = b"PK\x05\x06".to_vec();
        empty_zip.resize(22, 0);
        fs::write(p.roms.join("arcade/nothing.zip"), &empty_zip).unwrap();
        fs::create_dir_all(p.roms.join("nes")).unwrap();
        fs::write(p.roms.join("nes/a.nes"), image(1)).unwrap();
        let out = scan(&p, &Registry::default(), "nowhere").unwrap();
        assert_eq!(out.json["roms"].as_array().unwrap().len(), 1);
        assert_eq!(out.json["romsets"].as_array().unwrap().len(), 0);
        let problem = out.json["problems"][0]["line"].as_str().unwrap();
        assert!(problem.contains("no set gp-arcade knows"), "{problem}");
        let src = d.path().join("other.zip");
        fs::write(&src, &empty_zip).unwrap();
        let e = add(&[src], false, false, &p, &Registry::default(), "nowhere")
            .err()
            .unwrap();
        assert!(e.to_string().contains("not a romset gp knows"), "{e}");
    }

    #[test]
    fn scan_with_no_rom_folder_says_how_to_make_one() {
        let d = tempfile::tempdir().unwrap();
        let s = scan(&paths(d.path()), &Registry::default(), "").unwrap();
        assert_eq!(s.json["exists"], false);
        assert!(s.text.contains("gp roms add"));
    }
}
