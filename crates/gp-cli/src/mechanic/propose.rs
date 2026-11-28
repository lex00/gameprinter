//! `gp mechanic propose`: the loop adds a quest-log row on its own. What
//! the picture check finds (`gp picture --propose`) and what a worker finds
//! while it works become rows with no card: status next, a note that
//! starts "Proposed by the loop:" and says where it came from. A worker
//! that proposes a row gives its test too (`--goal`), written in the same
//! call to the home's triage.toml for the game (beside its quest log, never
//! the games checkout, which takes changes only through a gate), so the
//! triage tries the new row before an agent or the person does
//! (docs/loop.md).

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::session::{name_ok, queue_file, update_queue};
#[cfg(test)]
use super::session::{read_queue, write_queue};

/// The words every proposed row's note starts with.
pub(crate) const MARK: &str = "Proposed by the loop:";

/// A row the loop proposes.
#[derive(Default, Clone, Copy)]
pub(crate) struct Proposal<'a> {
    pub game: &'a str,
    pub row: &'a str,
    /// Where it came from: the command, the clip and frame, the step.
    pub from: &'a str,
    /// What is wrong or missing, in one line.
    pub what: &'a str,
    /// tools for a row of gameprinter's own work, games for a game's. None
    /// leaves it to [`repo_of`].
    pub repo: Option<&'a str>,
    /// The loop's step the proposal is made in (`GP_STEP`, a worker the
    /// runner started): a row proposed from a tools step is a tools row
    /// unless `repo` says otherwise ([`repo_of`]).
    pub step: Option<&'a str>,
    /// The row's triage test, with the person's request.
    pub test: Option<Test<'a>>,
    /// The clips the gap is seen in (`gp picture --propose`). A row the
    /// quest log has already gets each clip it doesn't name yet added to its
    /// `seen_in`; an open row has it added to its note, and a done row is
    /// reopened for it (see `line`, `shows` and `target`).
    pub clips: &'a [String],
    /// The scoreboard line the gap was found on (`gp picture --line`). A
    /// row whose target is on another line can't measure this gap: the gap
    /// becomes a row of its own, ROW-LINE. A new row and a reopened one
    /// keep it as their `line`, the line they claim in the loop's plan
    /// with no target (crate::steps::claim_of).
    pub line: Option<&'a str>,
    /// The clip and frames that show the gap, from its start: a new row
    /// and a reopened one take them as the row's clip.
    pub shows: Option<(&'a str, u64, u64)>,
    /// The target that measures the gap: a new row and a reopened one take
    /// it in place of any target the row had (one it met already would
    /// close it again at once). None leaves a reopened row with what would
    /// measure it.
    pub target: Option<&'a str>,
    /// How many frames above where the line reads now the target is, and
    /// the frames the gap covers on that layer (more when the target was
    /// capped at the line's total): the row's note says so
    /// (crate::scoreboard::past_gap).
    pub target_frames: Option<(u64, u64)>,
    /// Why the new row waits (`--parked`): it is added in line, not next,
    /// its note "Parked: WHY." before the proposal's words. Filing a row
    /// and parking it is then one call, not a propose and a `gp mechanic
    /// queue --status "in line"` the loop takes for a hand fix
    /// (crate::trouble::status_by_hand). A row the quest log has is left
    /// as it is.
    pub parked: Option<&'a str>,
    /// One sentence a person can read (`--plain`): the new row's `plain`,
    /// which the loop's page shows on its card in place of the note.
    pub plain: Option<&'a str>,
}

/// Whether the row names the clip: in `seen_in`, or (rows from before
/// `seen_in`) in its note or where it came from.
fn names_clip(r: &Value, clip: &str) -> bool {
    r["seen_in"]
        .as_array()
        .is_some_and(|a| a.iter().any(|c| c == clip))
        || r["note"].as_str().is_some_and(|s| s.contains(clip))
        || r["from"].as_str().is_some_and(|s| s.contains(clip))
}

/// The failures a from names: each `FILE:LINE FIELD` in it (`approved.toml:14
/// look[0].apart`), as the file's name and the field.
fn failures(from: &str) -> Vec<(String, String)> {
    let trim = |s: &str| {
        s.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';' | '\'' | '"' | '`'))
            .trim_end_matches(['.', ':'])
            .to_owned()
    };
    let words: Vec<&str> = from.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let w = trim(w);
        let Some((file, line)) = w.rsplit_once(':') else {
            continue;
        };
        let file = file.rsplit('/').next().unwrap_or(file);
        let ext = |(a, b): (&str, &str)| !a.is_empty() && !b.is_empty();
        let named = file.rsplit_once('.').is_some_and(ext);
        let digits = !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit());
        if !named || !digits {
            continue;
        }
        let Some(field) = words.get(i + 1).map(|f| trim(f)) else {
            continue;
        };
        if field.chars().any(char::is_alphabetic) && field.contains(['.', '[']) {
            out.push((file.to_owned(), field));
        }
    }
    out
}

/// An open row (not done, not out of scope) other than `row` whose from or
/// note names one of the failures: the file and the field.
fn same_failure(q: &[Value], row: &str, fails: &[(String, String)]) -> Option<usize> {
    if fails.is_empty() {
        return None;
    }
    q.iter().position(|r| {
        let closed = r["status"] == json!("done") || r["scope"] == json!("out");
        if closed || r["mechanic"] == json!(row) {
            return false;
        }
        let from = r["from"].as_str().unwrap_or("");
        let said = format!("{from} {}", r["note"].as_str().unwrap_or(""));
        let names = |s: &String| said.contains(s.as_str());
        fails
            .iter()
            .any(|(file, field)| names(file) && names(field))
    })
}

/// The scoreboard line of the row's target, lower case, or None.
fn target_line(r: &Value) -> Option<String> {
    r["target"]
        .as_str()
        .and_then(|t| crate::scoreboard::parse_target(t).ok())
        .map(|t| t.line)
}

/// The row a gap on `line` is when `row` measures another line: ROW-LINE,
/// at most 64 bytes. None for a row that is that already.
fn line_row(row: &str, line: &str) -> Option<String> {
    let tail = slug(line, 24);
    if tail.is_empty() || row.ends_with(&format!("-{tail}")) {
        return None;
    }
    let mut head = row.to_owned();
    head.truncate(64 - 1 - tail.len());
    Some(format!("{}-{tail}", head.trim_end_matches('-')))
}

/// What the note says of the proposal's target: what it is and how many
/// frames above where the line reads now, so the row isn't closed by a fix
/// that lifts the line by a frame. Empty with no target.
fn target_words(p: &Proposal) -> String {
    let Some(t) = p.target.map(str::trim) else {
        return String::new();
    };
    match p.target_frames {
        Some((n, gap)) if n < gap => format!(
            "Its target, {t}, is {n} frames above where the line reads now: the {gap} frames the gap covers on that layer, capped at the line's total."
        ),
        Some((n, _)) => format!(
            "Its target, {t}, is {n} frames above where the line reads now, the frames the gap covers on that layer."
        ),
        None => format!("Its target is {t}."),
    }
}

/// A done row seen in new clips (`list`) takes the new gap: next again,
/// the proposal's words as its note, the clip and frames that show it,
/// and the new target (or what would measure it) in place of the one it
/// met, which would close it again at once. Returns what it was, for its
/// log.
fn reopen(r: &mut Value, p: &Proposal, list: &str) -> String {
    let said = |k: &str| {
        r[k].as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let mut was = vec![format!(
        "note: {}",
        said("note").unwrap_or_else(|| "none".into())
    )];
    if let Some(t) = said("target") {
        was.push(format!("target: {t}"));
    }
    if let Some(w) = said("measure_needed") {
        was.push(format!("what would measure it: {w}"));
    }
    if let Some(c) = r.get("clip") {
        was.push(format!("clip: {}", super::clip_words(c)));
    }
    let (what, from) = (p.what.trim(), p.from.trim());
    let before = format!(
        "Reopened by the loop for {list} (from {from}). It was done; before, its {}.",
        was.join("; ")
    );
    let mut note = format!(
        "{MARK} {what} (from {from}). It was done and is open again for {list}; what it was before is in its log."
    );
    let tw = target_words(p);
    if !tw.is_empty() {
        note = format!("{note} {tw}");
    }
    if let Some(o) = r.as_object_mut() {
        for k in [
            "target",
            "awaiting_measure",
            "measure_needed",
            "clip",
            "line",
            "worker",
            "waits_on",
            "waits_since",
        ] {
            o.remove(k);
        }
        o.insert("status".into(), json!("next"));
        o.insert("note".into(), json!(note));
        match p.target {
            Some(t) => o.insert("target".into(), json!(t.trim())),
            None => o.insert(
                "measure_needed".into(),
                json!(format!("gp picture no longer finds it on {list}: {what}")),
            ),
        };
        if let Some(l) = line_of(p) {
            o.insert("line".into(), json!(l));
        }
        if let Some(c) = shows(p) {
            o.insert("clip".into(), c);
        }
    }
    before
}

/// The scoreboard line the gap was found on, for the row's `line`: what
/// the row claims in the loop's plan when it has no target
/// (crate::steps::claim_of).
fn line_of<'a>(p: &Proposal<'a>) -> Option<&'a str> {
    p.line.map(str::trim).filter(|l| !l.is_empty())
}

/// The row's clip for the proposal's `shows`: a corpus clip and frames.
fn shows(p: &Proposal) -> Option<Value> {
    p.shows
        .map(|(id, a, b)| json!({"id": id, "from": a, "to": b.max(a)}))
}

/// Keeps what a reopened row was in its log (`gp mechanic log`'s) and makes
/// the new note its session's words, so its worker reads the new gap (a
/// row with no session yet gets one, made from the new note).
fn log_reopen(paths: &Paths, game: &str, row: &str, note: &str, before: &str) -> Result<()> {
    let s = super::session::Session::open_or_make(paths, game, row)?;
    s.set_prompt(note)?;
    super::watch::log(&s.dir, before)?;
    Ok(())
}

/// The step ids a text names: `s` and 12 hex digits, as a word.
fn steps_named(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| {
            w.len() == 13
                && w.starts_with('s')
                && w[1..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .collect()
}

/// The repo the runner gave a step (`repo` of its step in its worker's
/// record, gp_mcp::steps::workers_dir), or None for a step it has no
/// record of.
fn step_repo(paths: &Paths, id: &str) -> Option<String> {
    let f = gp_mcp::steps::workers_dir(&paths.home).join(format!("{id}.json"));
    let v: Value = serde_json::from_slice(&fs::read(f).ok()?).ok()?;
    v["step"]["repo"].as_str().map(str::to_owned)
}

/// The repo a new row is filed with, and why when it wasn't given: tools
/// when the step it is proposed in (`step`) or a step its from names is a
/// step of gameprinter's own work, or when its words name gp's own code (a
/// path under crates/) or the tool gate; else games. On 2026-11-26 a row a
/// tools row's worker proposed about gp's loop health was filed as games,
/// so its worker got a games worktree and a step that can't run gp tool
/// gate: its patch passed its dry run and no card could be raised
/// (docs/loop.md).
pub(crate) fn repo_of(paths: &Paths, p: &Proposal) -> (&'static str, Option<String>) {
    match p.repo {
        Some("tools") => return ("tools", None),
        Some(_) => return ("games", None),
        None => {}
    }
    let mut ids: Vec<&str> = p.step.into_iter().collect();
    ids.extend(steps_named(p.from));
    if let Some(id) = ids
        .into_iter()
        .find(|id| step_repo(paths, id).as_deref() == Some("tools"))
    {
        return (
            "tools",
            Some(format!(
                "proposed from step {id}, a step of gameprinter's own work"
            )),
        );
    }
    let words = format!("{} {}", p.what, p.from).to_lowercase();
    if words.contains("crates/") || words.contains("tool gate") {
        return ("tools", Some("its words name gp's own code".into()));
    }
    ("games", None)
}

/// A triage.toml item's test and request (mechanic/triage.rs has the
/// format).
#[derive(Default, Clone, Copy)]
pub(crate) struct Test<'a> {
    pub goal: &'a str,
    pub near: Option<&'a str>,
    pub from: Option<&'a str>,
    pub place: Option<&'a str>,
    pub todo: Option<&'a str>,
    pub stop: Option<&'a str>,
}

/// A row name from words: lower case, a dash between words, at most `max`
/// bytes, cut at a dash.
pub(crate) fn slug(words: &str, max: usize) -> String {
    let mut s = String::new();
    for w in words
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !s.is_empty() && s.len() + 1 + w.len() > max {
            break;
        }
        if !s.is_empty() {
            s.push('-');
        }
        s.push_str(w);
    }
    s.truncate(max);
    s
}

/// Adds the row unless the quest log has it (any status: a row the person
/// finished or put out of scope stays so, unless seen in a new clip), and
/// its test to the home's triage.toml for the game (triage::home_file)
/// unless an item there or in `game_dir`'s names the row. A
/// failure an open row has already (a `FILE:LINE FIELD` in from whose file
/// and field that row's from or note names) is that row's: the step is
/// added to its note, and the test goes under its name. A done row seen in
/// a new clip is reopened with the new gap as its note, clip and target,
/// what it was kept in its log; a row whose target is on another
/// scoreboard line than the gap's leaves the gap to a row of its own. The
/// test is checked first, so a test the triage can't read writes nothing.
/// Says what it did.
pub(crate) fn propose(paths: &Paths, game_dir: Option<&Path>, p: &Proposal) -> Result<Value> {
    if !name_ok(p.row) {
        bail!(
            "a mechanic name is lower-case letters, digits and dashes: {}",
            p.row
        );
    }
    if p.from.trim().is_empty() || p.what.trim().is_empty() {
        bail!("a proposed row says what it is (--what) and where it came from (--from)");
    }
    let parked = p.parked.map(str::trim);
    if parked.is_some_and(str::is_empty) {
        bail!("a parked row says why it waits (--parked WHY)");
    }
    let f = queue_file(paths, p.game)?;
    // The log's read, change and write under its lock; a proposal sent on
    // to a row of its own (the line case) and the reopened row's log come
    // after, with the lock let go.
    let s = match update_queue(&f, |q| settle(paths, game_dir, p, parked, q))? {
        Settle::Instead(name) => {
            let mut v = propose(paths, game_dir, &Proposal { row: &name, ..*p })?;
            v["instead_of"] = json!(p.row);
            return Ok(v);
        }
        Settle::Done(s) => s,
    };
    let Settled {
        row,
        same,
        had_test,
        tests,
        had_row,
        new_clips,
        reopened,
        before,
        new_repo,
        repo_why,
        note,
    } = s;
    if let (Some(b), Some(note)) = (&before, &note) {
        log_reopen(paths, p.game, &row, note, b)?;
    }
    let mut wrote_test = None;
    if let (Some(t), false) = (&p.test, had_test) {
        // The home's file, not the games checkout's: a change there lands
        // only through a gate (triage::home_file).
        let file = tests;
        if let Some(d) = file.parent() {
            fs::create_dir_all(d)?;
        }
        let mut text = fs::read_to_string(&file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&item_toml(&row, t));
        fs::write(&file, text).with_context(|| file.display().to_string())?;
        wrote_test = Some(file.display().to_string());
    }
    let gave_target = reopened || !had_row;
    Ok(json!({
        "row": row,
        "proposed_as": p.row,
        "same_failure": same,
        "added": !had_row,
        "parked": !had_row && parked.is_some(),
        "clips_added": new_clips,
        "reopened": reopened,
        "before": before,
        "target": p.target.filter(|_| gave_target),
        "target_frames": p.target_frames.filter(|_| gave_target && p.target.is_some()).map(|(n, _)| n),
        "test": wrote_test,
        "test_was_there": had_test,
        "repo": new_repo,
        "repo_why": repo_why,
    }))
}

/// What [`settle`] did to the quest log, for [`propose`] to finish with.
enum Settle<'a> {
    /// The proposal is a row of its own by this name: nothing was changed.
    Instead(String),
    Done(Settled<'a>),
}

struct Settled<'a> {
    row: String,
    same: bool,
    had_test: bool,
    tests: std::path::PathBuf,
    had_row: bool,
    new_clips: Vec<&'a String>,
    reopened: bool,
    before: Option<String>,
    new_repo: Option<&'static str>,
    repo_why: Option<String>,
    /// The row's note after a reopen, for its log.
    note: Option<String>,
}

/// [`propose`]'s change to the quest log's rows `q`, under its lock.
fn settle<'a>(
    paths: &Paths,
    game_dir: Option<&Path>,
    p: &'a Proposal<'a>,
    parked: Option<&str>,
    q: &mut Vec<Value>,
) -> Result<Settle<'a>> {
    let mut at = q.iter().position(|r| r["mechanic"] == json!(p.row));
    let is_it = |r: &Value| r["mechanic"] == json!(p.row);
    // An archived row is still the row: proposed again it stays archived,
    // as a done row stays done, unless it was seen in a new clip, when it
    // comes back to the quest log to be reopened below.
    let in_archive = match at {
        None => super::archive::read_archive(&paths.home, p.game)
            .into_iter()
            .find(is_it),
        Some(_) => None,
    };
    // A gap seen in a new clip on another scoreboard line than the row's
    // target: that target can't measure it, so it is a row of its own.
    if let Some(line) = p.line.map(str::trim).filter(|l| !l.is_empty())
        && let Some(r) = at.map(|i| &q[i]).or(in_archive.as_ref())
        && p.clips.iter().any(|c| !names_clip(r, c))
        && target_line(r).is_some_and(|t| t != line.to_lowercase())
        && let Some(name) = line_row(p.row, line)
    {
        return Ok(Settle::Instead(name));
    }
    // A failure an open row has already (the same file and field) is that
    // row's: the step goes in its note instead of a second row and card.
    let same = match (at, &in_archive) {
        (None, None) => same_failure(q, p.row, &failures(p.from)),
        _ => None,
    };
    if same.is_some() {
        at = same;
    }
    let row = at
        .and_then(|i| q[i]["mechanic"].as_str())
        .unwrap_or(p.row)
        .to_owned();
    let mut had_test = false;
    let tests = super::triage::home_file(paths, p.game)?;
    if let Some(t) = &p.test {
        let dir = game_dir.context("a row's test is read with the game's names (probes.toml, triage.toml): no games checkout found (pass --games or set GP_GAMES_DIR)")?;
        had_test = super::triage::check_test(dir, &tests, &row, t.goal)?;
    }
    let archived = in_archive.is_some();
    if let Some(r) = &in_archive
        && p.clips.iter().any(|c| !names_clip(r, c))
        && let Some(r) = super::archive::bring_back(&paths.home, p.game, is_it)?
    {
        q.push(r);
        at = Some(q.len() - 1);
    }
    let had_row = at.is_some() || archived;
    let mut new_clips: Vec<&String> = Vec::new();
    let mut reopened = false;
    let mut before = None;
    // The new row's repo, and why when it wasn't given (repo_of).
    let (mut new_repo, mut repo_why): (Option<&str>, Option<String>) = (None, None);
    if let Some(i) = same {
        let r = &mut q[i];
        let from = p.from.trim();
        let note = r["note"].as_str().unwrap_or("").trim().to_owned();
        if !note.contains(from) && r["from"].as_str().map(str::trim) != Some(from) {
            let also = format!("Also found as {} (from {from}).", p.row);
            r["note"] = json!(if note.is_empty() {
                also
            } else {
                format!("{note} {also}")
            });
        }
    }
    match at {
        _ if same.is_some() => {}
        None if archived => {}
        None => {
            let mut proposed = format!("{MARK} {} (from {})", p.what.trim(), p.from.trim());
            let tw = target_words(p);
            if !tw.is_empty() {
                proposed = format!("{proposed}. {tw}");
            }
            let (status, note) = match parked {
                Some(why) => (
                    "in line",
                    format!("Parked: {}. {proposed}", why.trim_end_matches('.')),
                ),
                None => ("next", proposed),
            };
            let mut r = json!({
                "mechanic": p.row,
                "status": status,
                "note": note,
                "by": "loop",
                "from": p.from.trim(),
            });
            let (repo, why) = repo_of(paths, p);
            if repo == "tools" {
                r["repo"] = json!("tools");
            }
            (new_repo, repo_why) = (Some(repo), why);
            if let Some(s) = p.plain.map(str::trim).filter(|s| !s.is_empty()) {
                r["plain"] = json!(s);
            }
            if !p.clips.is_empty() {
                r["seen_in"] = json!(p.clips);
            }
            if let Some(t) = p.target {
                r["target"] = json!(t.trim());
            }
            if let Some(l) = line_of(p) {
                r["line"] = json!(l);
            }
            if let Some(c) = shows(p) {
                r["clip"] = c;
            }
            q.push(r);
        }
        Some(i) => {
            let r = &mut q[i];
            new_clips = p.clips.iter().filter(|c| !names_clip(r, c)).collect();
            if !new_clips.is_empty() {
                let mut seen: Vec<Value> = r["seen_in"].as_array().cloned().unwrap_or_default();
                seen.extend(new_clips.iter().map(|c| json!(c)));
                r["seen_in"] = Value::Array(seen);
                let list = new_clips
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if r["status"] == json!("done") && r["scope"] != json!("out") {
                    before = Some(reopen(r, p, &list));
                    reopened = true;
                } else {
                    let note = r["note"].as_str().unwrap_or("").trim().to_owned();
                    let also = format!("Also seen in {list} (from {}).", p.from.trim());
                    r["note"] = json!(if note.is_empty() {
                        also
                    } else {
                        format!("{note} {also}")
                    });
                }
            }
        }
    }
    let note = match (&before, at) {
        (Some(_), Some(i)) => Some(q[i]["note"].as_str().unwrap_or("").to_owned()),
        _ => None,
    };
    Ok(Settle::Done(Settled {
        row,
        same: same.is_some(),
        had_test,
        tests,
        had_row,
        new_clips,
        reopened,
        before,
        new_repo,
        repo_why,
        note,
    }))
}

/// One `[[item]]` of triage.toml, its strings quoted as TOML quotes them.
fn item_toml(row: &str, t: &Test) -> String {
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let mut s = format!("[[item]]\nrow = {}\ngoal = {}\n", q(row), q(t.goal));
    for (k, v) in [
        ("near", t.near),
        ("from", t.from),
        ("where", t.place),
        ("do", t.todo),
        ("stop", t.stop),
    ] {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            s.push_str(&format!("{k} = {}\n", q(v)));
        }
    }
    s
}

/// The line `gp mechanic propose` prints.
pub(crate) fn words(v: &Value) -> String {
    let row = v["row"].as_str().unwrap_or("");
    let added: Vec<&str> = v["clips_added"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let target = match (v["target"].as_str(), v["target_frames"].as_u64()) {
        (Some(t), Some(n)) => format!(", its target {t} ({n} frames above now)"),
        (Some(t), None) => format!(", its target {t}"),
        _ => String::new(),
    };
    let mut s = if v["same_failure"] == json!(true) {
        format!(
            "{}: the quest log has the same failure open in {row}; the step is in its note, no new row.",
            v["proposed_as"].as_str().unwrap_or(row)
        )
    } else if v["parked"] == json!(true) {
        format!("{row}: added to the quest log in line (parked), proposed by the loop.")
    } else if v["added"] == json!(true) {
        format!("{row}: added to the quest log as next, proposed by the loop{target}.")
    } else if v["reopened"] == json!(true) {
        format!(
            "{row}: it was done; it is next again for {}, the new gap its note{target}; what it was before is in its log.",
            added.join(", ")
        )
    } else if !added.is_empty() {
        format!(
            "{row}: the quest log has it; {} added to its note.",
            added.join(", ")
        )
    } else {
        format!("{row}: the quest log has it already; left as it is.")
    };
    if let Some(was) = v["instead_of"].as_str() {
        s = format!(
            "{was}'s target is on another scoreboard line, so the gap is a row of its own. {s}"
        );
    }
    if let Some(why) = v["repo_why"].as_str() {
        s.push_str(&format!(
            " Its work is in gameprinter (repo tools): {why}; --repo games files it as the game's."
        ));
    }
    if let Some(f) = v["test"].as_str() {
        s.push_str(&format!(" Its test is in {f}."));
    } else if v["test_was_there"] == json!(true) {
        s.push_str(" triage.toml has its test already.");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_paths as paths;

    #[test]
    fn a_slug_is_a_row_name() {
        assert_eq!(
            slug("Placed elsewhere: hero: 1 px further right", 36),
            "placed-elsewhere-hero-1-px-further"
        );
        assert!(name_ok(&slug("original only: tiles 2a 3b", 40)));
    }

    /// A proposed row is next with the mark and where it came from; its
    /// test lands once in the home's triage.toml for the game, the games
    /// checkout's left as it was; a second proposal of the row changes
    /// nothing, and a row the person finished stays done.
    #[test]
    fn a_proposed_row_is_next_with_its_test_in_the_home() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let game = d.path().join("games/made-up");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("triage.toml"), "[watch]\nlives = 0x10\n").unwrap();
        let p = paths(&home);
        let prop = Proposal {
            game: "made-up",
            row: "lose-a-life",
            from: "a worker on get-ready-screen",
            what: "no clip shows a life lost",
            test: Some(Test {
                goal: "lives == 2",
                todo: Some("Lose a life"),
                stop: Some("When the next life starts"),
                ..Test::default()
            }),
            ..Proposal::default()
        };
        let v = propose(&p, Some(&game), &prop).unwrap();
        assert_eq!(v["added"], json!(true));
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!(q[0]["status"], "next");
        assert_eq!(
            q[0]["note"],
            "Proposed by the loop: no clip shows a life lost (from a worker on get-ready-screen)"
        );
        assert_eq!(
            fs::read_to_string(game.join("triage.toml")).unwrap(),
            "[watch]\nlives = 0x10\n",
            "the games checkout's triage.toml is left as it was"
        );
        let home_tests = super::super::triage::home_file(&p, "made-up").unwrap();
        assert_eq!(v["test"], json!(home_tests.display().to_string()));
        let t = fs::read_to_string(&home_tests).unwrap();
        assert!(
            t.starts_with(
                "[[item]]\nrow = \"lose-a-life\"\ngoal = \"lives == 2\"\ndo = \"Lose a life\""
            ),
            "{t}"
        );
        assert!(
            super::super::triage::check_test(&game, &home_tests, "lose-a-life", "lives == 2")
                .unwrap()
        );
        assert_eq!(
            super::super::triage::item_rows(&game, &home_tests),
            ["lose-a-life"]
        );

        let again = propose(&p, Some(&game), &prop).unwrap();
        assert_eq!(again["added"], json!(false));
        assert_eq!(again["test"], json!(null));
        assert_eq!(
            fs::read_to_string(&home_tests).unwrap(),
            t,
            "the test is written once"
        );

        let mut q = read_queue(&queue_file(&p, "made-up").unwrap());
        q[0]["status"] = json!("done");
        write_queue(&queue_file(&p, "made-up").unwrap(), &q).unwrap();
        propose(&p, Some(&game), &prop).unwrap();
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!((q.len(), q[0]["status"].as_str()), (1, Some("done")));
    }

    /// A gap proposed from another clip is added to the row's `seen_in`,
    /// and a done row goes back to next with the new gap as its note; a
    /// clip the row names (in `seen_in` or, for an older row, its from)
    /// changes nothing.
    #[test]
    fn a_new_clip_is_added_to_its_row() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "rooftop").unwrap();
        let l1 = vec!["rooftop.l1.001".to_owned()];
        let first = Proposal {
            game: "rooftop",
            row: "picture-other-the-level-picture-main",
            from: "gp picture --run r, rooftop.l1.001 frame 0",
            what: "the picture differs",
            clips: &l1,
            ..Proposal::default()
        };
        propose(&p, None, &first).unwrap();
        assert_eq!(read_queue(&f)[0]["seen_in"], json!(["rooftop.l1.001"]));
        let mut q = read_queue(&f);
        q[0]["status"] = json!("done");
        q[0]["note"] = json!("Landed: level 1's start");
        write_queue(&f, &q).unwrap();

        let both = vec!["rooftop.demo.001".to_owned(), "rooftop.l1.001".to_owned()];
        let v = propose(
            &p,
            None,
            &Proposal {
                from: "gp picture --run r, rooftop.demo.001 frame 2",
                clips: &both,
                test: None,
                ..first
            },
        )
        .unwrap();
        assert_eq!(v["clips_added"], json!(["rooftop.demo.001"]));
        assert!(
            words(&v).contains("next again for rooftop.demo.001"),
            "{}",
            words(&v)
        );
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["status"], "next");
        assert_eq!(
            q[0]["seen_in"],
            json!(["rooftop.l1.001", "rooftop.demo.001"])
        );
        assert!(
            q[0]["note"]
                .as_str()
                .unwrap()
                .starts_with("Proposed by the loop: the picture differs (from gp picture --run r, rooftop.demo.001 frame 2)."),
            "{}",
            q[0]["note"]
        );

        // An open row seen in another clip has it added to its note.
        let l3 = vec!["rooftop.l3.001".to_owned()];
        propose(
            &p,
            None,
            &Proposal {
                clips: &l3,
                ..first
            },
        )
        .unwrap();
        let note = read_queue(&f)[0]["note"].as_str().unwrap().to_owned();
        assert!(
            note.ends_with(
                "Also seen in rooftop.l3.001 (from gp picture --run r, rooftop.l1.001 frame 0)."
            ),
            "{note}"
        );

        let again = propose(
            &p,
            None,
            &Proposal {
                clips: &both,
                test: None,
                ..first
            },
        )
        .unwrap();
        assert_eq!(again["clips_added"], json!([]));

        // A row from before seen_in names its clip in from.
        let mut q = read_queue(&f);
        q[0].as_object_mut().unwrap().remove("seen_in");
        q[0]["status"] = json!("done");
        write_queue(&f, &q).unwrap();
        propose(&p, None, &first).unwrap();
        assert_eq!(read_queue(&f)[0]["status"], "done");

        // Archived, the row stays archived when proposed again from a clip
        // it names, and comes back reopened from a new one: never made anew.
        let a = super::super::archive::archive_file(&p.home, "rooftop");
        write_queue(&a, &read_queue(&f)).unwrap();
        write_queue(&f, &[]).unwrap();
        let v = propose(&p, None, &first).unwrap();
        assert_eq!(v["added"], json!(false));
        assert!(read_queue(&f).is_empty());
        let l2 = vec!["rooftop.l2.001".to_owned()];
        propose(
            &p,
            None,
            &Proposal {
                clips: &l2,
                ..first
            },
        )
        .unwrap();
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["status"], "next");
        assert!(
            q[0]["note"]
                .as_str()
                .unwrap()
                .contains("open again for rooftop.l2.001")
        );
        assert!(read_queue(&a).is_empty());
    }

    /// The owner, 2026-11-22: a done row reopened for a new clip kept its
    /// old target, clip and note, so its worker closed it against the old
    /// target or fixed the old clip. Reopened, it takes the new gap's clip,
    /// frames and target, and what it was goes in its log and its session's
    /// words become the new note. The target is the gap's frames above
    /// where the line reads now, never one frame past it (a row for a
    /// 200-frame gap closed when another fix lifted the line by a frame),
    /// and the note says how many frames that is.
    #[test]
    fn a_reopened_row_takes_the_new_gap() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "hilltop").unwrap();
        write_queue(
            &f,
            &[json!({
                "mechanic": "picture-bonus-round",
                "status": "done",
                "note": "Met its target: bonus round sprites at least 100 of 336",
                "target": "bonus round sprites at least 100 of 336",
                "clip": {"id": "hilltop.demo.001", "from": 1340, "to": 1340},
                "seen_in": ["hilltop.demo.001"],
            })],
        )
        .unwrap();
        // The row has a session from its first time, with its old words.
        super::super::session::Session::create(
            &p,
            "hilltop",
            "picture-bonus-round",
            Some("the old gap"),
            "",
        )
        .unwrap();
        let clips = vec!["hilltop.bonus.001".to_owned()];
        let prop = Proposal {
            game: "hilltop",
            row: "picture-bonus-round",
            from: "scoreboard line bonus round: gp picture --run r, hilltop.bonus.001 frame 10",
            what: "the picture differs, placed elsewhere: mug (1 clips, 200 frames)",
            clips: &clips,
            line: Some("bonus round"),
            shows: Some(("hilltop.bonus.001", 10, 236)),
            // The line reads 131 now; the gap covers 200 frames.
            target: Some("bonus round sprites at least 331 of 336"),
            target_frames: Some((200, 200)),
            ..Proposal::default()
        };
        let v = propose(&p, None, &prop).unwrap();
        assert_eq!(v["reopened"], json!(true));
        assert!(
            words(&v).contains(
                "its target bonus round sprites at least 331 of 336 (200 frames above now)"
            ),
            "{}",
            words(&v)
        );
        let q = read_queue(&f);
        let r = &q[0];
        assert_eq!(r["status"], "next");
        assert_eq!(r["target"], "bonus round sprites at least 331 of 336");
        assert_eq!(
            r["clip"],
            json!({"id": "hilltop.bonus.001", "from": 10, "to": 236})
        );
        let note = r["note"].as_str().unwrap();
        assert!(note.contains("placed elsewhere: mug"), "{note}");
        assert!(
            note.ends_with(
                "Its target, bonus round sprites at least 331 of 336, is 200 frames above where the line reads now, the frames the gap covers on that layer."
            ),
            "{note}"
        );
        assert!(!note.contains("Met its target"), "{note}");

        let s = super::super::session::Session::open(&p, "hilltop", "picture-bonus-round").unwrap();
        assert_eq!(s.state().unwrap()["prompt"], json!(note));
        let log = fs::read_to_string(s.dir.join("watch/log.ndjson")).unwrap();
        for old in [
            "Met its target: bonus round sprites at least 100 of 336",
            "target: bonus round sprites at least 100 of 336",
            "clip: corpus clip hilltop.demo.001, frame 1340",
        ] {
            assert!(log.contains(old), "{old} not in {log}");
        }

        // A target capped at the line's total says so.
        let mut q = read_queue(&f);
        q[0]["status"] = json!("done");
        write_queue(&f, &q).unwrap();
        let other = vec!["hilltop.bonus.002".to_owned()];
        propose(
            &p,
            None,
            &Proposal {
                clips: &other,
                target: Some("bonus round sprites at least 336 of 336"),
                target_frames: Some((36, 200)),
                ..prop
            },
        )
        .unwrap();
        let note = read_queue(&f)[0]["note"].as_str().unwrap().to_owned();
        assert!(
            note.ends_with(
                "is 36 frames above where the line reads now: the 200 frames the gap covers on that layer, capped at the line's total."
            ),
            "{note}"
        );

        // With no target to give, it says what would measure it instead of
        // keeping the one it met; and a row with no session gets one.
        let mut q = read_queue(&f);
        q.push(json!({"mechanic": "picture-hud", "status": "done", "note": "Landed",
                      "target": "bonus round sprites at least 100 of 336", "seen_in": ["hilltop.demo.001"]}));
        write_queue(&f, &q).unwrap();
        let v = propose(
            &p,
            None,
            &Proposal {
                row: "picture-hud",
                target: None,
                target_frames: None,
                shows: None,
                ..prop
            },
        )
        .unwrap();
        assert_eq!(v["reopened"], json!(true));
        let r = read_queue(&f)[1].clone();
        assert!(r.get("target").is_none(), "{r}");
        assert!(
            r["measure_needed"]
                .as_str()
                .unwrap()
                .contains("hilltop.bonus.001")
        );
        assert!(super::super::session::Session::exists(&p, "hilltop", "picture-hud").unwrap());
    }

    /// A gap on another scoreboard line than the row's target is a row of
    /// its own, ROW-LINE, with the gap's target; the row is left as it is.
    #[test]
    fn a_gap_on_another_line_is_a_row_of_its_own() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "hilltop").unwrap();
        let old = json!({
            "mechanic": "picture-mug",
            "status": "done",
            "note": "Met its target",
            "target": "level 1 sprites at least 300 of 336",
            "seen_in": ["hilltop.l1.001"],
        });
        write_queue(&f, std::slice::from_ref(&old)).unwrap();
        let clips = vec!["hilltop.demo.001".to_owned()];
        let prop = Proposal {
            game: "hilltop",
            row: "picture-mug",
            from: "scoreboard line the demo: gp picture --run r, hilltop.demo.001 frame 1340",
            what: "the picture differs: mug (1 clips, 21 frames)",
            clips: &clips,
            line: Some("the demo"),
            shows: Some(("hilltop.demo.001", 1340, 1360)),
            target: Some("the demo sprites at least 921 of 1800"),
            target_frames: Some((21, 21)),
            ..Proposal::default()
        };
        let v = propose(&p, None, &prop).unwrap();
        assert_eq!(v["row"], "picture-mug-the-demo");
        assert_eq!(v["added"], json!(true));
        assert!(
            words(&v).starts_with("picture-mug's target is on another scoreboard line"),
            "{}",
            words(&v)
        );
        let q = read_queue(&f);
        assert_eq!(q.len(), 2);
        let mut kept = q[0].clone();
        kept.as_object_mut().unwrap().remove("changed_ms");
        assert_eq!(kept, old, "the row on the other line is left as it is");
        assert_eq!(q[1]["mechanic"], "picture-mug-the-demo");
        assert_eq!(q[1]["status"], "next");
        assert_eq!(q[1]["target"], "the demo sprites at least 921 of 1800");
        assert_eq!(q[1]["clip"]["id"], "hilltop.demo.001");
        let note = q[1]["note"].as_str().unwrap();
        assert!(
            note.contains("is 21 frames above where the line reads now"),
            "{note}"
        );
        // Proposed again, it is the new row's, and never ROW-LINE-LINE.
        let again = propose(&p, None, &prop).unwrap();
        assert_eq!(again["row"], "picture-mug-the-demo");
        assert_eq!(read_queue(&f).len(), 2);
        assert_eq!(line_row("picture-mug-the-demo", "the demo"), None);
        assert!(line_row(&"x".repeat(64), "the demo").is_some_and(|n| name_ok(&n)));
    }

    /// A failure an open row has already (the same file and field in its
    /// from or note) goes in that row's note under another name: one row,
    /// once. A done row with the same failure doesn't take it.
    #[test]
    fn the_same_failure_goes_to_its_open_row() {
        let hit = failures("step s1: hook FAIL approved.toml:14 look[0].apart");
        let want = ("approved.toml".to_owned(), "look[0].apart".to_owned());
        assert_eq!(hit, vec![want]);
        assert!(failures("gp picture --run r, hilltop.l1.001 frame 0").is_empty());

        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "hilltop").unwrap();
        write_queue(
            &f,
            &[
                json!({"mechanic": "old-hit", "status": "done",
                       "from": "step a: FAIL (hook) at approved.toml:14 look[0].apart"}),
                json!({"mechanic": "approved-file-passes-scan", "status": "in line", "repo": "tools",
                       "from": "the scan reads approved.toml's apart lists as ROM bytes (look[0].apart lines 14-18)"}),
            ],
        )
        .unwrap();
        let prop = Proposal {
            game: "hilltop",
            row: "approved-lock-trips-package-scan",
            from: "step s22475c59b953: gp model gate hilltop dry run, hook FAIL approved.toml:14 look[0].apart",
            what: "the package scan refuses approved.toml",
            repo: Some("tools"),
            ..Proposal::default()
        };
        let v = propose(&p, None, &prop).unwrap();
        assert_eq!(v["added"], json!(false));
        assert_eq!(v["same_failure"], json!(true));
        assert_eq!(v["row"], "approved-file-passes-scan");
        assert!(words(&v).contains("same failure open in approved-file-passes-scan"));
        let q = read_queue(&f);
        assert_eq!(q.len(), 2);
        assert_eq!(q[1]["status"], "in line");
        let note = q[1]["note"].as_str().unwrap().to_owned();
        let also = "Also found as approved-lock-trips-package-scan (from step s22475c59b953";
        assert!(note.contains(also), "{note}");
        propose(&p, None, &prop).unwrap();
        let again = read_queue(&f);
        assert_eq!(again[1]["note"], json!(note), "the step is added once");

        // With the open row done, the failure is a new row.
        let mut q = read_queue(&f);
        q[1]["status"] = json!("done");
        write_queue(&f, &q).unwrap();
        assert_eq!(propose(&p, None, &prop).unwrap()["added"], json!(true));
    }

    /// A row proposed parked is added in line with why before the
    /// proposal's words, in the one call; a row the quest log has stays as
    /// it is, and a park with no reason is refused.
    #[test]
    fn a_row_proposed_parked_is_in_line_in_one_call() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let f = queue_file(&p, "rooftop").unwrap();
        let prop = Proposal {
            game: "rooftop",
            row: "onboarding-later",
            from: "a smoke test",
            what: "the hint names the wrong genre",
            repo: Some("tools"),
            parked: Some("not a blocker for the first model step."),
            ..Proposal::default()
        };
        let v = propose(&p, None, &prop).unwrap();
        assert_eq!(
            (v["added"].clone(), v["parked"].clone()),
            (json!(true), json!(true))
        );
        assert!(words(&v).contains("in line (parked)"), "{}", words(&v));
        let q = read_queue(&f);
        assert_eq!(q[0]["status"], "in line");
        assert_eq!(
            q[0]["note"],
            "Parked: not a blocker for the first model step. Proposed by the loop: the hint names the wrong genre (from a smoke test)"
        );
        assert_eq!(q[0]["repo"], "tools");

        let mut q = read_queue(&f);
        q[0]["status"] = json!("next");
        write_queue(&f, &q).unwrap();
        let again = propose(&p, None, &prop).unwrap();
        assert_eq!(
            (again["added"].clone(), again["parked"].clone()),
            (json!(false), json!(false))
        );
        assert_eq!(
            read_queue(&f)[0]["status"],
            "next",
            "a row it has is left as it is"
        );

        let blank = Proposal {
            row: "other-row",
            parked: Some("  "),
            ..prop
        };
        assert!(propose(&p, None, &blank).is_err());
        assert_eq!(read_queue(&f).len(), 1);
    }

    /// A row proposed in a tools step (GP_STEP), or from one its from
    /// names, or whose words name gp's code, is a tools row unless --repo
    /// says games; a row proposed in a game's step is the game's.
    #[test]
    fn a_row_proposed_from_a_tools_step_is_a_tools_row() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(&d.path().join("home"));
        let workers = gp_mcp::steps::workers_dir(&p.home);
        fs::create_dir_all(&workers).unwrap();
        for (id, repo) in [("s49db893760f8", "tools"), ("s53bf298c837a", "games")] {
            let rec = json!({"step": {"id": id, "repo": repo}});
            fs::write(workers.join(format!("{id}.json")), rec.to_string()).unwrap();
        }
        let f = queue_file(&p, "rooftop").unwrap();
        let base = Proposal {
            game: "rooftop",
            row: "in-a-tools-step",
            from: "a worker",
            what: "a trouble row never hears its problem changed",
            step: Some("s49db893760f8"),
            ..Proposal::default()
        };
        let v = propose(&p, None, &base).unwrap();
        assert_eq!(v["repo"], "tools");
        assert!(
            words(&v).contains(
                "Its work is in gameprinter (repo tools): proposed from step s49db893760f8"
            ),
            "{}",
            words(&v)
        );
        let cases = [
            Proposal {
                row: "from-names-a-tools-step",
                from: "rooftop/trouble-x, step s49db893760f8: the fix was still running",
                step: None,
                ..base
            },
            Proposal {
                row: "names-gp-code",
                what: "crates/gp-cli/src/trouble.rs files the same problem twice",
                step: Some("s53bf298c837a"),
                ..base
            },
            Proposal {
                row: "in-a-games-step",
                from: "step s53bf298c837a on rooftop/hop",
                what: "the hop lands a frame late",
                step: Some("s53bf298c837a"),
                ..base
            },
            Proposal {
                row: "said-games",
                repo: Some("games"),
                ..base
            },
            Proposal {
                row: "unknown-step",
                what: "the hop lands a frame late",
                step: Some("s000000000000"),
                ..base
            },
        ];
        for c in &cases {
            propose(&p, None, c).unwrap();
        }
        let q = read_queue(&f);
        let repo = |row: &str| {
            let r = q.iter().find(|r| r["mechanic"] == row).unwrap();
            r["repo"].as_str().unwrap_or("games").to_owned()
        };
        for (row, want) in [
            ("in-a-tools-step", "tools"),
            ("from-names-a-tools-step", "tools"),
            ("names-gp-code", "tools"),
            ("in-a-games-step", "games"),
            ("said-games", "games"),
            ("unknown-step", "games"),
        ] {
            assert_eq!(repo(row), want, "{row}");
        }
        assert_eq!(steps_named("step s49db893760f8: x, s12"), ["s49db893760f8"]);
    }

    /// A test the triage can't read writes nothing: no row, no item.
    #[test]
    fn a_bad_test_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let game = d.path().join("games/made-up");
        fs::create_dir_all(&game).unwrap();
        let p = paths(&home);
        let prop = Proposal {
            game: "made-up",
            row: "lose-a-life",
            from: "a worker",
            what: "no clip shows it",
            test: Some(Test {
                goal: "lives is two",
                ..Test::default()
            }),
            ..Proposal::default()
        };
        assert!(propose(&p, Some(&game), &prop).is_err());
        assert!(read_queue(&queue_file(&p, "made-up").unwrap()).is_empty());
        assert!(!game.join("triage.toml").exists());
        assert!(
            !super::super::triage::home_file(&p, "made-up")
                .unwrap()
                .exists()
        );
    }
}
