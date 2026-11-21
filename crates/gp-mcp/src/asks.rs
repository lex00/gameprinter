//! The person's yes, as a record a click made. When `gp_run` refuses a
//! command for want of the person's yes, it writes an ask here; the page
//! (gp serve's `/api/asks`) shows each open ask as a card with YES and NO
//! and writes the answer; a later `gp_run` of the same command with the
//! same arguments runs only on a recorded yes not yet used, and uses it.
//! The tool `next` lists the open asks in `waits_for_yes`.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/asks/ID.json` | One ask: `{id, command, argv, key, game, needs, says, cost?, plan?, held?, made_ms, state, answered_ms?, used_ms?}`; state is `open`, `yes`, `no`, `running` (the runner runs it; `ran` says how far) or `used`. `plan` is what a call that replaces data would change, from its plan; `held` is a change gp_run's net put back ([`crate::held`]) |
//! | `$GP_HOME/work/serve/page.json` | `{pid, port}`: the gp serve that answers the cards for this home, written when it starts |
//!
//! The yes guards against a careless agent, not a hostile one: a program
//! that can write the home can write a yes (PRODUCT-REVIEW.md, "the yes").

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::Digest as _;

use crate::policy::Ask;

/// How long an open ask waits for an answer before it no longer shows: a
/// week, so a card the loop raised while the person is away waits for
/// them (an hour lost the workers' results).
pub const OPEN_FOR_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// How long a yes holds before it is used.
pub const YES_FOR_MS: u64 = 30 * 60 * 1000;
/// How long a no stops the same command being asked again.
pub const NO_FOR_MS: u64 = 10 * 60 * 1000;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn dir(home: &Path) -> PathBuf {
    home.join("work").join("asks")
}

/// The record of the gp serve that answers this home's cards.
pub fn page_file(home: &Path) -> PathBuf {
    home.join("work").join("serve").join("page.json")
}

/// Whether a process is running.
pub fn alive(pid: u64) -> bool {
    crate::proc::alive(pid)
}

/// Whether a gp serve is running for this home, so the person has a page
/// to answer on.
pub fn page_served(home: &Path) -> bool {
    std::fs::read_to_string(page_file(home))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["pid"].as_u64())
        .is_some_and(alive)
}

/// Written by gp serve when it starts.
pub fn mark_page_served(home: &Path, port: u16) -> std::io::Result<()> {
    let f = page_file(home);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(
        f,
        json!({"pid": std::process::id(), "port": port}).to_string(),
    )
}

/// The key of one exact call: the command and its arguments as gp gets
/// them.
pub fn key(command: &str, argv: &[String]) -> String {
    let mut h = sha2::Sha256::new();
    h.update(command.as_bytes());
    for a in argv {
        h.update([0u8]);
        h.update(a.as_bytes());
    }
    h.finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A game's folder name: letters, digits, `-` and `_`.
pub fn game_ok(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 64
        && g.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn file(home: &Path, id: &str) -> Option<PathBuf> {
    id_ok(id).then(|| dir(home).join(format!("{id}.json")))
}

/// One ask by its id.
pub fn read(home: &Path, id: &str) -> Option<Value> {
    let t = std::fs::read_to_string(file(home, id)?).ok()?;
    serde_json::from_str(&t).ok()
}

fn write(home: &Path, v: &Value) -> Result<(), String> {
    let id = v["id"].as_str().unwrap_or_default();
    let f = file(home, id).ok_or_else(|| format!("{id:?} isn't an ask's id"))?;
    std::fs::create_dir_all(dir(home)).map_err(|e| format!("making the asks' folder: {e}"))?;
    let tmp = f.with_extension("partial");
    std::fs::write(&tmp, serde_json::to_string_pretty(v).expect("JSON"))
        .and_then(|()| std::fs::rename(&tmp, &f))
        .map_err(|e| format!("writing {}: {e}", f.display()))
}

/// Every ask, newest first.
pub fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["made_ms"].as_u64().unwrap_or(0)));
    out
}

/// Whether an ask still waits for the person.
pub fn is_open(v: &Value, now: u64) -> bool {
    v["state"] == json!("open")
        && now.saturating_sub(v["made_ms"].as_u64().unwrap_or(0)) < OPEN_FOR_MS
}

/// Whether an ask holds a yes not yet used and not too old.
pub fn is_yes(v: &Value, now: u64) -> bool {
    v["state"] == json!("yes")
        && now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < YES_FOR_MS
}

/// The asks a person may still answer, and the yeses not yet used: what
/// the page shows and `next` lists.
pub fn pending(home: &Path) -> Vec<Value> {
    let now = now_ms();
    all(home)
        .into_iter()
        .filter(|v| is_open(v, now) || is_yes(v, now))
        .collect()
}

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
}

/// The page's YES or NO on an open ask.
pub fn answer(home: &Path, id: &str, a: Answer) -> Result<Value, String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no ask {id}"))?;
    if !is_open(&v, now_ms()) {
        return Err(format!(
            "this ask was answered already, or is too old ({})",
            v["state"].as_str().unwrap_or("?")
        ));
    }
    v["state"] = json!(match a {
        Answer::Yes => "yes",
        Answer::No => "no",
    });
    v["answered_ms"] = json!(now_ms());
    write(home, &v)?;
    Ok(v)
}

/// What the runner finds for a call that needs a yes.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    /// A yes for this exact call, now used: run it.
    Yes(Value),
    /// The person said no to it lately.
    No(Value),
    /// It waits for the person's answer.
    Open(Value),
    /// The ask named is for another command or other arguments.
    Other(Value),
    /// Nothing yet.
    None,
}

/// Looks for the person's answer to a call with this key, by `id` when the
/// agent names an ask; a yes found is marked used, so it runs once.
pub fn find(home: &Path, id: Option<&str>, key: &str) -> Found {
    let now = now_ms();
    let candidates: Vec<Value> = match id {
        Some(id) => match read(home, id) {
            Some(v) if v["key"] != json!(key) => return Found::Other(v),
            Some(v) => vec![v],
            None => Vec::new(),
        },
        None => all(home)
            .into_iter()
            .filter(|v| v["key"] == json!(key))
            .collect(),
    };
    if let Some(mut v) = candidates.iter().find(|v| is_yes(v, now)).cloned() {
        v["state"] = json!("used");
        v["used_ms"] = json!(now);
        if write(home, &v).is_ok() {
            return Found::Yes(v);
        }
    }
    if let Some(v) = candidates.iter().find(|v| is_open(v, now)) {
        return Found::Open(v.clone());
    }
    if let Some(v) = candidates.iter().find(|v| {
        v["state"] == json!("no")
            && (id.is_some()
                || now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < NO_FOR_MS)
    }) {
        return Found::No(v.clone());
    }
    Found::None
}

/// A new open ask.
pub struct New<'a> {
    pub command: &'a str,
    pub argv: &'a [String],
    pub values: &'a [(String, String)],
    pub about: &'a str,
    pub needs: Ask,
    pub cost: Option<String>,
    /// What the call would change of data already in the person's games,
    /// from its plan: `{says: [..], hash}`. Its words go on the card.
    pub plan: Option<Value>,
    /// A change gp_run's net held (`crate::held`): the ask's key is then
    /// [`held_key`]'s and the card says `held_says`.
    pub held: Option<(Value, String)>,
    /// The loop's step the call cited (gp_run's `step`), so the page can
    /// show the card with the quest-log row the step works on.
    pub step: Option<&'a str>,
}

/// The key of a change the net held for one exact call, apart from the
/// call's own asks.
pub fn held_key(command: &str, argv: &[String]) -> String {
    key(&format!("{command} (held)"), argv)
}

/// Marks an open ask used, when the call it asks for goes ahead on the
/// agent's confirmed_by_person with no page open.
pub fn mark_used(home: &Path, v: &Value) -> Result<(), String> {
    let mut v = v.clone();
    v["state"] = json!("used");
    v["used_ms"] = json!(now_ms());
    write(home, &v)
}

/// How long a call the runner ran on a yes still shows on the page once
/// it finished (the page itself drops it sooner).
pub const RAN_SHOWS_MS: u64 = 15 * 60 * 1000;

/// How long a run the runner started still shows as running with no end:
/// longer than a gate's time limit, so a run that died doesn't stay.
pub const RUNNING_SHOWS_MS: u64 = 2 * 60 * 60 * 1000;

/// How many times a yes's run may be cut off and run again before it
/// counts as failed.
pub const CUT_OFF_TRIES: u64 = 3;

/// Takes a yes for the loop's runner to run itself: state `running`, with
/// `ran` {state: "running", at_ms, pid}, so the yes runs once and gp_run's
/// later call with it finds what the runner did. The yes is used only
/// when the run ends ([`set_ran`]); a run its server stopped in is found
/// by the next one ([`end_dead_runs`]). None when it isn't a yes still to
/// use, or puts a held change back (gp_run's own road).
pub fn claim_for_runner(home: &Path, id: &str) -> Option<Value> {
    let mut v = read(home, id)?;
    if !is_yes(&v, now_ms()) || v.get("held").is_some_and(|h| !h.is_null()) {
        return None;
    }
    let now = now_ms();
    v["state"] = json!("running");
    v["ran"] = json!({"by": "runner", "state": "running", "at_ms": now, "pid": std::process::id()});
    write(home, &v).ok()?;
    Some(v)
}

/// The files the runner's run of ask `id` writes its output and errors
/// to, beside the ask, so a run that outlives its server leaves its words
/// for the next one.
pub fn run_files(home: &Path, id: &str) -> (PathBuf, PathBuf) {
    let d = dir(home);
    (d.join(format!("{id}.out")), d.join(format!("{id}.err")))
}

/// The pid of the command the runner started for a yes (`ran.child`), so
/// a server that starts after this one stopped can wait for it.
pub fn set_child(home: &Path, id: &str, pid: u32) -> Result<(), String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no ask {id}"))?;
    v["ran"]["child"] = json!(pid);
    write(home, &v)
}

/// What a run of a yes came to from its output, with its last lines: `ok`
/// is whether it exited 0, None when no one saw it exit (a run that
/// outlived its server), and then only its JSON says.
pub fn outcome(ok: Option<bool>, out: &[u8], err: &[u8]) -> (&'static str, Vec<String>) {
    let v: Value = serde_json::from_slice(out).unwrap_or(Value::Null);
    let mut lines: Vec<String> = v["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect();
    if lines.is_empty() {
        lines = String::from_utf8_lossy(err)
            .lines()
            .chain(String::from_utf8_lossy(out).lines())
            .filter(|l| !l.trim().is_empty())
            .map(str::to_owned)
            .collect();
    }
    let keep = lines.len().saturating_sub(6);
    let lines = lines.split_off(keep);
    let failed = match ok {
        Some(ok) => !ok,
        None => v["error"].is_string() || v["ok"] == json!(false),
    };
    let state = if failed || v["verdict"] == json!("FAIL") {
        "failed"
    } else if v["landed"].is_string() {
        "landed"
    } else {
        "passed"
    };
    (state, lines)
}

/// Why a run [`end_dead_runs`] ended failed: its card and its row say it.
const DIED: &str = "its run was cut off: gp serve stopped while it ran";

/// The runner's runs of a yes that a server stopped in before their result
/// was written (gp serve stopped, or moved onto a new build, while one
/// ran): each ask left `running` that `live` doesn't name, whose server
/// (`ran.pid`) is gone or is this one. A command it started that still
/// runs (`ran.child`, up to [`RUNNING_SHOWS_MS`]) is waited for. One that
/// ended with its whole JSON in its [`run_files`] ends the card as that
/// says. Else the run was cut off, and the yes is a yes again for the
/// runner to run again (yes-survives-restart); once it was cut off
/// [`CUT_OFF_TRIES`] times it fails with why, ended when it started, so
/// its quest-log row goes back to next. Returns the ids it settled.
pub fn end_dead_runs(home: &Path, live: impl Fn(&str) -> bool) -> Vec<String> {
    let me = u64::from(std::process::id());
    let now = now_ms();
    let mut out = Vec::new();
    for mut v in all(home) {
        let id = v["id"].as_str().unwrap_or("").to_owned();
        let ran = &v["ran"];
        if ran["by"] != json!("runner")
            || ran["state"] != json!("running")
            || !ran["done_ms"].is_null()
            || live(&id)
        {
            continue;
        }
        if ran["pid"]
            .as_u64()
            .is_some_and(|p| p != me && crate::proc::alive(p))
        {
            continue;
        }
        let at = ran["at_ms"].as_u64().unwrap_or(0);
        if ran["child"].as_u64().is_some_and(crate::proc::alive)
            && now.saturating_sub(at) < RUNNING_SHOWS_MS
        {
            continue;
        }
        let (out_f, err_f) = run_files(home, &id);
        let said = std::fs::read(&out_f).unwrap_or_default();
        let ended = serde_json::from_slice::<Value>(&said).is_ok_and(|j| j.is_object());
        let n = v["cut_off"].as_u64().unwrap_or(0) + 1;
        let wrote = if ended {
            let err = std::fs::read(&err_f).unwrap_or_default();
            let (state, mut lines) = outcome(None, &said, &err);
            lines.push("(it ended while gp serve was stopped)".into());
            set_ran(home, &id, state, &lines)
        } else if n >= CUT_OFF_TRIES {
            v["cut_off"] = json!(n);
            v["state"] = json!("used");
            v["used_ms"] = json!(now);
            v["ran"]["state"] = json!("failed");
            v["ran"]["done_ms"] = json!(at);
            v["ran"]["lines"] = json!([format!("{DIED} ({n} times)")]);
            write(home, &v)
        } else {
            v["cut_off"] = json!(n);
            v["state"] = json!("yes");
            v["answered_ms"] = json!(now);
            if let Some(o) = v.as_object_mut() {
                o.remove("ran");
                o.remove("used_ms");
            }
            write(home, &v)
        };
        if wrote.is_ok() {
            let _ = std::fs::remove_file(&out_f);
            let _ = std::fs::remove_file(&err_f);
            out.push(id);
        }
    }
    out
}

/// What the runner's run of a yes came to: `landed`, `passed` or
/// `failed`, with its last lines. The yes is now used.
pub fn set_ran(home: &Path, id: &str, state: &str, lines: &[String]) -> Result<(), String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no ask {id}"))?;
    let now = now_ms();
    v["state"] = json!("used");
    v["used_ms"] = json!(now);
    v["ran"]["state"] = json!(state);
    v["ran"]["done_ms"] = json!(now);
    v["ran"]["lines"] = json!(lines);
    write(home, &v)
}

/// Sets one field of an ask (the runner's `row_settled`, once the card's
/// quest-log row has followed it).
pub fn mark(home: &Path, id: &str, key: &str, value: Value) -> Result<(), String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no ask {id}"))?;
    v[key] = value;
    write(home, &v)
}

/// The asks the page shows: [`pending`], the yeses the runner is running
/// (up to [`RUNNING_SHOWS_MS`]) and those it finished in the last
/// [`RAN_SHOWS_MS`].
pub fn shown(home: &Path) -> Vec<Value> {
    let now = now_ms();
    all(home).into_iter().filter(|v| is_shown(v, now)).collect()
}

fn is_shown(v: &Value, now: u64) -> bool {
    let ago = |k: &str| v["ran"][k].as_u64().map(|t| now.saturating_sub(t));
    is_open(v, now)
        || is_yes(v, now)
        || match ago("done_ms") {
            Some(d) => d < RAN_SHOWS_MS,
            None => ago("at_ms").is_some_and(|t| t < RUNNING_SHOWS_MS),
        }
}

/// Writes a new open ask and returns it.
pub fn make(home: &Path, n: &New) -> Result<Value, String> {
    let made = now_ms();
    let k = if n.held.is_some() {
        held_key(n.command, n.argv)
    } else {
        key(n.command, n.argv)
    };
    let id = format!("{}-{}", made, &k[..8]);
    let game = n
        .values
        .iter()
        .find(|(k, _)| matches!(k.as_str(), "game" | "template" | "run"))
        .map(|(_, v)| v.clone())
        .filter(|g| game_ok(g));
    let mut v = json!({
        "id": id,
        "command": n.command,
        "argv": n.argv,
        "key": k,
        "game": game,
        "needs": n.needs.name(),
        "says": n.held.as_ref().map_or_else(|| says(n), |(_, s)| s.clone()),
        "made_ms": made,
        "state": "open",
    });
    if let Some(c) = &n.cost {
        v["cost"] = json!(c);
    }
    if let Some(p) = &n.plan {
        v["plan"] = p.clone();
    }
    if let Some((h, _)) = &n.held {
        v["held"] = h.clone();
    }
    // The clip that shows the change: the card gets LOOK AT IT.
    if n.command == "model gate"
        && let Some(c) = gate_clip(n.values)
    {
        v["clip"] = c;
    }
    // The step the call cited, or a runner's worker's own: the page shows
    // the card with the step's quest-log row, and its list of workers this
    // worker as waiting for a yes.
    if let Some(st) = n.step.map(str::trim).filter(|s| !s.is_empty()) {
        v["step"] = json!(st);
    } else if let Ok(s) = std::env::var(crate::steps::STEP_ENV)
        && !s.trim().is_empty()
    {
        v["step"] = json!(s.trim());
    }
    write(home, &v)?;
    Ok(v)
}

/// A model gate call's clip (its clip, frames and look), as the page reads
/// a row's: `{"n": N}`, `{"mechanic": ROW, "n": N}` or `{"id": ID}`, with
/// `from`, `to` and `look`. None without a clip or with one it can't read.
pub fn gate_clip(values: &[(String, String)]) -> Option<Value> {
    let clip = value(values, &["clip"])?.trim();
    let name = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    let mut v = if let Ok(n) = clip.parse::<u64>() {
        json!({"n": n})
    } else if let Some((m, n)) = clip.split_once('/')
        && let Ok(n) = n.parse::<u64>()
        && name(m)
    {
        json!({"mechanic": m, "n": n})
    } else if name(clip) {
        json!({"id": clip})
    } else {
        return None;
    };
    if let Some(f) = value(values, &["frames"]) {
        let num = |s: &str| s.trim().parse::<u64>().ok();
        let (a, b) = match f.split_once('-') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => (num(f)?, num(f)?),
        };
        if b < a {
            return None;
        }
        v["from"] = json!(a);
        v["to"] = json!(b);
    }
    if let Some(l) = value(values, &["look"]).map(str::trim).filter(|l| name(l)) {
        v["look"] = json!(l);
    }
    Some(v)
}

/// Why a model gate call may not put a card on the page, or None: a
/// landing changes what the person sees, so its card names the clip and
/// frames that show it (look-cards-carry-their-clip; owner 2026-11-20, "it
/// doesnt give me a clip to click"). An import or a score-only run lands
/// no change to judge (`switch` says whether a switch was given).
pub fn gate_card_refused(
    values: &[(String, String)],
    switch: impl Fn(&str) -> bool,
) -> Option<String> {
    if switch("dry-run") || switch("import") || switch("score-only") {
        return None;
    }
    if value(values, &["clip"]).is_none() {
        return Some("a model gate card names the clip that shows what the change does, so the person can watch it: add clip (N, ROW/N or a corpus clip's id, such as rooftop.attract.001), frames A-B and look (game, or the theme it is about). The card and its row then have LOOK AT IT".to_owned());
    }
    if gate_clip(values).is_none() || value(values, &["frames"]).is_none() {
        return Some("a model gate card's clip is N, ROW/N or a corpus clip's id, with frames A-B (or A) from the clip's start, so LOOK AT IT shows the frames in question".to_owned());
    }
    None
}

fn value<'a>(values: &'a [(String, String)], names: &[&str]) -> Option<&'a str> {
    values
        .iter()
        .find(|(k, _)| names.contains(&k.as_str()))
        .map(|(_, v)| v.as_str())
}

/// What the ask says to the person: what it does, in words they can judge,
/// and what it publishes and where.
/// The files a patch (`git diff` output, or a format-patch mail) changes,
/// by the paths its `diff --git` lines name; none when it can't be read.
fn patch_files(path: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("diff --git a/")
            && let Some((a, _)) = rest.split_once(" b/")
            && !out.iter().any(|x| x == a)
        {
            out.push(a.to_owned());
        }
    }
    out
}

pub fn says(n: &New) -> String {
    let v = |names: &[&str]| value(n.values, names);
    let game = v(&["game", "template", "run"]);
    let of = game.map(|g| format!(" of {g}")).unwrap_or_default();
    let target = v(&["target", "registry", "to", "dest", "remote"]);
    let to = target.map(|t| format!(" to {t}")).unwrap_or_default();
    let main = match n.command {
        "capture background" => {
            let who = match v(&["agents"]).unwrap_or("1") {
                "1" => "a clip agent".to_owned(),
                k => format!("up to {k} clip agents"),
            };
            format!(
                "Let {who} work on the clips{of} the tools are stuck on. Each one is a Claude session that costs money on your account while it runs, up to an hour each."
            )
        }
        "theme generate" => format!(
            "Pay an image service (Runway) to draw the pictures for a theme{of}. The pictures are made from our own words, never the game's art."
        ),
        // What lands, in the patch's own words, and the files it changes.
        "model gate" => {
            let what = v(&["message"]).map_or_else(
                || format!("the changed model{of}"),
                |m| format!("\"{}\"", m.trim()),
            );
            let files = v(&["patch"]).map(patch_files).filter(|f| !f.is_empty());
            let files = files
                .map(|f| format!(" It changes {}.", f.join(", ")))
                .unwrap_or_default();
            format!("Land {what}{of}, once it passes the game's checks.{files}")
        }
        "registry publish" => format!(
            "Publish{} in the registry{to}. Anyone who reads that registry can get it.",
            v(&["dir", "package", "path"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        "registry reindex" => format!("Rewrite the public index of the registry{to}."),
        "registry submit" => format!(
            "Send{} to the registry {} as a pull request on GitHub, from your fork (made if you have none). Everyone can see the pull request; the registry's owner decides whether it goes in.",
            v(&["name", "dir"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default(),
            v(&["to"]).unwrap_or("named with --to")
        ),
        "registry yank" => format!(
            "Withdraw{} in the registry{to}. Players are told it was withdrawn.",
            v(&["version"]).map(|d| format!(" {d}")).unwrap_or_default()
        ),
        "registry grant" => format!(
            "Grant the namespace{} in the registry{to}, so packages under that name can be installed from it.",
            v(&["namespace"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        "theme publish" | "theme recheck" => format!(
            "Publish the theme{} to the public list of themes{to}.",
            v(&["theme", "dir", "name"])
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        "game attest" => format!("Sign and publish a record that the game{of} plays as checked."),
        "game fetch" | "theme fetch" | "testroms fetch" | "testroms run" => format!(
            "Download files from the internet{}. It can be large.",
            v(&["name", "game", "theme", "url"])
                .map(|d| format!(": {d}"))
                .unwrap_or_default()
        ),
        "agent integrate" => "Land a clip agent's work in the game's code.".to_owned(),
        "corpus import" => {
            format!("Change or remove clips already in the corpus{of}, from the mechanic sessions.")
        }
        "self publish" => "Publish this build of Game Printer for others to install.".to_owned(),
        "tool gate" => "Land a change to Game Printer itself once it passes its checks.".to_owned(),
        "dataset grade" => {
            "Read the answer key of a test set, which the work is graded against.".to_owned()
        }
        _ => {
            let about = n.about.trim().trim_end_matches('.');
            let lead = match n.needs {
                Ask::Publish => "Publish or land something others get",
                Ask::Spend => "Spend money",
                Ask::Download => "Download a lot",
                Ask::Replace => "Change or remove what is already in your games",
            };
            if about.is_empty() {
                format!("{lead}.")
            } else {
                format!("{lead}: {about}.")
            }
        }
    };
    let main = match n.plan.as_ref().and_then(|p| p["says"].as_array()) {
        Some(each) if !each.is_empty() => {
            let each: Vec<&str> = each.iter().filter_map(Value::as_str).collect();
            format!("{main} It {}.", each.join("; it "))
        }
        _ => main,
    };
    match &n.cost {
        Some(c) => format!("{main} Cost: {c}."),
        None => main,
    }
}

/// The cost a dry run reported, in words, when its JSON has one (theme
/// generate's `dollars` and `to_make`).
pub fn cost_of(dry: &Value) -> Option<String> {
    let dollars = dry["dollars"].as_f64()?;
    let calls = dry["to_make"].as_u64();
    Some(match calls {
        Some(k) => format!("about ${dollars:.2} for {k} pictures"),
        None => format!("about ${dollars:.2}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run the runner started shows while it runs, past the time a
    /// finished one shows, and a finished one leaves after RAN_SHOWS_MS.
    #[test]
    fn a_running_card_stays_until_it_finishes() {
        let now = 10 * RUNNING_SHOWS_MS;
        let shows = |r: Value| is_shown(&json!({"state": "used", "ran": r}), now);
        let long = now - RAN_SHOWS_MS - 1;
        assert!(shows(json!({"state": "running", "at_ms": long})));
        assert!(!shows(
            json!({"state": "running", "at_ms": now - RUNNING_SHOWS_MS})
        ));
        assert!(shows(
            json!({"state": "landed", "at_ms": long, "done_ms": now - 1000})
        ));
        assert!(!shows(
            json!({"state": "landed", "at_ms": long, "done_ms": long})
        ));
    }

    /// A yes left running by a server that is gone (or by this one, after
    /// it moved onto a new build) is a yes again, or ends as the JSON its
    /// command left says; one whose command still runs, one this server
    /// still runs, one another live server runs and a finished one stay.
    /// Taken again it is the person's yes until it ends, and cut off
    /// CUT_OFF_TRIES times it fails (docs/mcp.md, "Every command").
    #[cfg(unix)]
    #[test]
    fn a_yes_survives_its_server_stopping() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let me = u64::from(std::process::id());
        // Another live process: the test's parent (cargo).
        let other = u64::from(std::os::unix::process::parent_id());
        let now = now_ms();
        let none = json!(null);
        let runs = [
            ("a-1", "running", none.clone(), none.clone()),
            ("b-2", "running", json!(me), none.clone()),
            ("c-3", "running", json!(me), none.clone()),
            ("d-4", "running", json!(other), none.clone()),
            ("e-5", "landed", json!(me), none.clone()),
            ("f-6", "running", none.clone(), json!(other)),
            ("g-7", "running", none.clone(), none.clone()),
        ];
        for (id, state, pid, child) in &runs {
            let mut ran = json!({"by": "runner", "state": state, "at_ms": now - 1000,
                "pid": pid, "child": child});
            if *state == "landed" {
                ran["done_ms"] = json!(now);
            }
            let v = json!({"id": id, "made_ms": 1, "state": "running", "ran": ran});
            write(home, &v).unwrap();
        }
        let g_out = run_files(home, "g-7").0;
        std::fs::write(&g_out, r#"{"landed": "abc", "lines": ["PASS"]}"#).unwrap();
        let mut ended = end_dead_runs(home, |id| id == "c-3");
        ended.sort();
        assert_eq!(ended, ["a-1", "b-2", "g-7"]);
        // Cut off: a yes again.
        let a = read(home, "a-1").unwrap();
        assert!(is_yes(&a, now_ms()) && a["ran"].is_null(), "{a}");
        assert_eq!(a["cut_off"], 1);
        // Ended while no server watched: as its JSON says.
        let g = read(home, "g-7").unwrap();
        assert_eq!(g["state"], "used");
        assert_eq!(g["ran"]["state"], "landed");
        assert!(!g_out.exists());
        for id in ["c-3", "d-4", "f-6"] {
            assert_eq!(read(home, id).unwrap()["ran"]["state"], "running", "{id}");
        }
        assert_eq!(read(home, "e-5").unwrap()["ran"]["state"], "landed");
        // Taken again, it is running, not used, until its run ends.
        let v = claim_for_runner(home, "a-1").unwrap();
        assert_eq!(v["state"], "running");
        assert!(claim_for_runner(home, "a-1").is_none(), "taken once");
        // Cut off again and again: it fails rather than run forever.
        let cut = json!({"by": "runner", "state": "running", "at_ms": 5});
        mark(home, "a-1", "ran", cut).unwrap();
        mark(home, "a-1", "cut_off", json!(CUT_OFF_TRIES - 1)).unwrap();
        assert_eq!(end_dead_runs(home, |id| id == "c-3"), ["a-1"]);
        let a = read(home, "a-1").unwrap();
        assert_eq!(a["state"], "used");
        assert_eq!(a["ran"]["state"], "failed");
        assert_eq!(a["ran"]["done_ms"], 5);
        assert!(a["ran"]["lines"][0].as_str().unwrap().starts_with(DIED));
        // Once ended, it isn't ended again.
        assert!(end_dead_runs(home, |id| id == "c-3").is_empty());
    }

    /// A run no one saw exit is judged by its JSON alone.
    #[test]
    fn a_run_that_outlived_its_server_ends_as_its_json_says() {
        let st = |out: &[u8]| outcome(None, out, b"").0;
        let landed = br#"{"landed": "abc", "lines": ["PASS"]}"#;
        assert_eq!(st(landed), "landed");
        assert_eq!(outcome(Some(false), landed, b"").0, "failed");
        assert_eq!(st(br#"{"error": "no"}"#), "failed");
        assert_eq!(st(br#"{"verdict": "FAIL"}"#), "failed");
        assert_eq!(st(br#"{"verdict": "PASS"}"#), "passed");
    }

    /// A gate's card says what lands in the patch's own words and the
    /// files it changes.
    #[test]
    fn a_gate_card_says_what_lands() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.patch");
        std::fs::write(&p, "diff --git a/g/model.toml b/g/model.toml\n--- a/g/model.toml\n+++ b/g/model.toml\n@@ -1 +1 @@\n-a\n+b\n").unwrap();
        let path = p.to_string_lossy().to_string();
        let argv: Vec<String> = Vec::new();
        let values = vec![
            ("game".to_owned(), "g".to_owned()),
            ("patch".to_owned(), path),
            ("message".to_owned(), "The ball bounces".to_owned()),
        ];
        let n = New {
            command: "model gate",
            needs: Ask::Publish,
            ..new(&argv, &values)
        };
        let s = says(&n);
        assert!(s.starts_with("Land \"The ball bounces\" of g"), "{s}");
        assert!(s.contains("It changes g/model.toml."), "{s}");
    }

    /// A model gate card carries the clip and frames its call names, and
    /// one with no clip (or no frames) is refused; a dry run, an import and
    /// a score-only run aren't (look-cards-carry-their-clip).
    #[test]
    fn a_gate_card_carries_its_clip() {
        let d = tempfile::tempdir().unwrap();
        let s = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect()
        };
        let none = |_: &str| false;
        let bare = s(&[("game", "rooftop"), ("patch", "x.patch")]);
        assert!(gate_card_refused(&bare, none).is_some());
        assert!(gate_card_refused(&bare, |n| n == "dry-run").is_none());
        assert!(gate_card_refused(&bare, |n| n == "import").is_none());
        let no_frames = s(&[("game", "rooftop"), ("clip", "rooftop.attract.001")]);
        assert!(gate_card_refused(&no_frames, none).is_some());
        let full = s(&[
            ("game", "rooftop"),
            ("clip", "rooftop.attract.001"),
            ("frames", "761-763"),
            ("look", "game"),
        ]);
        assert!(gate_card_refused(&full, none).is_none());
        let argv: Vec<String> = Vec::new();
        let n = New {
            command: "model gate",
            ..new(&argv, &full)
        };
        let v = make(d.path(), &n).unwrap();
        assert_eq!(
            v["clip"],
            json!({"id": "rooftop.attract.001", "from": 761, "to": 763, "look": "game"})
        );
        assert_eq!(
            gate_clip(&s(&[("clip", "credit-line/2"), ("frames", "5")])),
            Some(json!({"mechanic": "credit-line", "n": 2, "from": 5, "to": 5}))
        );
        assert_eq!(gate_clip(&s(&[("clip", "3"), ("frames", "9-4")])), None);
    }

    fn new<'a>(argv: &'a [String], values: &'a [(String, String)]) -> New<'a> {
        New {
            command: "registry publish",
            argv,
            values,
            about: "Publish a package",
            needs: Ask::Publish,
            cost: None,
            plan: None,
            held: None,
            step: None,
        }
    }

    #[test]
    fn a_yes_is_used_once_and_a_no_keeps_refusing() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let argv = vec!["--target".to_owned(), "/tmp/r".to_owned()];
        let vals = vec![("target".to_owned(), "/tmp/r".to_owned())];
        let k = key("registry publish", &argv);
        assert_eq!(find(home, None, &k), Found::None);
        let a = make(home, &new(&argv, &vals)).unwrap();
        let id = a["id"].as_str().unwrap();
        assert!(a["says"].as_str().unwrap().contains("to /tmp/r"));
        assert!(matches!(find(home, Some(id), &k), Found::Open(_)));
        assert_eq!(pending(home).len(), 1);
        answer(home, id, Answer::Yes).unwrap();
        assert!(answer(home, id, Answer::No).is_err(), "answered once");
        assert!(matches!(find(home, Some(id), &k), Found::Yes(_)));
        assert_eq!(find(home, Some(id), &k), Found::None, "used");
        assert!(pending(home).is_empty());
        // Another ask, answered no.
        let b = make(home, &new(&argv, &vals)).unwrap();
        let id = b["id"].as_str().unwrap();
        answer(home, id, Answer::No).unwrap();
        assert!(matches!(find(home, Some(id), &k), Found::No(_)));
        assert!(matches!(find(home, None, &k), Found::No(_)));
        // A changed argument is another call.
        let other = key("registry publish", &["--target".into(), "/tmp/s".into()]);
        assert!(matches!(find(home, Some(id), &other), Found::Other(_)));
        assert_eq!(find(home, None, &other), Found::None);
    }
}
