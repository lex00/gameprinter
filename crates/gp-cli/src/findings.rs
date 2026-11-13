//! `gp findings`: one record for everything a tool finds in a game, and one
//! gate that turns a finding into evidence.
//!
//! Every discovery method (the harvest of a recording, a farm run, the
//! jumpathon, the localizer, a poke, a mechanic session, a fitted rule)
//! writes its own output. `ingest` reads those and keeps one line a finding
//! in `$GP_HOME/findings/GAME.ndjson` (format "finding 0.1"): what it is,
//! which method found it, where, the lead's own evidence, and its status. A
//! finding is a lead until a clip of real inputs reproduces it. `confirm` is
//! the gate: the clip's start is checked against its saved state, its video
//! is made (every frame checked as it's drawn), the detector named by the
//! finding's kind runs again on the clip, and only if it fires is the
//! finding confirmed.
//!
//! Ported from game-printer's findings.rs. What gameprinter's layout
//! changed: the game's ROM is the one its package in the games checkout
//! names; the gate's video and detectors are crate::findings_clip, since
//! gameprinter has no `gp mechanic video`, `gp video` or `gp mechanic
//! contacts` (the clip is recorded with `gp record`, checked with `gp
//! replay` and drawn to an animated PNG); clips and the farm's places are
//! replayed on crate::farm::machine (gp-nes or gp-arcade), so an arcade
//! game's farm runs, sweep strips and sessions are read and cut the same
//! way, its RAM indexed from the board's work RAM at E000.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::Outcome;
use gp_evidence::Paths;

const FORMAT: &str = "finding 0.1";

#[derive(Subcommand, Debug)]
pub enum FindingsCmd {
    /// Read what the tools have written for a game (mechanic sessions and
    /// their measured clips, harvest clips, farm and blind runs, jumpathon
    /// strips, the localizer's egg clips, queue findings and fitted rules)
    /// and add or update one record each. A record already confirmed or
    /// rejected keeps its status.
    Ingest { game: String },
    /// Cut clips of real inputs for findings that are still leads: a farm
    /// run's places, deaths and level end come from its path log (real
    /// inputs from power-on), cut around the frame the player first does it.
    /// Each clip is written in the snapshot format (path, start, state hash,
    /// inputs) to the game's `findings` session.
    Cut {
        game: String,
        /// Then run the gate on every finding that has a clip.
        #[arg(long)]
        confirm: bool,
    },
    /// Run the gate on a finding. A finding with a clip is replayed from its
    /// checked start, its video made (every frame checked), and the detector
    /// its kind names run again. Confirmed only if the detector fires.
    Confirm { game: String, id: String },
    /// Counts by kind, method and status, and the top leads not yet
    /// confirmed.
    List {
        game: String,
        /// Only findings with this status: lead, clip, confirmed or rejected.
        #[arg(long, value_parser = ["lead", "clip", "confirmed", "rejected"])]
        status: Option<String>,
    },
}

pub(crate) fn run(cmd: &FindingsCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        FindingsCmd::Ingest { game } => ingest(paths, game),
        FindingsCmd::Confirm { game, id } => confirm(paths, game, id),
        FindingsCmd::Cut { game, confirm } => cut(paths, game, *confirm),
        FindingsCmd::List { game, status } => list(paths, game, status.as_deref()),
    }
}

fn file_of(paths: &Paths, game: &str) -> Result<PathBuf> {
    if !crate::mechanic::name_ok(game) {
        bail!("a game name is lower-case letters, digits and dashes: {game}");
    }
    Ok(paths.home.join("findings").join(format!("{game}.ndjson")))
}

fn read_all(f: &Path) -> Result<Vec<Value>> {
    if !f.is_file() {
        return Ok(Vec::new());
    }
    fs::read_to_string(f)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).with_context(|| format!("{}: a bad line", f.display())))
        .collect()
}

fn write_all(f: &Path, rows: &[Value]) -> Result<()> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let mut s = String::new();
    for r in rows {
        s.push_str(&r.to_string());
        s.push('\n');
    }
    let tmp = f.with_extension("ndjson.tmp");
    fs::write(&tmp, s)?;
    fs::rename(&tmp, f)?;
    Ok(())
}

/// The key a finding is deduplicated by: kind, method and where. A finding
/// placed only by its level (a session's clip) adds its clip, so two clips
/// of one kind in one session stay two findings; one with a place in the
/// level is the same finding whatever clip later reproduces it.
fn key_of(r: &Value) -> String {
    // Only a session clip's finding is told apart by its clip.
    let placed = r["from_session"] != json!(true);
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        r["kind"].as_str().unwrap_or(""),
        r["method"].as_str().unwrap_or(""),
        r["where"]["level"].as_str().unwrap_or(""),
        r["where"]["x"],
        r["where"]["y"],
        if placed {
            ""
        } else {
            r["clip"]["session"].as_str().unwrap_or("")
        },
        if placed {
            Value::Null
        } else {
            r["clip"]["n"].clone()
        },
    )
}

fn id_of(key: &str) -> String {
    gp_evidence::hash::Hash::of(key.as_bytes()).to_hex()[..10].to_owned()
}

/// A new record: status "clip" when it names a clip, "lead" when not.
#[allow(clippy::too_many_arguments)]
fn record(
    game: &str,
    what: String,
    kind: &str,
    method: &str,
    level: Option<String>,
    x: Option<i64>,
    y: Option<i64>,
    evidence: Value,
    clip: Option<Value>,
) -> Value {
    let status = if clip.is_some() { "clip" } else { "lead" };
    let mut r = json!({
        "format": FORMAT,
        "game": game,
        "what": what,
        "kind": kind,
        "method": method,
        "where": {"level": level, "x": x, "y": y},
        "evidence": evidence,
        "status": status,
        "clip": clip.unwrap_or(Value::Null),
        "video": Value::Null,
        "facts": [],
        "reason": Value::Null,
    });
    let key = key_of(&r);
    r["id"] = json!(id_of(&key));
    r
}

/// The clip fields a record keeps: where it lives and how its start is
/// checked (the path it starts on and the state hash there, when written
/// that way).
fn clip_ref(session_dir: &Path, session: &str, n: u64) -> Value {
    let t: toml::Table = fs::read_to_string(session_dir.join(format!("clips/{n:03}.toml")))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    json!({
        "session": session,
        "n": n,
        "path": t.get("path").and_then(|v| v.as_str()),
        "start": t.get("start").and_then(|v| v.as_integer()),
        "state": t.get("state").and_then(|v| v.as_str()),
        "frames": t.get("frames").and_then(|v| v.as_integer()),
        "note": t.get("note").and_then(|v| v.as_str()),
        "by": t.get("by").and_then(|v| v.as_str()),
    })
}

/// The detector output key for a kind (what `gp mechanic contacts` writes).
fn detector_of(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "head-hit" | "egg" => "hits",
        "side-hit" | "side-hit-air" => "walls",
        "landing" | "raised-landing" => "rests",
        "stomp" => "stomps",
        "item" | "item-taken" | "item-out" => "items",
        "death" | "hurt" | "pit" | "player-shrank" | "player-fell-off-the-screen" => "hurts",
        _ => return None,
    })
}

/// The kind a detector key's events make.
const KEYS: [(&str, &str); 6] = [
    ("hits", "head-hit"),
    ("walls", "side-hit"),
    ("stomps", "stomp"),
    ("items", "item"),
    ("hurts", "hurt"),
    ("rests", "landing"),
];

/// Whether a detector's events count for the kind: a landing is a rest
/// above the lowest one; an egg is a head hit that changed RAM for good.
fn fires(kind: &str, events: &[Value]) -> bool {
    match kind {
        "landing" | "raised-landing" => events
            .iter()
            .any(|e| e["up_px"].as_i64().is_some_and(|u| u > 0)),
        "egg" => events.iter().any(|e| {
            e["at_hit"]
                .as_array()
                .is_some_and(|a| a.iter().any(|b| b["lasting"] == json!(true)))
        }),
        _ => !events.is_empty(),
    }
}

fn method_of(session: &str, by: Option<&str>) -> &'static str {
    if session.starts_with("harvest-") {
        "harvest"
    } else if session.starts_with("farm-") || by == Some("laya") && session.starts_with("farm") {
        "farm"
    } else if session.starts_with("egg-") {
        "localize"
    } else if session == "jumpathon" {
        "sweep"
    } else if by == Some("player") {
        "user"
    } else {
        "mechanic"
    }
}

/// Numbers after "x " and "y " in a note ("level x 1024, y 128").
fn xy_in(note: &str) -> (Option<i64>, Option<i64>) {
    let num = |tag: &str| {
        note.find(tag).and_then(|i| {
            note[i + tag.len()..]
                .trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|d| d.parse().ok())
        })
    };
    (num("x "), num("y "))
}

/// Findings from a game's mechanic sessions: one a clip and detector key
/// with events (or, for harvest clips, the kind their note names).
fn from_sessions(paths: &Paths, game: &str, out: &mut Vec<Value>) -> Result<()> {
    let root = paths.home.join("mechanics").join(game);
    let Ok(rd) = fs::read_dir(&root) else {
        return Ok(());
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("session.toml").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let session = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        // Clips this command cut are findings' evidence, not new findings.
        if session == "findings" {
            continue;
        }
        let level = session
            .strip_prefix("harvest-")
            .unwrap_or(&session)
            .to_owned();
        let Ok(rd) = fs::read_dir(dir.join("clips")) else {
            continue;
        };
        let mut ns: Vec<u64> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.strip_suffix(".toml").and_then(|s| s.parse().ok())
            })
            .collect();
        ns.sort_unstable();
        for n in ns {
            let clip = clip_ref(&dir, &session, n);
            let note = clip["note"].as_str().unwrap_or("").to_owned();
            let method = method_of(&session, clip["by"].as_str());
            let mpath = dir.join(format!("clips/{n:03}.measure.json"));
            let measure: Value = fs::read_to_string(&mpath)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let (x, y) = xy_in(&note);
            let ev = |frame: Value| json!({"file": mpath.display().to_string(), "frame": frame});
            if method == "localize" {
                out.push(record(
                    game,
                    note.clone(),
                    "egg",
                    method,
                    Some(level.clone()),
                    x,
                    y,
                    ev(Value::Null),
                    Some(clip.clone()),
                ));
                continue;
            }
            // A harvest clip is one example of the kind its note names.
            if method == "harvest"
                && let Some((kind, what)) = note.split_once(':')
            {
                let first = detector_of(kind)
                    .and_then(|k| measure[k].as_array())
                    .and_then(|a| a.first())
                    .map(|e| e["frame"].clone())
                    .unwrap_or(Value::Null);
                out.push(record(
                    game,
                    format!("{kind}:{what}"),
                    kind.trim(),
                    method,
                    Some(level.clone()),
                    None,
                    None,
                    ev(first),
                    Some(clip.clone()),
                ));
                continue;
            }
            for (key, kind) in KEYS {
                let Some(events) = measure[key].as_array() else {
                    continue;
                };
                if !fires(kind, events) {
                    continue;
                }
                let first = events
                    .first()
                    .map(|e| e["frame"].clone())
                    .unwrap_or_default();
                let what = format!(
                    "{} {} in {session} clip {n}{}",
                    events.len(),
                    kind,
                    if note.is_empty() {
                        String::new()
                    } else {
                        format!(" ({note})")
                    }
                );
                out.push(record(
                    game,
                    what,
                    kind,
                    method,
                    Some(level.clone()),
                    None,
                    None,
                    ev(first),
                    Some(clip.clone()),
                ));
            }
        }
    }
    Ok(())
}

/// Findings from the newest jumpathon strip along a path: each place where
/// a jump hit something with a lasting change, and each place where code ran
/// for the first time. Leads: the sweep keeps their inputs in its archive,
/// not as clips.
fn from_sweep(paths: &Paths, game: &str, out: &mut Vec<Value>) -> Result<()> {
    let dir = paths.home.join("work/sweep").join(game).join("jump-along");
    let Ok(rd) = fs::read_dir(&dir) else {
        return Ok(());
    };
    let Some(f) = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .max()
    else {
        return Ok(());
    };
    let d: Value = serde_json::from_str(&fs::read_to_string(&f)?)?;
    let level = d["along"]
        .as_str()
        .and_then(|a| Path::new(a).ancestors().nth(2))
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_owned);
    for p in d["strip"].as_array().into_iter().flatten() {
        let (x, y) = (p["level_x"].as_i64(), p["y"].as_i64());
        let ev =
            json!({"file": f.display().to_string(), "frame": p["frame"], "column": p["column"]});
        let jumps = p["jumps"].as_array().cloned().unwrap_or_default();
        let mut hits: Vec<&Value> = jumps
            .iter()
            .filter(|j| {
                j["head_hits"].as_i64().unwrap_or(0) > 0
                    && j["lasting"].as_array().is_some_and(|a| !a.is_empty())
            })
            .collect();
        // A standing jump first: it hits what's over the place, where a
        // running one may hit something columns away.
        hits.sort_by_key(|j| j["jump"].as_str().is_some_and(|n| n.starts_with("run")));
        let ev_of = |j: &Value| {
            let mut e = ev.clone();
            e["jump"] = j["jump"].clone();
            e["place_y"] = json!(y);
            e
        };
        if let Some(j) = hits.first() {
            let standing = !j["jump"].as_str().unwrap_or("").starts_with("run");
            out.push(record(
                game,
                format!(
                    "a jump from x {} y {} hit something and changed RAM for good ({}; {} of {} jumps)",
                    x.unwrap_or(-1),
                    y.unwrap_or(-1),
                    j["jump"].as_str().unwrap_or(""),
                    hits.len(),
                    jumps.len()
                ),
                "head-hit",
                "sweep",
                level.clone(),
                // The cell hit is over the place for a standing jump; a
                // running jump's isn't placed.
                x.filter(|_| standing),
                None,
                ev_of(j),
                None,
            ));
        }
        let new: i64 = jumps.iter().filter_map(|j| j["new_code"].as_i64()).sum();
        if new > 0
            && let Some(j) = jumps
                .iter()
                .find(|j| j["new_code"].as_i64().unwrap_or(0) > 0)
        {
            out.push(record(
                game,
                format!(
                    "{new} code addresses ran for the first time from x {} y {}",
                    x.unwrap_or(-1),
                    y.unwrap_or(-1)
                ),
                "new-code",
                "sweep",
                level.clone(),
                x,
                y,
                ev_of(j),
                None,
            ));
        }
    }
    Ok(())
}

/// Findings from a blind farm run's findings.json: the level end, the death
/// signal, and the places it stood. Leads: the path log is evidence, but no
/// clip has been cut from it.
fn from_farm(paths: &Paths, game: &str, out: &mut Vec<Value>) -> Result<()> {
    let f = farm_file(paths, game)?;
    let Ok(t) = fs::read_to_string(&f) else {
        return Ok(());
    };
    let d: Value = serde_json::from_str(&t)?;
    let file = f.display().to_string();
    let path_log = d["path_log"].clone();
    if d["level_end"].is_object() {
        out.push(record(
            game,
            format!(
                "the level ends: the game froze and {} bytes changed",
                d["level_end"]["bytes"].as_array().map_or(0, Vec::len)
            ),
            "level-end",
            "farm",
            Some("main".into()),
            None,
            None,
            json!({"file": file, "frame": d["level_end"]["at_path_frame"], "path_log": path_log}),
            None,
        ));
    }
    if let Some(b) = d["death"]["bytes"].as_array().filter(|b| !b.is_empty()) {
        out.push(record(
            game,
            format!("the player dies: {} bytes change at a death", b.len()),
            "death",
            "farm",
            Some("main".into()),
            None,
            None,
            json!({"file": file, "frame": b[0]["first_at_path_frame"], "path_log": path_log}),
            None,
        ));
    }
    // Places are in 16 px columns of progress from the play-start position
    // (findings.json "units"): the level x there, from blind discovery's
    // start state, turns them into level x.
    let origin = farm_origin(paths, game)?;
    // Each place carries the first search run that got there (run_path, a
    // stored path, and run_frame), when the run recorded it.
    let ev = |p: &Value| json!({"file": file, "path_log": path_log, "run_path": p["run_path"], "run_frame": p["run_frame"]});
    for l in d["places"]["landings"].as_array().into_iter().flatten() {
        let (c, r) = (l["column"].as_i64(), l["y_row"].as_i64());
        out.push(record(
            game,
            format!(
                "the player stood at column {} row {}",
                c.unwrap_or(-1),
                r.unwrap_or(-1)
            ),
            "landing",
            "farm",
            Some(format!("scene-{}", l["scene"].as_i64().unwrap_or(0))),
            c.map(|c| origin + c * 16),
            r.map(|r| r * 16),
            ev(l),
            None,
        ));
    }
    for (list, kind, what) in [
        ("walls", "side-hit", "a wall stopped the player"),
        ("pits", "pit", "a move here led to a death"),
    ] {
        for w in d["places"][list].as_array().into_iter().flatten() {
            let c = w["column"].as_i64();
            out.push(record(
                game,
                format!("{what} at column {}", c.unwrap_or(-1)),
                kind,
                "farm",
                Some(format!("scene-{}", w["scene"].as_i64().unwrap_or(0))),
                c.map(|c| origin + c * 16),
                None,
                ev(w),
                None,
            ));
        }
    }
    Ok(())
}

/// Findings from the game's queue (what each mechanic says is measured) and
/// its fitted rules.
fn from_queue_and_rules(paths: &Paths, game: &str, out: &mut Vec<Value>) -> Result<()> {
    let root = paths.home.join("mechanics").join(game);
    let qf = root.join("queue.json");
    for row in crate::mechanic::session::read_queue(&qf) {
        let m = row["mechanic"].as_str().unwrap_or("").to_owned();
        for k in row["known"].as_array().into_iter().flatten() {
            let Some(k) = k.as_str() else { continue };
            out.push(record(
                game,
                format!("{m}: {k}"),
                "rule",
                "mechanic",
                Some(m.clone()),
                None,
                None,
                json!({"file": qf.display().to_string(), "session": m}),
                None,
            ));
        }
    }
    let Ok(rd) = fs::read_dir(&root) else {
        return Ok(());
    };
    for e in rd.filter_map(|e| e.ok()) {
        let f = e.path().join("refine.json");
        let Ok(t) = fs::read_to_string(&f) else {
            continue;
        };
        let Ok(d) = serde_json::from_str::<Value>(&t) else {
            continue;
        };
        let m = d["mechanic"].as_str().unwrap_or("rule").to_owned();
        out.push(record(
            game,
            format!(
                "the {m} rule matches {} of {} frames (fitted on {})",
                d["matched"],
                d["total"],
                d["fitted_on"].as_str().unwrap_or("?")
            ),
            "rule",
            "rule",
            Some(m),
            None,
            None,
            json!({"file": f.display().to_string()}),
            None,
        ));
    }
    Ok(())
}

fn ingest(paths: &Paths, game: &str) -> Result<Outcome> {
    let f = file_of(paths, game)?;
    let mut found = Vec::new();
    from_sessions(paths, game, &mut found)?;
    for r in &mut found {
        if !r["clip"].is_null() {
            r["from_session"] = json!(true);
            let k = key_of(r);
            r["id"] = json!(id_of(&k));
        }
    }
    from_sweep(paths, game, &mut found)?;
    from_farm(paths, game, &mut found)?;
    from_queue_and_rules(paths, game, &mut found)?;
    // Records written before `from_session` existed: a clip outside the
    // findings session came from a session.
    let mut rows: BTreeMap<String, Value> = read_all(&f)?
        .into_iter()
        .map(|mut r| {
            if r["from_session"].is_null()
                && r["clip"]["session"]
                    .as_str()
                    .is_some_and(|s| s != "findings")
            {
                r["from_session"] = json!(true);
            }
            (key_of(&r), r)
        })
        .collect();
    let (mut added, mut updated) = (0usize, 0usize);
    let mut seen = std::collections::BTreeSet::new();
    for r in found {
        let k = key_of(&r);
        seen.insert(k.clone());
        match rows.get_mut(&k) {
            Some(old) => {
                // What the tools say now, but a verdict and its video stay.
                let keep = matches!(old["status"].as_str(), Some("confirmed" | "rejected"));
                let mut n = r;
                n["id"] = old["id"].clone();
                if keep {
                    for f in ["status", "video", "reason", "facts"] {
                        n[f] = old[f].clone();
                    }
                }
                // A clip cut for a lead stays with it.
                if n["clip"].is_null() && !old["clip"].is_null() {
                    for f in ["clip", "status", "video", "reason"] {
                        n[f] = old[f].clone();
                    }
                }
                if *old != n {
                    *old = n;
                    updated += 1;
                }
            }
            None => {
                rows.insert(k, r);
                added += 1;
            }
        }
    }
    // A lead no tool reports any more (its run was replaced) is dropped;
    // anything with a clip or a verdict stays.
    let fresh: std::collections::BTreeSet<String> = seen;
    let before = rows.len();
    rows.retain(|k, r| fresh.contains(k) || r["status"] != json!("lead"));
    let dropped = before - rows.len();
    let rows: Vec<Value> = rows.into_values().collect();
    write_all(&f, &rows)?;
    Ok(Outcome::ok(
        json!({"file": f, "findings": rows.len(), "added": added, "updated": updated, "dropped": dropped}),
        format!(
            "{} findings in {} ({added} added, {updated} updated, {dropped} stale leads dropped)\n{}",
            rows.len(),
            f.display(),
            table(&rows)
        ),
    ))
}

fn confirm(paths: &Paths, game: &str, id: &str) -> Result<Outcome> {
    let f = file_of(paths, game)?;
    let mut rows = read_all(&f)?;
    let Some(i) = rows.iter().position(|r| r["id"].as_str() == Some(id)) else {
        bail!("no finding {id} for {game}: gp findings list {game}");
    };
    let r = rows[i].clone();
    let (verdict, reason, video) = gate(paths, game, &r);
    let row = &mut rows[i];
    row["status"] = json!(verdict);
    row["reason"] = json!(reason);
    if !video.is_null() {
        row["video"] = video;
    }
    let out = row.clone();
    write_all(&f, &rows)?;
    Ok(Outcome::ok(
        out.clone(),
        format!(
            "{id}: {verdict}. {reason}{}",
            out["video"]["video"]
                .as_str()
                .map(|v| format!(
                    "\nvideo {v} (sha256 {})",
                    out["video"]["sha256"].as_str().unwrap_or("")
                ))
                .unwrap_or_default()
        ),
    ))
}

/// The gate's verdict, its reason, and the video when one was made.
fn gate(paths: &Paths, game: &str, r: &Value) -> (&'static str, String, Value) {
    let (Some(session), Some(n)) = (r["clip"]["session"].as_str(), r["clip"]["n"].as_u64()) else {
        return (
            "lead",
            "no clip of real inputs reproduces it yet".into(),
            Value::Null,
        );
    };
    let Some(key) = detector_of(r["kind"].as_str().unwrap_or("")) else {
        return (
            "clip",
            format!(
                "no detector checks a {} on replay, so it can't be confirmed",
                r["kind"].as_str().unwrap_or("?")
            ),
            Value::Null,
        );
    };
    // The start is checked against its saved state, the clip recorded, and
    // every frame of its video checked as it's drawn.
    let video = match crate::findings_clip::video_of_clip(paths, game, session, n) {
        Ok(v) => v,
        Err(e) => {
            return (
                "rejected",
                format!("the clip doesn't replay: {e:#}"),
                Value::Null,
            );
        }
    };
    // A detector that panics on one clip mustn't stop the rest of a run.
    let measure = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (x, _, y) = player_addrs(paths, game)?;
        crate::findings_clip::detect_in_clip(paths, game, session, n, (Some(x), y))
    })) {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => return ("clip", format!("the detectors didn't run: {e:#}"), video),
        Err(p) => {
            let why = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            return (
                "clip",
                format!("a detector panicked on this clip: {why}"),
                video,
            );
        }
    };
    let kind = r["kind"].as_str().unwrap_or("");
    let events = measure[key].as_array().cloned().unwrap_or_default();
    // A finding with a place must have its detector fire there. Placed, a
    // landing is any rest (its height is checked at the place).
    let placed_landing = matches!(kind, "landing" | "raised-landing");
    if (fires(kind, &events) || placed_landing && !events.is_empty())
        && let (Some(wx), wy) = (r["where"]["x"].as_i64(), r["where"]["y"].as_i64())
    {
        let at: Vec<Value> = match positions(paths, game, session, n) {
            Ok((pos, start)) => events
                .iter()
                .filter(|e| {
                    let f = e["frame"]
                        .as_i64()
                        .or_else(|| e["from"].as_i64())
                        .unwrap_or(-1);
                    let i = start as i64 + f;
                    i >= 0 && (i as usize) < pos.len() && near(kind, e, pos[i as usize], wx, wy)
                })
                .cloned()
                .collect(),
            Err(e) => {
                return (
                    "clip",
                    format!("the player's place couldn't be read on replay: {e:#}"),
                    video,
                );
            }
        };
        // Placed, a landing counts at its height whether or not the clip
        // also has lower ground to measure it from.
        let ok = match kind {
            "landing" | "raised-landing" => !at.is_empty(),
            _ => fires(kind, &at),
        };
        return if ok {
            (
                "confirmed",
                format!(
                    "replayed from its checked start; every video frame matched; the {key} detector fired {} times at x {wx}{}",
                    at.len(),
                    wy.map(|y| format!(" y {y}")).unwrap_or_default()
                ),
                video,
            )
        } else {
            (
                "rejected",
                format!(
                    "the {key} detector fired {} times in the clip, but none at x {wx}{}",
                    events.len(),
                    wy.map(|y| format!(" y {y}")).unwrap_or_default()
                ),
                video,
            )
        };
    }
    if fires(kind, &events) {
        (
            "confirmed",
            format!(
                "replayed from its checked start; every video frame matched; the {key} detector fired {} times",
                events.len()
            ),
            video,
        )
    } else {
        (
            "clip",
            format!(
                "replayed and the video matched, but the {key} detector didn't fire for a {kind}"
            ),
            video,
        )
    }
}

/// The level x where a blind farm run measures progress from: the player's
/// place in blind discovery's start state.
fn farm_origin(paths: &Paths, game: &str) -> Result<i64> {
    let (_, rom) = crate::findings_clip::game_rom(paths, game)?;
    let d = crate::farm::blind::discover(&rom, paths)?;
    let (x, hi, _) = player_addrs(paths, game)?;
    let w = d.nes.wram();
    let b = d.nes.ram_base();
    Ok(hi.map_or(0, |h| i64::from(w[ix(h, b)])) * 256 + i64::from(w[ix(x, b)]))
}

/// The newest blind farm findings.json for a game's ROM: blind runs are
/// filed by the name they were started with (a template's, or the ROM
/// file's), so every run folder is read and matched by the ROM's hash.
fn farm_file(paths: &Paths, game: &str) -> Result<PathBuf> {
    let root = paths.home.join("work/farm/blind");
    let fallback = root.join(game).join("findings.json");
    let Ok((_, rom)) = crate::findings_clip::game_rom(paths, game) else {
        return Ok(fallback);
    };
    let sha = crate::farm::machine::rom_sha256(&rom)?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
    {
        let f = e.path().join("findings.json");
        let Ok(t) = fs::read_to_string(&f) else {
            continue;
        };
        let Ok(d) = serde_json::from_str::<Value>(&t) else {
            continue;
        };
        if d["rom_sha256"].as_str() != Some(sha.as_str()) {
            continue;
        }
        let m = fs::metadata(&f).and_then(|m| m.modified()).ok();
        if let Some(m) = m
            && best.as_ref().is_none_or(|(b, _)| m > *b)
        {
            best = Some((m, f));
        }
    }
    Ok(best.map_or(fallback, |(_, f)| f))
}

/// The player's bytes for a game: x, the x high byte when found, and y.
/// Blind discovery's findings come first, then the ROM dataset's.
fn player_addrs(paths: &Paths, game: &str) -> Result<(u16, Option<u16>, u16)> {
    let hex = |v: &Value| {
        v.as_str()
            .and_then(|s| u16::from_str_radix(s.trim_start_matches('$'), 16).ok())
    };
    let f = farm_file(paths, game)?;
    if let Ok(t) = fs::read_to_string(&f)
        && let Ok(d) = serde_json::from_str::<Value>(&t)
        && let (Some(x), Some(y)) = (
            hex(&d["player"]["x"]["addr"]),
            hex(&d["player"]["y"]["addr"]),
        )
    {
        return Ok((x, hex(&d["player"]["x_hi"]["addr"]), y));
    }
    let (x, y, _) =
        crate::mechanic::detect::player_bytes(&paths.home.join("work/datasets").join(game))?;
    Ok((x.context("no player x byte for this game")?, None, y))
}

/// A RAM address's index in the work RAM a machine gives: the NES's from
/// $0000, the arcade board's from E000 (an address the dataset states in
/// the CPU's space is moved down; blind discovery's are indices already).
fn ix(a: u16, base: usize) -> usize {
    let a = usize::from(a);
    if base > 0 && a >= base { a - base } else { a }
}

/// The player's place in the level (x with its high byte) and y on every
/// frame of a log from power-on.
fn track(
    rom: &[u8],
    log: &gp_evidence::input_log::InputLog,
    a: (u16, Option<u16>, u16),
) -> Result<Vec<(i64, i64)>> {
    let mut mc =
        crate::farm::machine::Machine::power_on(rom, gp_evidence::manifest::PowerOn::default())?;
    let b = mc.ram_base();
    let mut out = Vec::with_capacity(log.frames.len());
    for f in &log.frames {
        mc.step(*f)?;
        let w = mc.wram();
        let hi = a.1.map_or(0, |h| i64::from(w[ix(h, b)]));
        out.push((
            hi * 256 + i64::from(w[ix(a.0, b)]),
            i64::from(w[ix(a.2, b)]),
        ));
    }
    Ok(out)
}

/// A clip's player places, by frame from power-on, and its start.
fn positions(paths: &Paths, game: &str, session: &str, n: u64) -> Result<(Vec<(i64, i64)>, usize)> {
    let dir = paths.home.join("mechanics").join(game).join(session);
    let (text, start) = crate::mechanic::paths::full_log(&paths.home, &dir, n)?;
    let log = gp_evidence::input_log::InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (_, rom) = crate::findings_clip::game_rom(paths, game)?;
    Ok((track(&rom, &log, player_addrs(paths, game)?)?, start))
}

/// Whether a detector's event happened at a finding's place: a head hit's
/// cell above the player's head, a landing's column and height, any other
/// contact within a tile and a half.
fn near(kind: &str, e: &Value, (lx, y): (i64, i64), wx: i64, wy: Option<i64>) -> bool {
    let tile = |v: i64| v.div_euclid(16) * 16;
    match kind {
        "head-hit" | "egg" => {
            let (cx, cy) = (tile(lx + 8), tile(y - 1));
            (cx - wx).abs() <= 16 && wy.is_none_or(|wy| (cy - wy).abs() <= 16)
        }
        "landing" | "raised-landing" => {
            let ry = e["y"].as_i64().unwrap_or(y);
            (tile(lx) - wx).abs() <= 16 && wy.is_none_or(|wy| (tile(ry) - wy).abs() <= 16)
        }
        _ => (lx - wx).abs() <= 24 && wy.is_none_or(|wy| (y - wy).abs() <= 32),
    }
}

/// Cut clips for a game's farm leads from its blind run's path log, then
/// (with `confirm`) run the gate on every finding with a clip.
fn cut(paths: &Paths, game: &str, confirm_all: bool) -> Result<Outcome> {
    let f = file_of(paths, game)?;
    let mut rows = read_all(&f)?;
    if rows.is_empty() {
        bail!("no findings for {game}: run gp findings ingest {game} first");
    }
    let (_, rom) = crate::findings_clip::game_rom(paths, game)?;
    let farm: Value = fs::read_to_string(farm_file(paths, game)?)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let mut text = String::new();
    let mut cut_n = 0usize;
    let mut missed = 0usize;
    if let Some(pl) = farm["path_log"].as_str() {
        let log_text = fs::read_to_string(pl).with_context(|| format!("reading {pl}"))?;
        let log = crate::findings_clip::board_log(
            &rom,
            gp_evidence::input_log::InputLog::parse(&log_text)
                .map_err(|e| anyhow::anyhow!("{pl}: {e}"))?,
        )?;
        let (sha, _) = crate::mechanic::paths::put_path(&paths.home, &log.to_text())?;
        let pos = track(&rom, &log, player_addrs(paths, game)?)?;
        // How long y has held still at each frame (standing).
        let mut still = vec![0usize; pos.len()];
        for i in 1..pos.len() {
            still[i] = if pos[i].1 == pos[i - 1].1 {
                still[i - 1] + 1
            } else {
                0
            };
        }
        let dir = paths.home.join("mechanics").join(game).join("findings");
        fs::create_dir_all(dir.join("clips"))?;
        if !dir.join("session.toml").is_file() {
            let img = crate::farm::machine::rom_sha256(&rom)?;
            let mut t = toml::Table::new();
            t.insert("game".into(), game.into());
            t.insert("mechanic".into(), "findings".into());
            t.insert(
                "prompt".into(),
                "clips cut for findings that were leads".into(),
            );
            t.insert("rom".into(), img.into());
            fs::write(dir.join("session.toml"), toml::to_string(&t)?)?;
        }
        let mut next = fs::read_dir(dir.join("clips"))?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".toml")
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        for r in rows.iter_mut() {
            if r["method"] != json!("farm") || !r["clip"].is_null() {
                continue;
            }
            let kind = r["kind"].as_str().unwrap_or("").to_owned();
            // A place the search reached: cut from its own run's inputs.
            if let (Some(run), Some(at)) = (
                r["evidence"]["run_path"].as_str(),
                r["evidence"]["run_frame"].as_u64(),
            ) {
                let run_log = crate::findings_clip::board_log(
                    &rom,
                    crate::mechanic::paths::read_path(&paths.home, run)?,
                )?;
                // Stored again in the board's spelling (the same path on
                // the NES).
                let (run, _) = crate::mechanic::paths::put_path(&paths.home, &run_log.to_text())?;
                let run = run.as_str();
                let at = (at as usize).min(run_log.frames.len().saturating_sub(1));
                let start = at.saturating_sub(90);
                let end = (at + 30).min(run_log.frames.len());
                // A search run can stop the frame it got there: the clip
                // goes on with the pad let go, so what happened can settle.
                let mut inputs = run_log.frames[start..end].to_vec();
                inputs.resize(
                    at + 30 - start,
                    gp_evidence::input_log::InputFrame::default(),
                );
                let (mut t, clip_log) =
                    crate::findings_clip::new_clip(&paths.home, &rom, run, start, &inputs)?;
                t.insert("by".into(), "findings cut".into());
                t.insert(
                    "note".into(),
                    format!(
                        "{kind} from the search run that first got there, frame {at}: {}",
                        r["what"].as_str().unwrap_or("")
                    )
                    .into(),
                );
                fs::write(dir.join(format!("clips/{next:03}.log")), clip_log)?;
                fs::write(
                    dir.join(format!("clips/{next:03}.toml")),
                    toml::to_string(&t)?,
                )?;
                r["clip"] = clip_ref(&dir, "findings", next);
                r["status"] = json!("clip");
                r["reason"] = Value::Null;
                next += 1;
                cut_n += 1;
                continue;
            }
            let frame = match kind.as_str() {
                "landing" => {
                    let (Some(wx), Some(wy)) = (r["where"]["x"].as_i64(), r["where"]["y"].as_i64())
                    else {
                        continue;
                    };
                    (0..pos.len()).find(|&i| {
                        still[i] >= 8
                            && pos[i].0.div_euclid(16) == wx / 16
                            && pos[i].1.div_euclid(16) == wy / 16
                    })
                }
                // A level end can be the path's last frame.
                "death" | "level-end" => r["evidence"]["frame"]
                    .as_u64()
                    .map(|v| (v as usize).min(log.frames.len().saturating_sub(1))),
                _ => None,
            };
            let Some(at) = frame.filter(|&i| i < log.frames.len()) else {
                r["reason"] = json!("not found on the run's path, so no clip was cut");
                missed += 1;
                continue;
            };
            let start = at.saturating_sub(90);
            let end = (at + 30).min(log.frames.len());
            let mut inputs = log.frames[start..end].to_vec();
            inputs.resize(
                at + 30 - start,
                gp_evidence::input_log::InputFrame::default(),
            );
            let (mut t, clip_log) =
                crate::findings_clip::new_clip(&paths.home, &rom, &sha, start, &inputs)?;
            t.insert("by".into(), "findings cut".into());
            t.insert(
                "note".into(),
                format!(
                    "{kind} from the blind run's path at frame {at}: {}",
                    r["what"].as_str().unwrap_or("")
                )
                .into(),
            );
            fs::write(dir.join(format!("clips/{next:03}.log")), clip_log)?;
            fs::write(
                dir.join(format!("clips/{next:03}.toml")),
                toml::to_string(&t)?,
            )?;
            r["clip"] = clip_ref(&dir, "findings", next);
            r["status"] = json!("clip");
            r["reason"] = Value::Null;
            next += 1;
            cut_n += 1;
        }
        write_all(&f, &rows)?;
    }
    // The along-sweep's leads: each place's inputs and the jump's buttons
    // are rebuilt from its strip.
    let (mut swept, mut sweep_missed) = (0usize, 0usize);
    let sdir = paths.home.join("mechanics").join(game).join("findings");
    for r in rows.iter_mut() {
        if r["method"] != json!("sweep") || !r["clip"].is_null() {
            continue;
        }
        let e = &r["evidence"];
        let (Some(file), Some(col), Some(py), Some(jump)) = (
            e["file"].as_str(),
            e["column"].as_i64(),
            e["place_y"].as_i64(),
            e["jump"].as_str(),
        ) else {
            continue;
        };
        let (_, at, inputs) = match crate::sweep::place_clip(Path::new(file), col, py, jump) {
            Ok(v) => v,
            Err(err) => {
                r["reason"] = json!(format!("no clip: {err:#}"));
                sweep_missed += 1;
                continue;
            }
        };
        let path = gp_evidence::input_log::InputLog {
            controllers: 1,
            frames: inputs[..at].to_vec(),
            arcade: None,
        };
        let (sha, _) = crate::mechanic::paths::put_path(&paths.home, &path.to_text())?;
        let start = at.saturating_sub(20);
        let mut tail = inputs[start..].to_vec();
        tail.extend(std::iter::repeat_n(
            gp_evidence::input_log::InputFrame::default(),
            40,
        ));
        let (mut t, clip_log) =
            crate::findings_clip::new_clip(&paths.home, &rom, &sha, start, &tail)?;
        t.insert("by".into(), "findings cut".into());
        t.insert(
            "note".into(),
            format!(
                "{} from the along-sweep, {jump} at column {col}: {}",
                r["kind"].as_str().unwrap_or(""),
                r["what"].as_str().unwrap_or("")
            )
            .into(),
        );
        fs::create_dir_all(sdir.join("clips"))?;
        if !sdir.join("session.toml").is_file() {
            let img = crate::farm::machine::rom_sha256(&rom)?;
            let mut t = toml::Table::new();
            t.insert("game".into(), game.into());
            t.insert("mechanic".into(), "findings".into());
            t.insert(
                "prompt".into(),
                "clips cut for findings that were leads".into(),
            );
            t.insert("rom".into(), img.into());
            fs::write(sdir.join("session.toml"), toml::to_string(&t)?)?;
        }
        let n = fs::read_dir(sdir.join("clips"))?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".toml")
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0)
            + 1;
        fs::write(sdir.join(format!("clips/{n:03}.log")), clip_log)?;
        fs::write(
            sdir.join(format!("clips/{n:03}.toml")),
            toml::to_string(&t)?,
        )?;
        r["clip"] = clip_ref(&sdir, "findings", n);
        r["status"] = json!("clip");
        r["reason"] = Value::Null;
        swept += 1;
    }
    write_all(&f, &rows)?;
    let _ = writeln!(
        text,
        "cut {cut_n} clips for farm leads ({missed} not found on the path), {swept} for sweep leads ({sweep_missed} couldn't be rebuilt)"
    );
    if confirm_all {
        let ids: Vec<String> = rows
            .iter()
            .filter(|r| {
                !r["clip"].is_null() && matches!(r["status"].as_str(), Some("clip" | "confirmed"))
            })
            .filter_map(|r| r["id"].as_str().map(str::to_owned))
            .collect();
        for id in &ids {
            let mut rows = read_all(&f)?;
            let Some(i) = rows.iter().position(|r| r["id"].as_str() == Some(id)) else {
                continue;
            };
            let (verdict, reason, video) = gate(paths, game, &rows[i]);
            rows[i]["status"] = json!(verdict);
            rows[i]["reason"] = json!(reason);
            if !video.is_null() {
                rows[i]["video"] = video;
            }
            write_all(&f, &rows)?;
        }
        let _ = writeln!(text, "ran the gate on {} findings", ids.len());
    }
    let rows = read_all(&f)?;
    text.push_str(&table(&rows));
    Ok(Outcome::ok(json!({"cut": cut_n, "missed": missed}), text))
}

fn table(rows: &[Value]) -> String {
    let mut by: BTreeMap<(String, String), BTreeMap<String, usize>> = BTreeMap::new();
    for r in rows {
        let k = (
            r["kind"].as_str().unwrap_or("").to_owned(),
            r["method"].as_str().unwrap_or("").to_owned(),
        );
        *by.entry(k)
            .or_default()
            .entry(r["status"].as_str().unwrap_or("").to_owned())
            .or_default() += 1;
    }
    let mut s = String::from(
        "| Kind | Method | Lead | Clip | Confirmed | Rejected |\n|---|---|---|---|---|---|\n",
    );
    for ((k, m), c) in &by {
        let g = |s: &str| c.get(s).copied().unwrap_or(0);
        let _ = writeln!(
            s,
            "| {k} | {m} | {} | {} | {} | {} |",
            g("lead"),
            g("clip"),
            g("confirmed"),
            g("rejected")
        );
    }
    s
}

fn list(paths: &Paths, game: &str, status: Option<&str>) -> Result<Outcome> {
    let f = file_of(paths, game)?;
    let rows: Vec<Value> = read_all(&f)?
        .into_iter()
        .filter(|r| status.is_none_or(|s| r["status"].as_str() == Some(s)))
        .collect();
    let mut text = format!("{} findings for {game}\n{}", rows.len(), table(&rows));
    let open: Vec<&Value> = rows
        .iter()
        .filter(|r| matches!(r["status"].as_str(), Some("lead" | "clip")))
        .take(10)
        .collect();
    if !open.is_empty() {
        text.push_str("\nNot confirmed yet:\n");
        for r in &open {
            let _ = writeln!(
                text,
                "  {} {} ({}, {}): {}",
                r["id"].as_str().unwrap_or(""),
                r["kind"].as_str().unwrap_or(""),
                r["method"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or(""),
                r["what"].as_str().unwrap_or("")
            );
        }
    }
    Ok(Outcome::ok(json!({"findings": rows}), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_has_a_stable_id_and_its_status_follows_the_clip() {
        let a = record(
            "g",
            "w".into(),
            "head-hit",
            "sweep",
            Some("1-1".into()),
            Some(16),
            Some(32),
            json!({}),
            None,
        );
        let b = record(
            "g",
            "other words".into(),
            "head-hit",
            "sweep",
            Some("1-1".into()),
            Some(16),
            Some(32),
            json!({}),
            None,
        );
        assert_eq!(a["id"], b["id"]);
        assert_eq!(a["status"], "lead");
        let c = record(
            "g",
            "w".into(),
            "head-hit",
            "user",
            None,
            None,
            None,
            json!({}),
            Some(json!({"session": "s", "n": 1})),
        );
        assert_eq!(c["status"], "clip");
        assert_ne!(a["id"], c["id"]);
    }

    #[test]
    fn an_egg_needs_a_head_hit_that_changed_ram_for_good() {
        let brief = json!([{"at_hit": [{"lasting": false}]}]);
        let kept = json!([{"at_hit": [{"lasting": true}]}]);
        assert!(!fires("egg", brief.as_array().unwrap()));
        assert!(fires("egg", kept.as_array().unwrap()));
        assert!(fires("head-hit", brief.as_array().unwrap()));
        assert!(!fires("landing", json!([{"up_px": 0}]).as_array().unwrap()));
    }

    #[test]
    fn a_detector_counts_only_at_the_findings_place() {
        let e = json!({"frame": 3});
        // The head at y 144 hits the cell above, 128; x 1016 + 8 is in 1024.
        assert!(near("egg", &e, (1016, 144), 1024, Some(128)));
        assert!(!near("egg", &e, (1200, 144), 1024, Some(128)));
        assert!(!near("head-hit", &e, (1016, 80), 1024, Some(128)));
        let rest = json!({"from": 3, "y": 112});
        assert!(near("landing", &rest, (330, 112), 320, Some(112)));
        assert!(!near("landing", &rest, (400, 112), 320, Some(112)));
        // A placed finding's key ignores its clip; a session's doesn't.
        let mut a = record(
            "g",
            "w".into(),
            "landing",
            "farm",
            None,
            Some(16),
            Some(32),
            json!({}),
            None,
        );
        let k = key_of(&a);
        a["clip"] = json!({"session": "findings", "n": 4});
        assert_eq!(key_of(&a), k);
    }

    #[test]
    fn records_round_trip_and_a_note_gives_its_place() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths {
            home: d.path().to_path_buf(),
            store: d.path().join("store"),
            roms: d.path().join("roms"),
        };
        let f = file_of(&p, "g").unwrap();
        let mut r = record(
            "g",
            "w".into(),
            "rule",
            "rule",
            None,
            None,
            None,
            json!({}),
            None,
        );
        r["status"] = json!("confirmed");
        write_all(&f, &[r.clone()]).unwrap();
        let back = read_all(&f).unwrap();
        assert_eq!(back[0]["status"], "confirmed");
        assert_eq!(xy_in("to level x 1024, y 128: hi"), (Some(1024), Some(128)));
    }
}
