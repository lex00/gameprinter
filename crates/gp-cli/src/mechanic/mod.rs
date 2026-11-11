//! `gp mechanic`: one mechanic of one game, worked on by a person and their
//! agent. The person plays the game in the browser, records short clips of
//! the mechanic with a note each and hands them off; the agent reads the
//! session with `gp mechanic show`, answers with `gp mechanic say`, and asks
//! for the next clip.
//!
//! A session is a folder, `$GP_HOME/mechanics/GAME/MECHANIC/`:
//!
//! | File | What it holds |
//! |---|---|
//! | `session.toml` | The game, the mechanic, the person's words about it, and the ROM's SHA-256 |
//! | `clips/NNN.log` | A clip's own inputs (or, written the old way, its input log from power-on) |
//! | `clips/NNN.toml` | Its path and start frame, its frames, the state hash there, who made it and the note |
//! | `talk.ndjson` | The conversation, one line a turn: who, the words, the clips it hands off |
//! | `busy.json` | What the agent said it is doing, until its next turn |
//! | `watch/` | What the agent is doing, for the page's watch panel: its log, pins and problem table ([`watch`]) |
//!
//! Beside the sessions, `$GP_HOME/mechanics/GAME/queue.json` is the game's
//! quest log (seeded by `gp game new` from [`CHECKLISTS`]) and `current`
//! names the mechanic open in the page. Paths, the plays clips start from,
//! are kept once under `$GP_HOME/work/paths/` ([`paths`]).
//!
//! Clips are recipes, never pictures: anyone with the ROM plays them again.
//! `gp mechanic start` serves the page (`mechanic/` in the repo) on this
//! machine only, with the player module and the ROM ([`server`]).
//!
//! Ported from game-printer's mechanic.rs: the core loop only. The analysis
//! commands (contacts, sweep, probe, leads, harvest, demo, compare, video),
//! sealed runs, the corpus import and the rebuild review aren't here.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde::Deserialize;
use serde_json::{Value, json};

mod http;
pub(crate) mod paths;
pub(crate) mod review;
mod server;
mod session;
pub(crate) mod watch;

use crate::Outcome;
pub(crate) use session::name_ok;
use session::{Session, current, queue_file, read_queue, show_text, write_queue};

#[derive(Subcommand)]
pub enum MechanicCmd {
    /// Open a mechanic's session and serve its page on 127.0.0.1: the game
    /// to play, record clips of the mechanic in and hand them to your agent.
    /// Makes the session if it is new. Runs until stopped.
    Start {
        /// The game, as its package folder in the games checkout is named.
        game: String,
        /// The mechanic, one word or a few with dashes (jump). By default the
        /// one open last, else the first quest log row not done.
        mechanic: Option<String>,
        /// What the mechanic is, in your words, for a new session.
        #[arg(long)]
        prompt: Option<String>,
        /// The port, on 127.0.0.1 (0 picks a free one).
        #[arg(long, default_value_t = 7710)]
        port: u16,
        /// The games checkout (default: $GP_GAMES_DIR, else a `games` folder
        /// at or beside the current folder or one above it)
        #[arg(long)]
        games: Option<PathBuf>,
        /// The game's built module (`gp game build`), for reviewing clips on
        /// the model (default: $GP_HOME/work/modules/GAME.wasm if it's there)
        #[arg(long)]
        module: Option<PathBuf>,
    },
    /// Say what you're doing while it's your turn ("measuring the jump"):
    /// the person's page shows it until your next turn.
    Busy {
        game: String,
        mechanic: String,
        text: String,
    },
    /// Add a line to the log the person watches beside the game: what you're
    /// doing now. The newest line is your status.
    Log {
        game: String,
        mechanic: String,
        text: String,
    },
    /// Pin a picture or a clip (png, jpg, gif, webp, mp4, webm) for the person
    /// to see beside the game: a sheet of frames, the original against the
    /// model. It's copied into the session. The newest few show; `--keep`
    /// keeps one at the top. `--drop N` takes pin N off instead.
    Pin {
        game: String,
        mechanic: String,
        /// The file to pin.
        #[arg(required_unless_present = "drop")]
        file: Option<PathBuf>,
        /// What it shows, in a line.
        #[arg(long, default_value = "")]
        title: String,
        /// Keep it at the top.
        #[arg(long)]
        keep: bool,
        /// Take pin N off the list.
        #[arg(long, conflicts_with_all = ["file", "keep"])]
        drop: Option<u64>,
    },
    /// Add a row to the problem table the person watches: where, what is
    /// wrong, the change that fixes it and its state (open, working, fixed,
    /// wontfix). With `--n`, changes the fields given of row N.
    Problem {
        game: String,
        mechanic: String,
        /// The row to change.
        #[arg(long)]
        n: Option<u64>,
        /// Where it shows ("25m, the second ladder").
        #[arg(long = "where")]
        place: Option<String>,
        /// What is wrong.
        #[arg(long)]
        what: Option<String>,
        /// The change that fixes it.
        #[arg(long)]
        fix: Option<String>,
        /// open, working, fixed or wontfix (a new row is open).
        #[arg(long)]
        state: Option<String>,
    },
    /// Wait until it is the agent's turn (the person handed off a clip or
    /// answered), then print the session as `show` does. Run it in the
    /// background after each turn you give the person. Without a mechanic,
    /// waits on the one the person has open, and also returns when they pick
    /// another from the quest log.
    Wait {
        game: String,
        mechanic: Option<String>,
    },
    /// Show what a session holds, for the agent: the mechanic and its clips
    /// with their notes, the conversation and whose turn it is.
    Show { game: String, mechanic: String },
    /// Say something to the person in a session: an answer, what is now
    /// known, a question or the clip you want next. It shows over their game,
    /// which pauses until they answer. With `--choice` they pick one with the
    /// controller. A clip replays under the words when `--show` is given.
    Say {
        game: String,
        mechanic: String,
        text: String,
        /// A choice they can pick (2 to 4 of them).
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// A clip to replay in the window while they answer.
        #[arg(long)]
        show: Option<u64>,
        /// A clip to compare the shown one with: the page plays both from
        /// their starts, in step, as an overlay, side by side, or a diff.
        #[arg(long, requires = "show")]
        against: Option<u64>,
        /// A fact worked out from the measurements ("71 px is 5 above the
        /// standing jump's 66"), shown to the person under the words.
        #[arg(long = "fact")]
        facts: Vec<String>,
    },
    /// The game's quest log, shown beside the game: each mechanic with its
    /// status (done, asking you, agent working, in line, next) and one plain
    /// line. With `--status`, adds the mechanic or changes it; without,
    /// prints the quest log.
    Queue {
        game: String,
        mechanic: Option<String>,
        #[arg(long, requires = "mechanic", value_parser = ["done", "asking you", "agent working", "in line", "next"])]
        status: Option<String>,
        /// One plain line: what is known, or what it's waiting on.
        #[arg(long, requires = "status")]
        note: Option<String>,
        /// Something measured, exact (repeat for each). Replaces the list.
        #[arg(long, requires = "mechanic")]
        known: Vec<String>,
        /// The agent's reading of a measurement, a name or a cause the
        /// tool didn't find itself: worth a check by eye.
        #[arg(long = "read", requires = "mechanic")]
        reads: Vec<String>,
        /// What isn't found yet, and why.
        #[arg(long = "open", requires = "mechanic")]
        opens: Vec<String>,
        /// Mark the row out of the corpus's scope, with why.
        #[arg(
            long,
            value_name = "WHY",
            requires = "mechanic",
            conflicts_with = "in_scope"
        )]
        out_of_scope: Option<String>,
        /// Take the out-of-scope mark off the row.
        #[arg(long, requires = "mechanic")]
        in_scope: bool,
    },
}

pub(crate) fn run(cmd: &MechanicCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        MechanicCmd::Start {
            game,
            mechanic,
            prompt,
            port,
            games,
            module,
        } => server::start(
            paths,
            &server::Start {
                game,
                mechanic: mechanic.as_deref(),
                prompt: prompt.as_deref(),
                port: *port,
                games: games.as_deref(),
                module: module.as_deref(),
            },
        ),
        MechanicCmd::Busy {
            game,
            mechanic,
            text,
        } => {
            Session::open(paths, game, mechanic)?.set_busy(text)?;
            Ok(Outcome::ok(json!({}), format!("busy: {text}")))
        }
        MechanicCmd::Log {
            game,
            mechanic,
            text,
        } => {
            let line = watch::log(&Session::open(paths, game, mechanic)?.dir, text)?;
            Ok(Outcome::ok(
                line.clone(),
                format!("{} {}", line["at"].as_str().unwrap_or(""), text.trim()),
            ))
        }
        MechanicCmd::Pin {
            game,
            mechanic,
            file,
            title,
            keep,
            drop,
        } => {
            let dir = Session::open(paths, game, mechanic)?.dir;
            if let Some(n) = drop {
                watch::unpin(&dir, *n)?;
                return Ok(Outcome::ok(
                    json!({"dropped": n}),
                    format!("pin {n} taken off"),
                ));
            }
            let Some(file) = file else {
                bail!("give a file to pin, or --drop N");
            };
            let row = watch::pin(&dir, file, title, *keep)?;
            Ok(Outcome::ok(
                row.clone(),
                format!("pin {}: {}", row["n"], row["file"].as_str().unwrap_or("")),
            ))
        }
        MechanicCmd::Problem {
            game,
            mechanic,
            n,
            place,
            what,
            fix,
            state,
        } => {
            let row = watch::problem(
                &Session::open(paths, game, mechanic)?.dir,
                &watch::Problem {
                    n: *n,
                    place: place.as_deref(),
                    what: what.as_deref(),
                    fix: fix.as_deref(),
                    state: state.as_deref(),
                },
            )?;
            Ok(Outcome::ok(
                row.clone(),
                format!(
                    "problem {} ({}): {}: {}",
                    row["n"],
                    row["state"].as_str().unwrap_or(""),
                    row["where"].as_str().unwrap_or(""),
                    row["what"].as_str().unwrap_or("")
                ),
            ))
        }
        MechanicCmd::Wait { game, mechanic } => wait(paths, game, mechanic.as_deref()),
        MechanicCmd::Show { game, mechanic } => {
            let v = Session::open(paths, game, mechanic)?.state()?;
            Ok(Outcome::ok(v.clone(), show_text(&v)))
        }
        MechanicCmd::Say {
            game,
            mechanic,
            text,
            choices,
            show,
            against,
            facts,
        } => {
            if choices.len() == 1 || choices.len() > 4 {
                bail!("give 2 to 4 choices, or none");
            }
            let s = Session::open(paths, game, mechanic)?;
            for c in show.iter().chain(against) {
                if !s.dir.join(format!("clips/{c:03}.toml")).is_file() {
                    bail!("no clip {c} in this session");
                }
            }
            let n = s.turn(json!({"by": "agent", "text": text, "choices": choices,
                "show": show, "against": against, "facts": facts}))?;
            Ok(Outcome::ok(json!({"turn": n}), format!("said (turn {n})")))
        }
        MechanicCmd::Queue {
            game,
            mechanic,
            status,
            note,
            known,
            reads,
            opens,
            out_of_scope,
            in_scope,
        } => queue(
            paths,
            game,
            mechanic.as_deref(),
            &RowChange {
                status: status.as_deref(),
                note: note.as_deref(),
                known,
                reads,
                opens,
                out_of_scope: out_of_scope.as_deref(),
                in_scope: *in_scope,
            },
        ),
    }
}

/// `gp mechanic wait`: polls the session until it is the agent's turn.
fn wait(paths: &Paths, game: &str, mechanic: Option<&str>) -> Result<Outcome> {
    let open_now = || -> Result<String> {
        match mechanic {
            Some(m) => Ok(m.to_owned()),
            None => current(paths, game),
        }
    };
    let m0 = open_now()?;
    let s = Session::open(paths, game, &m0)?;
    let from = s.turns()?.len();
    loop {
        let m = open_now()?;
        if m != m0 {
            let v = Session::open(paths, game, &m)?.state()?;
            return Ok(Outcome::ok(v.clone(), show_text(&v)));
        }
        let v = s.state()?;
        let n = v["talk"].as_array().map_or(0, Vec::len);
        if n > from && v["waiting_for"] == "agent" {
            return Ok(Outcome::ok(v.clone(), show_text(&v)));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// What `gp mechanic queue` changes on a row.
struct RowChange<'a> {
    status: Option<&'a str>,
    note: Option<&'a str>,
    known: &'a [String],
    reads: &'a [String],
    opens: &'a [String],
    out_of_scope: Option<&'a str>,
    in_scope: bool,
}

fn queue(paths: &Paths, game: &str, mechanic: Option<&str>, c: &RowChange) -> Result<Outcome> {
    let f = queue_file(paths, game)?;
    let mut q = read_queue(&f);
    if let Some(m) = mechanic
        && (c.status.is_some()
            || !c.known.is_empty()
            || !c.reads.is_empty()
            || !c.opens.is_empty()
            || c.out_of_scope.is_some()
            || c.in_scope)
    {
        if !name_ok(m) {
            bail!("a mechanic name is lower-case letters, digits and dashes: {m}");
        }
        if !q.iter().any(|r| r["mechanic"] == json!(m)) {
            if c.status.is_none() {
                bail!("{m} isn't in the quest log: give it a --status");
            }
            q.push(json!({"mechanic": m}));
        }
        let r = q
            .iter_mut()
            .find(|r| r["mechanic"] == json!(m))
            .expect("just added");
        if let Some(st) = c.status {
            r["status"] = json!(st);
            r["note"] = json!(c.note.unwrap_or_default());
        }
        for (k, v) in [("known", c.known), ("read", c.reads), ("open", c.opens)] {
            if !v.is_empty() {
                r[k] = json!(v);
            }
        }
        if let Some(o) = r.as_object_mut() {
            if let Some(why) = c.out_of_scope {
                o.insert("scope".into(), json!("out"));
                o.insert("scope_why".into(), json!(why));
            }
            if c.in_scope {
                o.remove("scope");
                o.remove("scope_why");
            }
        }
        write_queue(&f, &q)?;
    }
    let text = q
        .iter()
        .map(|r| {
            let mut l = format!(
                "{}: {}.",
                r["mechanic"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or("")
            );
            if let Some(t) = r["note"].as_str().filter(|t| !t.is_empty()) {
                l.push_str("\n    ");
                l.push_str(t);
            }
            if r["scope"] == json!("out") {
                l.push_str(&format!(
                    " (out of scope: {})",
                    r["scope_why"].as_str().unwrap_or("")
                ));
            }
            l
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(json!({"queue": q}), text))
}

/// Each genre's quest log rows, in order (checklists.toml).
pub(crate) const CHECKLISTS: &str = include_str!("checklists.toml");

/// One row of a genre's checklist.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct Row {
    pub row: String,
    pub what: String,
}

/// A genre's checklist from [`CHECKLISTS`].
pub(crate) fn checklist(genre: &str) -> Result<Vec<Row>> {
    let mut all: std::collections::BTreeMap<String, Vec<Row>> = toml::from_str(CHECKLISTS)?;
    match all.remove(genre) {
        Some(rows) => Ok(rows),
        None => bail!("no quest log checklist for the genre {genre}"),
    }
}

/// Seeds a game's quest log from its genre's checklist: each row it lacks
/// is added as "next" with its line, and each row gets a session if it has
/// none. Rows already there are left as they are. Returns the quest log's
/// folder, the rows and how many were new.
pub(crate) fn seed_quest_log(
    paths: &Paths,
    game: &str,
    genre: &str,
    rom_sha: &str,
) -> Result<(PathBuf, Vec<Row>, usize)> {
    let rows = checklist(genre)?;
    let f = queue_file(paths, game)?;
    let mut q: Vec<Value> = read_queue(&f);
    let mut added = 0;
    for r in &rows {
        if !q.iter().any(|x| x["mechanic"] == json!(r.row)) {
            q.push(json!({"mechanic": r.row, "status": "next", "note": r.what}));
            added += 1;
        }
        Session::create(paths, game, &r.row, Some(&r.what), rom_sha)?;
    }
    write_queue(&f, &q)?;
    let dir = f.parent().map(PathBuf::from).unwrap_or_default();
    Ok((dir, rows, added))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(home: &std::path::Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            store: home.join("store"),
            roms: home.join("roms"),
        }
    }

    /// Every genre `gp game new` takes has a checklist of plain rows.
    #[test]
    fn every_genre_has_a_checklist() {
        let all: std::collections::BTreeMap<String, Vec<Row>> = toml::from_str(CHECKLISTS).unwrap();
        assert_eq!(
            all.keys().map(String::as_str).collect::<Vec<_>>(),
            {
                let mut g = crate::game_new::GENRES.to_vec();
                g.sort_unstable();
                g
            },
            "checklists.toml's genres are gp game new's"
        );
        for (g, rows) in &all {
            assert!(rows.len() >= 5, "{g}: {rows:?}");
            assert_eq!(rows[0].row, "title", "{g}");
            let mut seen = std::collections::BTreeSet::new();
            for r in rows {
                assert!(name_ok(&r.row), "{g}: {}", r.row);
                assert!(!r.what.is_empty(), "{g}: {}", r.row);
                assert!(seen.insert(&r.row), "{g}: {} twice", r.row);
            }
        }
        assert!(checklist("racing").is_err());
    }

    /// Seeding writes each row as next with a session, and seeding again
    /// adds nothing and keeps what changed.
    #[test]
    fn the_quest_log_is_seeded_once() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let (dir, rows, added) = seed_quest_log(&p, "test-hop", "platformer", "ab").unwrap();
        assert_eq!(added, rows.len());
        let q = read_queue(&dir.join("queue.json"));
        assert_eq!(q.len(), rows.len());
        assert_eq!(q[2]["mechanic"], "jump");
        assert_eq!(q[2]["status"], "next");
        let s = Session::open(&p, "test-hop", "jump").unwrap();
        let v = s.state().unwrap();
        assert_eq!(v["prompt"], rows[2].what);
        assert_eq!(v["rom"], "ab");
        let mut q = q;
        q[2]["status"] = json!("done");
        write_queue(&dir.join("queue.json"), &q).unwrap();
        let (_, _, added) = seed_quest_log(&p, "test-hop", "platformer", "ab").unwrap();
        assert_eq!(added, 0);
        assert_eq!(read_queue(&dir.join("queue.json"))[2]["status"], "done");
    }

    #[test]
    fn the_queue_command_adds_and_changes_rows() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let none: &[String] = &[];
        let plain = RowChange {
            status: None,
            note: None,
            known: none,
            reads: none,
            opens: none,
            out_of_scope: None,
            in_scope: false,
        };
        assert!(
            queue(&p, "test-hop", Some("hop"), &plain)
                .unwrap()
                .text
                .is_empty()
        );
        let known = vec!["66 px high".to_owned()];
        let o = queue(
            &p,
            "test-hop",
            Some("hop"),
            &RowChange {
                status: Some("agent working"),
                note: Some("the hop's height"),
                known: &known,
                ..plain
            },
        )
        .unwrap();
        assert_eq!(o.json["queue"][0]["known"][0], "66 px high");
        assert!(o.text.contains("hop: agent working."), "{}", o.text);
        assert!(
            queue(
                &p,
                "test-hop",
                Some("other"),
                &RowChange {
                    known: &known,
                    ..plain
                }
            )
            .is_err()
        );
    }
}
