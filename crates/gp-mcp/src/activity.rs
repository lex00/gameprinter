//! What the agent runs, for the person to see on the page: one plain line
//! per tool call in the agent's log the page's side panel shows, a record
//! while a long call runs (the status line's `running`), and a counter of
//! changes per game the page polls to know its views went stale.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/mechanics/GAME/TOPIC/watch/log.ndjson` | The log the side panel reads (gp-cli's mechanic/watch.rs): TOPIC is the mechanic open on the page (`mechanics/GAME/current`), else `agent` |
//! | `$GP_HOME/work/runs/ID.json` | `{what, command, game, started_ms, pid}`: a call still running after a few seconds; gone when it ends |
//! | `$GP_HOME/work/serve/GAME/changes` | How many times the agent changed GAME through gp_run (a number) |
//!
//! No arguments' values but the game's name and no output go in a line, so
//! no secret or long output reaches the page.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::asks::{alive, game_ok, now_ms};

/// How long a call runs before the page hears it is running.
pub const RUNNING_AFTER: Duration = Duration::from_secs(3);
/// The topic the lines go to when no mechanic is open.
pub const TOPIC: &str = "agent";

/// The game a call concerns: its `game`, `template` or `run` argument (in
/// `args`, or gp_run's `args`), else the game whose page was opened last.
pub fn game_of(home: &Path, args: &Value) -> Option<String> {
    for a in [args, &args["args"]] {
        for k in ["game", "template", "run"] {
            if let Some(g) = a[k].as_str().filter(|g| game_ok(g)) {
                return Some(g.to_owned());
            }
        }
    }
    std::fs::read_to_string(home.join("work/serve/last-game"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|g| game_ok(g))
}

/// The session folder the game's lines go to.
pub fn topic_dir(home: &Path, game: &str) -> PathBuf {
    let g = home.join("mechanics").join(game);
    let open = std::fs::read_to_string(g.join("current"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|m| game_ok(m) && g.join(m).is_dir());
    g.join(open.as_deref().unwrap_or(TOPIC))
}

/// The time of day in UTC, as the log's lines have it.
fn clock() -> String {
    let s = now_ms() / 1000 % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Appends one line to the game's log.
pub fn log(home: &Path, game: &str, text: &str) {
    log_cmd(home, game, text, None);
}

/// Appends one line to the game's log, with the command path or tool that
/// made it (`command`: for the details and the JSON, not the sentence).
pub fn log_cmd(home: &Path, game: &str, text: &str, cmd: Option<&str>) {
    let d = topic_dir(home, game).join("watch");
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let mut line = json!({"at": clock(), "text": text, "by": "tools"});
    if let Some(c) = cmd {
        line["command"] = json!(c);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(d.join("log.ndjson"))
    {
        let _ = std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes());
    }
}

fn runs_dir(home: &Path) -> PathBuf {
    home.join("work").join("runs")
}

/// The calls running now, oldest first: `{what, game, started_ms, pid}`.
pub fn running(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(runs_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v["pid"].as_u64().is_some_and(alive))
        .collect();
    out.sort_by_key(|v| v["started_ms"].as_u64().unwrap_or(0));
    out
}

fn changes_file(home: &Path, game: Option<&str>) -> PathBuf {
    let d = home.join("work").join("serve");
    match game {
        Some(g) => d.join(g).join("changes"),
        None => d.join("changes"),
    }
}

/// How many changes the agent made to a game (and to no game in
/// particular) through gp_run.
pub fn changes(home: &Path, game: Option<&str>) -> u64 {
    let n = |f: PathBuf| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    n(changes_file(home, None)) + game.map_or(0, |g| n(changes_file(home, Some(g))))
}

/// Counts a change to a game, or to no game in particular.
pub fn bump(home: &Path, game: Option<&str>) {
    let f = changes_file(home, game);
    let n = std::fs::read_to_string(&f)
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, format!("{}\n", n + 1));
}

/// What a call did, in the person's words: "Scored the game's tests" when
/// it ended, "scoring the game's tests" while it runs, "publish the game"
/// when it waits for a yes ("Asked to publish the game").
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
    pub did: String,
    pub doing: String,
    pub ask: String,
}

/// A verb's forms: as asked, done and going on.
const VERBS: &[(&str, &str, &str)] = &[
    ("add", "Added", "adding"),
    ("build", "Built", "building"),
    ("change", "Changed", "changing"),
    ("check", "Checked", "checking"),
    ("compare", "Compared", "comparing"),
    ("copy", "Copied", "copying"),
    ("fetch", "Fetched", "fetching"),
    ("find", "Found", "finding"),
    ("fill", "Filled", "filling"),
    ("fit", "Fitted", "fitting"),
    ("land", "Landed", "landing"),
    ("list", "Listed", "listing"),
    ("look", "Looked", "looking"),
    ("make", "Made", "making"),
    ("measure", "Measured", "measuring"),
    ("open", "Opened", "opening"),
    ("pack", "Packed", "packing"),
    ("paint", "Painted", "painting"),
    ("pin", "Pinned", "pinning"),
    ("place", "Placed", "placing"),
    ("play", "Played", "playing"),
    ("publish", "Published", "publishing"),
    ("read", "Read", "reading"),
    ("refine", "Refined", "refining"),
    ("run", "Ran", "running"),
    ("save", "Saved", "saving"),
    ("score", "Scored", "scoring"),
    ("search", "Searched", "searching"),
    ("sort", "Sorted", "sorting"),
    ("start", "Started", "starting"),
    ("take", "Took", "taking"),
    ("test", "Tested", "testing"),
    ("turn", "Turned", "turning"),
    ("wait", "Waited", "waiting"),
    ("write", "Wrote", "writing"),
];

/// The commands and tools a person's journey uses, by command path or tool
/// name: the verb and the rest of what it did. A command path also matches
/// the commands under it ("rule" holds "rule lane").
const PHRASES: &[(&str, &str, &str)] = &[
    // Scoring and the game's tests.
    ("corpus score", "score", "the game's tests"),
    ("corpus show", "look", "up the game's tests"),
    ("corpus verify", "check", "the game's tests"),
    ("corpus import", "turn", "your clips into the game's tests"),
    ("corpus diff", "compare", "two versions of the game's tests"),
    ("clip score", "score", "some of the game's tests"),
    ("clip presses", "save", "a clip as one of the game's tests"),
    ("game score", "score", "the game from its ROM"),
    // The model.
    ("model check", "check", "the model"),
    ("model refine", "refine", "the model"),
    ("model_refine", "refine", "the model"),
    ("model blind", "play", "the ROM to find the player"),
    ("model diff", "compare", "two models"),
    ("model questions", "read", "the model's questions"),
    ("model_questions", "read", "the model's questions"),
    ("model_edit", "change", "the model"),
    ("contract_edit", "change", "the model's checks"),
    ("validate", "check", "the model against recordings"),
    ("rule", "find", "the rule behind a mechanic"),
    ("level", "measure", "a level"),
    // Reading the code.
    ("model static", "read", "the game's code"),
    ("code", "read", "the game's code"),
    ("reach", "run", "the game's code on clips"),
    ("element", "read", "an element's code"),
    ("mechanic leads", "look", "for new mechanics"),
    // The ROM.
    ("roms add", "add", "your ROM"),
    ("roms scan", "look", "for your ROMs"),
    ("template check", "check", "your ROM against the template"),
    ("assets lift", "take", "the pictures from your ROM"),
    // Where the game stands, the next step, the quest log.
    ("progress", "check", "how far the game is"),
    ("mechanic triage", "sort", "the quest log"),
    ("capture background", "start", "the clip agents"),
    ("mechanic queue", "read", "the quest log"),
    ("mechanic show", "read", "your clips and the talk"),
    ("mechanic say", "write", "to you"),
    ("mechanic pin", "pin", "a picture for you"),
    ("mechanic wait", "wait", "for your turn"),
    ("mechanic start", "open", "the game's page"),
    ("mechanic video", "make", "a clip's video"),
    ("view_now", "look", "at your page"),
    // Themes.
    ("theme generate", "make", "the theme's pictures"),
    ("theme check", "check", "the theme"),
    ("theme look check", "check", "the theme"),
    ("theme look new", "start", "a new theme"),
    ("theme kit", "write", "the game's theme kit"),
    ("theme fit", "fit", "the pictures to the game"),
    ("theme sheet", "write", "the prompts for the pictures"),
    ("theme style", "read", "the theme styles"),
    ("theme paint", "paint", "stand-in pictures"),
    ("theme video", "make", "a video with the theme"),
    ("theme list", "list", "the game's themes"),
    ("theme fetch", "fetch", "a theme"),
    ("theme publish", "publish", "the theme"),
    ("assets_generate", "make", "a new picture"),
    ("assets_place", "place", "a picture in the game"),
    ("assets_edit", "change", "a picture"),
    ("sound_edit", "change", "the sound"),
    ("sound_preview", "play", "the sound"),
    // A new game, a template, the browser, landing, publishing.
    ("game new", "start", "a new game from your ROM"),
    ("game fetch", "fetch", "a published game"),
    ("game build", "build", "the game's code"),
    ("game test", "test", "the game's code"),
    ("registry_search", "search", "the templates"),
    ("template_fork", "copy", "a template"),
    ("template extract", "fill", "a template from your ROM"),
    ("template build", "build", "the game's pictures and levels"),
    ("template served", "check", "the game holds no ROM data"),
    ("template pack", "pack", "the game to publish"),
    ("run_preview", "play", "a preview of the game"),
    ("build_publish", "build", "the game for the browser"),
    ("model gate", "land", "a change to the game"),
    ("tool gate", "land", "a change to Game Printer"),
    ("registry publish", "publish", "the game"),
];

/// What a call did in the person's words: from [`PHRASES`] by its command
/// path or tool name, else "Ran corpus show" with the path as it is.
pub fn said(what: &str) -> Said {
    let what = what.trim();
    let mut key = what;
    let found = loop {
        if let Some(p) = PHRASES.iter().find(|p| p.0 == key) {
            break Some(p);
        }
        match key.rsplit_once(' ') {
            Some((head, _)) => key = head,
            None => break None,
        }
    };
    let (verb, rest) = found.map_or(("run", what), |p| (p.1, p.2));
    let (_, done, going) = VERBS
        .iter()
        .find(|v| v.0 == verb)
        .copied()
        .unwrap_or(("run", "Ran", "running"));
    Said {
        did: format!("{done} {rest}"),
        doing: format!("{going} {rest}"),
        ask: format!("{verb} {rest}"),
    }
}

/// How a call ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ended {
    /// "done", "failed (no such game)": the line ends with the time.
    As(String),
    /// Refused until the person says yes on the page.
    WaitsForYes,
    /// Refused: the person said no to it on the page.
    SaidNo,
}

/// The line a call leaves: "Scored the game's tests: done, 41 s".
pub fn line(said: &Said, how: &Ended, took: Duration) -> String {
    match how {
        Ended::WaitsForYes => format!("Asked to {}: waiting for your yes", said.ask),
        Ended::SaidNo => format!("Asked to {}: you said no", said.ask),
        Ended::As(how) => {
            let s = took.as_secs_f64();
            let took = if s < 10.0 {
                format!("{s:.1} s")
            } else {
                format!("{} s", s.round())
            };
            format!("{}: {how}, {took}", said.did)
        }
    }
}

/// A call being watched: a line and a record once it runs long.
pub struct Watch {
    home: PathBuf,
    game: Option<String>,
    said: Said,
    cmd: String,
    started: Instant,
    stop: Option<mpsc::Sender<()>>,
    timer: Option<std::thread::JoinHandle<()>>,
    record: PathBuf,
}

/// Starts watching a call, `cmd` its command path or tool name: after
/// [`RUNNING_AFTER`] the page hears it runs.
pub fn start(home: &Path, game: Option<String>, cmd: String) -> Watch {
    let said = said(&cmd);
    let started_ms = now_ms();
    let record = runs_dir(home).join(format!("{started_ms}-{}.json", std::process::id()));
    let (tx, rx) = mpsc::channel::<()>();
    let (h, g, s, c, r) = (
        home.to_owned(),
        game.clone(),
        said.clone(),
        cmd.clone(),
        record.clone(),
    );
    let timer = std::thread::spawn(move || {
        if rx.recv_timeout(RUNNING_AFTER) != Err(mpsc::RecvTimeoutError::Timeout) {
            return;
        }
        let _ = std::fs::create_dir_all(runs_dir(&h));
        let rec = json!({"what": s.doing, "command": c, "game": g, "started_ms": started_ms, "pid": std::process::id()});
        let _ = std::fs::write(&r, rec.to_string());
        if let Some(g) = &g {
            let mut d = s.doing.clone();
            if let Some(f) = d.get_mut(..1) {
                f.make_ascii_uppercase();
            }
            log_cmd(&h, g, &format!("{d}, still running"), Some(&c));
        }
    });
    Watch {
        home: home.to_owned(),
        game,
        said,
        cmd,
        started: Instant::now(),
        stop: Some(tx),
        timer: Some(timer),
        record,
    }
}

impl Watch {
    /// Ends the watch with how the call ended: a line in the log.
    pub fn end(mut self, how: &Ended) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.timer.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.record);
        if let Some(g) = &self.game {
            let text = line(&self.said, how, self.started.elapsed());
            log_cmd(&self.home, g, &text, Some(&self.cmd));
        }
    }
}

/// The first short reason in a command's errors: its first line that
/// starts "error" or "gp:", else its last line, without that in front and
/// with each path's folders left out ("no-such-game has neither a
/// corpus.toml nor a template.toml").
pub fn reason(errors: &str) -> Option<String> {
    let lines: Vec<&str> = errors
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let said = |l: &&&str| {
        let l = l.to_ascii_lowercase();
        l.starts_with("error") || l.starts_with("gp:")
    };
    let l = lines.iter().find(said).or(lines.last())?;
    let l = ["Error:", "error:", "gp:"]
        .iter()
        .find_map(|p| l.strip_prefix(p))
        .unwrap_or(l)
        .trim();
    let words: Vec<&str> = l
        .split(' ')
        .map(|w| match w.trim_end_matches('/').rsplit_once('/') {
            Some((_, last)) if w.contains('/') && !last.is_empty() => last,
            _ => w,
        })
        .collect();
    let l = words.join(" ");
    (!l.is_empty()).then(|| cut(&l, 80))
}

/// Cuts words to `n` characters at a word.
pub fn cut(s: &str, n: usize) -> String {
    let s = s.split(['\n', '\r']).next().unwrap_or("").trim();
    if s.chars().count() <= n {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(n).collect();
    if let Some(i) = out.rfind(' ') {
        out.truncate(i);
    }
    format!("{}...", out.trim_end_matches([',', ';', ':']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_leaves_a_line_in_the_open_mechanic_s_log() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        std::fs::create_dir_all(home.join("mechanics/dk/walk")).unwrap();
        std::fs::write(home.join("mechanics/dk/current"), "walk\n").unwrap();
        assert_eq!(
            game_of(home, &json!({"args": {"game": "dk"}})),
            Some("dk".into())
        );
        assert_eq!(game_of(home, &json!({"game": "../x"})), None);
        let w = start(home, Some("dk".into()), "corpus score".into());
        w.end(&Ended::As("done".into()));
        let log = std::fs::read_to_string(home.join("mechanics/dk/walk/watch/log.ndjson")).unwrap();
        let line: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert!(
            line["text"]
                .as_str()
                .unwrap()
                .starts_with("Scored the game's tests: done, ")
        );
        // The command is in the record, not in the sentence.
        assert_eq!(line["command"], "corpus score");
        assert!(running(home).is_empty());
        // No mechanic open: the agent topic.
        log_to(home, "tp");
        assert!(home.join("mechanics/tp/agent/watch/log.ndjson").is_file());
        assert_eq!(changes(home, Some("dk")), 0);
        bump(home, Some("dk"));
        bump(home, None);
        assert_eq!(changes(home, Some("dk")), 2);
        assert_eq!(changes(home, Some("tp")), 1);
        assert_eq!(cut("one two three four", 9), "one two...");
    }

    #[test]
    fn a_line_says_what_the_agent_did_in_a_few_plain_words() {
        let t = |w: &str, how: Ended, ms: u64| line(&said(w), &how, Duration::from_millis(ms));
        let done = || Ended::As("done".into());
        assert_eq!(
            t("corpus score", done(), 41_200),
            "Scored the game's tests: done, 41 s"
        );
        assert_eq!(
            t("model check", done(), 2_000),
            "Checked the model: done, 2.0 s"
        );
        assert_eq!(
            t(
                "corpus show",
                Ended::As("failed (no such game)".into()),
                100
            ),
            "Looked up the game's tests: failed (no such game), 0.1 s"
        );
        assert_eq!(
            t("registry publish", Ended::WaitsForYes, 0),
            "Asked to publish the game: waiting for your yes"
        );
        assert_eq!(
            t("registry publish", Ended::SaidNo, 0),
            "Asked to publish the game: you said no"
        );
        // A command under a listed one, and a tool.
        assert_eq!(said("rule lane").did, "Found the rule behind a mechanic");
        assert_eq!(said("template_fork").did, "Copied a template");
        assert_eq!(said("corpus score").doing, "scoring the game's tests");
        assert_eq!(said("theme look check").did, "Checked the theme");
        assert_eq!(said("theme look new").did, "Started a new theme");
        // Anything else: the command path, never its help.
        assert_eq!(
            t("corpus reduce", Ended::As("failed".into()), 0),
            "Ran corpus reduce: failed, 0.0 s"
        );
        assert_eq!(said("assets_history").ask, "run assets_history");
        // Every phrase's verb has its forms.
        for p in PHRASES {
            assert!(VERBS.iter().any(|v| v.0 == p.1), "{}: no verb {}", p.0, p.1);
        }
        assert_eq!(
            reason("warning: x\nError: no game dk in the games checkout\n").as_deref(),
            Some("no game dk in the games checkout")
        );
        assert_eq!(
            reason("gp: /tmp/x/no-such-game has neither a corpus.toml nor a template.toml")
                .as_deref(),
            Some("no-such-game has neither a corpus.toml nor a template.toml")
        );
        assert_eq!(reason("  \n"), None);
        assert!(reason(&"word ".repeat(40)).unwrap().len() <= 84);
    }

    fn log_to(home: &Path, game: &str) {
        log(home, game, "x");
    }
}
