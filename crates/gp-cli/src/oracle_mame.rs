//! `gp oracle record` and `gp oracle diff` on an arcade recording: the MAME
//! oracle (docs/oracle-mame.md) in place of Mesen.
//!
//! game-printer's oracle ran Mesen for gp-nes. gameprinter's arcade side has
//! MAME in its own image, driven by `scripts/oracle/mame-record.sh` and
//! `scripts/oracle/mame-capture.lua` (both built into gp, as the Mesen
//! script is, so no checkout is needed). `record` plays a gp-arcade
//! recording's input log on its romset in MAME and keeps the capture under
//! `$GP_HOME/work/oracle/` (it is derived from the ROM, so never in the
//! store or a repo), with the input log beside it. `diff` plays the
//! recording again on gp-arcade and compares every frame with the capture,
//! as gp-arcade's `compare` example does: the RAM record (work, sprite,
//! video, palette and sound RAM), the input ports, the picture's hash and
//! the AY writes. A capture isn't a store recording, so `golden`, `check`
//! and the triage classes stay the NES's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde_json::json;

use gp_arcade::oracle::{Capture, FrameDiff, InputLog as OracleLog, compare};
use gp_evidence::manifest::{RecordingManifest, SourceRef};
use gp_evidence::recordings::IndexEntry;
use gp_evidence::{Paths, Store};

use crate::Outcome;

/// The capture script and the runner, built in as the Mesen script is.
const CAPTURE_LUA: &str = include_str!("../../../scripts/oracle/mame-capture.lua");
const RECORD_SH: &str = include_str!("../../../scripts/oracle/mame-record.sh");

/// The MAME the image holds (the justfile's `mame_version`).
pub(crate) const MAME_VERSION: &str = "0.276";

/// The image `just mame-build` tags for this architecture.
pub(crate) fn image(arch: &str) -> String {
    format!("gameprinter-mame:{MAME_VERSION}-{arch}")
}

/// Whether a recording is of an arcade romset.
pub(crate) fn is_arcade(m: &RecordingManifest) -> bool {
    matches!(m.source, SourceRef::Romset { .. })
}

/// The romset zip in the ROM folder (or `given`) whose identity is the
/// recording's, and its MAME system.
fn romset(m: &RecordingManifest, given: Option<&Path>, paths: &Paths) -> Result<(PathBuf, String)> {
    let SourceRef::Romset { identity, system } = &m.source else {
        bail!("not an arcade recording");
    };
    let matches = |p: &Path| {
        crate::roms::Rom::read(p)
            .ok()
            .is_some_and(|r| r.sha256 == *identity)
    };
    if let Some(g) = given {
        if !matches(g) {
            bail!(
                "{} isn't the romset the recording was made from",
                g.display()
            );
        }
        return Ok((g.to_path_buf(), system.clone()));
    }
    let mut files = Vec::new();
    walk(&paths.roms, &mut files);
    files.sort();
    files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "zip"))
        .find(|p| matches(p))
        .map(|p| (p, system.clone()))
        .with_context(|| {
            format!(
                "no romset with identity {}... in {}; give --rom",
                &identity[..16.min(identity.len())],
                paths.roms.display()
            )
        })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// `gp oracle record` on an arcade recording: its input log played on its
/// romset in MAME, the capture kept under `$GP_HOME/work/oracle/`.
pub(crate) fn record(
    store: &Store,
    entry: &IndexEntry,
    manifest: &RecordingManifest,
    arch: &str,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let image = image(arch);
    let inspect = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", &image])
        .output()
        .context("running docker; is it installed and running?")?;
    if !inspect.status.success() {
        bail!(
            "the MAME image {image} isn't built; run `just mame-build linux/{arch}` (docs/oracle-mame.md)"
        );
    }
    let (zip, system) = romset(manifest, rom, paths)?;
    let input = String::from_utf8(store.get(&manifest.input_log)?)
        .context("the recording's input log isn't text")?;
    OracleLog::parse(&input).map_err(|e| anyhow::anyhow!("the recording's input log: {e}"))?;
    let oracle = paths.home.join("work").join("oracle");
    fs::create_dir_all(&oracle)?;
    let work = tempfile::Builder::new()
        .prefix("mame-")
        .tempdir_in(&oracle)
        .context("creating a work directory")?;
    let w = work.path();
    fs::write(w.join("mame-record.sh"), RECORD_SH)?;
    fs::write(w.join("mame-capture.lua"), CAPTURE_LUA)?;
    fs::write(w.join("input.log"), &input)?;
    // MAME names the set by the zip's name: a copy named for its system
    // when the file in the ROM folder is named otherwise.
    let zip = if zip.file_stem().and_then(|s| s.to_str()) == Some(system.as_str()) {
        zip
    } else {
        let to = w.join(format!("{system}.zip"));
        fs::copy(&zip, &to)?;
        to
    };
    let id = entry.id.to_hex();
    let base = format!("{}-{}", safe(&entry.name), &id[..12]);
    let mut name = base.clone();
    let mut n = 2;
    while oracle.join(&name).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    let out = oracle.join(&name);
    let start = Instant::now();
    let run = Command::new("bash")
        .arg(w.join("mame-record.sh"))
        .arg(&image)
        .arg(&zip)
        .arg(w.join("input.log"))
        .arg(&out)
        .arg("0")
        .env("GP_HOME", &paths.home)
        .output()
        .context("running the MAME oracle")?;
    let seconds = start.elapsed().as_secs_f64();
    if !run.status.success() {
        bail!(
            "the MAME oracle failed: {}{}",
            String::from_utf8_lossy(&run.stderr).trim(),
            String::from_utf8_lossy(&run.stdout).trim()
        );
    }
    fs::write(out.join("input.log"), &input)?;
    fs::write(out.join("recording.txt"), format!("{id}\n"))?;
    let cap = Capture::load(&out).map_err(anyhow::Error::msg)?;
    crate::runrec::metric("oracle.record.frames", cap.rows.len() as f64, "count");
    crate::runrec::metric("oracle.record.seconds", seconds, "s");
    let text = format!(
        "ran {} frames of {system} in MAME {MAME_VERSION} (linux/{arch}) in {seconds:.0} s\ncapture: {}\ncompare: gp oracle diff {} {name}",
        cap.rows.len(),
        out.display(),
        &id[..12]
    );
    Ok(Outcome::ok(
        json!({"capture": out, "name": name, "from": entry.id, "platform": format!("linux/{arch}"),
               "frames": cap.rows.len(), "seconds": seconds, "mame": MAME_VERSION}),
        text,
    ))
}

/// A name a folder may have, from a recording's name.
fn safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "recording".into() } else { s }
}

/// A capture by its folder: a path, or a name under `$GP_HOME/work/oracle/`.
pub(crate) fn capture_dir(b: &str, paths: &Paths) -> Option<PathBuf> {
    let p = Path::new(b);
    if p.join("frames.tsv").is_file() {
        return Some(p.to_path_buf());
    }
    let named = paths.home.join("work").join("oracle").join(b);
    named.join("frames.tsv").is_file().then_some(named)
}

/// `gp oracle diff` of an arcade recording against a MAME capture: the
/// recording's input played on gp-arcade, every frame compared.
pub(crate) fn diff(
    store: &Store,
    entry: &IndexEntry,
    manifest: &RecordingManifest,
    capture: &Path,
    rom: Option<&Path>,
    max: usize,
    paths: &Paths,
) -> Result<Outcome> {
    let (zip, system) = romset(manifest, rom, paths)?;
    let set = gp_arcade::Romset::from_zip(&fs::read(&zip)?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let input = String::from_utf8(store.get(&manifest.input_log)?)
        .context("the recording's input log isn't text")?;
    let log = OracleLog::parse(&input).map_err(|e| anyhow::anyhow!("the input log: {e}"))?;
    let cap = Capture::load(capture).map_err(anyhow::Error::msg)?;
    let cap_system = fs::read_to_string(capture.join("capture.txt"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("system ").map(str::to_owned))
        });
    if let Some(s) = &cap_system
        && *s != system
    {
        bail!("the capture is of {s}, the recording of {system}");
    }
    let frames = cap.rows.len().min(log.frames.len() + 1);
    let mut t = gp_arcade::Tapper::power_on(&set);
    let mut shown: Vec<(usize, FrameDiff)> = Vec::new();
    let (mut differ, mut ram_f, mut port_f, mut pic_f, mut ay_f) = (0, 0, 0, 0, 0);
    let mut first: Option<usize> = None;
    for n in 0..frames {
        if n > 0 {
            t.step(log.controls(n));
        }
        let d = compare(&t, &cap, n);
        if d.is_empty() {
            continue;
        }
        differ += 1;
        ram_f += usize::from(!d.ram.is_empty());
        port_f += usize::from(d.ports.is_some());
        pic_f += usize::from(d.picture.is_some());
        ay_f += usize::from(d.ay.is_some());
        first.get_or_insert(n);
        if shown.len() < max {
            shown.push((n, d));
        }
    }
    let mut lines = vec![
        format!(
            "a: {} ({}), {} {}",
            entry.name,
            &entry.id.to_hex()[..12],
            manifest.runtime.name,
            manifest.runtime.version
        ),
        format!(
            "b: MAME capture {} ({} frames)",
            capture.display(),
            cap.rows.len()
        ),
        format!(
            "compared frames 0 to {}: the RAM record, the input ports, the picture's hash{}",
            frames.saturating_sub(1),
            if cap.ay.is_some() {
                " and the AY writes"
            } else {
                ""
            }
        ),
    ];
    match first {
        None => lines.push("no divergence: every compared frame matches".into()),
        Some(f) => lines.push(format!(
            "first divergence at frame {f}; {differ} frames differ (RAM {ram_f}, ports {port_f}, picture {pic_f}, AY {ay_f})"
        )),
    }
    for (n, d) in &shown {
        lines.push(format!("  frame {n}:"));
        for l in d.describe(8).lines() {
            lines.push(format!("      {l}"));
        }
    }
    if differ > shown.len() {
        lines.push(format!(
            "  shown the first {} frames that differ (--max)",
            shown.len()
        ));
    }
    crate::runrec::metric("oracle.diff.divergences", differ as f64, "count");
    let json = json!({
        "a": entry.id,
        "b": capture,
        "frames": frames,
        "differ": differ,
        "first": first,
        "by_kind": {"ram": ram_f, "ports": port_f, "picture": pic_f, "ay": ay_f},
        "shown": shown.iter().map(|(n, d)| json!({"frame": n, "says": d.describe(64)})).collect::<Vec<_>>(),
    });
    Ok(Outcome {
        json,
        text: lines.join("\n"),
        exit: u8::from(differ > 0),
    })
}
