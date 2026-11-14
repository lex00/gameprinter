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

/// A game's quest log; empty when there's none yet.
pub(crate) fn read_queue(f: &Path) -> Vec<Value> {
    fs::read_to_string(f)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn write_queue(f: &Path, q: &[Value]) -> Result<()> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(f, serde_json::to_string_pretty(q)?)?;
    Ok(())
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

    /// Opens the session, making it with `prompt` as what the mechanic is
    /// when it is new.
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
    pub fn clips(&self) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut names: Vec<PathBuf> = fs::read_dir(self.dir.join("clips"))?
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

    /// Everything the page and `show` read: the mechanic, its clips, the
    /// conversation, whose turn it is and the game's quest log.
    pub fn state(&self) -> Result<Value> {
        let info = self.info()?;
        let clips = self.clips()?;
        let turns = self.turns()?;
        let waiting = match turns.last().and_then(|t| t["by"].as_str()) {
            Some("player") | None => "agent",
            Some(_) => "player",
        };
        // What the agent says it is doing, while no turn has come since.
        let busy = fs::read_to_string(self.dir.join("busy.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(|b| b["after"].as_u64() == Some(turns.len() as u64))
            .and_then(|b| b["text"].as_str().map(str::to_owned));
        Ok(json!({
            "busy": busy,
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

    /// Notes what the agent is doing until its next turn.
    pub fn set_busy(&self, text: &str) -> Result<()> {
        let n = self.turns()?.len();
        fs::write(
            self.dir.join("busy.json"),
            json!({"text": text, "after": n}).to_string(),
        )?;
        Ok(())
    }

    /// Appends a turn (`by`, `text` and whatever else it carries); returns
    /// its number.
    pub fn turn(&self, v: Value) -> Result<usize> {
        let _g = self.lock.lock().expect("lock");
        self.append_turn(v)
    }

    /// [`Self::turn`] for a caller that holds the lock.
    fn append_turn(&self, mut v: Value) -> Result<usize> {
        let n = self.turns()?.len() + 1;
        v["n"] = json!(n);
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("talk.ndjson"))?;
        writeln!(f, "{v}")?;
        // The queue follows whose turn it is: the person's turn makes the
        // mechanic "agent working", the agent's "asking you". Done stays done.
        let qf = self.dir.with_file_name("queue.json");
        let mut q = read_queue(&qf);
        let me = self.mechanic();
        if let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(me))
            && r["status"] != json!("done")
        {
            r["status"] = json!(if v["by"] == json!("player") {
                "agent working"
            } else {
                "asking you"
            });
            write_queue(&qf, &q)?;
        }
        Ok(n)
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
        s.turn(
            json!({"by": "agent", "text": "show me a running jump", "choices": ["ok", "not now"]}),
        )
        .unwrap();
        let v = s.state().unwrap();
        assert_eq!(v["waiting_for"], "player");
        // Busy lasts only until the agent's next turn.
        assert!(v["busy"].is_null());
        let text = show_text(&v);
        assert!(text.contains("a held jump"), "{text}");
        assert!(text.contains("[choices: ok | not now]"), "{text}");
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
    /// stays done.
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
        s.turn(json!({"by": "agent", "text": "and back?"})).unwrap();
        assert_eq!(read_queue(&qf)[0]["status"], "asking you");
        let j = Session::create(&p, "test-hop", "jump", None, "ab").unwrap();
        j.turn(json!({"by": "player", "text": "again"})).unwrap();
        assert_eq!(read_queue(&qf)[1]["status"], "done");
        assert_eq!(s.state().unwrap()["queue"].as_array().unwrap().len(), 2);
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
