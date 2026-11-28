//! The MAME captures a game names (GAME/oracle/mame-captures.toml in the
//! games checkout), and `gp oracle golden` and `gp oracle check` on arcade
//! against them.
//!
//! A capture is derived from the ROM, so it lives on each machine under
//! `$GP_HOME/work/oracle/NAME` (where `just mame-record` and `gp oracle
//! record` put them, and `just check-all`'s MAME match reads them), never
//! in a repo. The game's file holds only how to make each one again (the
//! MAME version, the romset's MAME name and the input log, with its hash)
//! and the hash the capture must have, so a missing capture is named with
//! the command that makes it, and one that isn't the capture named is
//! refused. The tool knows no game by name, so it holds no captures file of
//! its own: `--against NAME` looks in every game's. docs/oracle-mame.md,
//! "Captures the repo names".

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use sha2::Digest as _;

use gp_arcade::oracle::{Capture, InputLog as OracleLog};
use gp_evidence::manifest::{RecordingManifest, SourceRef};
use gp_evidence::recordings::IndexEntry;
use gp_evidence::{Paths, Store};

use crate::Outcome;
use crate::open::now_rfc3339;
use crate::oracle_mame::MAME_VERSION;

/// The file, from a game's folder.
pub(crate) const FILE: &str = "oracle/mame-captures.toml";

/// The files a capture's hash covers, in this order, those it has: what
/// `just mame-record` digests into SHA256SUMS, less the pictures and the
/// speakers' WAV, which a capture made for comparing needn't have.
const HASHED: [&str; 4] = ["ay.tsv", "capture.txt", "frames.tsv", "ram.bin"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Captures {
    pub schema: u32,
    #[serde(default, rename = "capture")]
    pub captures: Vec<Entry>,
    /// The captures `gp oracle board` replays against the board
    /// (crate::oracle_board): each by its folder's name and its input
    /// log. Made once and kept on this machine, with no hash.
    #[serde(default, rename = "replay")]
    pub replays: Vec<Replay>,
    /// Where the file is: input paths are from its game's folder.
    #[serde(skip)]
    pub root: PathBuf,
    #[serde(skip)]
    pub path: PathBuf,
}

/// A capture the arcade tests replay: its folder's name and input log.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replay {
    pub name: String,
    /// The input log, from the game's folder.
    pub input: String,
}

/// One capture: its folder's name, how to make it again, and its hash.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub name: String,
    /// MAME's name for the romset (roms.toml's `system`).
    pub system: String,
    /// The MAME version that made it (the image `just mame-build` makes).
    pub mame: String,
    /// The input log, from the game's folder.
    pub input: String,
    pub input_sha256: String,
    pub frames: u64,
    /// The capture's hash ([`capture_hash`]).
    pub sha256: String,
}

impl Captures {
    /// Whether a TOML file is a captures file rather than a baseline.
    pub(crate) fn is_one(path: &Path) -> bool {
        fs::read_to_string(path)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .is_some_and(|t| t.contains_key("capture"))
    }

    pub(crate) fn load(path: &Path) -> Result<Captures> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut c: Captures =
            toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        if c.schema != 0 {
            bail!("{} is schema {}; this gp reads 0", path.display(), c.schema);
        }
        let abs = std::path::absolute(path)?;
        c.root = abs
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        c.path = abs;
        // A capture the tests replay must name an input log the game has.
        for r in &c.replays {
            if !c.root.join(&r.input).is_file() {
                bail!(
                    "{}: [[replay]] {}'s input {} isn't in the game's folder",
                    path.display(),
                    r.name,
                    r.input
                );
            }
        }
        Ok(c)
    }

    /// Every captures file there is: the one in the current folder or
    /// above it (a game's folder), then each game's of the games checkout
    /// ([`crate::roms_cmd::games_dir`]).
    pub(crate) fn files() -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir()
            && let Some(p) = cwd.ancestors().map(|d| d.join(FILE)).find(|p| p.is_file())
        {
            out.push(p);
        }
        if let Some(games) = crate::roms_cmd::games_dir(None) {
            let mut found: Vec<PathBuf> = fs::read_dir(&games)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join(FILE))
                .filter(|p| p.is_file())
                .collect();
            found.sort();
            out.extend(found);
        }
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|p| seen.insert(fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
        out
    }

    /// The captures file that names capture `name`, among [`Captures::files`].
    pub(crate) fn find(name: &str) -> Result<Captures> {
        let files = Captures::files();
        if files.is_empty() {
            bail!(
                "no game names a MAME capture: no {FILE} in a game's folder of the games checkout ($GP_GAMES_DIR, else a games folder beside the working folder), or in the current folder or above it"
            );
        }
        let mut names = Vec::new();
        for f in &files {
            let c = Captures::load(f)?;
            if c.named(name).is_some() {
                return Ok(c);
            }
            names.extend(c.captures.iter().map(|e| e.name.clone()));
        }
        bail!(
            "{name} isn't a capture a game names; {} name {}. A capture is held to only once a game names it with its hash",
            files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            names.join(", ")
        )
    }

    pub(crate) fn named(&self, name: &str) -> Option<&Entry> {
        self.captures.iter().find(|e| e.name == name)
    }

    /// The entry's input log, checked against its hash.
    pub(crate) fn input(&self, e: &Entry) -> Result<String> {
        let path = self.root.join(&e.input);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading {}, capture {}'s input", path.display(), e.name))?;
        let h = hex(&sha2::Sha256::digest(text.as_bytes()));
        if h != e.input_sha256 {
            bail!(
                "{} has SHA-256 {h}, and {} says {} for capture {}: the input changed since the capture was made",
                path.display(),
                self.path.display(),
                e.input_sha256,
                e.name
            );
        }
        Ok(text)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A capture folder's hash: the SHA-256 of `shasum -a 256`'s line for each
/// of [`HASHED`] it has (`HASH  ./FILE`), in that order. For a capture made
/// with no pictures and no WAV, it is the SHA-256 of its SHA256SUMS file.
pub(crate) fn capture_hash(dir: &Path) -> Result<String> {
    let mut lines = String::new();
    for f in HASHED {
        let p = dir.join(f);
        if !p.is_file() {
            continue;
        }
        let mut h = sha2::Sha256::new();
        let mut file = fs::File::open(&p)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        lines.push_str(&format!("{}  ./{f}\n", hex(&h.finalize())));
    }
    if lines.is_empty() {
        bail!("{} holds none of {}", dir.display(), HASHED.join(", "));
    }
    Ok(hex(&sha2::Sha256::digest(lines.as_bytes())))
}

/// Where an entry's capture is kept on this machine.
pub(crate) fn dir_of(e: &Entry, paths: &Paths) -> PathBuf {
    paths.home.join("work").join("oracle").join(&e.name)
}

/// How to make an entry's capture, in words.
pub(crate) fn how_to_make(c: &Captures, e: &Entry, zip: Option<&Path>, paths: &Paths) -> String {
    let zip = zip.map_or_else(
        || format!("$GP_ROM_DIR/arcade/{}.zip", e.system),
        |z| z.display().to_string(),
    );
    let image = if e.mame == MAME_VERSION {
        format!("`just mame-build` once (MAME {})", e.mame)
    } else {
        format!(
            "MAME {} (this checkout's image is MAME {MAME_VERSION}, so the hash may not come out the same)",
            e.mame
        )
    };
    format!(
        "make it from a gameprinter checkout: {image}, then `just mame-record {zip} {} {} 0` ({} frames), which writes {} with SHA-256 {} ({})",
        c.root.join(&e.input).display(),
        e.name,
        e.frames,
        dir_of(e, paths).display(),
        e.sha256,
        c.path.display()
    )
}

/// The capture of `e` on this machine, checked against its hash.
fn checked(
    c: &Captures,
    e: &Entry,
    zip: Option<&Path>,
    paths: &Paths,
) -> Result<(PathBuf, Capture)> {
    let dir = dir_of(e, paths);
    if !dir.join("frames.tsv").is_file() {
        bail!(
            "no MAME capture {} at {}; {}",
            e.name,
            dir.display(),
            how_to_make(c, e, zip, paths)
        );
    }
    let h = capture_hash(&dir)?;
    if h != e.sha256 {
        bail!(
            "the capture at {} has SHA-256 {h}, not the {} {} names: it isn't the capture the repo holds gp-arcade to (made with pictures, another MAME or another input?). Move it aside and {}",
            dir.display(),
            e.sha256,
            c.path.display(),
            how_to_make(c, e, zip, paths)
        );
    }
    let cap = Capture::load(&dir).map_err(anyhow::Error::msg)?;
    Ok((dir, cap))
}

/// `gp oracle golden ID --against CAPTURE` on an arcade recording: the
/// capture is one the repo names, the recording ran its input on its
/// romset, and every frame matches; then the verdict is stored and the
/// recordings log gets a golden entry.
pub(crate) fn golden(
    store: &Store,
    entry: &IndexEntry,
    manifest: &RecordingManifest,
    against: &str,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let c = Captures::find(against)?;
    let e = c
        .named(against)
        .context("the captures file lost the capture it was found by")?;
    let SourceRef::Romset { system, .. } = &manifest.source else {
        bail!("not an arcade recording");
    };
    if *system != e.system {
        bail!(
            "the recording is of {system}, capture {} of {}",
            e.name,
            e.system
        );
    }
    let zip = gp_arcade::romset::set(&e.system)
        .map(|s| crate::oracle_mame::find_zip(&s.identity(), rom, paths))
        .transpose()?
        .flatten();
    let (dir, _) = checked(&c, e, zip.as_deref(), paths)?;
    let theirs = OracleLog::parse(&c.input(e)?).map_err(|m| anyhow::anyhow!("{}: {m}", e.input))?;
    let ours = String::from_utf8(store.get(&manifest.input_log)?)
        .context("the recording's input log isn't text")?;
    let ours =
        OracleLog::parse(&ours).map_err(|m| anyhow::anyhow!("the recording's input log: {m}"))?;
    if ours.frames != theirs.frames || ours.four_way != theirs.four_way {
        bail!(
            "the recording didn't run capture {}'s input ({}): record it with `gp record --rom ZIP --input {}`",
            e.name,
            e.input,
            c.root.join(&e.input).display()
        );
    }
    let d = crate::oracle_mame::diff(store, entry, manifest, &dir, rom, 40, paths)?;
    if d.exit != 0 {
        return Ok(Outcome {
            json: json!({"golden": false, "diff": d.json}),
            text: format!("{}\nnot golden: gp-arcade differs from MAME", d.text),
            exit: 1,
        });
    }
    let verdict = json!({
        "kind": "gp-oracle-verdict",
        "schema": 0,
        "recording": entry.id,
        "oracle": {"runtime": "mame", "version": e.mame, "system": e.system,
                   "capture": e.name, "sha256": e.sha256, "input": e.input},
        "report": d.json,
        "triage": [],
        "date": now_rfc3339(),
    });
    let verdict_id = store.put_bytes(serde_json::to_string_pretty(&verdict)?.as_bytes())?;
    let added = gp_evidence::recordings::append(
        store,
        &IndexEntry {
            golden: true,
            notes: Some(format!(
                "golden: oracle verdict blake3:{verdict_id} (every frame matches MAME {} capture {}, sha256 {})",
                e.mame,
                e.name,
                &e.sha256[..12]
            )),
            created_at: now_rfc3339(),
            ..entry.clone()
        },
    )?;
    crate::runrec::output("verdict", format!("blake3:{verdict_id}"), None);
    Ok(Outcome::ok(
        json!({ "golden": true, "verdict": verdict_id, "added": added, "capture": e.name }),
        format!(
            "{}\ngolden: {} ({}), verdict blake3:{verdict_id}",
            d.text,
            entry.name,
            &entry.id.to_hex()[..12]
        ),
    ))
}

/// `gp oracle check` of a captures file: each capture's input played on
/// gp-arcade and compared with the capture on every frame. A capture
/// without its romset or its folder on this machine is skipped, the
/// latter with how to make it; one whose hash isn't the file's, or that
/// differs on a frame, fails.
pub(crate) fn check(
    file: &Path,
    paths: &Paths,
) -> Result<(Vec<String>, Vec<serde_json::Value>, [u64; 3])> {
    let c = Captures::load(file)?;
    let (mut lines, mut results) = (Vec::new(), Vec::new());
    let (mut passed, mut failed, mut skipped) = (0u64, 0u64, 0u64);
    for e in &c.captures {
        let label = format!("{}#{}", file.display(), e.name);
        let Some(info) = gp_arcade::romset::set(&e.system) else {
            failed += 1;
            lines.push(format!(
                "FAILED {label}  gp-arcade doesn't run a set named {}",
                e.system
            ));
            continue;
        };
        let Some(zip) = crate::oracle_mame::find_zip(&info.identity(), None, paths)? else {
            skipped += 1;
            lines.push(format!(
                "skip   {label}  (no {} romset in {})",
                e.system,
                paths.roms.display()
            ));
            continue;
        };
        if !dir_of(e, paths).join("frames.tsv").is_file() {
            skipped += 1;
            lines.push(format!(
                "skip   {label}  (no capture at {}; {})",
                dir_of(e, paths).display(),
                how_to_make(&c, e, Some(&zip), paths)
            ));
            results.push(json!({"capture": e.name, "ok": null, "missing": true}));
            continue;
        }
        let run = || -> Result<crate::oracle_mame::Compared> {
            let (_, cap) = checked(&c, e, Some(&zip), paths)?;
            let log =
                OracleLog::parse(&c.input(e)?).map_err(|m| anyhow::anyhow!("{}: {m}", e.input))?;
            let set = gp_arcade::Romset::from_zip(&fs::read(&zip)?)
                .map_err(|m| anyhow::anyhow!("{m}"))?;
            if cap.rows.len() as u64 != e.frames + 1 && cap.rows.len() as u64 != e.frames {
                bail!(
                    "the capture has {} frames, the file says {}",
                    cap.rows.len(),
                    e.frames
                );
            }
            Ok(crate::oracle_mame::play(&set, &log, &cap, 3))
        };
        match run() {
            Err(err) => {
                failed += 1;
                lines.push(format!("FAILED {label}  {err:#}"));
                results.push(json!({"capture": e.name, "ok": false, "error": format!("{err:#}")}));
            }
            Ok(r) => {
                let ok = r.differ == 0;
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
                lines.push(format!(
                    "{} {label}  {} on gp-arcade against MAME {}: {}",
                    if ok { "ok    " } else { "FAILED" },
                    e.system,
                    e.mame,
                    if ok {
                        format!("all {} frames match", r.frames)
                    } else {
                        format!("{} of {} frames differ", r.differ, r.frames)
                    }
                ));
                if !ok {
                    lines.extend(r.lines().into_iter().map(|l| format!("       {l}")));
                }
                let mut j = r.json();
                j["capture"] = json!(e.name);
                j["ok"] = json!(ok);
                results.push(j);
            }
        }
    }
    Ok((lines, results, [passed, failed, skipped]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_the_digest_lines_hash() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("capture.txt"), "gp-mame-capture 0\n").unwrap();
        fs::write(d.path().join("frames.tsv"), "frame\n0\n").unwrap();
        fs::write(d.path().join("mame.log"), "left out").unwrap();
        let lines = format!(
            "{}  ./capture.txt\n{}  ./frames.tsv\n",
            hex(&sha2::Sha256::digest(b"gp-mame-capture 0\n")),
            hex(&sha2::Sha256::digest(b"frame\n0\n"))
        );
        let want = hex(&sha2::Sha256::digest(lines.as_bytes()));
        assert_eq!(capture_hash(d.path()).unwrap(), want);
        fs::create_dir(d.path().join("shots")).unwrap();
        fs::write(d.path().join("sound.wav"), "left out too").unwrap();
        assert_eq!(capture_hash(d.path()).unwrap(), want);
        fs::write(d.path().join("frames.tsv"), "frame\n1\n").unwrap();
        assert_ne!(capture_hash(d.path()).unwrap(), want);
    }

    /// A made-up game folder with a captures file naming one capture of a
    /// made-up set, and the input log it names.
    fn made_up_game() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("made-up");
        fs::create_dir_all(game.join("oracle")).unwrap();
        fs::create_dir_all(game.join("inputs/oracle")).unwrap();
        let log = "gp-arcade-input 0\nsystem madeup\ncolumns CSRLDUT\n2 .......\n1 C......\n";
        fs::write(game.join("inputs/oracle/first.input"), log).unwrap();
        let file = game.join(FILE);
        fs::write(
            &file,
            format!(
                "schema = 0\n\n[[capture]]\nname = \"made-up-first\"\nsystem = \"madeup\"\nmame = \"{MAME_VERSION}\"\ninput = \"inputs/oracle/first.input\"\ninput_sha256 = \"{}\"\nframes = 3\nsha256 = \"{}\"\n",
                hex(&sha2::Sha256::digest(log.as_bytes())),
                "0".repeat(64)
            ),
        )
        .unwrap();
        (d, file)
    }

    /// A game's captures file reads, and its inputs are from the game's
    /// folder, checked by their hash.
    #[test]
    fn a_games_file_reads_and_names_its_inputs() {
        let (_d, path) = made_up_game();
        assert!(Captures::is_one(&path));
        let c = Captures::load(&path).unwrap();
        assert_eq!(c.root, path.parent().unwrap().parent().unwrap());
        let e = c.named("made-up-first").unwrap();
        let log = OracleLog::parse(&c.input(e).unwrap()).unwrap();
        assert_eq!(log.system, "madeup");
        fs::write(c.root.join(&e.input), "gp-arcade-input 0\n").unwrap();
        let err = c.input(e).unwrap_err().to_string();
        assert!(err.contains("the input changed"), "{err}");
        let baseline =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baselines/gp-testrom/buttons.toml");
        assert!(!Captures::is_one(&baseline));
    }

    /// A missing capture is named with the command that makes it.
    #[test]
    fn a_missing_capture_says_how_to_make_it() {
        let (_d, path) = made_up_game();
        let c = Captures::load(&path).unwrap();
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(home.path().to_path_buf()), None, None).unwrap();
        let e = &c.captures[0];
        let Err(err) = checked(&c, e, None, &paths) else {
            panic!("no capture, yet it was found");
        };
        let err = err.to_string();
        assert!(err.contains("just mame-record"), "{err}");
        assert!(err.contains(&e.input), "{err}");
        assert!(err.contains(&e.sha256), "{err}");
    }
}
