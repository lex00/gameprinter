//! What the agent is doing, for the person to watch beside the game: a
//! timestamped log whose newest line is its status, the clips and picture
//! sheets it pins, and a table of the problems it has found with the state
//! of each. Kept in the session folder under `watch/`, written only by the
//! agent's commands (`gp mechanic log`, `pin`, `problem`) and read by the
//! page once a second. From the watch page of the clay theme work
//! (~/.game-printer/work/clay/hires/live/watch.html), which the owner found
//! the most useful way to follow an agent at work.
//!
//! | File | What |
//! |---|---|
//! | `watch/log.ndjson` | `{"at": "HH:MM:SS", "text": ...}` a line |
//! | `watch/pins.json` | `[{"n", "file", "title", "keep"}]`: newest last |
//! | `watch/files/` | The pinned files, copied in under `N-NAME` |
//! | `watch/problems.json` | `[{"n", "where", "what", "fix", "state"}]` |

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

/// The kinds of file a pin may be, by extension, and how they're served.
pub(crate) const KINDS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
];

/// The largest file a pin copies in.
const MAX_PIN: u64 = 64 << 20;
/// The log lines the page is sent, newest last.
const LOG_LINES: usize = 200;
/// The pins not kept at the top that the page shows, newest first: older
/// ones drop off, as finished clips did on the watch page.
const RECENT: usize = 6;

pub(crate) const STATES: &[&str] = &["open", "working", "fixed", "wontfix"];

fn dir(session: &Path) -> PathBuf {
    session.join("watch")
}

fn read_list(f: &Path) -> Vec<Value> {
    fs::read_to_string(f)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
        .unwrap_or_default()
}

fn write_list(f: &Path, v: &[Value]) -> Result<()> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = f.with_extension("json.partial");
    fs::write(&tmp, serde_json::to_string_pretty(v)?)?;
    fs::rename(&tmp, f).with_context(|| format!("writing {}", f.display()))?;
    Ok(())
}

/// The time of day in UTC, `HH:MM:SS`.
fn clock() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Appends a line to the log: what the agent is doing now.
pub(crate) fn log(session: &Path, text: &str) -> Result<Value> {
    let text = text.trim();
    if text.is_empty() {
        bail!("say what you're doing");
    }
    let d = dir(session);
    fs::create_dir_all(&d)?;
    let line = json!({"at": clock(), "text": text});
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(d.join("log.ndjson"))?;
    std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes())?;
    Ok(line)
}

/// Copies a picture or a clip into the session and pins it, at the top of
/// the list when `keep`.
pub(crate) fn pin(session: &Path, file: &Path, title: &str, keep: bool) -> Result<Value> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !KINDS.iter().any(|(k, _)| *k == ext) {
        bail!(
            "{} isn't a picture or a clip the page shows: one of {}",
            file.display(),
            KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
        );
    }
    let size = fs::metadata(file)
        .with_context(|| format!("reading {}", file.display()))?
        .len();
    if size > MAX_PIN {
        bail!(
            "{} is {size} bytes, more than the {MAX_PIN} a pin takes",
            file.display()
        );
    }
    let d = dir(session);
    let list_file = d.join("pins.json");
    let mut list = read_list(&list_file);
    let n = list
        .iter()
        .filter_map(|p| p["n"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let stem: String = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pin")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    let name = format!("{n:03}-{stem}.{ext}");
    fs::create_dir_all(d.join("files"))?;
    fs::copy(file, d.join("files").join(&name))
        .with_context(|| format!("copying {}", file.display()))?;
    let row = json!({"n": n, "file": name, "title": title.trim(), "keep": keep});
    list.push(row.clone());
    write_list(&list_file, &list)?;
    Ok(row)
}

/// Takes pin `n` off the list. Its file stays in the folder.
pub(crate) fn unpin(session: &Path, n: u64) -> Result<()> {
    let f = dir(session).join("pins.json");
    let mut list = read_list(&f);
    let before = list.len();
    list.retain(|p| p["n"].as_u64() != Some(n));
    if list.len() == before {
        bail!("no pin {n}");
    }
    write_list(&f, &list)
}

/// What `gp mechanic problem` was asked: a new row, or with `n` a change to
/// one.
#[derive(Default)]
pub(crate) struct Problem<'a> {
    pub n: Option<u64>,
    pub place: Option<&'a str>,
    pub what: Option<&'a str>,
    pub fix: Option<&'a str>,
    pub state: Option<&'a str>,
}

/// Adds a row to the problem table, or changes the fields given of row `n`.
pub(crate) fn problem(session: &Path, p: &Problem<'_>) -> Result<Value> {
    if let Some(s) = p.state
        && !STATES.contains(&s)
    {
        bail!("a problem's state is one of {}, not {s}", STATES.join(", "));
    }
    let f = dir(session).join("problems.json");
    let mut list = read_list(&f);
    let row = match p.n {
        Some(n) => {
            let Some(row) = list.iter_mut().find(|r| r["n"].as_u64() == Some(n)) else {
                bail!("no problem {n}");
            };
            for (k, v) in [
                ("where", p.place),
                ("what", p.what),
                ("fix", p.fix),
                ("state", p.state),
            ] {
                if let Some(v) = v {
                    row[k] = json!(v);
                }
            }
            row.clone()
        }
        None => {
            let (Some(place), Some(what)) = (p.place, p.what) else {
                bail!("a new problem needs --where and --what");
            };
            let n = list
                .iter()
                .filter_map(|r| r["n"].as_u64())
                .max()
                .unwrap_or(0)
                + 1;
            let row = json!({
                "n": n, "where": place, "what": what,
                "fix": p.fix.unwrap_or(""), "state": p.state.unwrap_or("open"),
            });
            list.push(row.clone());
            row
        }
    };
    write_list(&f, &list)?;
    Ok(row)
}

/// What the page shows: the status (the newest log line), the log, the
/// pins to show (kept ones, then the newest few others) and the problems.
pub(crate) fn view(session: &Path) -> Value {
    let d = dir(session);
    let lines: Vec<Value> = fs::read_to_string(d.join("log.ndjson"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let log: Vec<Value> = lines[lines.len().saturating_sub(LOG_LINES)..].to_vec();
    let pins = read_list(&d.join("pins.json"));
    let mut shown: Vec<Value> = pins
        .iter()
        .filter(|p| p["keep"] == json!(true))
        .cloned()
        .collect();
    shown.extend(
        pins.iter()
            .rev()
            .filter(|p| p["keep"] != json!(true))
            .take(RECENT)
            .cloned(),
    );
    for p in &mut shown {
        let name = p["file"].as_str().unwrap_or_default();
        let kind = kind_of(name).unwrap_or("");
        p["kind"] = json!(if kind.starts_with("video/") {
            "video"
        } else {
            "image"
        });
    }
    json!({
        "status": log.last().cloned(),
        "log": log,
        "pins": shown,
        "problems": read_list(&d.join("problems.json")),
    })
}

/// Milliseconds since 1970 of a file's last change, 0 when it can't be read.
fn modified(f: &Path) -> u64 {
    fs::metadata(f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `HH:MM:SS` as seconds into the day.
fn secs_of(at: &str) -> Option<u64> {
    let mut p = at.split(':').map(|x| x.parse::<u64>().ok());
    let (h, m, s) = (p.next()??, p.next()??, p.next()??);
    Some(h * 3600 + m * 60 + s)
}

const DAY_MS: u64 = 86_400_000;

/// A topic's log lines with the time each was written (milliseconds since
/// 1970), newest last. A line keeps only its time of day, so the day comes
/// from the file: the last line was written when the file last changed (to
/// the millisecond, which orders lines of two topics in the same second),
/// and walking back, a time later than the line after it was the day
/// before.
fn dated_log(f: &Path) -> Vec<(u64, Value)> {
    let lines: Vec<Value> = fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let changed = modified(f);
    let mut next = changed + 60_000;
    let mut out: Vec<(u64, Value)> = lines
        .into_iter()
        .rev()
        .enumerate()
        .map(|(i, line)| {
            if let Some(s) = line["at"].as_str().and_then(secs_of) {
                let mut t = next / DAY_MS * DAY_MS + s * 1000;
                if t > next {
                    t = t.saturating_sub(DAY_MS);
                }
                if i == 0 && (t..t + 1000).contains(&changed) {
                    t = changed;
                }
                next = t;
            }
            (next, line)
        })
        .collect();
    out.reverse();
    out
}

/// The topics of a game with watch files: its session folders that have
/// `watch/`, and `open` (the mechanic open in the page) though it has none.
fn topics(game: &Path, open: &str) -> Vec<String> {
    let mut all: Vec<String> = fs::read_dir(game)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("watch").is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                .filter(|n| super::session::name_ok(n))
                .collect()
        })
        .unwrap_or_default();
    if super::session::name_ok(open) && !all.iter().any(|t| t == open) {
        all.push(open.to_owned());
    }
    all.sort();
    all
}

/// The problem states still to do.
fn unresolved(row: &Value) -> bool {
    matches!(row["state"].as_str(), Some("open" | "working") | None)
}

/// What the page's panel shows for the whole game: every topic's log under
/// `GAME/*/watch`, merged newest first with each line's topic; every topic's
/// pins (kept ones, then the newest few of each); every topic's problem
/// rows, `todo` on those still open or being worked on (the panel shows
/// those of every topic and all of `open`'s, the mechanic open in the
/// page); and the topics, with how many lines each has, when it last wrote (in
/// milliseconds since 1970) and how many problems it has still to do.
pub(crate) fn game_view(game: &Path, open: &str) -> Value {
    let mut log: Vec<(u64, usize, Value)> = Vec::new();
    let mut kept: Vec<Value> = Vec::new();
    let mut recent: Vec<(u64, Value)> = Vec::new();
    let mut problems: Vec<Value> = Vec::new();
    let mut listed: Vec<Value> = Vec::new();
    for t in topics(game, open) {
        let d = game.join(&t).join("watch");
        let mut lines = dated_log(&d.join("log.ndjson"));
        let count = lines.len();
        let last = lines.last().map_or(0, |(at, _)| *at);
        lines.drain(..lines.len().saturating_sub(LOG_LINES));
        for (at, mut line) in lines {
            line["topic"] = json!(t);
            log.push((at, log.len(), line));
        }
        let pins_file = d.join("pins.json");
        let pinned_at = modified(&pins_file);
        let pins = read_list(&pins_file);
        let mut mark = |mut p: Value| {
            let name = p["file"].as_str().unwrap_or_default();
            let video = kind_of(name).is_some_and(|k| k.starts_with("video/"));
            p["kind"] = json!(if video { "video" } else { "image" });
            p["topic"] = json!(t);
            p
        };
        kept.extend(
            pins.iter()
                .filter(|p| p["keep"] == json!(true))
                .cloned()
                .map(&mut mark),
        );
        for (i, p) in pins
            .iter()
            .rev()
            .filter(|p| p["keep"] != json!(true))
            .take(RECENT)
            .enumerate()
        {
            // Newest of the topic first; topics by their newest pin.
            recent.push((pinned_at.saturating_sub(i as u64), mark(p.clone())));
        }
        let rows = read_list(&d.join("problems.json"));
        let still = rows.iter().filter(|r| unresolved(r)).count();
        for mut r in rows {
            r["topic"] = json!(t);
            r["todo"] = json!(unresolved(&r));
            problems.push(r);
        }
        listed.push(json!({"topic": t, "lines": count, "last": last, "problems": still}));
    }
    // Newest first; of lines in the same second, the one written after.
    log.sort_by_key(|l| std::cmp::Reverse((l.0, l.1)));
    recent.sort_by_key(|p| std::cmp::Reverse(p.0));
    let log: Vec<Value> = log.into_iter().map(|(_, _, l)| l).collect();
    kept.extend(recent.into_iter().map(|(_, p)| p));
    json!({
        "open": open,
        "status": log.first().cloned(),
        "log": log,
        "pins": kept,
        "problems": problems,
        "topics": listed,
    })
}

/// A pinned file of any topic of the game, by the topic and its name.
pub(crate) fn topic_file(game: &Path, topic: &str, name: &str) -> Option<(&'static str, Vec<u8>)> {
    if !super::session::name_ok(topic) {
        return None;
    }
    file(&game.join(topic), name)
}

/// A pinned file's media type, by its extension; none for a name that isn't
/// one a pin writes (so nothing else in the session is served).
pub(crate) fn kind_of(name: &str) -> Option<&'static str> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return None;
    }
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    KINDS.iter().find(|(k, _)| *k == ext).map(|(_, t)| *t)
}

/// A pinned file's bytes and type, by its name in `watch/files/`.
pub(crate) fn file(session: &Path, name: &str) -> Option<(&'static str, Vec<u8>)> {
    let kind = kind_of(name)?;
    let bytes = fs::read(dir(session).join("files").join(name)).ok()?;
    Some((kind, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_pins_and_problems_make_the_view() {
        let d = tempfile::tempdir().unwrap();
        let s = d.path().join("session");
        fs::create_dir_all(&s).unwrap();
        assert!(view(&s)["status"].is_null());
        log(&s, "Recording the first clips").unwrap();
        log(&s, "Measuring the jump").unwrap();
        assert!(log(&s, "  ").is_err());
        let v = view(&s);
        assert_eq!(v["status"]["text"], "Measuring the jump");
        assert_eq!(v["log"].as_array().unwrap().len(), 2);

        // Pins: a kept one stays at the top; of the rest only the newest few.
        let pic = d.path().join("sheet one.png");
        fs::write(&pic, b"not really a png").unwrap();
        let kept = pin(&s, &pic, "The sheet", true).unwrap();
        for i in 0..8 {
            pin(&s, &pic, &format!("clip {i}"), false).unwrap();
        }
        let v = view(&s);
        let pins = v["pins"].as_array().unwrap();
        assert_eq!(pins.len(), 1 + RECENT);
        assert_eq!(pins[0]["n"], kept["n"]);
        assert_eq!(pins[1]["title"], "clip 7");
        assert_eq!(pins[0]["kind"], "image");
        let name = kept["file"].as_str().unwrap();
        assert_eq!(name, "001-sheet-one.png");
        assert_eq!(file(&s, name).unwrap().0, "image/png");
        unpin(&s, 1).unwrap();
        assert!(unpin(&s, 1).is_err());
        assert!(pin(&s, &d.path().join("notes.txt"), "x", false).is_err());

        // Problems: added, then changed by number.
        let p = problem(
            &s,
            &Problem {
                place: Some("25m"),
                what: Some("the jump is 2 px short"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p["state"], "open");
        problem(
            &s,
            &Problem {
                n: Some(1),
                state: Some("fixed"),
                fix: Some("gravity 0.25"),
                ..Default::default()
            },
        )
        .unwrap();
        let v = view(&s);
        assert_eq!(v["problems"][0]["state"], "fixed");
        assert_eq!(v["problems"][0]["what"], "the jump is 2 px short");
        assert!(
            problem(
                &s,
                &Problem {
                    n: Some(1),
                    state: Some("done?"),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(
            problem(
                &s,
                &Problem {
                    what: Some("no place"),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    /// The game's view: every topic's log merged newest first with its
    /// topic, pins from every topic, and the rows still to do in the other
    /// topics beside all of the open one's.
    #[test]
    fn the_game_view_takes_every_topics_watch_files() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("tapper");
        let (title, emu) = (game.join("title"), game.join("emulator"));
        fs::create_dir_all(title.join("watch")).unwrap();
        fs::create_dir_all(emu.join("watch")).unwrap();
        fs::write(
            emu.join("watch/log.ndjson"),
            "{\"at\":\"23:59:50\",\"text\":\"Booting the Z80\"}\n{\"at\":\"00:00:05\",\"text\":\"The bar draws\"}\n",
        )
        .unwrap();
        log(&title, "Recording the title").unwrap();
        let pic = d.path().join("frame.png");
        fs::write(&pic, b"png").unwrap();
        pin(&emu, &pic, "the bar", false).unwrap();
        for (s, what, state) in [
            (&emu, "sprites off by one", "open"),
            (&emu, "old", "fixed"),
            (&title, "fades late", "fixed"),
        ] {
            let n = problem(
                s,
                &Problem {
                    place: Some("x"),
                    what: Some(what),
                    ..Default::default()
                },
            )
            .unwrap()["n"]
                .as_u64();
            problem(
                s,
                &Problem {
                    n,
                    state: Some(state),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        // Not a topic: no watch folder.
        fs::create_dir_all(game.join("move")).unwrap();

        let v = game_view(&game, "title");
        let log: Vec<(&str, &str)> = v["log"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| (l["topic"].as_str().unwrap(), l["text"].as_str().unwrap()))
            .collect();
        // The emulator's lines span midnight: the later one is the newer.
        let (a, b) = (
            log.iter().position(|l| l.1 == "The bar draws").unwrap(),
            log.iter().position(|l| l.1 == "Booting the Z80").unwrap(),
        );
        assert!(a < b, "{log:?}");
        assert_eq!(log.len(), 3);
        assert!(log.contains(&("title", "Recording the title")));
        assert_eq!(v["status"], v["log"][0]);
        assert_eq!(v["pins"][0]["topic"], "emulator");
        let rows: Vec<(&str, bool)> = v["problems"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["what"].as_str().unwrap(), r["todo"] == true))
            .collect();
        assert_eq!(
            rows,
            [
                ("sprites off by one", true),
                ("old", false),
                ("fades late", false)
            ]
        );
        assert_eq!(v["topics"][0]["problems"], 1);
        let topics: Vec<&str> = v["topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["topic"].as_str().unwrap())
            .collect();
        assert_eq!(topics, ["emulator", "title"]);
        assert_eq!(
            topic_file(&game, "emulator", "001-frame.png").unwrap().1,
            b"png"
        );
        assert!(topic_file(&game, "..", "001-frame.png").is_none());
    }

    #[test]
    fn only_pinned_kinds_of_file_are_served() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("watch/files")).unwrap();
        fs::write(d.path().join("watch/files/001-a.png"), b"x").unwrap();
        fs::write(d.path().join("session.toml"), b"x").unwrap();
        assert!(file(d.path(), "001-a.png").is_some());
        for bad in [
            "../session.toml",
            "..%2Fsession.toml",
            ".png",
            "a/b.png",
            "x.toml",
            "",
        ] {
            assert!(file(d.path(), bad).is_none(), "{bad}");
        }
    }
}
