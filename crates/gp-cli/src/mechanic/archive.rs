//! The quest log's archive: `$GP_HOME/mechanics/GAME/archive.json`, beside
//! `queue.json`. gp serve's runner moves a row there once it has been done
//! for [`ARCHIVE_AFTER_MS`] with no live worker on it and no card of it
//! still open or unsettled (serve/runner.rs, `archive_finished`), so the
//! page shows the work still going on. Each archived row is the row as it
//! was, with `closed_ms` (its last change), `archived_ms`, its session (the
//! folder, which stays where it is, and how many clips and turns it holds)
//! and the commit its card landed, when one did.
//!
//! The archive is read by asking an agent: `gp mechanic queue GAME
//! --archived [NAME-OR-WORDS]` lists it. A row the loop takes up again (a
//! proposal or trouble of the same row, or `gp mechanic queue` setting its
//! status) comes back out of the archive into the quest log with its
//! history ([`bring_back`]).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use super::session::{read_queue, update_queue};

/// How long a row stays done in the quest log before it is archived.
pub(crate) const ARCHIVE_AFTER_MS: u64 = 60 * 60 * 1000;

/// The game's archive file.
pub(crate) fn archive_file(home: &Path, game: &str) -> PathBuf {
    home.join("mechanics").join(game).join("archive.json")
}

/// The game's archived rows, oldest archived first; empty when none.
pub(crate) fn read_archive(home: &Path, game: &str) -> Vec<Value> {
    read_queue(&archive_file(home, game))
}

fn write_archive(home: &Path, game: &str, rows: &[Value]) -> Result<()> {
    let f = archive_file(home, game);
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = f.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(rows)?)?;
    fs::rename(&tmp, &f)?;
    Ok(())
}

/// What the runner knows of one row beside the quest log.
#[derive(Default)]
pub(crate) struct Beside {
    /// A live worker's step touches the row.
    pub worker: bool,
    /// A card of the row is open, running or not yet settled onto the row.
    pub card: bool,
    /// The newest card of the row that landed: its id, what it did and
    /// the commit (`{"card", "what", "commit"}`).
    pub landed: Option<Value>,
}

/// Whether row `r` is to be archived at `now`: done for
/// [`ARCHIVE_AFTER_MS`], with nothing beside it still going.
pub(crate) fn finished(r: &Value, beside: &Beside, now: u64) -> bool {
    r["status"] == json!("done")
        && r["mechanic"].is_string()
        && r["changed_ms"]
            .as_u64()
            .is_some_and(|c| now.saturating_sub(c) >= ARCHIVE_AFTER_MS)
        && !beside.worker
        && !beside.card
}

/// Moves `game`'s finished rows from its quest log to its archive, and says
/// which. `beside(row)` is what the runner knows of each row. A row the
/// archive already has (archived, then written back by a writer that read
/// the quest log before) takes the old entry's place.
pub(crate) fn sweep(
    home: &Path,
    game: &str,
    now: u64,
    beside: impl Fn(&str) -> Beside,
) -> Result<Vec<String>> {
    let qf = home.join("mechanics").join(game).join("queue.json");
    update_queue(&qf, |q| {
        let mut keep = Vec::new();
        let mut gone = Vec::new();
        for r in std::mem::take(q) {
            let name = r["mechanic"].as_str().unwrap_or("").to_owned();
            let b = beside(&name);
            if finished(&r, &b, now) {
                gone.push(entry(home, game, r, b, now));
            } else {
                keep.push(r);
            }
        }
        *q = keep;
        if gone.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = gone
            .iter()
            .filter_map(|r| r["mechanic"].as_str().map(str::to_owned))
            .collect();
        let mut a = read_archive(home, game);
        a.retain(|r| !names.iter().any(|n| r["mechanic"] == json!(n)));
        a.extend(gone);
        // The archive first: a row is never in neither file. A failed
        // archive write leaves the log as it was (update_queue writes
        // nothing on an error).
        write_archive(home, game, &a)?;
        Ok(names)
    })
}

/// Row `r` as the archive keeps it.
fn entry(home: &Path, game: &str, mut r: Value, b: Beside, now: u64) -> Value {
    let closed = r["changed_ms"].clone();
    let name = r["mechanic"].as_str().unwrap_or("").to_owned();
    if let Some(o) = r.as_object_mut() {
        o.insert("closed_ms".into(), closed);
        o.insert("archived_ms".into(), json!(now));
        if let Some(l) = b.landed {
            o.insert("landed".into(), l);
        }
        let dir = home.join("mechanics").join(game).join(&name);
        if dir.join("session.toml").is_file() {
            let count = |sub: &str, ext: &str| {
                fs::read_dir(dir.join(sub)).map_or(0, |d| {
                    d.flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
                        .count()
                })
            };
            let turns = fs::read_to_string(dir.join("talk.ndjson"))
                .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());
            o.insert(
                "session".into(),
                json!({
                    "dir": dir.display().to_string(),
                    "clips": count("clips", "toml"),
                    "turns": turns,
                }),
            );
        }
    }
    r
}

/// Takes the first archived row of `game` that `pick` chooses out of the
/// archive and gives it back, without its archive fields, for the caller
/// to put in the quest log; None when the archive has none.
pub(crate) fn bring_back(
    home: &Path,
    game: &str,
    pick: impl Fn(&Value) -> bool,
) -> Result<Option<Value>> {
    let mut a = read_archive(home, game);
    let Some(i) = a.iter().position(pick) else {
        return Ok(None);
    };
    let mut r = a.remove(i);
    write_archive(home, game, &a)?;
    if let Some(o) = r.as_object_mut() {
        for k in ["closed_ms", "archived_ms", "session"] {
            o.remove(k);
        }
    }
    Ok(Some(r))
}

/// The archived rows `query` asks for, newest closed first: all of them
/// with none, the row of that name when there is one, else each row whose
/// name, note, asked words or facts hold the words (case left aside).
pub(crate) fn find(home: &Path, game: &str, query: Option<&str>) -> Vec<Value> {
    let mut a = read_archive(home, game);
    a.sort_by_key(|r| std::cmp::Reverse(r["closed_ms"].as_u64().unwrap_or(0)));
    let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) else {
        return a;
    };
    if let Some(r) = a.iter().find(|r| r["mechanic"] == json!(q)) {
        return vec![r.clone()];
    }
    let q = q.to_lowercase();
    a.into_iter()
        .filter(|r| {
            let mut text = String::new();
            for k in ["mechanic", "note", "asked", "from"] {
                text.push_str(r[k].as_str().unwrap_or(""));
                text.push('\n');
            }
            for k in ["known", "read", "open"] {
                for x in r[k].as_array().into_iter().flatten() {
                    text.push_str(x.as_str().unwrap_or(""));
                    text.push('\n');
                }
            }
            text.to_lowercase().contains(&q)
        })
        .collect()
}

/// How long ago `then` was at `now` (ms), in plain words.
fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then) / 1000;
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", s / 60),
        3600..7200 => "1 hour ago".to_owned(),
        7200..86400 => format!("{} hours ago", s / 3600),
        86400..172800 => "yesterday".to_owned(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// The archived rows in words, one paragraph each.
pub(crate) fn words(rows: &[Value], now: u64) -> String {
    if rows.is_empty() {
        return "no archived row".to_owned();
    }
    rows.iter()
        .map(|r| {
            let mut l = format!(
                "{}: {}, closed {}.",
                r["mechanic"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or(""),
                r["closed_ms"]
                    .as_u64()
                    .map_or_else(|| "at an unknown time".to_owned(), |c| ago(now, c)),
            );
            if let Some(n) = r["note"].as_str().filter(|n| !n.is_empty()) {
                l.push_str(&format!("\n    {n}"));
            }
            if let Some(a) = r["asked"].as_str().filter(|a| !a.is_empty()) {
                l.push_str(&format!("\n    asked: {a}"));
            }
            for (k, say) in [("known", "known"), ("read", "read"), ("open", "open")] {
                for x in r[k].as_array().into_iter().flatten() {
                    if let Some(x) = x.as_str() {
                        l.push_str(&format!("\n    {say}: {x}"));
                    }
                }
            }
            if let Some(c) = r["landed"]["commit"].as_str() {
                l.push_str(&format!(
                    "\n    landed {c} on its card {}",
                    r["landed"]["card"].as_str().unwrap_or("")
                ));
            }
            if r["session"].is_object() {
                l.push_str(&format!(
                    "\n    session: {} clips, {} turns, in {}",
                    r["session"]["clips"],
                    r["session"]["turns"],
                    r["session"]["dir"].as_str().unwrap_or("")
                ));
            }
            l
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The commit a landed card's lines name (the gate's "PASS: landed K on
/// main"), if any.
pub(crate) fn commit_of(lines: &Value) -> Option<String> {
    lines
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|l| {
            let (_, rest) = l.split_once("landed ")?;
            let k = rest.split_whitespace().next()?;
            (k.len() >= 7 && k.bytes().all(|b| b.is_ascii_hexdigit())).then(|| k.to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 60 * 60 * 1000;

    fn home_with(rows: &[Value]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("mechanics/hop");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue.json"), serde_json::to_string(rows).unwrap()).unwrap();
        d
    }

    /// A row done for an hour with nothing going on it moves to the
    /// archive with its notes, session and landed commit; a row done
    /// lately, one with a worker or an open card, and one not done stay.
    #[test]
    fn a_row_done_for_an_hour_moves_to_the_archive() {
        let now = 10 * HOUR;
        let old = now - 2 * HOUR;
        let d = home_with(&[
            json!({"mechanic": "jump", "status": "done", "note": "Landed on your YES: the jump.", "known": ["66 px"], "changed_ms": old}),
            json!({"mechanic": "run", "status": "done", "note": "done lately", "changed_ms": now - 10 * 60 * 1000}),
            json!({"mechanic": "climb", "status": "done", "changed_ms": old}),
            json!({"mechanic": "duck", "status": "done", "changed_ms": old}),
            json!({"mechanic": "swim", "status": "next", "changed_ms": old}),
        ]);
        let h = d.path();
        let s = h.join("mechanics/hop/jump");
        fs::create_dir_all(s.join("clips")).unwrap();
        fs::write(s.join("session.toml"), "game = \"hop\"\n").unwrap();
        fs::write(s.join("clips/001.toml"), "").unwrap();
        fs::write(s.join("clips/001.log"), "").unwrap();
        fs::write(s.join("talk.ndjson"), "{}\n{}\n").unwrap();
        let moved = sweep(h, "hop", now, |row| Beside {
            worker: row == "climb",
            card: row == "duck",
            landed: (row == "jump")
                .then(|| json!({"card": "a-1", "what": "the jump", "commit": "abc1234"})),
        })
        .unwrap();
        assert_eq!(moved, vec!["jump"]);
        let q = read_queue(&h.join("mechanics/hop/queue.json"));
        let left: Vec<&str> = q.iter().filter_map(|r| r["mechanic"].as_str()).collect();
        assert_eq!(left, vec!["run", "climb", "duck", "swim"]);
        let a = read_archive(h, "hop");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["closed_ms"], json!(old));
        assert_eq!(a[0]["archived_ms"], json!(now));
        assert_eq!(a[0]["landed"]["commit"], "abc1234");
        assert_eq!(a[0]["session"]["clips"], 1);
        assert_eq!(a[0]["session"]["turns"], 2);
        let w = words(&find(h, "hop", Some("jump")), now);
        assert!(w.contains("jump: done, closed 2 hours ago."), "{w}");
        assert!(w.contains("known: 66 px"), "{w}");
        assert!(w.contains("landed abc1234 on its card a-1"), "{w}");
        assert!(w.contains("1 clips, 2 turns"), "{w}");
        // Text search finds it by its note; a word nowhere finds nothing.
        assert_eq!(find(h, "hop", Some("YOUR yes")).len(), 1);
        assert!(find(h, "hop", Some("nowhere")).is_empty());
        // Once the worker and the card are gone their rows move too; swept
        // again, nothing more moves.
        assert_eq!(
            sweep(h, "hop", now, |_| Beside::default()).unwrap(),
            vec!["climb", "duck"]
        );
        assert!(
            sweep(h, "hop", now, |_| Beside::default())
                .unwrap()
                .is_empty()
        );
    }

    /// A row brought back leaves the archive without its archive fields.
    #[test]
    fn an_archived_row_comes_back() {
        let d = home_with(&[]);
        let h = d.path();
        write_archive(
            h,
            "hop",
            &[json!({"mechanic": "jump", "status": "done", "closed_ms": 1, "archived_ms": 2, "session": {}})],
        )
        .unwrap();
        assert!(
            bring_back(h, "hop", |r| r["mechanic"] == "run")
                .unwrap()
                .is_none()
        );
        let r = bring_back(h, "hop", |r| r["mechanic"] == "jump")
            .unwrap()
            .unwrap();
        assert_eq!(r, json!({"mechanic": "jump", "status": "done"}));
        assert!(read_archive(h, "hop").is_empty());
    }

    #[test]
    fn a_landed_cards_commit_is_read_from_its_lines() {
        assert_eq!(
            commit_of(&json!([
                "checks ok",
                "PASS: landed 1a2b3c4d on main",
                "(ran in /x)"
            ])),
            Some("1a2b3c4d".to_owned())
        );
        assert_eq!(commit_of(&json!(["PASS, but not landed: x"])), None);
    }
}
