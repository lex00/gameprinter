//! The loop's own trouble becomes quest-log rows with no one noticing
//! first (the owner, 2026-11-20: "this whole thing should be a self
//! improvement loop"). Four kinds:
//!
//! - page: an error the screen page shows the person (a clip that won't set
//!   up, a refused answer, a model that won't start), posted by the page to
//!   `POST /api/trouble` as it shows it.
//! - health: a PROBLEM `gp loop health` prints in two checks a minute or
//!   more apart (a card that failed, a row marked working with no worker, a
//!   server on an old build). gp serve's runner runs the check every few
//!   minutes, so a problem files itself even when no one runs it.
//! - hand: a fix made by hand while the runner goes: gp serve started again
//!   after the one before it stopped, a row's status set from outside a
//!   step.
//! - worker: a step whose workers failed `fail_tries` tries (loop.toml),
//!   filed by gp serve's runner with the tries' endings
//!   (serve/runner_limits.rs).
//!
//! Each cause is one tools row, named `trouble-WORDS-HASH` from its message
//! with the numbers and ids taken out, so the same trouble again adds to its
//! count (`trouble.count`, `trouble.last_ms`) and leaves the note as it is
//! (the note is in the row's step key: crate::steps). A row the trouble
//! comes back to after it was done goes back to next; one put out of scope
//! stays so. A row parked in line goes back to next when the commits its
//! trouble names change (a newer push failing the same way), and a health
//! row whose problem two checks a minute apart no longer print is set done.
//! docs/loop.md, "The loop files its own trouble".

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::mechanic::propose::{MARK, slug};
use crate::mechanic::session::{queue_file, update_queue};
#[cfg(test)]
use crate::mechanic::session::{read_queue, write_queue};

/// Where a trouble was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Page,
    Health,
    Hand,
    /// A step whose workers failed too many tries (serve/runner_limits.rs).
    Worker,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Health => "health",
            Kind::Hand => "hand",
            Kind::Worker => "worker",
        }
    }
}

/// One trouble, as it is filed.
pub(crate) struct Trouble<'a> {
    pub game: &'a str,
    pub kind: Kind,
    /// The message, as the person or the check saw it.
    pub what: &'a str,
    /// Where it came from.
    pub from: &'a str,
}

/// One writer at a time in this process: the page's threads and the
/// runner's file into the same quest log.
static WRITING: Mutex<()> = Mutex::new(());

/// A message as its cause: lower case, its words of letters and digits,
/// each word with a digit in it (a number, a pid, a card or step id) one
/// `#`, and a run of those one `#` (a clip id, rooftop.l15.001). Two
/// messages that differ only there are the same trouble.
pub(crate) fn key(what: &str) -> String {
    key_named(what, &[])
}

/// The key, with each row name of `names` in it one `#` too: "Couldn't
/// set up the clip for double-drink-end" and the same for
/// theme-drink-bartender are one cause (docs/loop.md, "The loop files its
/// own trouble"). A name of one word (jump, drink) is left in, as it could
/// be any word of the message.
fn key_named(what: &str, names: &[Vec<String>]) -> String {
    let words: Vec<String> = what
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            if w.bytes().any(|b| b.is_ascii_digit()) {
                "#".to_owned()
            } else {
                w.to_owned()
            }
        })
        .collect();
    crate::next::cut(&unnamed(words, names).join(" "), 240)
}

/// The words with each run that is one of the names made `#`, longest name
/// first, and a run of `#` one `#`.
fn unnamed(mut words: Vec<String>, names: &[Vec<String>]) -> Vec<String> {
    let mut names: Vec<&Vec<String>> = names.iter().filter(|n| n.len() > 1).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for n in names {
        let mut i = 0;
        while i + n.len() <= words.len() {
            if words[i..i + n.len()] == n[..] {
                words.splice(i..i + n.len(), ["#".to_owned()]);
            }
            i += 1;
        }
    }
    words.dedup_by(|a, b| a == "#" && b == "#");
    words
}

/// The words of the game's row names, for key_named: the quest log's and
/// its archive's, not the trouble rows' own.
fn row_names(q: &[Value], archived: &[Value]) -> Vec<Vec<String>> {
    q.iter()
        .chain(archived)
        .filter(|r| !r["trouble"].is_object())
        .filter_map(|r| r["mechanic"].as_str())
        .map(|m| {
            m.to_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(|w| {
                    if w.bytes().any(|b| b.is_ascii_digit()) {
                        "#".to_owned()
                    } else {
                        w.to_owned()
                    }
                })
                .collect()
        })
        .collect()
}

/// Whether a trouble row is of the cause `k`: its key is, or is once the
/// row names in it are `#` (a row filed before names were taken out).
fn of_cause(r: &Value, k: &str, names: &[Vec<String>]) -> bool {
    let Some(had) = r["trouble"]["key"].as_str() else {
        return false;
    };
    had == k || unnamed(had.split(' ').map(str::to_owned).collect(), names).join(" ") == k
}

/// The row a cause files into: `trouble-` and its first words, and six
/// hex digits of its key so two causes that start alike stay apart.
pub(crate) fn row_name(key: &str) -> String {
    let h = blake3::hash(key.as_bytes()).to_hex();
    let words = slug(&key.replace('#', " "), 40);
    if words.is_empty() {
        format!("trouble-{}", &h[..6])
    } else {
        format!("trouble-{words}-{}", &h[..6])
    }
}

/// The commits a message names, in order and each once: a word of 7 to 40
/// hex digits with a digit and a letter in it (aa015082, 8f3989f0ac37), so
/// a run id or a count isn't one. A trouble row keeps them (`commits`), and
/// a parked row goes back to next when they change.
pub(crate) fn commits(what: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in what.split(|c: char| !c.is_ascii_alphanumeric()) {
        let w = w.to_lowercase();
        if (7..=40).contains(&w.len())
            && w.bytes().all(|b| b.is_ascii_hexdigit())
            && w.bytes().any(|b| b.is_ascii_digit())
            && w.bytes().any(|b| b.is_ascii_alphabetic())
            && !out.contains(&w)
        {
            out.push(w);
        }
    }
    out
}

/// Files the trouble: a new tools row, or one more on its cause's row.
/// Says the row, whether it was added, its count, whether it came back and
/// whether it moved (a parked row whose commits changed, set next).
pub(crate) fn file(paths: &Paths, t: &Trouble, now: u64) -> Result<Value> {
    let what = crate::next::cut(t.what, 400);
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let f = queue_file(paths, t.game)?;
    update_queue(&f, |q| file_rows(paths, t, &what, now, q))
}

/// [`file`]'s change to the quest log's rows `q`, under its lock.
fn file_rows(
    paths: &Paths,
    t: &Trouble,
    what: &str,
    now: u64,
    q: &mut Vec<Value>,
) -> Result<Value> {
    let names = row_names(
        q,
        &crate::mechanic::archive::read_archive(&paths.home, t.game),
    );
    let k = key_named(what, &names);
    let at = commits(what);
    // Trouble that comes back to an archived row brings the row back.
    if !q.iter().any(|r| of_cause(r, &k, &names))
        && let Some(r) =
            crate::mechanic::archive::bring_back(&paths.home, t.game, |r| of_cause(r, &k, &names))?
    {
        q.push(r);
    }
    if let Some(r) = q.iter_mut().find(|r| of_cause(r, &k, &names)) {
        let count = r["trouble"]["count"].as_u64().unwrap_or(1) + 1;
        r["trouble"]["count"] = json!(count);
        r["trouble"]["last_ms"] = json!(now);
        let back = r["status"] == json!("done") && r["scope"] != json!("out");
        if back {
            r["status"] = json!("next");
            r["trouble"]["came_back"] = json!(now);
        }
        let moved = !back && moved_on(r, &at, now);
        if !at.is_empty() {
            r["trouble"]["commits"] = json!(at);
        }
        let row = r["mechanic"].clone();
        let mut v = json!({"row": row, "added": false, "count": count, "came_back": back});
        v["moved"] = json!(moved);
        return Ok(v);
    }
    let row = row_name(&k);
    q.push(json!({
        "mechanic": row,
        "status": "next",
        "repo": "tools",
        "by": "loop",
        "from": t.from.trim(),
        "note": format!("{MARK} {} (from {})", what.trim(), t.from.trim()),
        "trouble": {
            "key": k,
            "kind": t.kind.word(),
            "count": 1,
            "first_ms": now,
            "last_ms": now,
            "commits": at,
        },
    }));
    Ok(json!({"row": row, "added": true, "count": 1, "came_back": false}))
}

/// A parked row of the trouble whose commits changed goes back to next,
/// its park taken off and its note led by where the trouble is now
/// (2026-11-26: check-web's row stayed parked while GitHub failed on newer
/// pushes). A row waiting on another row stays: the wait is its reason.
/// Says whether it moved.
fn moved_on(r: &mut Value, at: &[String], now: u64) -> bool {
    let had: Vec<&str> = r["trouble"]["commits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if at.is_empty()
        || had.is_empty()
        || had == at
        || !crate::mechanic::is_parked(r)
        || r.get("waits_on").is_some()
    {
        return false;
    }
    let lead = format!("Its trouble is now on {}. ", at.join(", "));
    let bare = crate::mechanic::unparked_note(r);
    let bare = r["trouble"]["moved_words"]
        .as_str()
        .and_then(|w| bare.strip_prefix(w))
        .unwrap_or(&bare)
        .to_owned();
    r["note"] = json!(format!("{lead}{bare}"));
    r["status"] = json!("next");
    r["trouble"]["moved_words"] = json!(lead);
    r["trouble"]["moved_ms"] = json!(now);
    if let Some(o) = r.as_object_mut() {
        o.remove("parked");
        o.remove("parked_ms");
    }
    true
}

/// The trouble rows of a quest log not done yet.
pub(crate) fn open(q: &[Value]) -> Vec<&Value> {
    q.iter()
        .filter(|r| r["trouble"].is_object() && r["status"] != json!("done"))
        .collect()
}

/// A home trouble is filed for: the person's, not a test's in the temp
/// folder.
pub(crate) fn live_home(home: &Path) -> bool {
    if cfg!(test) {
        return false;
    }
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let h = canon(home);
    let tmp = canon(&std::env::temp_dir());
    !(h.starts_with(&tmp) || h.starts_with("/tmp") || h.starts_with("/private/tmp"))
}

/// The game the loop's own trouble is filed under when nothing names one:
/// the loop's first game (loop.toml), else the games checkout's first.
pub(crate) fn loop_game(home: &Path, games: Option<&Path>) -> Option<String> {
    let s = gp_mcp::steps::settings(home);
    if let Some(g) = s.games.first() {
        return Some(g.clone());
    }
    let dir = crate::roms_cmd::games_dir(games)?;
    crate::next::games_in(&dir).into_iter().next()
}

/// How long the page's same trouble waits before it is filed again: a poll
/// that fails every second adds to its row once a minute.
const PAGE_EVERY_MS: u64 = 60_000;

/// An error the screen page showed the person. Filed once a minute at most
/// for the same message.
pub(crate) fn from_page(paths: &Paths, game: &str, text: &str, place: &str) -> Result<Value> {
    static LAST: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
    let now = gp_mcp::asks::now_ms();
    let k = format!("{game}\n{}", key(text));
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let m = last.get_or_insert_with(HashMap::new);
        if m.get(&k)
            .is_some_and(|t| now.saturating_sub(*t) < PAGE_EVERY_MS)
        {
            return Ok(json!({"filed": false, "why": "filed under a minute ago"}));
        }
        m.insert(k, now);
    }
    let from = if place.trim().is_empty() {
        format!("the screen page of {game} showed it to the person")
    } else {
        format!(
            "the screen page of {game} showed it to the person, {}",
            place.trim()
        )
    };
    let what = format!("The page showed the person an error: {}", text.trim());
    let mut v = file(
        paths,
        &Trouble {
            game,
            kind: Kind::Page,
            what: &what,
            from: &from,
        },
        now,
    )?;
    v["filed"] = json!(true);
    Ok(v)
}

/// How far apart two checks are for a problem in both to have lasted.
const HEALTH_LASTS_MS: u64 = 60_000;

/// The health check's file of what it saw last.
fn health_file(home: &Path) -> std::path::PathBuf {
    gp_mcp::steps::loop_dir(home).join("trouble-health.json")
}

/// The problems a health check printed: each one the check before also
/// printed, a minute or more ago, is filed (and again each check it lasts,
/// which adds to its count). `held` are failures the check holds back
/// rather than prints (a GitHub check failed with a newer run still going:
/// crate::github_checks::held), None when GitHub couldn't be read. The
/// open health rows of no problem printed or held are settled
/// (settle_gone). Says the rows filed.
pub(crate) fn health_seen(
    paths: &Paths,
    game: &str,
    problems: &[String],
    held: Option<&[String]>,
    now: u64,
) -> Vec<Value> {
    let f = health_file(&paths.home);
    let before: Value = fs::read(&f)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    let mut seen = serde_json::Map::new();
    let mut filed = Vec::new();
    for p in problems {
        let k = key(p);
        let first = before["seen"][&k]["first_ms"].as_u64();
        seen.insert(
            k.clone(),
            json!({"first_ms": first.unwrap_or(now), "what": p}),
        );
        if let Some(first) = first
            && now.saturating_sub(first) >= HEALTH_LASTS_MS
            && let Ok(v) = file(
                paths,
                &Trouble {
                    game,
                    kind: Kind::Health,
                    what: p,
                    from: "gp loop health, in two checks in a row",
                },
                now,
            )
        {
            filed.push(v);
        }
    }
    let missing = settle_gone(paths, game, problems, held, &before["missing"], now);
    let _ = fs::create_dir_all(gp_mcp::steps::loop_dir(&paths.home));
    let kept = json!({"at_ms": now, "seen": seen, "missing": missing});
    let _ = fs::write(&f, serde_json::to_vec_pretty(&kept).unwrap_or_default());
    filed
}

/// Settles the open health rows of the game that no problem of this check
/// is of, printed or held back: one missed by two checks a minute or more
/// apart is set done, its problem gone (2026-11-26: trouble rows stayed
/// open after their checks said OK). A row a worker is on waits for it to
/// end, and a GitHub row waits while GitHub can't be read. Says the rows
/// missed so far, each with the first check that missed it.
fn settle_gone(
    paths: &Paths,
    game: &str,
    problems: &[String],
    held: Option<&[String]>,
    before: &Value,
    now: u64,
) -> serde_json::Map<String, Value> {
    let mut missing = serde_json::Map::new();
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(f) = queue_file(paths, game) else {
        return missing;
    };
    let _ = update_queue(&f, |q| {
        let names = row_names(
            q,
            &crate::mechanic::archive::read_archive(&paths.home, game),
        );
        let keys: Vec<String> = problems
            .iter()
            .chain(held.unwrap_or_default())
            .map(|p| key_named(&crate::next::cut(p, 400), &names))
            .collect();
        for r in q.iter_mut() {
            let Some(row) = r["mechanic"].as_str().map(str::to_owned) else {
                continue;
            };
            let status = r["status"].as_str().unwrap_or("");
            let had = r["trouble"]["key"].as_str().unwrap_or("");
            if r["trouble"]["kind"] != json!("health")
                || status == "done"
                || status.contains("working")
                || (had.starts_with("github s ") && held.is_none())
                || keys.iter().any(|k| of_cause(r, k, &names))
            {
                continue;
            }
            let first = before[&row]["first_ms"].as_u64().unwrap_or(now);
            let checks = before[&row]["checks"].as_u64().unwrap_or(0) + 1;
            if checks >= 2 && now.saturating_sub(first) >= HEALTH_LASTS_MS {
                r["status"] = json!("done");
                r["trouble"]["gone_ms"] = json!(now);
            } else {
                missing.insert(row, json!({"first_ms": first, "checks": checks}));
            }
        }
        Ok(())
    });
    missing
}

/// The environment a server that moves itself onto a new build sets for
/// the one it starts, so that start isn't a restart by hand.
pub(crate) const RELOADED_ENV: &str = "GP_SERVE_RELOADED";

/// How recent the server before must have been for a start to be a
/// restart.
const RESTART_WITHIN_MS: u64 = 3_600_000;

/// Whether a start is a restart, from the home's heartbeats: the newest
/// one of another process that ended, if it beat within the hour. Says the
/// words of the trouble.
pub(crate) fn restart_words(
    beats: &[Value],
    own_pid: u64,
    now: u64,
    alive: impl Fn(u64) -> bool,
) -> Option<String> {
    let b = beats
        .iter()
        .filter(|b| {
            let pid = b["pid"].as_u64().unwrap_or(0);
            pid != own_pid && pid != 0 && !alive(pid)
        })
        .max_by_key(|b| b["at_ms"].as_u64().unwrap_or(0))?;
    let at = b["at_ms"].as_u64().unwrap_or(0);
    let ago = now.saturating_sub(at);
    (ago <= RESTART_WITHIN_MS).then(|| {
        format!(
            "gp serve was started again by hand: the one before it (process {}) stopped {} s before, and nothing in the loop moved it onto the new build",
            b["pid"].as_u64().unwrap_or(0),
            ago / 1000
        )
    })
}

/// Every server heartbeat on the home (gp_mcp::steps::beat).
fn beats(home: &Path) -> Vec<Value> {
    fs::read_dir(gp_mcp::steps::loop_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("runner") && n.ends_with(".json")
        })
        .filter_map(|e| fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}

/// gp serve starting: a restart by hand is filed (not one a server that
/// moved itself started, with RELOADED_ENV set).
pub(crate) fn serve_started(paths: &Paths, games: Option<&Path>, port: u16) {
    // Files nothing, like a status set by hand (the owner, 2026-11-22): a
    // server started by hand is the person's to do.
    if true {
        let _ = (paths, games, port);
        return;
    }
    if !live_home(&paths.home) || std::env::var_os(RELOADED_ENV).is_some_and(|v| !v.is_empty()) {
        return;
    }
    let now = gp_mcp::asks::now_ms();
    let Some(words) = restart_words(
        &beats(&paths.home),
        u64::from(std::process::id()),
        now,
        gp_mcp::proc::alive,
    ) else {
        return;
    };
    let Some(game) = loop_game(&paths.home, games) else {
        return;
    };
    let from = format!("gp serve starting on port {port}");
    let _ = file(
        paths,
        &Trouble {
            game: &game,
            kind: Kind::Hand,
            what: &words,
            from: &from,
        },
        now,
    );
}

/// `gp mechanic queue GAME ROW --status S` from outside the loop's steps
/// (no GP_STEP, no GP_WORKER) while the runner goes: a hand fix, filed. A
/// trouble row set by hand isn't filed again, nor a row of a game whose
/// work the owner holds (gp loop set --hold): setting it is theirs. The
/// queue command doesn't call this for a deliberate park (`--parked`,
/// `--waits-on`) or a parked row set next (crate::mechanic::is_parked).
pub(crate) fn status_by_hand(paths: &Paths, game: &str, row: &str, status: &str) {
    // Files nothing (the owner, 2026-11-22): 74 of the 88 trouble rows the
    // loop ever filed were this kind, each took a worker and a card, and
    // none found a fault. A person setting a row is the person's to do.
    let _ = (paths, game, row, status);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_paths as paths;

    /// Numbers and ids are out of the key; the words stay.
    #[test]
    fn a_key_is_the_cause() {
        let a = key("Card a1b2c3 failed in the last hour: rooftop.l15.001 ran past 170 ms");
        let b = key("Card ff09e7 failed in the last hour: rooftop.l15.001 ran past 171 ms");
        assert_eq!(a, b);
        assert_eq!(a, "card # failed in the last hour rooftop # ran past # ms");
        assert_ne!(a, key("Card a1b2c3 is waiting"));
        let r = row_name(&a);
        assert!(r.starts_with("trouble-card-failed-in-the-last-hour"), "{r}");
        assert!(crate::mechanic::session::name_ok(&r), "{r}");
    }

    /// The same trouble twice is one row with a count of 2, its note as it
    /// was; after it is done it comes back as next; out of scope stays.
    #[test]
    fn the_same_trouble_adds_to_its_row() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let set_mismatch = "Couldn't set up the clip for picture-other-the-level-picture-main: the log is of rooftop with columns 6; this is rooftopg with 6";
        let t = Trouble {
            game: "made-up",
            kind: Kind::Page,
            what: set_mismatch,
            from: "the screen page",
        };
        let v = file(&p, &t, 1000).unwrap();
        assert_eq!(v["added"], json!(true));
        let f = queue_file(&p, "made-up").unwrap();
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        let r = &q[0];
        assert_eq!(r["status"], "next");
        assert_eq!(r["repo"], "tools");
        assert_eq!(r["by"], "loop");
        assert_eq!(r["trouble"]["kind"], "page");
        assert_eq!(
            r["note"],
            format!("Proposed by the loop: {set_mismatch} (from the screen page)")
        );
        let note = r["note"].clone();

        let again = file(&p, &t, 2000).unwrap();
        assert_eq!(
            (again["added"].clone(), again["count"].clone()),
            (json!(false), json!(2))
        );
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["trouble"]["count"], 2);
        assert_eq!(q[0]["trouble"]["last_ms"], 2000);
        assert_eq!(q[0]["note"], note, "the note, in the step's key, stays");
        assert_eq!(open(&q).len(), 1);

        let mut q = read_queue(&f);
        q[0]["status"] = json!("done");
        write_queue(&f, &q).unwrap();
        assert!(open(&read_queue(&f)).is_empty());
        let back = file(&p, &t, 3000).unwrap();
        assert_eq!(back["came_back"], json!(true));
        assert_eq!(read_queue(&f)[0]["status"], "next");

        let mut q = read_queue(&f);
        q[0]["status"] = json!("done");
        q[0]["scope"] = json!("out");
        write_queue(&f, &q).unwrap();
        file(&p, &t, 4000).unwrap();
        assert_eq!(read_queue(&f)[0]["status"], "done");

        // Archived, the same trouble brings its row back rather than
        // filing a new one.
        let mut q = read_queue(&f);
        q[0]["scope"] = json!(null);
        let a = crate::mechanic::archive::archive_file(&p.home, "made-up");
        write_queue(&a, &q).unwrap();
        write_queue(&f, &[]).unwrap();
        let back = file(&p, &t, 5000).unwrap();
        assert_eq!(
            (back["added"].clone(), back["came_back"].clone()),
            (json!(false), json!(true))
        );
        let q = read_queue(&f);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["status"], "next");
        assert!(read_queue(&a).is_empty());
    }

    /// Two faults that differ only in the row they name are one row, count
    /// 2, and a row filed before names were taken out takes the next one
    /// (2026-11-25: the same NES log on an arcade board made a row for
    /// theme-drink-bartender and another for double-drink-end). A name of
    /// one word stays in the key.
    #[test]
    fn two_faults_naming_two_rows_are_one_trouble() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let f = queue_file(&p, "made-up").unwrap();
        let row = |m: &str| json!({"mechanic": m, "status": "next"});
        write_queue(
            &f,
            &[
                row("theme-drink-bartender"),
                row("double-drink-end"),
                row("drink"),
            ],
        )
        .unwrap();
        let fault = |m: &str| {
            format!(
                "The page showed the person an error: Couldn't set up the clip for {m}: the log is an NES input log; this board is arcade"
            )
        };
        let t = |what: &str| {
            file(
                &p,
                &Trouble {
                    game: "made-up",
                    kind: Kind::Page,
                    what,
                    from: "the screen page",
                },
                1000,
            )
            .unwrap()
        };
        let a = t(&fault("theme-drink-bartender"));
        let b = t(&fault("double-drink-end"));
        assert_eq!(a["added"], json!(true));
        assert_eq!(
            (b["row"].clone(), b["count"].clone()),
            (a["row"].clone(), json!(2))
        );
        let q = read_queue(&f);
        assert_eq!(open(&q).len(), 1);
        let k = q[3]["trouble"]["key"].as_str().unwrap();
        assert!(k.contains("the clip for # the log"), "{k}");
        assert!(!k.contains("drink"), "{k}");
        assert_eq!(t(&fault("drink"))["added"], json!(true), "one word stays");

        // A row keyed with the name in it, as filed before, takes the fault.
        let mut q = read_queue(&f);
        q.retain(|r| !r["trouble"].is_object());
        q.push(json!({
            "mechanic": "trouble-old",
            "status": "next",
            "trouble": {"key": key(&fault("theme-drink-bartender")), "count": 1},
        }));
        write_queue(&f, &q).unwrap();
        let c = t(&fault("double-drink-end"));
        assert_eq!(
            (c["row"].clone(), c["count"].clone()),
            (json!("trouble-old"), json!(2))
        );
    }

    /// The page's same error within a minute is filed once.
    #[test]
    fn the_page_files_once_a_minute() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let v = from_page(
            &p,
            "made-up",
            "Couldn't open clip 3: no such clip",
            "opening a clip",
        )
        .unwrap();
        assert_eq!(v["filed"], json!(true));
        let w = from_page(
            &p,
            "made-up",
            "Couldn't open clip 4: no such clip",
            "opening a clip",
        )
        .unwrap();
        assert_eq!(w["filed"], json!(false));
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!(q.len(), 1);
        assert!(q[0]["note"].as_str().unwrap().starts_with(
            "Proposed by the loop: The page showed the person an error: Couldn't open clip 3"
        ));
    }

    /// A problem in one check isn't filed; in the next a minute later it
    /// is; one gone from a check starts over.
    #[test]
    fn a_problem_that_lasts_is_filed() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let card = "Card c1 failed in the last hour: FAIL (corpus)".to_owned();
        let row = "made-up/jump is marked agent working, and no worker is on it".to_owned();
        let both = [card.clone(), row];
        assert!(health_seen(&p, "made-up", &both, None, 0).is_empty());
        let once = health_seen(&p, "made-up", std::slice::from_ref(&card), None, 30_000);
        assert!(once.is_empty(), "under a minute");
        let filed = health_seen(&p, "made-up", &both, None, 61_000);
        assert_eq!(filed.len(), 1, "the row's problem started over");
        assert_eq!(filed[0]["added"], json!(true));
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["trouble"]["kind"], "health");
        let again = health_seen(&p, "made-up", &[card], None, 200_000);
        assert_eq!(again[0]["count"], json!(2));
    }

    /// A parked row whose trouble names a newer commit goes back to next,
    /// its note led by the commit and its park taken off; the same commit
    /// again, or a row waiting on another row, stays (2026-11-26).
    #[test]
    fn a_parked_row_follows_its_commit() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let red = |sha: &str| format!("GitHub's check fails on main, the newest on {sha}");
        let t = |what: &str| {
            file(
                &p,
                &Trouble {
                    game: "made-up",
                    kind: Kind::Health,
                    what,
                    from: "gp loop health",
                },
                1000,
            )
            .unwrap()
        };
        assert_eq!(t(&red("aa015082"))["added"], json!(true));
        let f = queue_file(&p, "made-up").unwrap();
        let mut q = read_queue(&f);
        assert_eq!(q[0]["trouble"]["commits"], json!(["aa015082"]));
        crate::mechanic::park_row(&mut q[0], "a person's look");
        write_queue(&f, &q).unwrap();
        assert_eq!(t(&red("aa015082"))["moved"], json!(false));
        assert_eq!(read_queue(&f)[0]["status"], "in line");
        assert_eq!(t(&red("b32f4590"))["moved"], json!(true));
        let q = read_queue(&f);
        assert_eq!(q[0]["status"], "next");
        assert!(q[0].get("parked").is_none());
        let note = q[0]["note"].as_str().unwrap();
        assert!(note.starts_with("Its trouble is now on b32f4590. Proposed"));

        let mut q = read_queue(&f);
        crate::mechanic::park_row(&mut q[0], "again");
        write_queue(&f, &q).unwrap();
        t(&red("c0ffee12"));
        let q = read_queue(&f);
        let note = q[0]["note"].as_str().unwrap();
        assert!(note.starts_with("Its trouble is now on c0ffee12. Proposed"));
        assert_eq!(note.matches("Its trouble").count(), 1);

        let mut q = read_queue(&f);
        q[0]["status"] = json!("in line");
        q[0]["waits_on"] = json!("made-up/other");
        write_queue(&f, &q).unwrap();
        assert_eq!(t(&red("d00d1234"))["moved"], json!(false));
    }

    /// A health row whose problem two checks a minute apart no longer print
    /// is done; one check isn't enough, a failure held back keeps its row,
    /// and a GitHub row waits while GitHub can't be read (2026-11-26).
    #[test]
    fn a_health_row_whose_problem_is_gone_is_done() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let g = "made-up";
        let lock = "gate.lock is held by process 12 with no step logged".to_owned();
        let red = "GitHub's check-web fails on main, the newest on aa015082".to_owned();
        let none: &[String] = &[];
        let both = [lock, red.clone()];
        health_seen(&p, g, &both, Some(none), 0);
        assert_eq!(health_seen(&p, g, &both, Some(none), 61_000).len(), 2);
        let f = queue_file(&p, g).unwrap();
        assert_eq!(open(&read_queue(&f)).len(), 2);
        // A newer push's run going: its failure is held back, and the
        // lock's problem is gone from this check and the next.
        let held = std::slice::from_ref(&red);
        health_seen(&p, g, none, Some(held), 120_000);
        assert_eq!(open(&read_queue(&f)).len(), 2, "one check");
        health_seen(&p, g, none, Some(held), 130_000);
        assert_eq!(open(&read_queue(&f)).len(), 2, "under a minute");
        health_seen(&p, g, none, Some(held), 181_000);
        let q = read_queue(&f);
        assert_eq!(q[0]["status"], "done");
        assert_eq!(q[0]["trouble"]["gone_ms"], 181_000);
        assert_eq!(q[1]["status"], "next", "held back, it stays");
        // GitHub unread: its row stays, however long.
        health_seen(&p, g, none, None, 300_000);
        health_seen(&p, g, none, None, 400_000);
        assert_eq!(read_queue(&f)[1]["status"], "next");
        // Read again and passing: two checks, and it is done.
        health_seen(&p, g, none, Some(none), 500_000);
        health_seen(&p, g, none, Some(none), 600_000);
        assert_eq!(read_queue(&f)[1]["status"], "done");
    }

    /// A start after a server that stopped within the hour is a restart; a
    /// first start, one beside a live server, or one after a day isn't.
    #[test]
    fn a_restart_is_seen_from_the_heartbeats() {
        let beat = |pid: u64, at: u64| json!({"pid": pid, "at_ms": at});
        let now = 10_000_000;
        assert_eq!(restart_words(&[], 7, now, |_| false), None);
        assert_eq!(
            restart_words(&[beat(7, now)], 7, now, |_| false),
            None,
            "its own"
        );
        assert_eq!(
            restart_words(&[beat(5, now - 1000)], 7, now, |_| true),
            None,
            "still running"
        );
        assert_eq!(
            restart_words(&[beat(5, now - 2 * RESTART_WITHIN_MS)], 7, now, |_| false),
            None
        );
        let w = restart_words(
            &[beat(4, now - 50_000), beat(5, now - 5_000)],
            7,
            now,
            |_| false,
        )
        .unwrap();
        assert!(
            w.contains("process 5") && w.contains("stopped 5 s before"),
            "{w}"
        );
        // Two restarts by hand are one cause.
        let w2 = restart_words(&[beat(9, now - 8_000)], 7, now, |_| false).unwrap();
        assert_eq!(key(&w), key(&w2));
    }
}
