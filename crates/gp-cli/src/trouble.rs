//! The loop's own trouble becomes quest-log rows with no one noticing
//! first (the owner, 2026-11-20: "this whole thing should be a self
//! improvement loop"). Three kinds:
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
//!
//! Each cause is one tools row, named `trouble-WORDS-HASH` from its message
//! with the numbers and ids taken out, so the same trouble again adds to its
//! count (`trouble.count`, `trouble.last_ms`) and leaves the note as it is
//! (the note is in the row's step key: crate::steps). A row the trouble
//! comes back to after it was done goes back to next; one put out of scope
//! stays so. docs/loop.md, "The loop files its own trouble".

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::mechanic::propose::{MARK, slug};
use crate::mechanic::session::{queue_file, read_queue, write_queue};

/// Where a trouble was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Page,
    Health,
    Hand,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Health => "health",
            Kind::Hand => "hand",
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
/// `#`, and a run of those one `#` (a clip id, tapper.l15.001). Two
/// messages that differ only there are the same trouble.
pub(crate) fn key(what: &str) -> String {
    let mut words: Vec<String> = what
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
    words.dedup_by(|a, b| a == "#" && b == "#");
    crate::next::cut(&words.join(" "), 240)
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

/// Files the trouble: a new tools row, or one more on its cause's row.
/// Says the row, whether it was added, its count and whether it came back.
pub(crate) fn file(paths: &Paths, t: &Trouble, now: u64) -> Result<Value> {
    let what = crate::next::cut(t.what, 400);
    let k = key(&what);
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let f = queue_file(paths, t.game)?;
    let mut q = read_queue(&f);
    // Trouble that comes back to an archived row brings the row back.
    if !q.iter().any(|r| r["trouble"]["key"] == json!(k))
        && let Some(r) = crate::mechanic::archive::bring_back(&paths.home, t.game, |r| {
            r["trouble"]["key"] == json!(k)
        })?
    {
        q.push(r);
    }
    if let Some(r) = q.iter_mut().find(|r| r["trouble"]["key"] == json!(k)) {
        let count = r["trouble"]["count"].as_u64().unwrap_or(1) + 1;
        r["trouble"]["count"] = json!(count);
        r["trouble"]["last_ms"] = json!(now);
        let back = r["status"] == json!("done") && r["scope"] != json!("out");
        if back {
            r["status"] = json!("next");
            r["trouble"]["came_back"] = json!(now);
        }
        let row = r["mechanic"].clone();
        write_queue(&f, &q)?;
        return Ok(json!({"row": row, "added": false, "count": count, "came_back": back}));
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
        },
    }));
    write_queue(&f, &q)?;
    Ok(json!({"row": row, "added": true, "count": 1, "came_back": false}))
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
/// which adds to its count). Says the rows filed.
pub(crate) fn health_seen(paths: &Paths, game: &str, problems: &[String], now: u64) -> Vec<Value> {
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
    let _ = fs::create_dir_all(gp_mcp::steps::loop_dir(&paths.home));
    let _ = fs::write(
        &f,
        serde_json::to_vec_pretty(&json!({"at_ms": now, "seen": seen})).unwrap_or_default(),
    );
    filed
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

/// Whether the runner goes on the home: it is on, and a live server beat
/// in the last 20 s.
fn runner_going(home: &Path) -> bool {
    let now = gp_mcp::asks::now_ms();
    gp_mcp::steps::settings(home).runner.on
        && beats(home).iter().any(|b| {
            b["pid"].as_u64().is_some_and(gp_mcp::proc::alive)
                && now.saturating_sub(b["at_ms"].as_u64().unwrap_or(0))
                    <= gp_mcp::steps::BEAT_FOR_MS
        })
}

/// `gp mechanic queue GAME ROW --status S` from outside the loop's steps
/// (no GP_STEP, no GP_WORKER) while the runner goes: a hand fix, filed. A
/// trouble row set by hand isn't filed again.
pub(crate) fn status_by_hand(paths: &Paths, game: &str, row: &str, status: &str) {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if row.starts_with("trouble-")
        || set(gp_mcp::steps::STEP_ENV)
        || set("GP_WORKER")
        || !live_home(&paths.home)
        || !runner_going(&paths.home)
    {
        return;
    }
    let what = format!(
        "The quest-log row {row} was set to {status} by hand, outside the loop's steps, while the runner was on"
    );
    let _ = file(
        paths,
        &Trouble {
            game,
            kind: Kind::Hand,
            what: &what,
            from: "gp mechanic queue --status, run by hand",
        },
        gp_mcp::asks::now_ms(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_paths as paths;

    /// Numbers and ids are out of the key; the words stay.
    #[test]
    fn a_key_is_the_cause() {
        let a = key("Card a1b2c3 failed in the last hour: tapper.l15.001 ran past 170 ms");
        let b = key("Card ff09e7 failed in the last hour: tapper.l15.001 ran past 171 ms");
        assert_eq!(a, b);
        assert_eq!(a, "card # failed in the last hour tapper # ran past # ms");
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
        let set_mismatch = "Couldn't set up the clip for picture-other-the-level-picture-main: the log is of tapper with columns 6; this is tapperg with 6";
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
        assert!(health_seen(&p, "made-up", &[card.clone(), row.clone()], 0).is_empty());
        assert!(
            health_seen(&p, "made-up", std::slice::from_ref(&card), 30_000).is_empty(),
            "under a minute"
        );
        let filed = health_seen(&p, "made-up", &[card.clone(), row.clone()], 61_000);
        assert_eq!(filed.len(), 1, "the row's problem started over");
        assert_eq!(filed[0]["added"], json!(true));
        let q = read_queue(&queue_file(&p, "made-up").unwrap());
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["trouble"]["kind"], "health");
        assert_eq!(
            health_seen(&p, "made-up", &[card], 200_000)[0]["count"],
            json!(2)
        );
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
