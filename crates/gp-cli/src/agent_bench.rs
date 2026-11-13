//! `gp agent bench`: the loop's efficiency in one table (docs/agents.md,
//! "Loop efficiency"). It reads a run's background ledger and history, the
//! clip agent's work record and the logs the agents leave, and writes
//! nothing but, with `--log`, one row in the deterministic inference note.
//!
//! The ledger has no times, so a window is cut by history rounds: each
//! refine entry in the ledger is matched, in order, to the next history round
//! on the same clip, and the score entries that follow it belong to that
//! round. Time comes from the history's `when_ms`. Gaps between rounds over
//! `IDLE_MS` (the server was off) are left out of the work time.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

/// A gap between two rounds longer than this is not work.
const IDLE_MS: u64 = 30 * 60 * 1000;
/// The NES frame rate, for footage seconds.
const FPS: f64 = 60.0988;

/// Where a window starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Since {
    Round(u64),
    Ms(u64),
}

/// `--since`: a round number (`300`), a time as epoch seconds or ms, a UTC
/// date and time (`2026-10-30 08:00`, `2026-10-30T08:00:00Z`, `2026-10-30`)
/// or an age before the last round (`2h`, `30m`, `1d`).
pub fn parse_since(s: &str, last_ms: u64) -> anyhow::Result<Since> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u64>() {
        return Ok(if n < 1_000_000_000 {
            Since::Round(n)
        } else if n < 100_000_000_000 {
            Since::Ms(n * 1000)
        } else {
            Since::Ms(n)
        });
    }
    if let Some(unit) = s.chars().last().filter(|c| "mhd".contains(*c))
        && let Ok(n) = s[..s.len() - 1].parse::<u64>()
    {
        let ms = n * match unit {
            'm' => 60_000,
            'h' => 3_600_000,
            _ => 86_400_000,
        };
        return Ok(Since::Ms(last_ms.saturating_sub(ms)));
    }
    let t = s.trim_end_matches('Z').replace('T', " ");
    let (d, hm) = t.split_once(' ').unwrap_or((&t, "00:00"));
    let bad = || {
        anyhow::anyhow!(
            "can't read --since {s:?}: use a round number, a time like \"2026-10-30 08:00\", or an age like 2h"
        )
    };
    let mut dp = d.split('-').map(|x| x.parse::<i32>());
    let (Some(Ok(y)), Some(Ok(mo)), Some(Ok(da))) = (dp.next(), dp.next(), dp.next()) else {
        return Err(bad());
    };
    let mut tp = hm.split(':').map(|x| x.parse::<u8>());
    let h = tp.next().and_then(Result::ok).ok_or_else(bad)?;
    let mi = tp.next().and_then(Result::ok).unwrap_or(0);
    let se = tp.next().and_then(Result::ok).unwrap_or(0);
    let date = Date::from_calendar_date(y, Month::try_from(mo as u8).map_err(|_| bad())?, da as u8)
        .map_err(|_| bad())?;
    let time = Time::from_hms(h, mi, se).map_err(|_| bad())?;
    let ts = PrimitiveDateTime::new(date, time)
        .assume_utc()
        .unix_timestamp();
    Ok(Since::Ms(ts.max(0) as u64 * 1000))
}

fn read_jsonl(f: &Path) -> Vec<Value> {
    std::fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn u(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}

/// The source a round's kept change is credited to.
fn source_of(h: &Value) -> String {
    if let Some(s) = h["source"].as_str() {
        return s.to_owned();
    }
    if let Some(s) = h["solver"].as_str() {
        return format!("solver:{s}");
    }
    "fit".into()
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn frames_of(score: &Value) -> u64 {
    score["screen"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(|l| l["total"].as_u64())
        .max()
        .unwrap_or(0)
}

/// Per-clip exactness after each ledger score entry.
struct Replay {
    exact: BTreeMap<String, bool>,
}

impl Replay {
    fn counts(&self) -> Option<(usize, usize)> {
        (!self.exact.is_empty()).then(|| {
            (
                self.exact.values().filter(|e| **e).count(),
                self.exact.len(),
            )
        })
    }
}

fn pct(c: Option<(usize, usize)>) -> String {
    match c {
        Some((e, n)) => format!("{e} of {n} exact ({}%)", e * 100 / n.max(1)),
        None => "no scores".into(),
    }
}

/// Hours and minutes, or minutes and seconds under an hour.
fn dur(s: f64) -> String {
    let s = s.round() as u64;
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

fn by_source(m: &BTreeMap<String, f64>, hours: f64, per_hour: bool) -> String {
    let mut m = m.clone();
    for k in ["fit", "early stop", "LLM attempt"] {
        m.entry(k.into()).or_insert(0.0);
    }
    let rank = |k: &str| match k {
        "fit" => 0,
        "early stop" => 2,
        "LLM attempt" => 3,
        "user note" => 4,
        _ => 1,
    };
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort_by_key(|k| (rank(k), (*k).clone()));
    keys.into_iter()
        .map(|k| {
            let v = m[k];
            if per_hour {
                format!("{k} {:.2}", v / hours.max(1e-9))
            } else {
                format!("{k} {}", v as u64)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reads the run and computes every measure. `home` is `$GP_HOME`.
pub fn measure(home: &Path, run_dir: &Path, since: Option<&str>) -> anyhow::Result<Value> {
    let bg = run_dir.join("background");
    let ledger = read_jsonl(&bg.join("ledger.jsonl"));
    let history: Vec<Value> = read_jsonl(&bg.join("history.jsonl"))
        .into_iter()
        .filter(|h| h.get("round").is_some())
        .collect();
    if history.is_empty() && ledger.is_empty() {
        bail!("no background ledger or history in {}", bg.display());
    }
    let last_ms = history.iter().map(|h| u(h, "when_ms")).max().unwrap_or(0);
    let since_v = since.map(|s| parse_since(s, last_ms)).transpose()?;
    let start_idx = match since_v {
        None => Some(0),
        Some(Since::Round(n)) => history.iter().position(|h| u(h, "round") >= n),
        Some(Since::Ms(t)) => history.iter().position(|h| u(h, "when_ms") >= t),
    }
    .unwrap_or(history.len());
    let first_round = history.get(start_idx).map_or(u64::MAX, |h| u(h, "round"));
    let win = &history[start_idx..];

    // Round times and gaps.
    let mut prev = start_idx.checked_sub(1).map(|i| u(&history[i], "when_ms"));
    let (mut gaps, mut idle_ms, mut work_ms) = (Vec::<f64>::new(), 0u64, 0u64);
    let wall_start = prev.or_else(|| win.first().map(|h| u(h, "when_ms")));
    for h in win {
        let t = u(h, "when_ms");
        if let Some(p) = prev {
            let g = t.saturating_sub(p);
            if g > IDLE_MS {
                idle_ms += g;
            } else {
                work_ms += g;
                gaps.push(g as f64 / 1000.0);
            }
        }
        prev = Some(t);
    }
    let wall_ms = match (wall_start, win.last()) {
        (Some(a), Some(h)) => u(h, "when_ms").saturating_sub(a),
        _ => 0,
    };
    let work_s = work_ms as f64 / 1000.0;
    let hours = work_s / 3600.0;
    let rounds = win.len();

    // Kept changes and fixes by source.
    let mut kept: BTreeMap<String, f64> = BTreeMap::new();
    let mut fixed: BTreeMap<String, f64> = BTreeMap::new();
    let mut skipped = 0u64;
    for h in win {
        let src = source_of(h);
        if h["changes"].as_array().is_some_and(|c| !c.is_empty()) {
            *kept.entry(src.clone()).or_default() += 1.0;
        }
        let f = u(h, "fixed");
        if f > 0 {
            *fixed.entry(src).or_default() += f as f64;
        }
        if h["skipped"].as_bool() == Some(true) {
            skipped += 1;
        }
    }

    // Ledger: match refine entries to rounds, then replay scores.
    let mut cursor = 0usize;
    let mut round_of: Vec<u64> = Vec::with_capacity(ledger.len());
    let mut cur_round = 0u64;
    for e in &ledger {
        if e["kind"] == "refine" {
            let clip = e["clip"].as_str().unwrap_or("");
            if let Some(off) = history[cursor..].iter().position(|h| h["clip"] == clip) {
                cur_round = u(&history[cursor + off], "round");
                cursor += off + 1;
            }
        }
        round_of.push(cur_round);
    }
    let mut replay = Replay {
        exact: BTreeMap::new(),
    };
    let mut at_start: Option<(usize, usize)> = None;
    let mut started = false;
    let (mut frames, mut scored) = (0u64, 0u64);
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut refines_before = 0u64;
    let mut refines_in = 0u64;
    for (e, &r) in ledger.iter().zip(&round_of) {
        let in_win = r >= first_round && r != 0 || since_v.is_none();
        if in_win && !started {
            at_start = replay.counts();
            started = true;
        }
        let key = e["key"].as_str().unwrap_or("");
        match e["kind"].as_str() {
            Some("score") => {
                let res = std::fs::read(home.join("results/score").join(format!("{key}.json")))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                if let Some(res) = res {
                    let s = &res["result"];
                    if let Some(x) = s["exact"].as_bool() {
                        replay
                            .exact
                            .insert(e["clip"].as_str().unwrap_or("").to_owned(), x);
                    }
                    if in_win {
                        frames += frames_of(s);
                        scored += 1;
                    }
                }
            }
            Some("refine") => {
                if in_win {
                    refines_in += 1;
                } else {
                    refines_before += 1;
                }
                let sig = (
                    e["clip"].as_str().unwrap_or("").to_owned(),
                    e["inputs"]["model"].as_str().unwrap_or("").to_owned(),
                    e["result"].as_str().unwrap_or("").to_owned(),
                );
                if !seen.insert(sig) && in_win && e["decision"]["changed"] == false {
                    skipped += 1;
                }
            }
            _ => {}
        }
    }
    if !started {
        at_start = replay.counts();
    }
    let at_end = replay.counts();

    // Fidelity: the last entry at or before each end of the window.
    let fid: Vec<Value> = std::fs::read(run_dir.join("fidelity/history.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let fid_at = |rounds: u64| {
        fid.iter()
            .rev()
            .find(|f| u(f, "rounds") <= rounds)
            .map(|f| u(f, "score"))
    };
    let f_start = fid_at(refines_before).or_else(|| fid.first().map(|f| u(f, "score")));
    let f_end = fid_at(refines_before + refines_in);

    // The agents: attempts, and the logs each call leaves.
    let in_time = |ms: u64| match (win.first(), win.last()) {
        (Some(a), Some(b)) => ms >= wall_start.unwrap_or(u(a, "when_ms")) && ms <= u(b, "when_ms"),
        _ => false,
    };
    let agent = run_dir.join("agent");
    let (mut llm_kept, mut llm_fixed, mut note_fixed) = (0u64, 0u64, 0u64);
    let (mut clip_calls, mut request_calls) = (0u64, 0u64);
    for ent in std::fs::read_dir(&agent).into_iter().flatten().flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if let Some(ms) = name
            .strip_prefix("clip-agent-")
            .and_then(|n| n.strip_suffix(".log"))
            .and_then(|n| n.parse::<u64>().ok())
        {
            if in_time(ms) {
                clip_calls += 1;
            }
        } else if name.starts_with("request-agent-") && name.ends_with(".log") {
            let ms = ent
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            if in_time(ms) {
                request_calls += 1;
            }
        } else if let Ok(a) = std::fs::read(ent.path().join("attempt.json"))
            && let Ok(a) = serde_json::from_slice::<Value>(&a)
            && a["by"] == "clip agent"
            && in_time(u(&a, "when_ms"))
        {
            if a["approved"] == true {
                llm_kept += 1;
            }
            if a["before"]["exact"] == false && a["after"]["exact"] == true {
                llm_fixed += 1;
                // A fix by an attempt linked to a tell is the person's.
                if a["for_tells"].as_array().is_some_and(|t| !t.is_empty()) {
                    note_fixed += 1;
                }
            }
        }
    }
    if llm_kept > 0 {
        kept.insert("LLM attempt".into(), llm_kept as f64);
    }
    if llm_fixed > note_fixed {
        fixed.insert("LLM attempt".into(), (llm_fixed - note_fixed) as f64);
    }
    if note_fixed > 0 {
        fixed.insert("user note".into(), note_fixed as f64);
    }
    let llm_calls = clip_calls + request_calls;
    let total_fixed: f64 = fixed.values().fold(0.0, |a, b| a + b);

    let footage_s = frames as f64 / FPS;
    let mut gaps2 = gaps.clone();
    Ok(json!({
        "run": run_dir.file_name().map(|n| n.to_string_lossy().into_owned()),
        "since": since,
        "first_round": win.first().map(|h| u(h, "round")),
        "last_round": win.last().map(|h| u(h, "round")),
        "wall_s": wall_ms as f64 / 1000.0,
        "work_s": work_s,
        "idle_s": idle_ms as f64 / 1000.0,
        "rounds": rounds,
        "rounds_per_hour": rounds as f64 / hours.max(1e-9),
        "kept_by_source": kept,
        "fixed_by_source": fixed,
        "fixed_per_hour_by_source": fixed.iter().map(|(k, v)| (k.clone(), v / hours.max(1e-9))).collect::<BTreeMap<_, _>>(),
        "mean_round_s": if gaps.is_empty() { 0.0 } else { gaps.iter().sum::<f64>() / gaps.len() as f64 },
        "median_round_s": median(&mut gaps2),
        "scores_read": scored,
        "footage_s": footage_s,
        "work_per_footage": if footage_s > 0.0 { Some(work_s / footage_s) } else { None },
        "repeat_rounds_skipped": skipped,
        "llm": {"clip_agent_calls": clip_calls, "request_agent_calls": request_calls,
                "calls_per_fix": if llm_fixed > 0 { Some(llm_calls as f64 / llm_fixed as f64) } else { None },
                "fixes": llm_fixed},
        "exact_start": at_start.map(|(e, n)| json!({"exact": e, "clips": n})),
        "exact_end": at_end.map(|(e, n)| json!({"exact": e, "clips": n})),
        "fidelity_start": f_start,
        "fidelity_end": f_end,
        "total_fixed": total_fixed,
    }))
}

/// The table.
pub fn table(m: &Value) -> String {
    let hours = m["work_s"].as_f64().unwrap_or(0.0) / 3600.0;
    let map = |k: &str| -> BTreeMap<String, f64> {
        m[k].as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let n = |k: &str| m[k].as_f64().unwrap_or(0.0);
    let ex = |k: &str| {
        m[k].as_object().map(|o| {
            (
                o["exact"].as_u64().unwrap_or(0) as usize,
                o["clips"].as_u64().unwrap_or(0) as usize,
            )
        })
    };
    let fid = |k: &str| m[k].as_u64().map_or("n/a".into(), |f| format!("{f}%"));
    let ratio = m["work_per_footage"].as_f64().map_or(
        format!("n/a ({} score entries had no footage)", m["scores_read"]),
        |r| format!("{r:.1} (over {} of footage)", dur(n("footage_s"))),
    );
    let llm = &m["llm"];
    let calls = llm["clip_agent_calls"].as_u64().unwrap_or(0)
        + llm["request_agent_calls"].as_u64().unwrap_or(0);
    let per_fix = llm["calls_per_fix"]
        .as_f64()
        .map_or("n/a (0 fixes)".to_string(), |p| format!("{p:.1}"));
    let rows: Vec<(&str, String)> = vec![
        (
            "Wall time",
            format!(
                "{} (work {}, {} idle left out)",
                dur(n("wall_s")),
                dur(n("work_s")),
                dur(n("idle_s"))
            ),
        ),
        ("Rounds", m["rounds"].to_string()),
        ("Rounds per hour", format!("{:.1}", n("rounds_per_hour"))),
        (
            "Rounds that kept a change, by source",
            by_source(&map("kept_by_source"), hours, false),
        ),
        (
            "Clips fixed per hour, by source",
            format!(
                "{} (total {})",
                by_source(&map("fixed_per_hour_by_source"), 1.0, true),
                n("total_fixed") as u64
            ),
        ),
        (
            "Seconds per round, mean and median",
            format!("{:.1} and {:.1}", n("mean_round_s"), n("median_round_s")),
        ),
        ("Seconds of work per second of footage scored", ratio),
        (
            "Repeat rounds skipped",
            m["repeat_rounds_skipped"].to_string(),
        ),
        (
            "LLM calls (clip agent, request agent)",
            format!(
                "{calls} ({}, {})",
                llm["clip_agent_calls"], llm["request_agent_calls"]
            ),
        ),
        ("LLM calls per fix", per_fix),
        (
            "Exact at start and end",
            match (ex("exact_start"), ex("exact_end")) {
                (Some((a, _)), Some((b, _))) => format!(
                    "{} -> {} ({:+} clips)",
                    pct(ex("exact_start")),
                    pct(ex("exact_end")),
                    b as i64 - a as i64
                ),
                _ => format!("{} -> {}", pct(ex("exact_start")), pct(ex("exact_end"))),
            },
        ),
        (
            "Fidelity at start and end",
            format!("{} -> {}", fid("fidelity_start"), fid("fidelity_end")),
        ),
    ];
    let mut out = String::from("| Measure | Value |\n|---|---|\n");
    for (k, v) in rows {
        out.push_str(&format!("| {k} | {v} |\n"));
    }
    out.trim_end().to_owned()
}

/// One line for the notes log: the table's numbers in a sentence.
pub fn log_text(m: &Value) -> String {
    let n = |k: &str| m[k].as_f64().unwrap_or(0.0);
    let kept: f64 = m["kept_by_source"]
        .as_object()
        .map_or(0.0, |o| o.values().filter_map(Value::as_f64).sum());
    let ex = |k: &str| {
        m[k].as_object().map_or("n/a".to_owned(), |o| {
            format!("{}/{}", o["exact"], o["clips"])
        })
    };
    let fid = |k: &str| m[k].as_u64().map_or("n/a".into(), |f| format!("{f}%"));
    format!(
        "Loop bench, run `{}`, rounds {} to {}: {} work ({} idle left out), {} rounds ({:.1}/h, median {:.0} s), {} kept a change, {} clips fixed ({:.2}/h), {} repeats skipped, {} LLM calls; exact {} -> {}, fidelity {} -> {}",
        m["run"].as_str().unwrap_or("?"),
        m["first_round"],
        m["last_round"],
        dur(n("work_s")),
        dur(n("idle_s")),
        m["rounds"],
        n("rounds_per_hour"),
        n("median_round_s"),
        kept as u64,
        n("total_fixed") as u64,
        n("total_fixed") / (n("work_s") / 3600.0).max(1e-9),
        m["repeat_rounds_skipped"],
        m["llm"]["clip_agent_calls"].as_u64().unwrap_or(0)
            + m["llm"]["request_agent_calls"].as_u64().unwrap_or(0),
        ex("exact_start"),
        ex("exact_end"),
        fid("fidelity_start"),
        fid("fidelity_end"),
    )
}

/// Appends one dated row to the "Notes log" table in `doc`.
pub fn append_log(doc: &Path, row_text: &str) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(doc).with_context(|| format!("reading {}", doc.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let h = lines
        .iter()
        .position(|l| l.starts_with("## ") && l.contains("Notes log"))
        .context("no \"Notes log\" heading in the note")?;
    let mut last = None;
    for (i, l) in lines.iter().enumerate().skip(h + 1) {
        if l.starts_with("## ") {
            break;
        }
        if l.starts_with('|') {
            last = Some(i);
        }
    }
    let at = last.context("no table under the Notes log heading")?;
    let now = OffsetDateTime::now_utc();
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(doc.parent().unwrap_or(Path::new(".")))
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_owned());
    let row = format!(
        "| {:04}-{:02}-{:02} | {} | {} |",
        now.year(),
        u8::from(now.month()),
        now.day(),
        branch,
        row_text.replace('|', "/")
    );
    let mut out: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
    out.insert(at + 1, row);
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    std::fs::write(doc, s)?;
    Ok(())
}

/// Entry for the CLI.
pub fn run(
    home: &Path,
    dir: &Path,
    since: Option<&str>,
    log: bool,
    note: Option<PathBuf>,
) -> anyhow::Result<crate::Outcome> {
    let m = measure(home, dir, since)?;
    let mut text = table(&m);
    if log {
        let doc = note.unwrap_or_else(|| PathBuf::from("docs/research/deterministic-inference.md"));
        append_log(&doc, &log_text(&m))?;
        text.push_str(&format!("\nLogged one row in {}.", doc.display()));
    }
    Ok(crate::Outcome::ok(m, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn put(dir: &Path, name: &str, rows: &[Value]) {
        fs::create_dir_all(dir.join("background")).unwrap();
        let s: String = rows.iter().map(|r| format!("{r}\n")).collect();
        fs::write(dir.join("background").join(name), s).unwrap();
    }

    /// A made-up run: six rounds, two minutes apart, one idle gap, a fit
    /// that fixes a clip, a repeat, and a clip agent attempt.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let run = home.path().join("capture/r1");
        let t0 = 1_800_000_000_000u64;
        let h = |round: u64, at: u64, clip: &str, changes: Value, fixed: u64| json!({"round": round, "when_ms": t0 + at * 1000, "clip": clip, "changes": changes, "fixed": fixed});
        put(
            &run,
            "history.jsonl",
            &[
                h(1, 0, "a", json!([]), 0),
                h(2, 120, "b", json!([{"param": "x"}]), 1),
                h(3, 240, "a", json!([]), 0),
                h(4, 360, "a", json!([]), 0),
                {
                    let mut r = h(5, 360 + 7200, "b", json!([{"param": "y"}]), 0);
                    r["source"] = json!("solver:foo");
                    r
                },
                h(6, 360 + 7320, "c", json!([]), 0),
            ],
        );
        let score = |clip: &str, key: &str, exact: bool| {
            let dir = home.path().join("results/score");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(format!("{key}.json")),
                json!({"result": {"exact": exact, "screen": {"sprites": {"total": 601}}}})
                    .to_string(),
            )
            .unwrap();
            json!({"kind": "score", "clip": clip, "key": key, "result": "h"})
        };
        let refine = |clip: &str, key: &str, model: &str, res: &str, changed: bool| {
            json!({"kind": "refine", "clip": clip, "key": key, "inputs": {"model": model}, "result": res,
                "decision": {"changed": changed}})
        };
        put(
            &run,
            "ledger.jsonl",
            &[
                score("a", "k1", false),
                score("b", "k2", false),
                refine("a", "r1", "m1", "x1", false),
                refine("b", "r2", "m1", "x2", true),
                score("b", "k3", true),
                refine("a", "r3", "m1", "x1", false),
                refine("a", "r4", "m2", "x1", false),
                refine("b", "r5", "m2", "x5", true),
                refine("c", "r6", "m2", "x6", false),
            ],
        );
        fs::create_dir_all(run.join("agent/1")).unwrap();
        fs::write(
            run.join("agent/1/attempt.json"),
            json!({"by": "clip agent", "approved": true, "when_ms": t0 + 100_000,
                "before": {"exact": false}, "after": {"exact": true}})
            .to_string(),
        )
        .unwrap();
        fs::write(
            run.join(format!("agent/clip-agent-{}.log", t0 + 200_000)),
            "",
        )
        .unwrap();
        (home, run)
    }

    #[test]
    fn measures_a_made_up_run() {
        let (home, run) = fixture();
        let m = measure(home.path(), &run, None).unwrap();
        assert_eq!(m["rounds"], 6);
        assert_eq!(m["idle_s"], 7200.0);
        assert_eq!(m["work_s"], 480.0);
        assert_eq!(m["kept_by_source"]["solver:foo"], 1.0);
        assert_eq!(m["kept_by_source"]["fit"], 1.0);
        assert_eq!(m["fixed_by_source"]["fit"], 1.0);
        assert_eq!(m["kept_by_source"]["LLM attempt"], 1.0);
        assert_eq!(m["fixed_by_source"]["LLM attempt"], 1.0);
        assert_eq!(m["fixed_by_source"].get("user note"), None);
        assert_eq!(m["llm"]["clip_agent_calls"], 1);
        assert_eq!(m["llm"]["calls_per_fix"], 1.0);
        // r3 repeats r1 (same clip, model and result, nothing changed).
        assert_eq!(m["repeat_rounds_skipped"], 1);
        assert_eq!(m["exact_end"], json!({"exact": 1, "clips": 2}));
        assert_eq!(m["median_round_s"], 120.0);
        let t = table(&m);
        assert!(t.contains("| Rounds | 6 |"), "{t}");
        assert!(t.contains("fit 1, solver:foo 1, early stop 0"), "{t}");
    }

    #[test]
    fn a_fix_by_an_attempt_linked_to_a_tell_is_the_users() {
        let (home, run) = fixture();
        let t0 = 1_800_000_000_000u64;
        fs::create_dir_all(run.join("agent/2")).unwrap();
        fs::write(
            run.join("agent/2/attempt.json"),
            json!({"by": "clip agent", "approved": true, "when_ms": t0 + 110_000, "for_tells": [1],
                "before": {"exact": false}, "after": {"exact": true}})
            .to_string(),
        )
        .unwrap();
        let m = measure(home.path(), &run, None).unwrap();
        assert_eq!(m["fixed_by_source"]["user note"], 1.0);
        assert_eq!(m["fixed_by_source"]["LLM attempt"], 1.0);
        assert_eq!(m["total_fixed"], 3.0);
        assert_eq!(m["llm"]["fixes"], 2);
        let t = table(&m);
        assert!(t.contains("LLM attempt 7.50, user note 7.50"), "{t}");
    }

    #[test]
    fn since_cuts_the_window() {
        let (home, run) = fixture();
        let m = measure(home.path(), &run, Some("4")).unwrap();
        assert_eq!(m["rounds"], 3);
        assert_eq!(m["first_round"], 4);
        // The window starts at round 3's time, so round 4's 120 s counts.
        assert_eq!(m["work_s"], 120.0 + 120.0);
        assert_eq!(m["exact_start"], json!({"exact": 1, "clips": 2}));
        assert_eq!(m["llm"]["fixes"], 0);
        let t = table(&m);
        assert!(t.contains("n/a (0 fixes)"), "{t}");
    }

    #[test]
    fn since_forms() {
        assert_eq!(parse_since("300", 0).unwrap(), Since::Round(300));
        assert_eq!(
            parse_since("1800000000000", 0).unwrap(),
            Since::Ms(1_800_000_000_000)
        );
        assert_eq!(parse_since("2h", 10_000_000).unwrap(), Since::Ms(2_800_000));
        assert_eq!(
            parse_since("2027-01-15 08:30", 0).unwrap(),
            Since::Ms(1_800_001_800_000)
        );
        assert!(parse_since("soon", 0).is_err());
    }

    #[test]
    fn log_row_goes_in_the_notes_table() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("note.md");
        fs::write(&f, "## 5. Notes log\n\n| Date | Branch | Finding |\n|---|---|---|\n| d | b | first |\n\n## Sources\n").unwrap();
        append_log(&f, "bench | row").unwrap();
        let s = fs::read_to_string(&f).unwrap();
        let rows: Vec<&str> = s.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 4);
        assert!(rows[3].ends_with("| bench / row |"), "{rows:?}");
        assert!(s.contains("\n\n## Sources"));
    }
}
