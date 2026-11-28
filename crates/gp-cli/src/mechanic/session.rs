//! The session store: one folder a mechanic, and the game's quest log
//! beside them. See the table in [`super`] for the files.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

/// A name a session's folder may have: lower-case letters, digits, dashes
/// (and underscores, which some old sessions have).
pub(crate) fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `$GP_HOME/mechanics/GAME`, the game's sessions and quest log.
pub(crate) fn game_dir(paths: &Paths, game: &str) -> Result<PathBuf> {
    if !name_ok(game) {
        bail!("a game name is lower-case letters, digits and dashes: {game}");
    }
    Ok(paths.home.join("mechanics").join(game))
}

/// The game's quest log file.
pub(crate) fn queue_file(paths: &Paths, game: &str) -> Result<PathBuf> {
    Ok(game_dir(paths, game)?.join("queue.json"))
}

/// A game's quest log, for a caller that only reads it; empty when there's
/// none yet, and empty too when the file doesn't parse (a caller that
/// changes the log goes through [`update_queue`], which refuses that file).
pub(crate) fn read_queue(f: &Path) -> Vec<Value> {
    try_read_queue(f).unwrap_or_default()
}

/// A game's quest log: empty when there's no file yet, an error when there
/// is one that doesn't parse as a list of rows.
pub(crate) fn try_read_queue(f: &Path) -> Result<Vec<Value>> {
    let t = match fs::read_to_string(f) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", f.display())),
    };
    serde_json::from_str(&t).with_context(|| {
        format!(
            "{} doesn't parse as a quest log; it is left as it is",
            f.display()
        )
    })
}

/// The lock file beside a quest log: `queue.json.lock`.
fn queue_lock_file(f: &Path) -> PathBuf {
    let mut n = f.file_name().unwrap_or_default().to_os_string();
    n.push(".lock");
    f.with_file_name(n)
}

thread_local! {
    /// The quest logs this thread holds the lock of, so that a change that
    /// calls [`update_queue`] on the same log again fails at once in place
    /// of waiting on itself.
    static HELD: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The one way the quest log changes: takes the lock beside the file (an
/// flock on `queue.json.lock`, held by whichever gp process or thread is in
/// here), reads the log, runs `change` on its rows and writes them back
/// through a temp file and a rename, then lets the lock go. A log that
/// doesn't parse is an error and isn't written over. `change` is the read,
/// change and write only: work that is slow (scoring clips, running a
/// command) is done before, and the rows it needs are read again in here.
pub(crate) fn update_queue<R>(
    f: &Path,
    change: impl FnOnce(&mut Vec<Value>) -> Result<R>,
) -> Result<R> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let key = f.to_path_buf();
    if HELD.with(|h| h.borrow().contains(&key)) {
        bail!(
            "{} is changed again inside its own change (update_queue called within update_queue)",
            f.display()
        );
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(queue_lock_file(f))
        .with_context(|| format!("opening the lock beside {}", f.display()))?;
    lock.lock()
        .with_context(|| format!("locking {}", f.display()))?;
    HELD.with(|h| h.borrow_mut().push(key.clone()));
    let out = (|| {
        let old = try_read_queue(f)?;
        let mut q = old.clone();
        let r = change(&mut q)?;
        if q != old {
            write_rows(f, &old, &q)?;
        }
        Ok(r)
    })();
    HELD.with(|h| h.borrow_mut().retain(|p| p != &key));
    drop(lock);
    out
}

/// Puts `q` in place of the quest log, under its lock: for a test's
/// fixture, which writes a whole log. Code that changes the log goes
/// through [`update_queue`].
#[cfg(test)]
pub(crate) fn write_queue(f: &Path, q: &[Value]) -> Result<()> {
    update_queue(f, |old| {
        *old = q.to_vec();
        Ok(())
    })?;
    if !f.exists() {
        fs::write(f, "[]")?;
    }
    Ok(())
}

/// Writes the quest log, stamping `changed_ms` on each row that is new or
/// differs from the file's (a card that ended before it can't move it:
/// the runner's `settle_card_rows`); a row as it was keeps its stamp. A row
/// that goes in line gets `parked_ms` too. The rows go to a temp file
/// beside the log, which is then renamed over it, so a reader never sees
/// half a log. Only [`update_queue`] calls it, holding the lock.
fn write_rows(f: &Path, old: &[Value], q: &[Value]) -> Result<()> {
    let bare = |r: &Value| {
        let mut r = r.clone();
        if let Some(o) = r.as_object_mut() {
            o.remove("changed_ms");
        }
        r
    };
    let now = gp_mcp::asks::now_ms();
    let q: Vec<Value> = q
        .iter()
        .map(|r| {
            let was = old
                .iter()
                .find(|o| o["mechanic"] == r["mechanic"] && !r["mechanic"].is_null());
            let mut r = r.clone();
            let stamp = match was {
                Some(o) if bare(o) == bare(&r) => o["changed_ms"].clone(),
                _ => json!(now),
            };
            if let Some(o) = r.as_object_mut()
                && !stamp.is_null()
            {
                o.insert("changed_ms".into(), stamp);
            }
            // A row that goes in line is parked from now (`parked_ms`), unless
            // the change stamped its own: the person's words written before
            // it don't wake it (crate::steps::row_steps).
            let into_line = r["status"] == json!("in line")
                && was.is_none_or(|o| o["status"] != json!("in line"));
            let own =
                r["parked_ms"].is_u64() && was.is_none_or(|o| o["parked_ms"] != r["parked_ms"]);
            if into_line
                && !own
                && let Some(o) = r.as_object_mut()
            {
                o.insert("parked_ms".into(), json!(now));
            }
            r
        })
        .collect();
    let mut tmp = f.file_name().unwrap_or_default().to_os_string();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = f.with_file_name(tmp);
    fs::write(&tmp, serde_json::to_string_pretty(&q)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, f).with_context(|| format!("renaming {} over its log", tmp.display()))?;
    Ok(())
}

/// Whether nobody works quest-log row `r` of `game` while the loop's
/// runner does the work for `home`: it is on, the row has no worker of its
/// own (`worker`, a session's by hand or a loop worker's) and no live
/// worker of the runner's holds `row:GAME/ROW`. The person's turn on such a
/// row would set it "agent working" with no agent at it: on 2026-11-25
/// space-bar-lights sat so for five minutes after the owner's answer, the
/// page showing AGENT WORKING and nobody on it, and the owner took their
/// words for lost (answer-on-a-look-row-was-lost). With the runner off, a
/// session at the page answers the row's talk itself, and the row is its.
pub(crate) fn nobody_on(home: &Path, game: &str, r: &Value) -> bool {
    if !gp_mcp::steps::settings(home).runner.on {
        return false;
    }
    if r["worker"].as_str().is_some_and(|w| !w.trim().is_empty()) {
        return false;
    }
    let me = format!("row:{game}/{}", r["mechanic"].as_str().unwrap_or(""));
    !gp_mcp::steps::held(home).iter().any(|s| {
        s["touches"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t == &json!(me)))
    })
}

/// The words a row's note starts with when the person answered `w` in its
/// talk, as the step for it says them (crate::steps::row_steps).
pub(crate) fn answered_words(w: &str) -> String {
    format!("{ANSWERED}\"{w}\". Act on it now.")
}

/// How [`answered_words`] start.
pub(crate) const ANSWERED: &str = "The person answered in the row's talk: ";

/// Row `r` after the person answered `words` in its talk with nobody at
/// work on it ([`nobody_on`]): next, for the loop's next worker, with their
/// words at the front of its note (an earlier answer's taken off) and its
/// plain line saying they are kept. The person's own note stays theirs
/// (`asked`), as gp mechanic queue keeps it. A park ends.
pub(crate) fn answered_row(r: &mut Value, words: &str) {
    let w = crate::next::cut(words, 300);
    let Some(o) = r.as_object_mut() else { return };
    if o.get("by") == Some(&json!("person")) && !o.contains_key("asked") {
        let said = o.get("note").cloned().unwrap_or(json!(""));
        o.insert("asked".into(), said);
    }
    let note = o
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let rest = match o.get("answered").and_then(Value::as_str) {
        Some(prev) => note
            .strip_prefix(&answered_words(prev))
            .unwrap_or(&note)
            .trim_start()
            .to_owned(),
        None => note,
    };
    o.insert(
        "note".into(),
        json!(format!("{} {rest}", answered_words(&w)).trim_end()),
    );
    o.insert("answered".into(), json!(w));
    o.insert(
        "plain".into(),
        json!(format!(
            "You said \"{}\": it is kept on this row, and the loop's next worker takes it up.",
            crate::next::cut(&w, 120)
        )),
    );
    o.insert("status".into(), json!("next"));
    o.remove("parked");
}

/// The mechanic the person has open in the page.
pub(crate) fn current(paths: &Paths, game: &str) -> Result<String> {
    let f = game_dir(paths, game)?.join("current");
    Ok(fs::read_to_string(&f)
        .with_context(|| format!("no mechanic open for {game}: start one with gp mechanic start"))?
        .trim()
        .to_owned())
}

pub(crate) fn set_current(paths: &Paths, game: &str, mechanic: &str) -> Result<()> {
    let d = game_dir(paths, game)?;
    fs::create_dir_all(&d)?;
    fs::write(d.join("current"), mechanic)?;
    Ok(())
}

pub(crate) struct Session {
    pub dir: PathBuf,
    /// Held while a file of the session is appended or a clip numbered.
    lock: Mutex<()>,
}

/// Whose turn the talk waits for: the agent's after the person's turn (or
/// none), else the person's.
pub fn waiting_for(turns: &[Value]) -> &'static str {
    match turns.last().and_then(|t| t["by"].as_str()) {
        Some("player") | None => "agent",
        Some(_) => "player",
    }
}

/// What the agent says it is doing, while no turn has come since its
/// status line.
pub fn busy_of(busy_file: Option<&Value>, turns: &[Value]) -> Option<String> {
    busy_file
        .filter(|b| b["after"].as_u64() == Some(turns.len() as u64))
        .and_then(|b| b["text"].as_str().map(str::to_owned))
}

impl Session {
    fn dir_of(paths: &Paths, game: &str, mechanic: &str) -> Result<PathBuf> {
        if !name_ok(game) || !name_ok(mechanic) {
            bail!(
                "a game or mechanic name is lower-case letters, digits and dashes: {game} {mechanic}"
            );
        }
        Ok(paths.home.join("mechanics").join(game).join(mechanic))
    }

    /// Whether a session for the mechanic exists.
    pub fn exists(paths: &Paths, game: &str, mechanic: &str) -> Result<bool> {
        Ok(Self::dir_of(paths, game, mechanic)?
            .join("session.toml")
            .is_file())
    }

    pub fn open(paths: &Paths, game: &str, mechanic: &str) -> Result<Session> {
        let dir = Self::dir_of(paths, game, mechanic)?;
        if !dir.join("session.toml").is_file() {
            bail!("no session for {game} {mechanic}: start one with gp mechanic start");
        }
        Ok(Session {
            dir,
            lock: Mutex::new(()),
        })
    }

    /// What `gp mechanic show` prints for a quest-log row with no session
    /// yet: the row's note as its words, no clips, no talk, writing nothing
    /// (`gp mechanic busy` or `log` makes the session from the row).
    pub fn row_state(paths: &Paths, game: &str, mechanic: &str) -> Result<Option<Value>> {
        Self::dir_of(paths, game, mechanic)?;
        let q = read_queue(&queue_file(paths, game)?);
        let archived = super::archive::read_archive(&paths.home, game);
        Ok(q.iter()
            .chain(&archived)
            .find(|r| r["mechanic"] == json!(mechanic))
            .map(|row| {
                json!({
                    "busy": null,
                    "agent_ms": null,
                    "game": game,
                    "mechanic": mechanic,
                    "prompt": row["note"].as_str().unwrap_or(""),
                    "rom": null,
                    "folder": null,
                    "clips": [],
                    "talk": [],
                    "waiting_for": "agent",
                    "queue": super::scores::scored_queue(&paths.home, game),
                })
            }))
    }

    /// The session of a quest-log row, made from its note when it has none
    /// yet (as the page's pick makes one), with the ROM another of the
    /// game's sessions was made with: so a loop worker can post the row's
    /// status and log from its first step.
    pub fn open_or_make(paths: &Paths, game: &str, mechanic: &str) -> Result<Session> {
        if Self::exists(paths, game, mechanic)? {
            return Self::open(paths, game, mechanic);
        }
        let q = read_queue(&queue_file(paths, game)?);
        let Some(row) = q
            .iter()
            .find(|r| r["mechanic"] == serde_json::json!(mechanic))
        else {
            return Self::open(paths, game, mechanic);
        };
        let note = row["note"].as_str().unwrap_or("").to_owned();
        let rom = fs::read_dir(game_dir(paths, game)?)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| fs::read_to_string(e.path().join("session.toml")).ok())
            .find_map(|t| {
                t.parse::<toml::Table>()
                    .ok()
                    .and_then(|v| v.get("rom").and_then(|r| r.as_str()).map(str::to_owned))
            })
            .unwrap_or_default();
        Self::create(paths, game, mechanic, Some(&note), &rom)
    }

    pub fn create(
        paths: &Paths,
        game: &str,
        mechanic: &str,
        prompt: Option<&str>,
        rom_sha: &str,
    ) -> Result<Session> {
        let dir = Self::dir_of(paths, game, mechanic)?;
        fs::create_dir_all(dir.join("clips"))?;
        let f = dir.join("session.toml");
        if !f.is_file() {
            let mut t = toml::Table::new();
            t.insert("game".into(), game.into());
            t.insert("mechanic".into(), mechanic.into());
            t.insert("prompt".into(), prompt.unwrap_or("").into());
            t.insert("rom".into(), rom_sha.into());
            fs::write(&f, toml::to_string(&t)?)?;
        }
        Ok(Session {
            dir,
            lock: Mutex::new(()),
        })
    }

    /// A store kept as a session for its clip numbering and its lock, at
    /// `dir`, outside every game's sessions: the sealed runs' store
    /// (`$GP_HOME/sealed/GAME/sealed-run`). It has no session.toml.
    pub fn store(dir: PathBuf) -> Result<Session> {
        fs::create_dir_all(dir.join("clips"))?;
        Ok(Session {
            dir,
            lock: Mutex::new(()),
        })
    }

    /// The mechanic's name, the folder's.
    pub fn mechanic(&self) -> &str {
        self.dir.file_name().and_then(|n| n.to_str()).unwrap_or("")
    }

    fn info(&self) -> Result<toml::Table> {
        Ok(toml::from_str(&fs::read_to_string(
            self.dir.join("session.toml"),
        )?)?)
    }

    /// Clips by number, each with its start, frames, maker and note.
    /// The session's clips in order; none when it has no clips folder (a
    /// session made from a quest-log row by `gp mechanic busy` or `log`
    /// has none until its first clip). Until 2026-11-26 such a session made
    /// every call that lists clips fail: `api/screen/clips` answered 500
    /// "No such file or directory" on a game with such a session, so the
    /// screen page's lists under the screens never filled.
    pub fn clips(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let dir = self.dir.join("clips");
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut names: Vec<PathBuf> = fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        names.sort();
        for p in names {
            let t: toml::Table = toml::from_str(&fs::read_to_string(&p)?)?;
            let n: u64 = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let mut v = serde_json::to_value(&t)?;
            v["n"] = json!(n);
            out.push(v);
        }
        Ok(out)
    }

    pub fn turns(&self) -> Result<Vec<Value>> {
        let p = self.dir.join("talk.ndjson");
        if !p.is_file() {
            return Ok(Vec::new());
        }
        fs::read_to_string(p)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).context("a line of talk.ndjson"))
            .collect()
    }

    /// The agent's status line as written (`busy.json`), if any.
    pub fn busy_file(&self) -> Option<Value> {
        fs::read_to_string(self.dir.join("busy.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    }

    /// Everything the page and `show` read: the mechanic, its clips, the
    /// conversation, whose turn it is and the game's quest log.
    pub fn state(&self) -> Result<Value> {
        let info = self.info()?;
        let clips = self.clips()?;
        let turns = self.turns()?;
        let waiting = waiting_for(&turns);
        let busy_file = self.busy_file();
        let busy = busy_of(busy_file.as_ref(), &turns);
        // When the agent last acted here: its newest status line or turn
        // (ms since the epoch; null when neither carries a time).
        let agent_ms = busy_file
            .as_ref()
            .and_then(|b| b["at_ms"].as_u64())
            .into_iter()
            .chain(
                turns
                    .iter()
                    .filter(|t| t["by"] == json!("agent"))
                    .filter_map(|t| t["at_ms"].as_u64()),
            )
            .max();
        Ok(json!({
            "busy": busy,
            "agent_ms": agent_ms,
            "game": info.get("game").and_then(|v| v.as_str()),
            "mechanic": info.get("mechanic").and_then(|v| v.as_str()),
            "prompt": info.get("prompt").and_then(|v| v.as_str()),
            "rom": info.get("rom").and_then(|v| v.as_str()),
            "folder": self.dir.display().to_string(),
            "clips": clips,
            "talk": turns,
            "waiting_for": waiting,
            // The quest log with the gate's scores, as `gp mechanic queue`
            // prints it: $GP_HOME is three folders up (mechanics/GAME/M).
            "queue": super::scores::scored_queue(
                self.dir
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .unwrap_or(Path::new("")),
                self.dir
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
            ),
        }))
    }

    /// Sets the session's words (its prompt, what `gp mechanic show` prints
    /// first): a row reopened for a new gap takes its new note
    /// (mechanic::propose).
    pub fn set_prompt(&self, prompt: &str) -> Result<()> {
        let _g = self.lock.lock().expect("lock");
        let mut t = self.info()?;
        t.insert("prompt".into(), prompt.into());
        fs::write(self.dir.join("session.toml"), toml::to_string(&t)?)?;
        Ok(())
    }

    /// Notes what the agent is doing until its next turn.
    pub fn set_busy(&self, text: &str) -> Result<()> {
        let n = self.turns()?.len();
        fs::write(
            self.dir.join("busy.json"),
            json!({"text": text, "after": n, "at_ms": gp_mcp::asks::now_ms()}).to_string(),
        )?;
        Ok(())
    }

    /// Appends a turn (`by`, `text` and whatever else it carries); returns
    /// its number.
    pub fn turn(&self, v: Value) -> Result<usize> {
        let _g = self.lock.lock().expect("lock");
        self.append_turn(v)
    }

    /// A turn that leaves the row's status as it is: the person's words
    /// beside their answer to a card, which the runner settles the row by
    /// (serve::runner's settle_card_rows), so the turn mustn't set it
    /// "agent working" first.
    pub fn turn_keeping_status(&self, v: Value) -> Result<usize> {
        let _g = self.lock.lock().expect("lock");
        self.write_turn(v)
    }

    /// Writes turn `v` to the talk, numbered; returns its number.
    fn write_turn(&self, mut v: Value) -> Result<usize> {
        let n = self.turns()?.len() + 1;
        v["n"] = json!(n);
        if v["at_ms"].is_null() {
            v["at_ms"] = json!(gp_mcp::asks::now_ms());
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("talk.ndjson"))?;
        writeln!(f, "{v}")?;
        Ok(n)
    }

    /// The home this session is in: `$GP_HOME/mechanics/GAME/ROW` is its
    /// folder.
    fn home(&self) -> Option<&Path> {
        self.dir.parent()?.parent()?.parent()
    }

    /// [`Self::turn`] for a caller that holds the lock.
    fn append_turn(&self, v: Value) -> Result<usize> {
        let by_player = v["by"] == json!("player");
        let words = v["text"].as_str().unwrap_or("").to_owned();
        let n = self.write_turn(v)?;
        // The queue follows whose turn it is: the person's turn makes the
        // mechanic "agent working" when someone works it ([`nobody_on`]),
        // else "next" with their words on the row; the agent's "asking
        // you" only when the row has a look for them (a clip, or a card):
        // an answer with nothing to look at leaves the status for the
        // worker, or the runner at its end, to set (ask-only-about-looks; a
        // row left asking with nothing to see was set right by hand). Done
        // stays done.
        let qf = self.dir.with_file_name("queue.json");
        let me = self.mechanic();
        let game = self
            .dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        update_queue(&qf, |q| {
            self.row_follows_turn(q, me, &game, by_player, &words);
            Ok(())
        })?;
        Ok(n)
    }

    /// The row's status after a turn, for [`Self::append_turn`] (under the
    /// quest log's lock).
    fn row_follows_turn(
        &self,
        q: &mut [Value],
        me: &str,
        game: &str,
        by_player: bool,
        words: &str,
    ) {
        if let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(me))
            && r["status"] != json!("done")
        {
            if by_player && self.home().is_some_and(|h| nobody_on(h, game, r)) {
                answered_row(r, words);
                return;
            }
            let st = if by_player {
                Some("agent working")
            } else {
                let mut asked = r.clone();
                asked["status"] = json!("asking you");
                // The row's newest card (HOME/mechanics/GAME/ROW is the dir).
                let card = match (self.dir.parent(), self.dir.parent().and_then(Path::parent)) {
                    (Some(g), Some(m)) => m.parent().and_then(|home| {
                        let game = g.file_name()?.to_str()?;
                        crate::serve::runner::row_card(home, game, me, gp_mcp::asks::now_ms())
                    }),
                    _ => None,
                };
                super::worker_ask_refused(&asked, card.map(|(_, s)| s))
                    .is_none()
                    .then_some("asking you")
            };
            if let Some(st) = st
                && r["status"] != json!(st)
            {
                r["status"] = json!(st);
            }
        }
    }

    /// A turn from the page, kept once: the person's turn saying the same
    /// words with the same clips and choice as the turn just before it
    /// (sent twice by a double press) gives that turn's number.
    pub fn turn_once(&self, v: Value) -> Result<(usize, bool)> {
        let _g = self.lock.lock().expect("lock");
        let turns = self.turns()?;
        if let Some(last) = turns.last()
            && last["by"] == json!("player")
            && v["by"] == json!("player")
            && ["text", "clips", "choice"]
                .iter()
                .all(|k| last[*k] == v[*k])
        {
            return Ok((turns.len(), true));
        }
        Ok((self.append_turn(v)?, false))
    }

    /// Saves a clip sent as a full log from power-on and where it starts.
    pub fn add_clip(&self, log: &str, start: u64, note: &str, by: &str) -> Result<u64> {
        let parsed = gp_evidence::input_log::InputLog::parse(log)
            .map_err(|e| anyhow::anyhow!("the clip's input log: {e}"))?;
        let frames = (parsed.frames.len() as u64).saturating_sub(start);
        if frames == 0 {
            bail!("the clip has no frames after its start");
        }
        let _g = self.lock.lock().expect("lock");
        let mut t = toml::Table::new();
        t.insert("start".into(), (start as i64).into());
        t.insert("frames".into(), (frames as i64).into());
        self.write_clip(t, log, note, by)
    }

    /// A clip the page handed off, saved once: the same path, start and
    /// inputs handed off again (a second press while the first was on its
    /// way) give the clip already saved. Returns its number and whether it
    /// was already there.
    pub fn add_player_clip(&self, t: toml::Table, log: &str, note: &str) -> Result<(u64, bool)> {
        let _g = self.lock.lock().expect("lock");
        for c in self.clips()? {
            let n = c["n"].as_u64().unwrap_or(0);
            if c["path"].as_str() == t.get("path").and_then(|v| v.as_str())
                && c["start"].as_i64() == t.get("start").and_then(|v| v.as_integer())
                && fs::read_to_string(self.dir.join(format!("clips/{n:03}.log")))
                    .is_ok_and(|l| l == log)
            {
                return Ok((n, true));
            }
        }
        Ok((self.write_clip(t, log, note, "player")?, false))
    }

    /// Adds a field to a saved clip's `.toml` (a state hash found after the
    /// clip was saved).
    pub fn set_clip_field(&self, n: u64, key: &str, value: toml::Value) -> Result<()> {
        let _g = self.lock.lock().expect("lock");
        let f = self.dir.join(format!("clips/{n:03}.toml"));
        let mut t: toml::Table = toml::from_str(&fs::read_to_string(&f)?)?;
        t.insert(key.into(), value);
        let tmp = f.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string(&t)?)?;
        fs::rename(&tmp, &f)?;
        Ok(())
    }

    /// Saves a clip from its `.toml` fields and its own log, as the next
    /// clip (`gp mechanic probe --save`).
    pub fn add_clip_meta(&self, t: toml::Table, log: &str, note: &str, by: &str) -> Result<u64> {
        let _g = self.lock.lock().expect("lock");
        self.write_clip(t, log, note, by)
    }

    /// Saves a clip as clip `n`, over any clip `n` there (`gp mechanic
    /// probe --save` for a probe that names its clips).
    pub fn put_clip(
        &self,
        n: u64,
        mut t: toml::Table,
        log: &str,
        note: &str,
        by: &str,
    ) -> Result<u64> {
        let _g = self.lock.lock().expect("lock");
        fs::create_dir_all(self.dir.join("clips"))?;
        fs::write(self.dir.join(format!("clips/{n:03}.log")), log)?;
        t.insert("by".into(), by.into());
        t.insert("note".into(), note.into());
        fs::write(
            self.dir.join(format!("clips/{n:03}.toml")),
            toml::to_string(&t)?,
        )?;
        Ok(n)
    }

    /// Writes the next clip; the caller holds the lock.
    fn write_clip(&self, mut t: toml::Table, log: &str, note: &str, by: &str) -> Result<u64> {
        let n = self
            .clips()?
            .iter()
            .filter_map(|c| c["n"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        fs::create_dir_all(self.dir.join("clips"))?;
        fs::write(self.dir.join(format!("clips/{n:03}.log")), log)?;
        t.insert("by".into(), by.into());
        t.insert("note".into(), note.into());
        fs::write(
            self.dir.join(format!("clips/{n:03}.toml")),
            toml::to_string(&t)?,
        )?;
        Ok(n)
    }
}

/// A session as `gp mechanic show` prints it.
pub(crate) fn show_text(v: &Value) -> String {
    let mut t = String::new();
    let _ = writeln!(
        t,
        "{} / {}: {}",
        v["game"].as_str().unwrap_or(""),
        v["mechanic"].as_str().unwrap_or(""),
        v["prompt"]
            .as_str()
            .filter(|p| !p.is_empty())
            .unwrap_or("(no words yet)")
    );
    let clips = v["clips"].as_array().cloned().unwrap_or_default();
    let _ = writeln!(t, "\n{} clips", clips.len());
    for c in &clips {
        let _ = writeln!(
            t,
            "  {}: {} frames from frame {}, by {}: {}",
            c["n"],
            c["frames"],
            c["start"],
            c["by"].as_str().unwrap_or(""),
            c["note"].as_str().unwrap_or("")
        );
    }
    let talk = v["talk"].as_array().cloned().unwrap_or_default();
    let _ = writeln!(t, "\nTalk ({} turns)", talk.len());
    for x in &talk {
        let clips: Vec<String> = x["clips"]
            .as_array()
            .map(|a| a.iter().map(|c| c.to_string()).collect())
            .unwrap_or_default();
        let mut extra = String::new();
        if !clips.is_empty() {
            let _ = write!(extra, " [clips {}]", clips.join(", "));
        }
        if let Some(c) = x["show"].as_u64() {
            let _ = write!(extra, " [showing clip {c}]");
        }
        if let Some(c) = x["against"].as_u64() {
            let _ = write!(extra, " [against clip {c}]");
        }
        if let Some(ch) = x["choices"].as_array().filter(|a| !a.is_empty()) {
            let names: Vec<&str> = ch.iter().filter_map(Value::as_str).collect();
            let _ = write!(extra, " [choices: {}]", names.join(" | "));
        }
        if let Some(c) = x["choice"].as_u64() {
            let _ = write!(extra, " [picked choice {}]", c + 1);
        }
        let p = &x["point"];
        if p.is_object() {
            let r: Vec<String> = p["rect"]
                .as_array()
                .map(|a| a.iter().map(|v| v.to_string()).collect())
                .unwrap_or_default();
            let things: Vec<String> = p["things"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t["label"].as_str().or(t["name"].as_str()).or(t.as_str()))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let _ = write!(
                extra,
                " [pointing at {} in {} at frame {}, box {}{}]",
                p["label"].as_str().unwrap_or("a spot"),
                p["recording"].as_str().unwrap_or("the recording"),
                p["frame"],
                r.join(","),
                if things.is_empty() {
                    String::new()
                } else {
                    format!(": {}", things.join(", "))
                }
            );
            // "What is this?" (issue #10): the call that answers it.
            if p["ask"] == "explain" {
                match p["explain"].as_str() {
                    Some(c) => {
                        let _ = write!(extra, " [asks what this is: answer from `{c}`]");
                    }
                    None => {
                        let _ = write!(
                            extra,
                            " [asks what this is; no gp element explain call: {}]",
                            p["explain_missing"].as_str().unwrap_or("unknown")
                        );
                    }
                }
            }
        }
        let _ = writeln!(
            t,
            "  {} {}: {}{extra}",
            x["n"],
            x["by"].as_str().unwrap_or(""),
            x["text"].as_str().unwrap_or("")
        );
    }
    let _ = writeln!(
        t,
        "\nWaiting for: {}",
        v["waiting_for"].as_str().unwrap_or("")
    );
    let _ = writeln!(
        t,
        "Clip logs: {}/clips/",
        v["folder"].as_str().unwrap_or("")
    );
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_paths as paths;

    #[test]
    fn two_threads_adding_rows_through_the_lock_lose_none() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("mechanics/rooftop/queue.json");
        let adders: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|who| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for k in 0..200 {
                        update_queue(&f, |q| {
                            q.push(json!({"mechanic": format!("{who}-{k}"), "status": "next"}));
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for a in adders {
            a.join().unwrap();
        }
        assert_eq!(read_queue(&f).len(), 400);
    }

    #[test]
    fn a_quest_log_that_does_not_parse_is_an_error_and_is_not_written_over() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("mechanics/rooftop/queue.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "[{\"mechanic\": \"jump\", \"status\": \"ne").unwrap();
        let e = update_queue(&f, |q| {
            q.clear();
            Ok(())
        })
        .unwrap_err();
        assert!(format!("{e:#}").contains("doesn't parse"), "{e:#}");
        assert!(try_read_queue(&f).is_err());
        assert_eq!(
            fs::read_to_string(&f).unwrap(),
            "[{\"mechanic\": \"jump\", \"status\": \"ne"
        );
        // No file yet is an empty log, not an error.
        assert!(
            try_read_queue(&d.path().join("none.json"))
                .unwrap()
                .is_empty()
        );
        // A change inside its own change fails at once, not waiting on itself.
        let g = d.path().join("mechanics/hilltop/queue.json");
        let e = update_queue(&g, |_| update_queue(&g, |_| Ok(()))).unwrap_err();
        assert!(format!("{e:#}").contains("inside its own change"), "{e:#}");
    }

    #[test]
    fn the_quest_log_is_written_through_a_temp_file_and_a_rename() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("mechanics/rooftop");
        let f = dir.join("queue.json");
        let long = "x".repeat(20_000);
        let writer = {
            let (f, long) = (f.clone(), long.clone());
            std::thread::spawn(move || {
                for k in 0..150 {
                    update_queue(&f, |q| {
                        q.push(json!({"mechanic": format!("r{k}"), "note": long}));
                        Ok(())
                    })
                    .unwrap();
                }
            })
        };
        // A reader that doesn't take the lock never sees half a log.
        let mut reads = 0;
        while !writer.is_finished() {
            if let Ok(t) = fs::read_to_string(&f) {
                serde_json::from_str::<Vec<Value>>(&t).expect("a whole log");
                reads += 1;
            }
        }
        writer.join().unwrap();
        assert!(reads > 0);
        assert_eq!(read_queue(&f).len(), 150);
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_pointed_turn_says_where_in_show() {
        let v = json!({"game": "mosshop", "mechanic": "walk", "talk": [{
            "n": 1, "by": "player", "text": "stop at the top",
            "point": {"label": "the frog", "recording": "walk.001", "frame": 412,
                      "rect": [96, 120, 24, 24], "things": [{"label": "ladder"}]}}]});
        let t = show_text(&v);
        assert!(
            t.contains("1 player: stop at the top [pointing at the frog in walk.001 at frame 412, box 96,120,24,24: ladder]"),
            "{t}"
        );
    }

    #[test]
    fn a_question_says_the_call_that_answers_it_in_show() {
        let v = json!({"game": "mosshop", "mechanic": "walk", "talk": [{
            "n": 2, "by": "player", "text": "what is this?",
            "point": {"label": "a spot", "recording": "mosshop.walk.001", "frame": 412, "rect": [96, 120, 16, 16],
                      "ask": "explain", "explain": "gp element explain --run g --clip mosshop.walk.001 --log-frame 412 --box 96,120,16,16 --side original"}}]});
        let t = show_text(&v);
        assert!(
            t.contains("[asks what this is: answer from `gp element explain --run g --clip mosshop.walk.001 --log-frame 412 --box 96,120,16,16 --side original`]"),
            "{t}"
        );
    }

    #[test]
    fn a_session_keeps_its_clips_and_its_talk_in_order() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let s = Session::create(&p, "test-hop", "jump", Some("the jump"), "ab").unwrap();
        assert_eq!(s.state().unwrap()["waiting_for"], "agent");
        let log = "gp-input-log 1\ncontrollers 1\nP ........\nP ........\nP .......A\nP .......A\n";
        let n = s.add_clip(log, 2, "a held jump", "player").unwrap();
        assert_eq!(n, 1);
        assert!(s.add_clip(log, 4, "", "player").is_err());
        s.turn(json!({"by": "player", "text": "here is a jump", "clips": [1]}))
            .unwrap();
        let v = s.state().unwrap();
        assert_eq!(v["clips"][0]["frames"], 2);
        assert_eq!(v["clips"][0]["note"], "a held jump");
        assert_eq!(v["waiting_for"], "agent");
        s.set_busy("measuring").unwrap();
        assert_eq!(s.state().unwrap()["busy"], "measuring");
        // The page says when the agent last acted: its status line's time.
        let busy_ms = s.state().unwrap()["agent_ms"].as_u64().unwrap();
        assert!(busy_ms > 0);
        s.turn(
            json!({"by": "agent", "text": "show me a running jump", "choices": ["ok", "not now"]}),
        )
        .unwrap();
        let v = s.state().unwrap();
        assert_eq!(v["waiting_for"], "player");
        // Busy lasts only until the agent's next turn, which has a time.
        assert!(v["busy"].is_null());
        assert!(v["agent_ms"].as_u64().unwrap() >= busy_ms);
        assert!(v["talk"][1]["at_ms"].as_u64().is_some());
        let text = show_text(&v);
        assert!(text.contains("a held jump"), "{text}");
        assert!(text.contains("[choices: ok | not now]"), "{text}");
    }

    /// A session whose clips folder is gone (one made from a row, or
    /// emptied by hand) has no clips, and its state reads.
    #[test]
    fn a_session_with_no_clips_folder_has_no_clips() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::create(&paths(d.path()), "test-hop", "jump", None, "ab").unwrap();
        let _ = fs::remove_dir_all(s.dir.join("clips"));
        assert!(!s.dir.join("clips").exists());
        assert_eq!(s.clips().unwrap().len(), 0);
        assert_eq!(s.state().unwrap()["clips"], json!([]));
        // The next clip makes the folder again.
        let log = "gp-input-log 1\ncontrollers 1\nP ........\nP ........\n";
        assert_eq!(s.add_clip(log, 1, "", "player").unwrap(), 1);
        assert_eq!(s.clips().unwrap().len(), 1);
    }

    #[test]
    fn a_clip_handed_off_twice_is_kept_once() {
        let d = tempfile::tempdir().unwrap();
        let s = Session::create(&paths(d.path()), "test-hop", "jump", None, "ab").unwrap();
        let sha = "0".repeat(64);
        let mut a = gp_evidence::InputLog::new(1);
        a.frames = vec![gp_evidence::input_log::InputFrame::default(); 3];
        let (t, log) = super::super::paths::clip_fields(&sha, 40, &a);
        assert_eq!(s.add_player_clip(t.clone(), &log, "").unwrap(), (1, false));
        assert_eq!(s.add_player_clip(t, &log, "").unwrap(), (1, true));
        // Another start, or other inputs, is another clip.
        let (t, log) = super::super::paths::clip_fields(&sha, 41, &a);
        assert_eq!(s.add_player_clip(t, &log, "").unwrap(), (2, false));
        s.set_clip_field(2, "state", "cafe".into()).unwrap();
        assert_eq!(s.clips().unwrap()[1]["state"], "cafe");
        // The same words again straight after are one turn; after the
        // agent's answer they are a new one.
        let t = json!({"by": "player", "text": "full jump", "clips": [1]});
        assert_eq!(s.turn_once(t.clone()).unwrap(), (1, false));
        assert_eq!(s.turn_once(t.clone()).unwrap(), (1, true));
        s.turn(json!({"by": "agent", "text": "thanks"})).unwrap();
        assert_eq!(s.turn_once(t).unwrap(), (3, false));
    }

    /// The quest log row follows whose turn it is, and a row marked done
    /// stays done. The agent's turn asks the person only with a look on the
    /// row (a clip, or its card on the page): an answer with nothing to see
    /// leaves the row as it is.
    #[test]
    fn the_quest_log_follows_the_turns() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let s = Session::create(&p, "test-hop", "walk", None, "ab").unwrap();
        let qf = queue_file(&p, "test-hop").unwrap();
        write_queue(
            &qf,
            &[
                json!({"mechanic": "walk", "status": "next", "note": ""}),
                json!({"mechanic": "jump", "status": "done", "note": ""}),
            ],
        )
        .unwrap();
        s.turn(json!({"by": "player", "text": "a walk"})).unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "agent working");
        s.turn(json!({"by": "agent", "text": "Got your yes; it waits on a tools fix."}))
            .unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "agent working");
        let mut q = read_queue(&qf);
        q[0]["clip"] = json!({"n": 1});
        write_queue(&qf, &q).unwrap();
        s.turn(json!({"by": "agent", "text": "and back?"})).unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "asking you");
        let mut q = read_queue(&qf);
        q[0]["status"] = json!("agent working");
        q[0].as_object_mut().unwrap().remove("clip");
        q[0]["note"] = json!("Its card is on your page: press YES");
        write_queue(&qf, &q).unwrap();
        // A note naming a card asks only while the row's card is on the
        // page (trouble-rows-dont-ask-the-person-open-questions).
        s.turn(json!({"by": "agent", "text": "the card is up"}))
            .unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "agent working");
        let now = gp_mcp::asks::now_ms();
        crate::serve::runner::test_card(
            &p.home,
            1,
            now,
            "test-hop",
            "walk",
            json!({"state": "open"}),
        );
        s.turn(json!({"by": "agent", "text": "the card is up"}))
            .unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "asking you");
        let j = Session::create(&p, "test-hop", "jump", None, "ab").unwrap();
        j.turn(json!({"by": "player", "text": "again"})).unwrap();
        assert_eq!(read_queue(&qf)[1]["status"], "done");
        assert_eq!(s.state().unwrap()["queue"].as_array().unwrap().len(), 2);
    }

    /// The person's answer on a row asking for a look (a clip and a look
    /// folder, iterating, no card), with the loop's runner on and nobody
    /// at work on it: their words are in its talk, and the row goes next
    /// with them on its note and plain line, not agent working with no
    /// worker (answer-on-a-look-row-was-lost). A row a session works by
    /// hand, or one a live worker of the runner's holds, is theirs.
    #[test]
    fn an_answer_on_a_row_nobody_works_goes_next_with_the_words() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        fs::write(p.home.join("loop.toml"), "[runner]\non = true\n").unwrap();
        let qf = queue_file(&p, "test-hop").unwrap();
        let look = json!({"mechanic": "lights", "status": "asking you",
            "note": "Each bar pulses blue and white. Not landed: it waits for the look.",
            "plain": "Is this the look you want?",
            "clip": {"id": "test-hop.l10.001", "frames": [0, 900], "look": "/themes/last-call"},
            "iterating": "the owner is looking at the lights"});
        write_queue(
            &qf,
            &[
                look.clone(),
                json!({"mechanic": "byhand", "status": "agent working", "worker": "main-session", "note": "n"}),
                json!({"mechanic": "held", "status": "asking you", "note": "h"}),
            ],
        )
        .unwrap();
        let s = Session::create(&p, "test-hop", "lights", None, "ab").unwrap();
        let said = "i said the original bars alternate blue and white";
        s.turn_once(json!({"by": "player", "text": said, "clips": []}))
            .unwrap();
        assert_eq!(s.turns().unwrap()[0]["text"], said);
        let row = |m: &str| {
            read_queue(&qf)
                .into_iter()
                .find(|r| r["mechanic"] == json!(m))
                .unwrap()
        };
        let r = row("lights");
        assert_eq!(r["status"], "next", "{r}");
        assert_eq!(
            r["note"],
            format!(
                "{} {}",
                answered_words(said),
                look["note"].as_str().unwrap()
            )
        );
        assert!(r["plain"].as_str().unwrap().contains(said), "{r}");
        assert_eq!(r["clip"], look["clip"]);
        assert_eq!(r["iterating"], look["iterating"]);
        // A second answer takes the first one's place on the note.
        s.turn(json!({"by": "agent", "text": "Is this it?"}))
            .unwrap();
        s.turn(json!({"by": "player", "text": "almost"})).unwrap();
        let r = row("lights");
        assert_eq!(
            r["note"],
            format!(
                "{} {}",
                answered_words("almost"),
                look["note"].as_str().unwrap()
            )
        );
        // A session's row by hand stays theirs.
        let h = Session::create(&p, "test-hop", "byhand", None, "ab").unwrap();
        h.turn(json!({"by": "player", "text": "go on"})).unwrap();
        assert_eq!(row("byhand")["status"], "agent working");
        assert_eq!(row("byhand")["note"], "n");
        // A live worker of the runner's holds the row: it is at work.
        let w = gp_mcp::steps::workers_dir(&p.home);
        fs::create_dir_all(&w).unwrap();
        fs::write(
            w.join("loop-s1.json"),
            json!({"state": "working", "pid": std::process::id(), "started_ms": 1,
                "step": {"id": "s1", "touches": ["row:test-hop/held"]}})
            .to_string(),
        )
        .unwrap();
        let k = Session::create(&p, "test-hop", "held", None, "ab").unwrap();
        k.turn(json!({"by": "player", "text": "yes but dimmer"}))
            .unwrap();
        assert_eq!(row("held")["status"], "agent working");
        assert_eq!(row("held")["note"], "h");
    }

    #[test]
    fn a_name_with_a_path_in_it_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        assert!(Session::create(&p, "../x", "jump", None, "ab").is_err());
        assert!(Session::create(&p, "x", "a/b", None, "ab").is_err());
        assert!(queue_file(&p, "..").is_err());
    }
}
