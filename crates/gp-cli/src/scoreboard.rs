//! A game's scoreboard (docs/loop.md, "The scoreboard"): a line for each
//! part of the game the owner works on now, each measured on one clip of
//! the game's corpus. A line has the game state (the gate's straight-through
//! match for its clip, `$GP_HOME/work/refine/GAME/status.json`), the
//! background and the sprites (frames matched of frames, from `gp picture
//! GAME --run GAME --clip ID`), and the looks the owner approved (none yet).
//!
//! Which parts is the game's own file, `GAME/scoreboard.toml`: each
//! `[[line]]` has a `name` and the `clip` it is measured on, or a `level`
//! of the corpus (`corpus.toml`'s clips' `level`), whose first clip is
//! taken. A line with `own_draws = true` measures instead the clip's frames
//! together straight through with the model on its own random generator
//! (`gp clip score --own-draws`). Nothing here names a game.
//!
//! A picture check takes about a minute a clip, so gp serve measures in the
//! background ([`start`]): a line is measured again when its key changes
//! (the model the gate last landed, the clip's recipe and files, the gp
//! that measures), and the result is kept in
//! `$GP_HOME/work/scoreboard/GAME.json` with the measure before it, so each
//! number says how it moved (up, down, same). `gp loop health` and the
//! screen page (`GET /api/scoreboard/{game}`) only read that file.
//!
//! A quest-log row's target is a measure on one of the lines (`level 1
//! sprites at least 330 of 336`): the measurer closes the row when the
//! board gets there ([`close_rows`]), and `gp mechanic queue` refuses to
//! set it done before ([`may_close`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest as _;

/// The game's file naming the lines.
pub(crate) const FILE: &str = "scoreboard.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardFile {
    #[serde(default)]
    line: Vec<LineSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineSpec {
    name: String,
    #[serde(default)]
    clip: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    own_draws: bool,
}

/// One line of the board: its name, the clip it is measured on and that
/// clip's level in the corpus. An `own_draws` line measures the model on
/// its own random generator instead of the pictures: the clip's frames
/// together straight through with none of the original's draws handed
/// over (`gp clip score --own-draws`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    pub name: String,
    pub clip: String,
    pub level: String,
    pub own_draws: bool,
}

impl Line {
    /// Where the line's measure is kept: its clip, and apart from it for
    /// an `own_draws` line, so a clip can have both.
    pub(crate) fn slot(&self) -> String {
        if self.own_draws {
            format!("{} own draws", self.clip)
        } else {
            self.clip.clone()
        }
    }
}

/// The corpus's clips as tables, in the file's order.
fn corpus_clips(game_dir: &Path) -> Result<Vec<toml::Table>, String> {
    let f = game_dir.join(crate::corpus_score::FILE);
    let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    let t: toml::Table = toml::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
    Ok(t.get("clip")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_table().cloned())
        .collect())
}

fn field<'a>(t: &'a toml::Table, k: &str) -> &'a str {
    t.get(k).and_then(toml::Value::as_str).unwrap_or("")
}

/// The board's lines, or None when the game has no scoreboard file. A line
/// whose clip the corpus doesn't have is an error that names it.
pub(crate) fn lines(game_dir: &Path) -> Option<Result<Vec<Line>, String>> {
    let f = game_dir.join(FILE);
    let text = std::fs::read_to_string(&f).ok()?;
    Some((|| {
        let b: BoardFile = toml::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        let clips = corpus_clips(game_dir)?;
        let mut out = Vec::new();
        for l in b.line {
            let found = if l.clip.is_empty() {
                clips.iter().find(|c| field(c, "level") == l.level)
            } else {
                clips.iter().find(|c| field(c, "id") == l.clip)
            };
            let Some(c) = found else {
                return Err(if l.clip.is_empty() {
                    format!(
                        "{}: line {:?}: the corpus has no clip of level {:?}",
                        f.display(),
                        l.name,
                        l.level
                    )
                } else {
                    format!(
                        "{}: line {:?}: the corpus has no clip {}",
                        f.display(),
                        l.name,
                        l.clip
                    )
                });
            };
            out.push(Line {
                name: l.name,
                clip: field(c, "id").to_owned(),
                level: field(c, "level").to_owned(),
                own_draws: l.own_draws,
            });
        }
        Ok(out)
    })())
}

/// The gate's status for the game, or Null.
fn status(home: &Path, game: &str) -> Value {
    std::fs::read(home.join("work/refine").join(game).join("status.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// A clip's straight-through match in the gate's status: its percent and
/// frames.
fn state_of(status: &Value, clip: &str) -> Option<(f64, u64)> {
    let c = status["clips"]
        .as_array()?
        .iter()
        .find(|c| c["id"] == json!(clip))?;
    Some((c["straight"]["pct"].as_f64()?, c["frames"].as_u64()?))
}

/// The gp that measures: the gates' published build when there is one,
/// else this one.
fn measuring_gp(home: &Path) -> PathBuf {
    let published = home.join("bin").join("gp");
    if published.is_file() {
        return published;
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gp"))
}

/// What a line's measure depends on, hashed: the model the gate last
/// landed (its hash in the status, else model.toml), the game's hook code,
/// the clip's recipe and
/// the files it names, and the gp that measures (its size and time). A
/// change to any of them makes the line out of date.
pub(crate) fn key_of(home: &Path, game_dir: &Path, game: &str, clip: &str, gp: &Path) -> String {
    let mut h = sha2::Sha256::new();
    hash_model(&mut h, home, game_dir, game);
    if let Ok(clips) = corpus_clips(game_dir)
        && let Some(c) = clips.iter().find(|c| field(c, "id") == clip)
    {
        h.update(toml::to_string(c).unwrap_or_default());
        for k in ["start", "presses"] {
            let f = field(c, k);
            if !f.is_empty() {
                h.update(std::fs::read(game_dir.join(f)).unwrap_or_default());
            }
        }
    }
    hash_gp(&mut h, gp);
    hex::encode(h.finalize())[..16].to_owned()
}

/// The model a measure is of: the one the gate last landed (its hash in
/// the status), else model.toml; and the game's own hooks, which the
/// picture check and `gp sealed score` build its module from.
fn hash_model(h: &mut sha2::Sha256, home: &Path, game_dir: &Path, game: &str) {
    let st = status(home, game);
    match st["model"]["hash"].as_str() {
        Some(m) => h.update(format!("model {m}\n")),
        None => h.update(std::fs::read(game_dir.join(gp_model::MODEL_FILE)).unwrap_or_default()),
    }
    if let Ok(Some(code)) = crate::package::hook_code(game_dir) {
        h.update(format!("hooks {code}\n"));
    }
}

/// The gp that measures, by its size and time.
fn hash_gp(h: &mut sha2::Sha256, gp: &Path) {
    let m = std::fs::metadata(gp).ok();
    let t = m
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis());
    h.update(format!("gp {} {t}\n", m.map_or(0, |m| m.len())));
}

/// The held-out runs of a game: the sealed runs filed under
/// `$GP_HOME/sealed/FOLDER/` for the game's folder name or a source ID its
/// roms.toml lists ([`crate::sealed::named_runs`]). Their presses are never
/// read here; `gp sealed score` alone plays them.
pub(crate) fn held_runs(home: &Path, games: &Path, game: &str) -> Vec<crate::sealed::Named> {
    let dir = games.join(game);
    crate::sealed::named_runs(home, |f| {
        f == game || crate::labels::game_folder(games, f).is_some_and(|d| d == dir)
    })
}

/// A held-out run's line key in the cache: FOLDER/RUN.
fn held_id(r: &crate::sealed::Named) -> String {
    format!("{}/{}", r.folder, r.run)
}

/// What a held-out run's measure depends on: the model, the run's files
/// (hashed, never shown) and the gp that measures.
pub(crate) fn held_key(
    home: &Path,
    game_dir: &Path,
    game: &str,
    r: &crate::sealed::Named,
    gp: &Path,
) -> String {
    let mut h = sha2::Sha256::new();
    hash_model(&mut h, home, game_dir, game);
    h.update(format!("sealed {}\n", r.run));
    h.update(std::fs::read(&r.log).unwrap_or_default());
    h.update(std::fs::read(r.log.with_extension("toml")).unwrap_or_default());
    hash_gp(&mut h, gp);
    hex::encode(h.finalize())[..16].to_owned()
}

/// One score of a held-out run, as `gp sealed score` gives it: frames
/// together of frames straight through and with resync, and the longest
/// stretch in seconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Held {
    pub straight: [u64; 2],
    pub resync: [u64; 2],
    #[serde(default)]
    pub longest: String,
}

/// A held-out run's kept score.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct HeldEntry {
    pub key: String,
    pub measured_ms: u64,
    #[serde(default)]
    pub now: Option<Held>,
    #[serde(default)]
    pub before: Option<Held>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One measure of a line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Numbers {
    pub background: [u64; 2],
    pub sprites: [u64; 2],
    /// The game state's percent when it was measured.
    #[serde(default)]
    pub state: Option<f64>,
    /// An `own_draws` line's frames together straight through, of frames,
    /// with the model on its own generator.
    #[serde(default)]
    pub own: Option<[u64; 2]>,
}

/// A line's kept measure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub key: String,
    pub measured_ms: u64,
    #[serde(default)]
    pub now: Option<Numbers>,
    /// The measure before `now`, for how each number moved.
    #[serde(default)]
    pub before: Option<Numbers>,
    /// Why the last measure gave no numbers.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Cache {
    /// By clip id.
    #[serde(default)]
    pub lines: BTreeMap<String, Entry>,
    /// The held-out runs' scores, by FOLDER/RUN.
    #[serde(default)]
    pub held: BTreeMap<String, HeldEntry>,
}

fn cache_file(home: &Path, game: &str) -> PathBuf {
    home.join("work")
        .join("scoreboard")
        .join(format!("{game}.json"))
}

pub(crate) fn read_cache(home: &Path, game: &str) -> Cache {
    std::fs::read(cache_file(home, game))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_cache(home: &Path, game: &str, c: &Cache) -> std::io::Result<()> {
    let f = cache_file(home, game);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).unwrap_or_default())?;
    std::fs::rename(tmp, f)
}

/// Keeps a measure of `clip` under `key`: numbers that differ from the
/// last measure's move it to `before`; the same numbers again keep the
/// `before` they had, so a number still says how it moved last.
pub(crate) fn record(
    home: &Path,
    game: &str,
    clip: &str,
    key: &str,
    got: Result<Numbers, String>,
) -> std::io::Result<()> {
    let mut c = read_cache(home, game);
    let e = c.lines.entry(clip.to_owned()).or_default();
    e.key = key.to_owned();
    e.measured_ms = gp_mcp::asks::now_ms();
    match got {
        Ok(n) => {
            if e.now.is_some_and(|old| old != n) {
                e.before = e.now;
            }
            e.now = Some(n);
            e.error = None;
        }
        Err(why) => e.error = Some(why),
    }
    write_cache(home, game, &c)
}

/// Keeps a held-out run's score under `key`, as [`record`] keeps a line's.
pub(crate) fn record_held(
    home: &Path,
    game: &str,
    id: &str,
    key: &str,
    got: Result<Held, String>,
) -> std::io::Result<()> {
    let mut c = read_cache(home, game);
    let e = c.held.entry(id.to_owned()).or_default();
    e.key = key.to_owned();
    e.measured_ms = gp_mcp::asks::now_ms();
    match got {
        Ok(n) => {
            if e.now.as_ref().is_some_and(|old| *old != n) {
                e.before = e.now.take();
            }
            e.now = Some(n);
            e.error = None;
        }
        Err(why) => e.error = Some(why),
    }
    write_cache(home, game, &c)
}

/// A share of frames as a percent to a tenth.
fn pct(f: [u64; 2]) -> f64 {
    if f[1] == 0 {
        return 0.0;
    }
    (f[0] as f64 * 1000.0 / f[1] as f64).round() / 10.0
}

/// How `now` moved from `before`, as frames matched of frames: up, down or
/// same; empty with no measure before.
fn moved(now: [u64; 2], before: Option<[u64; 2]>) -> &'static str {
    let Some(b) = before else { return "" };
    // Each as a share of its own frames, so a longer clip compares too.
    let (a, z) = (
        u128::from(now[0]) * u128::from(b[1].max(1)),
        u128::from(b[0]) * u128::from(now[1].max(1)),
    );
    match a.cmp(&z) {
        std::cmp::Ordering::Greater => "up",
        std::cmp::Ordering::Less => "down",
        std::cmp::Ordering::Equal => "same",
    }
}

fn moved_pct(now: f64, before: Option<f64>) -> &'static str {
    match before {
        None => "",
        Some(b) if (now - b).abs() < 0.05 => "same",
        Some(b) if now > b => "up",
        Some(_) => "down",
    }
}

/// The clip being measured now in this process, as GAME/CLIP.
static MEASURING: Mutex<Option<String>> = Mutex::new(None);

fn measuring_now() -> Option<String> {
    MEASURING.lock().map(|m| m.clone()).unwrap_or_default()
}

/// The game's board as the page and `gp loop health` show it: `{game,
/// lines, error}`, each line with its state, background, sprites and
/// approved, each number with how it moved, and whether it is out of date
/// or being measured. None when the game has no scoreboard file.
pub(crate) fn board(home: &Path, games: &Path, game: &str) -> Option<Value> {
    let dir = games.join(game);
    let lines = match lines(&dir)? {
        Ok(l) => l,
        Err(e) => return Some(json!({"game": game, "lines": [], "error": e})),
    };
    let st = status(home, game);
    let cache = read_cache(home, game);
    let gp = measuring_gp(home);
    let busy = measuring_now();
    let looks = crate::approved::read(&dir).unwrap_or_default();
    let rows: Vec<Value> = lines
        .iter()
        .map(|l| {
            let e = cache.lines.get(&l.slot());
            let now = e.and_then(|e| e.now);
            let before = e.and_then(|e| e.before);
            let frames = |f: fn(&Numbers) -> [u64; 2]| -> Value {
                match now {
                    Some(n) if l.own_draws == n.own.is_some() => json!({
                        "matched": f(&n)[0],
                        "total": f(&n)[1],
                        "moved": moved(f(&n), before.as_ref().map(f)),
                    }),
                    _ => Value::Null,
                }
            };
            let (background, sprites, own) = if l.own_draws {
                (
                    Value::Null,
                    Value::Null,
                    frames(|n| n.own.unwrap_or_default()),
                )
            } else {
                (frames(|n| n.background), frames(|n| n.sprites), Value::Null)
            };
            // The state is the gate's, read now; it moved since the last
            // measure when it differs from the state then.
            let state = state_of(&st, &l.clip).map(|(pct, frames)| {
                let then = now.and_then(|n| n.state);
                let mv = match then {
                    Some(t) if (t - pct).abs() >= 0.05 => moved_pct(pct, Some(t)),
                    _ => moved_pct(pct, before.and_then(|b| b.state)),
                };
                json!({"pct": pct, "frames": frames, "moved": mv})
            });
            json!({
                "name": l.name,
                "clip": l.clip,
                "level": l.level,
                "own_draws": l.own_draws,
                "state": state,
                "background": background,
                "sprites": sprites,
                "own": own,
                "approved": [],
                "locked": locked(&looks, &l.clip, background["total"].as_u64()),
                "confirm_row": confirm_row(&l.name),
                "measured_ms": e.map(|e| e.measured_ms),
                "stale": e.is_none_or(|e| e.key != key_of(home, &dir, game, &l.clip, &gp)),
                "measuring": busy.as_deref() == Some(&format!("{game}/{}", l.slot())),
                "error": e.and_then(|e| e.error.clone()),
            })
        })
        .collect();
    let held: Vec<Value> = held_runs(home, games, game)
        .iter()
        .map(|r| {
            let id = held_id(r);
            let e = cache.held.get(&id);
            let now = e.and_then(|e| e.now.clone());
            let before = e.and_then(|e| e.before.clone());
            let share = |f: fn(&Held) -> [u64; 2]| -> Value {
                match &now {
                    Some(n) => json!({
                        "together": f(n)[0],
                        "frames": f(n)[1],
                        "pct": pct(f(n)),
                        "moved": moved(f(n), before.as_ref().map(f)),
                    }),
                    None => Value::Null,
                }
            };
            json!({
                "name": if r.note.trim().is_empty() { format!("sealed run {}", r.run) } else { r.note.trim().to_owned() },
                "run": r.run,
                "folder": r.folder,
                "straight": share(|n| n.straight),
                "resync": share(|n| n.resync),
                "longest": now.as_ref().map(|n| n.longest.clone()),
                "measured_ms": e.map(|e| e.measured_ms),
                "stale": e.is_none_or(|e| e.key != held_key(home, &dir, game, r, &gp)),
                "measuring": busy.as_deref() == Some(&format!("{game}/sealed:{id}")),
                "error": e.and_then(|e| e.error.clone()),
            })
        })
        .collect();
    Some(json!({"game": game, "lines": rows, "held_out": held, "error": null}))
}

/// The owner's lock on a line's clip, as the page shows it (`{day, row,
/// frames}`), or null: a look of approved.toml on the clip from its first
/// frame to its last (`total` frames, when measured; any look from frame 0
/// before the line is measured).
fn locked(looks: &[crate::approved::Look], clip: &str, total: Option<u64>) -> Value {
    looks
        .iter()
        .filter(|k| k.clip == clip && k.frames[0] == 0)
        .filter(|k| total.is_none_or(|t| k.frames[1] >= t.saturating_sub(1)))
        .max_by(|a, b| a.approved.cmp(&b.approved))
        .map_or(
            Value::Null,
            |k| json!({"day": k.approved, "row": k.row, "frames": k.frames}),
        )
}

/// The board's line a target names, by its words (`level 1 sprites at
/// least 330 of 336` is line `level 1`, in lower case), or None when the
/// target doesn't read. The page counts a line's rows by it.
pub(crate) fn target_line(target: &str) -> Option<String> {
    parse_target(target).ok().map(|t| t.line)
}

/// The board as text lines, for `gp loop health`.
pub(crate) fn text(b: &Value) -> Vec<String> {
    let game = b["game"].as_str().unwrap_or("");
    if let Some(e) = b["error"].as_str() {
        return vec![format!("SCORE    {game}'s scoreboard can't be read: {e}")];
    }
    let word = |m: &Value| match m.as_str().unwrap_or("") {
        "" => String::new(),
        m => format!(" {m}"),
    };
    let frames = |v: &Value| {
        if v.is_null() {
            "not measured yet".to_owned()
        } else {
            format!("{} of {}{}", v["matched"], v["total"], word(&v["moved"]))
        }
    };
    let mut out = vec![format!("SCORE    {game}'s scoreboard:")];
    for l in b["lines"].as_array().into_iter().flatten() {
        let state = if l["state"].is_null() {
            "no gate score".to_owned()
        } else {
            format!(
                "{}%{}",
                l["state"]["pct"].as_f64().unwrap_or(0.0),
                word(&l["state"]["moved"])
            )
        };
        let own = l["own_draws"] == json!(true);
        let numbers = if own { &l["own"] } else { &l["background"] };
        let mut s = if own {
            format!(
                "           {} ({}): on the model's own random draws, straight through {} (with the original's: {state})",
                l["name"].as_str().unwrap_or(""),
                l["clip"].as_str().unwrap_or(""),
                frames(&l["own"]),
            )
        } else {
            format!(
                "           {} ({}): state {state}, background {}, sprites {}, approved none",
                l["name"].as_str().unwrap_or(""),
                l["clip"].as_str().unwrap_or(""),
                frames(&l["background"]),
                frames(&l["sprites"]),
            )
        };
        if let Some(e) = l["error"].as_str() {
            s.push_str(&format!(
                " (the last measure failed: {})",
                crate::next::cut(e, 120)
            ));
        } else if l["stale"] == json!(true) && !numbers.is_null() {
            s.push_str(" (out of date: gp serve measures it again)");
        }
        out.push(s);
    }
    let held = b["held_out"].as_array().cloned().unwrap_or_default();
    if held.is_empty() {
        out.push(
            "           held out: no sealed runs yet (STOP AND SEAL on the screen page keeps a recording no worker sees)"
                .to_owned(),
        );
    }
    for r in &held {
        let share = |v: &Value| {
            if v.is_null() {
                "not scored yet".to_owned()
            } else {
                format!(
                    "{}% ({} of {}){}",
                    v["pct"].as_f64().unwrap_or(0.0),
                    v["together"],
                    v["frames"],
                    word(&v["moved"])
                )
            }
        };
        let mut s = format!(
            "           held out {} (sealed run {}): straight {}, resync {}",
            r["name"].as_str().unwrap_or(""),
            r["run"].as_str().unwrap_or(""),
            share(&r["straight"]),
            share(&r["resync"]),
        );
        if let Some(l) = r["longest"].as_str() {
            s.push_str(&format!(", longest {l} s"));
        }
        if let Some(e) = r["error"].as_str() {
            s.push_str(&format!(
                " (the last score failed: {})",
                crate::next::cut(e, 120)
            ));
        } else if r["stale"] == json!(true) && !r["straight"].is_null() {
            s.push_str(" (out of date: gp serve scores it again)");
        }
        out.push(s);
    }
    out
}

/// Scores one held-out run with `gp sealed score` (the coordinator's
/// command; gp serve's measurer is the coordinator) and reads its numbers.
fn measure_held(
    gp: &Path,
    games: &Path,
    r: &crate::sealed::Named,
    module: Option<&Path>,
) -> Result<Held, String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(gp);
    cmd.arg("--json")
        .args(["sealed", "score", &r.folder, &r.run])
        .arg("--games")
        .arg(games);
    if let Some(m) = module {
        cmd.arg("--module").arg(m);
    }
    let out = cmd
        .current_dir(games)
        .env("GP_GAMES_DIR", games)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", gp.display()))?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    held_of(&v).ok_or_else(|| {
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "gp sealed score gave no numbers".into())
    })
}

/// The numbers of `gp sealed score`'s JSON.
fn held_of(v: &Value) -> Option<Held> {
    let pair = |k: &str| -> Option<[u64; 2]> {
        let s = &v["score"][k];
        Some([s["together"].as_u64()?, s["frames"].as_u64()?])
    };
    Some(Held {
        straight: pair("straight")?,
        resync: pair("resync")?,
        longest: v["longest"].as_str().unwrap_or("").to_owned(),
    })
}

/// The held-out runs out of date, as (game, run, key), as
/// [`out_of_date`] finds lines.
pub(crate) fn held_out_of_date(
    home: &Path,
    games: &Path,
    gp: &Path,
) -> Vec<(String, crate::sealed::Named, String)> {
    let now = gp_mcp::asks::now_ms();
    let mut out = Vec::new();
    for g in crate::next::games_in(games) {
        if !matches!(lines(&games.join(&g)), Some(Ok(_))) {
            continue;
        }
        let cache = read_cache(home, &g);
        for r in held_runs(home, games, &g) {
            let key = held_key(home, &games.join(&g), &g, &r, gp);
            let due = cache.held.get(&held_id(&r)).is_none_or(|e| {
                e.key != key || (e.error.is_some() && now.saturating_sub(e.measured_ms) > RETRY_MS)
            });
            if due {
                out.push((g.clone(), r, key));
            }
        }
    }
    out
}

/// Scores every held-out run out of date once, one at a time, with `run`
/// doing the score.
pub(crate) fn refresh_held(
    home: &Path,
    games: &Path,
    gp: &Path,
    mut run: impl FnMut(&str, &crate::sealed::Named) -> Result<Held, String>,
) {
    for (g, r, key) in held_out_of_date(home, games, gp) {
        let id = held_id(&r);
        if let Ok(mut m) = MEASURING.lock() {
            *m = Some(format!("{g}/sealed:{id}"));
        }
        let got = run(&g, &r);
        if let Ok(mut m) = MEASURING.lock() {
            *m = None;
        }
        let _ = record_held(home, &g, &id, &key, got);
    }
}

/// Runs the picture check on one clip and reads its numbers.
fn measure(
    gp: &Path,
    games: &Path,
    game: &str,
    clip: &str,
    module: Option<&Path>,
) -> Result<Numbers, String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(gp);
    cmd.arg("--json")
        .arg("picture")
        .arg(games.join(game))
        .args(["--run", game, "--clip", clip]);
    if let Some(m) = module {
        cmd.arg("--module").arg(m);
    }
    let out = cmd
        .current_dir(games)
        .env("GP_GAMES_DIR", games)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", gp.display()))?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let c = &v["clips"][clip];
    let pair = |k: &str| -> Option<[u64; 2]> {
        let a = c["screen"][k].as_array()?;
        Some([a.first()?.as_u64()?, a.get(1)?.as_u64()?])
    };
    match (pair("background"), pair("sprites")) {
        (Some(background), Some(sprites)) => Ok(Numbers {
            background,
            sprites,
            state: None,
            own: None,
        }),
        _ => Err(c["error"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "the picture check gave no numbers".into())),
    }
}

/// Scores one clip with the model on its own random draws (`gp clip score
/// --own-draws`), into the board's own folder, and reads its frames
/// together straight through.
fn measure_own(
    gp: &Path,
    home: &Path,
    games: &Path,
    game: &str,
    clip: &str,
    module: Option<&Path>,
) -> Result<Numbers, String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(gp);
    cmd.arg("--json")
        .args(["clip", "score"])
        .arg(games.join(game))
        .arg(clip)
        .arg("--own-draws")
        .arg("--out")
        .arg(home.join("work/scoreboard/own").join(game));
    if let Some(m) = module {
        cmd.arg("--module").arg(m);
    }
    let out = cmd
        .current_dir(games)
        .env("GP_GAMES_DIR", games)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", gp.display()))?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    own_numbers(&v, clip).ok_or_else(|| {
        v["clips"][0]["error"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "the clip's score gave no numbers".into())
    })
}

/// An `own_draws` line's numbers from `gp clip score --json`'s output.
fn own_numbers(v: &Value, clip: &str) -> Option<Numbers> {
    let c = v["clips"]
        .as_array()?
        .iter()
        .find(|c| c["clip"] == json!(clip))?;
    let s = &c["straight"];
    Some(Numbers {
        own: Some([s["together"].as_u64()?, s["frames"].as_u64()?]),
        ..Numbers::default()
    })
}

/// A failed measure is tried again after this long, though nothing changed.
const RETRY_MS: u64 = 10 * 60 * 1000;

/// The lines out of date in the games checkout, as (game, line, key): each
/// game with a scoreboard file, each line whose kept key differs, or whose
/// last measure failed long enough ago.
pub(crate) fn out_of_date(home: &Path, games: &Path, gp: &Path) -> Vec<(String, Line, String)> {
    let now = gp_mcp::asks::now_ms();
    let mut out: Vec<(String, Line, String)> = Vec::new();
    for g in crate::next::games_in(games) {
        let Some(Ok(lines)) = lines(&games.join(&g)) else {
            continue;
        };
        let cache = read_cache(home, &g);
        for l in lines {
            let key = key_of(home, &games.join(&g), &g, &l.clip, gp);
            let due = cache.lines.get(&l.slot()).is_none_or(|e| {
                e.key != key || (e.error.is_some() && now.saturating_sub(e.measured_ms) > RETRY_MS)
            });
            if due
                && !out
                    .iter()
                    .any(|(og, ol, _)| og == &g && ol.slot() == l.slot())
            {
                out.push((g.clone(), l, key));
            }
        }
    }
    out
}

/// Measures every line out of date once, one clip at a time, with
/// `run` doing the measure; the state is the gate's at the time.
pub(crate) fn refresh(
    home: &Path,
    games: &Path,
    gp: &Path,
    mut run: impl FnMut(&str, &Line) -> Result<Numbers, String>,
) {
    for (g, line, key) in out_of_date(home, games, gp) {
        let slot = line.slot();
        if let Ok(mut m) = MEASURING.lock() {
            *m = Some(format!("{g}/{slot}"));
        }
        let got = run(&g, &line).map(|mut n| {
            n.state = state_of(&status(home, &g), &line.clip).map(|s| s.0);
            n
        });
        if let Ok(mut m) = MEASURING.lock() {
            *m = None;
        }
        let _ = record(home, &g, &slot, &key, got);
    }
}

/// The lines below full, as (game, line, key): measured on the current key
/// with no error, background or sprites short of their frames. An
/// `own_draws` line is left out: the picture check doesn't measure it.
pub(crate) fn short_lines(home: &Path, games: &Path, gp: &Path) -> Vec<(String, Line, String)> {
    let mut out = Vec::new();
    for g in crate::next::games_in(games) {
        let Some(Ok(lines)) = lines(&games.join(&g)) else {
            continue;
        };
        let cache = read_cache(home, &g);
        for l in lines {
            if l.own_draws {
                continue;
            }
            let key = key_of(home, &games.join(&g), &g, &l.clip, gp);
            let Some(e) = cache.lines.get(&l.slot()) else {
                continue;
            };
            let Some(n) = e.now else { continue };
            let short = n.background[0] < n.background[1] || n.sprites[0] < n.sprites[1];
            if e.key == key && e.error.is_none() && short {
                out.push((g.clone(), l, key));
            }
        }
    }
    out
}

/// The short lines the finder looked at already, as GAME/SLOT/KEY: a line
/// is looked at again once its measure's key changes.
static FOUND: Mutex<std::collections::BTreeSet<String>> =
    Mutex::new(std::collections::BTreeSet::new());

/// gp serve's finder (docs/loop.md, "When the loop runs out of rows"): while
/// the runner is idle, each line below full not looked at on its current
/// key has `look` run on it (the picture check with `--propose`, which adds
/// a row per gap it finds, naming the line). Returns the lines it looked at.
pub(crate) fn find_work(
    home: &Path,
    games: &Path,
    gp: &Path,
    idle: impl Fn() -> bool,
    mut look: impl FnMut(&str, &Line),
) -> Vec<String> {
    let mut looked = Vec::new();
    for (g, line, key) in short_lines(home, games, gp) {
        if !idle() {
            break;
        }
        let id = format!("{g}/{}/{key}", line.slot());
        if !FOUND.lock().map(|mut f| f.insert(id)).unwrap_or(false) {
            continue;
        }
        look(&g, &line);
        looked.push(format!("{g}/{}", line.name));
    }
    looked
}

/// The picture check with `--propose` on a line's clip: each lasting gap
/// becomes a next row whose note names the line.
fn propose_gaps(gp: &Path, games: &Path, game: &str, line: &Line, module: Option<&Path>) {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(gp);
    cmd.arg("--json")
        .arg("picture")
        .arg(games.join(game))
        .args([
            "--run",
            game,
            "--clip",
            &line.clip,
            "--top",
            "2",
            "--propose",
        ])
        .args(["--line", &line.name]);
    if let Some(m) = module {
        cmd.arg("--module").arg(m);
    }
    let _ = cmd
        .current_dir(games)
        .env("GP_GAMES_DIR", games)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// How often gp serve looks for lines out of date.
const LOOK: std::time::Duration = std::time::Duration::from_secs(30);

/// The module a line is measured with, as `gp picture` run by hand picks
/// it: one given to gp serve with `--module`; else, for a game with hook
/// code of its own, none, so picture builds it from the game's hooks and
/// a stale `$GP_HOME/work/modules/GAME.wasm` is never measured; else that
/// file when it is there.
fn module_for(
    home: &Path,
    game_dir: &Path,
    game: &str,
    given: &BTreeMap<String, PathBuf>,
) -> Option<PathBuf> {
    if let Some(m) = given.get(game) {
        return Some(m.clone());
    }
    if matches!(crate::package::hook_code(game_dir), Ok(Some(_))) {
        return None;
    }
    let m = home.join("work/modules").join(format!("{game}.wasm"));
    m.is_file().then_some(m)
}

/// gp serve's measurer: every half minute, the lines out of date measured
/// again, once per server.
pub(crate) fn start(home: PathBuf, games: PathBuf, modules: BTreeMap<String, PathBuf>) {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            let gp = measuring_gp(&home);
            refresh(&home, &games, &gp, |g, line| {
                let module = module_for(&home, &games.join(g), g, &modules);
                if line.own_draws {
                    measure_own(&gp, &home, &games, g, &line.clip, module.as_deref())
                } else {
                    measure(&gp, &games, g, &line.clip, module.as_deref())
                }
            });
            refresh_held(&home, &games, &gp, |g, r| {
                let module = module_for(&home, &games.join(g), g, &modules);
                measure_held(&gp, &games, r, module.as_deref())
            });
            // The rows whose target the board now meets close themselves,
            // and a line near full asks the owner to look at it.
            for g in crate::next::games_in(&games) {
                close_rows(&home, &games, &g);
                ask_looks(&home, &games, &g);
            }
            // Room for a worker and no row for it: rows from the lines'
            // gaps, so the loop doesn't sit idle while a line isn't full.
            find_work(&home, &games, &gp, crate::serve::runner::idle, |g, line| {
                let module = module_for(&home, &games.join(g), g, &modules);
                propose_gaps(&gp, &games, g, line, module.as_deref());
            });
            std::thread::sleep(LOOK);
        }
    });
}

// A quest-log row's target (docs/loop.md, "A row's target"): a measure on
// one of the board's lines that closes the row when the measure gets there.

/// The board's column a target measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    State,
    Background,
    Sprites,
    /// An `own_draws` line's frames together straight through.
    Own,
}

impl Column {
    fn key(self) -> &'static str {
        match self {
            Column::State => "state",
            Column::Background => "background",
            Column::Sprites => "sprites",
            Column::Own => "own",
        }
    }
}

/// A row's target: the line (its name, lower case), the column and the
/// least it must read: frames matched, or the state's percent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Target {
    pub line: String,
    pub column: Column,
    pub at_least: f64,
}

/// Reads a target: `LINE state|background|sprites|own at least N`, frames
/// with an optional `of M` (`level 1 sprites at least 330 of 336`), the
/// state as a percent (`the demo state at least 99.5%`).
pub(crate) fn parse_target(s: &str) -> Result<Target, String> {
    let how = || {
        format!(
            "a target is LINE state|background|sprites|own at least N, a line of the game's scoreboard (level 1 sprites at least 330 of 336, the demo state at least 99.5%): {s}"
        )
    };
    let low = s.trim().to_lowercase();
    for column in [
        Column::State,
        Column::Background,
        Column::Sprites,
        Column::Own,
    ] {
        let pat = format!(" {} at least ", column.key());
        let Some(i) = low.find(&pat) else { continue };
        let line = low[..i].trim().to_owned();
        let rest = low[i + pat.len()..].trim();
        if line.is_empty() {
            return Err(how());
        }
        let at_least = if column == Column::State {
            let p: f64 = rest
                .trim_end_matches('%')
                .trim()
                .parse()
                .map_err(|_| how())?;
            if !(0.0..=100.0).contains(&p) {
                return Err(how());
            }
            p
        } else {
            let (n, of) = match rest.split_once(" of ") {
                Some((n, m)) => (n.trim(), Some(m.trim())),
                None => (rest, None),
            };
            let n: u64 = n.parse().map_err(|_| how())?;
            if let Some(m) = of {
                let m: u64 = m.parse().map_err(|_| how())?;
                if n > m {
                    return Err(format!("{}: {n} is more than {m}", how()));
                }
            }
            n as f64
        };
        return Ok(Target {
            line,
            column,
            at_least,
        });
    }
    Err(how())
}

/// What the board reads now for a target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    pub value: f64,
    /// The clip's frames, for background, sprites and own.
    pub total: Option<u64>,
    pub clip: String,
    /// When the line was measured (None for the state, read live).
    pub measured_ms: Option<u64>,
    /// The number is the current model's and gp's: not out of date, not
    /// being measured, the last measure didn't fail.
    pub current: bool,
}

impl Reading {
    pub fn meets(&self, t: &Target) -> bool {
        self.current && self.value >= t.at_least
    }
}

/// The board's line a target names, or None.
fn line_of<'a>(board: &'a Value, t: &Target) -> Option<&'a Value> {
    board["lines"].as_array()?.iter().find(|l| {
        l["name"]
            .as_str()
            .is_some_and(|n| n.to_lowercase() == t.line)
    })
}

/// What the board reads for `t` now, or None when its line has no number.
pub(crate) fn reading(board: &Value, t: &Target) -> Option<Reading> {
    let l = line_of(board, t)?;
    let clip = l["clip"].as_str().unwrap_or("").to_owned();
    if t.column == Column::State {
        return Some(Reading {
            value: l["state"]["pct"].as_f64()?,
            total: None,
            clip,
            measured_ms: None,
            current: true,
        });
    }
    let v = &l[t.column.key()];
    Some(Reading {
        value: v["matched"].as_u64()? as f64,
        total: v["total"].as_u64(),
        clip,
        measured_ms: l["measured_ms"].as_u64(),
        current: l["stale"] != json!(true) && l["measuring"] != json!(true) && l["error"].is_null(),
    })
}

/// A reading in words: "sprites 322 of 336", "state 99.1%".
fn reading_words(t: &Target, r: &Reading) -> String {
    let mut s = match r.total {
        Some(n) => format!("{} {} of {n}", t.column.key(), r.value),
        None => format!("{} {}%", t.column.key(), r.value),
    };
    if !r.current {
        s.push_str(" (out of date: gp serve measures it again)");
    }
    s
}

/// A row's target and where the board stands on it, as the quest log and
/// the page show it: "level 1 sprites at least 330 of 336: now sprites 322
/// of 336". `board` is the game's, None when it has none.
pub(crate) fn target_says(board: Option<&Value>, target: &str) -> String {
    let t = match parse_target(target) {
        Ok(t) => t,
        Err(e) => return format!("{target}: can't be read ({e})"),
    };
    let Some(b) = board else {
        return format!("{target}: the game has no scoreboard to measure it");
    };
    if line_of(b, &t).is_none() {
        return format!("{target}: the scoreboard has no line {:?}", t.line);
    }
    match reading(b, &t) {
        Some(r) if r.meets(&t) => format!("{target}: met, now {}", reading_words(&t, &r)),
        Some(r) => format!("{target}: now {}", reading_words(&t, &r)),
        None => format!("{target}: not measured yet"),
    }
}

/// Whether a row's `target` is met on `game` as the checkout `games` (a
/// worker's tree, main with the game's pending patches) has it: its line's
/// clip measured there now, with the reading in words. Err when it can't
/// be measured there: the state column (the gate's status gives it), or a
/// line the tree's board hasn't. An own-draws score is written under
/// `scratch`, apart from the board's.
pub(crate) fn met_in(
    home: &Path,
    scratch: &Path,
    games: &Path,
    game: &str,
    target: &str,
    gp: &Path,
) -> Result<(bool, String), String> {
    let t = parse_target(target)?;
    if t.column == Column::State {
        return Err("a state target is read from the gate's status, not measured on a tree".into());
    }
    let dir = games.join(game);
    let lines = lines(&dir).ok_or_else(|| format!("{game} has no {FILE}"))??;
    let Some(line) = lines.iter().find(|l| l.name.to_lowercase() == t.line) else {
        return Err(format!("{game}'s scoreboard has no line {:?}", t.line));
    };
    let module = module_for(home, &dir, game, &BTreeMap::new());
    let n = if line.own_draws {
        measure_own(gp, scratch, games, game, &line.clip, module.as_deref())?
    } else {
        measure(gp, games, game, &line.clip, module.as_deref())?
    };
    let [m, of] = match t.column {
        Column::Background => n.background,
        Column::Sprites => n.sprites,
        Column::Own => n.own.ok_or("the measure gave no own-draws numbers")?,
        Column::State => unreachable!("refused above"),
    };
    Ok((
        m as f64 >= t.at_least,
        format!("{} {m} of {of}", t.column.key()),
    ))
}

/// The target a row for a gap found on `line` closes on (`gp picture
/// --propose --line`): where the line reads now on `column` (sprites or
/// background) plus the `frames` the gap covers there, capped at the
/// line's total, and how many frames above now that is. A target one
/// frame past now closed a row for a 200-frame gap as soon as another fix
/// lifted the line by a frame (the owner, 2026-11-22). None when the line
/// has no current number there, is full, or the gap covers no frame.
pub(crate) fn past_gap(
    board: &Value,
    line: &str,
    column: &str,
    frames: u64,
) -> Option<(String, u64)> {
    let t = parse_target(&format!("{line} {column} at least 0")).ok()?;
    let r = reading(board, &t).filter(|r| r.current)?;
    let total = r.total?;
    let now = r.value as u64;
    let to = now.saturating_add(frames).min(total);
    (to > now).then(|| {
        (
            format!("{line} {column} at least {to} of {total}"),
            to - now,
        )
    })
}

/// Checks a target names a line of the game's board; Err says why not.
pub(crate) fn check_target(board: Option<&Value>, game: &str, target: &str) -> Result<(), String> {
    let t = parse_target(target)?;
    let Some(b) = board else {
        return Err(format!(
            "{game} has no scoreboard ({game}/{FILE} in the games checkout): a target names one of its lines"
        ));
    };
    let Some(l) = line_of(b, &t) else {
        let names: Vec<&str> = b["lines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["name"].as_str())
            .collect();
        return Err(format!(
            "{game}'s scoreboard has no line {:?}; its lines: {}",
            t.line,
            names.join(", ")
        ));
    };
    // An own-draws line has only its own column; a picture line has no own.
    let own = l["own_draws"] == json!(true);
    if own != (t.column == Column::Own) && t.column != Column::State {
        return Err(format!(
            "line {:?} {}: {target}",
            t.line,
            if own {
                "measures its own draws (LINE own at least N)"
            } else {
                "measures background and sprites, not own"
            }
        ));
    }
    Ok(())
}

/// Whether a row with `target` may be set done now: only when the board's
/// current measure meets it. Err says where the measure stands.
pub(crate) fn may_close(board: Option<&Value>, target: &str) -> Result<(), String> {
    let t = parse_target(target)?;
    if let Some(b) = board
        && let Some(r) = reading(b, &t)
        && r.meets(&t)
    {
        return Ok(());
    }
    Err(format!(
        "the row closes itself when its target is met, not by a worker's word: {}. Set it next, in line or asking you; gp serve's scoreboard sets it done when the measure gets there",
        target_says(board, target)
    ))
}

/// Closes the game's quest-log rows whose target the board now meets
/// (done, with the measure as the note), and sends back to next a row
/// whose change landed (`awaiting_measure`, the time it did) once the line
/// is measured after it and falls short. Returns the rows it changed.
pub(crate) fn close_rows(home: &Path, games: &Path, game: &str) -> Vec<String> {
    let Some(b) = board(home, games, game) else {
        return Vec::new();
    };
    let f = home.join("mechanics").join(game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    let mut changed = Vec::new();
    for r in &mut q {
        if r["status"] == json!("done") {
            continue;
        }
        let Some(target) = r["target"].as_str().map(str::to_owned) else {
            continue;
        };
        let Ok(t) = parse_target(&target) else {
            continue;
        };
        let Some(now) = reading(&b, &t) else {
            continue;
        };
        let awaiting = r["awaiting_measure"].as_u64();
        // A landing's measure: the line measured since it landed.
        let after = awaiting.is_some_and(|a| now.measured_ms.is_none_or(|m| m > a));
        let (status, note) = if now.meets(&t) {
            (
                "done",
                format!(
                    "Met its target: {target}. Now {}, measured on {}",
                    reading_words(&t, &now),
                    now.clip
                ),
            )
        } else if now.current && after {
            (
                "next",
                format!(
                    "Its change landed and the measure is short of its target ({target}): now {}, measured on {}. The loop takes it up again",
                    reading_words(&t, &now),
                    now.clip
                ),
            )
        } else {
            continue;
        };
        let Some(o) = r.as_object_mut() else { continue };
        o.insert("status".into(), json!(status));
        o.insert("note".into(), json!(note));
        o.remove("awaiting_measure");
        o.remove("worker");
        if status == "done" {
            o.remove("clip");
        }
        changed.push(r["mechanic"].as_str().unwrap_or("").to_owned());
    }
    if !changed.is_empty() {
        let _ = crate::mechanic::session::write_queue(&f, &q);
    }
    changed
}

/// How much of a line's frames its background and its sprites each match
/// for the line to be the owner's to look at: 335 of 336 is near enough.
const LOOK_SHARE: f64 = 0.99;

/// The quest-log row that asks the owner to look at a line: `confirm-` and
/// the line's name in a row's letters (`level 1`, `confirm-level-1`).
pub(crate) fn confirm_row(line: &str) -> String {
    let mut s = String::from("confirm");
    for w in line
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && *w != "the")
    {
        s.push('-');
        s.push_str(w);
    }
    s
}

/// Asks the owner to look at each picture line that measures near full
/// (background and sprites each at least [`LOOK_SHARE`] of the frames, on a
/// current measure): a row `confirm-LINE`, asking you, with the line's
/// clip over all its frames in the game's own pictures, so LOOK AT IT sets
/// it up beside the original. A row the quest log has already by that
/// name, or any row with the line's clip in the game's own pictures,
/// whatever its status, is left as it is and no row is added. Returns the
/// rows it added.
pub(crate) fn ask_looks(home: &Path, games: &Path, game: &str) -> Vec<String> {
    let Some(b) = board(home, games, game) else {
        return Vec::new();
    };
    let f = home.join("mechanics").join(game).join("queue.json");
    let mut q = crate::mechanic::session::read_queue(&f);
    let looks = crate::approved::read(&games.join(game)).unwrap_or_default();
    let mut added = Vec::new();
    for l in b["lines"].as_array().into_iter().flatten() {
        if l["own_draws"] == json!(true)
            || l["stale"] == json!(true)
            || l["measuring"] == json!(true)
            || !l["error"].is_null()
        {
            continue;
        }
        let near = |c: &str| -> Option<(u64, u64)> {
            let (m, t) = (l[c]["matched"].as_u64()?, l[c]["total"].as_u64()?);
            (t > 0 && m as f64 >= LOOK_SHARE * t as f64).then_some((m, t))
        };
        let (Some(bg), Some(sp)) = (near("background"), near("sprites")) else {
            continue;
        };
        let name = l["name"].as_str().unwrap_or("");
        let row = confirm_row(name);
        // A row by that name, or any row asking about this clip in the
        // game's own pictures (the owner's confirm-bonus for the first
        // bonus round), already has the owner's look.
        let asked = |r: &Value| {
            r["mechanic"] == json!(row)
                || (r["clip"]["id"] == l["clip"] && r["clip"]["look"] == json!("game"))
        };
        if row == "confirm" || l["clip"].is_null() || q.iter().any(asked) {
            continue;
        }
        // A look approved.toml has saved over all the line's frames isn't
        // asked again once its row has left for the archive (owner,
        // confirmed-row-closes: a row popped up they had confirmed).
        let last = bg.1.saturating_sub(1);
        if l["clip"]
            .as_str()
            .is_some_and(|c| crate::approved::holds_clip(&looks, c, last))
        {
            continue;
        }
        let state = l["state"]["pct"]
            .as_f64()
            .map(|p| format!("state {p}%, "))
            .unwrap_or_default();
        q.push(json!({
            "mechanic": row,
            "status": "asking you",
            "by": "loop",
            "from": format!("scoreboard line {name}, measured near full"),
            "note": format!(
                "Watch {name} beside the original (LOOK AT IT). If it looks right, say YES and it stays that way: no later change can make it look worse. Measured: {state}background {} of {}, sprites {} of {}.",
                bg.0, bg.1, sp.0, sp.1
            ),
            "clip": {
                "id": l["clip"],
                "from": 0,
                "to": bg.1.saturating_sub(1),
                "look": "game",
            },
        }));
        added.push(row);
    }
    if !added.is_empty() {
        let _ = crate::mechanic::session::write_queue(&f, &q);
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gap's target is where the line reads now plus the frames the gap
    /// covers on its layer, capped at the line's total; never one frame past
    /// now (the owner, 2026-11-22), and none for a full or stale line.
    #[test]
    fn a_gap_target_is_now_plus_its_frames() {
        let board = |matched: u64, stale: bool| {
            json!({"lines": [{"name": "Bonus round", "clip": "hop.bonus.001", "stale": stale,
                              "error": null, "sprites": {"matched": matched, "total": 336}}]})
        };
        assert_eq!(
            past_gap(&board(100, false), "bonus round", "sprites", 200),
            Some(("bonus round sprites at least 300 of 336".into(), 200))
        );
        assert_eq!(
            past_gap(&board(300, false), "bonus round", "sprites", 200),
            Some(("bonus round sprites at least 336 of 336".into(), 36)),
            "capped at the line's total"
        );
        assert_eq!(
            past_gap(&board(336, false), "bonus round", "sprites", 5),
            None
        );
        assert_eq!(
            past_gap(&board(100, true), "bonus round", "sprites", 5),
            None
        );
        assert_eq!(
            past_gap(&board(100, false), "bonus round", "sprites", 0),
            None
        );
        assert_eq!(past_gap(&board(100, false), "level 1", "sprites", 5), None);
    }

    fn put(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// A games checkout with one made-up game: a corpus of three clips over
    /// two levels and a model.
    fn games() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("hop");
        put(&g.join("model.toml"), "# a model\n");
        put(
            &g.join("corpus.toml"),
            r#"[corpus]
format = "1.0.0"

[[clip]]
id = "hop.title.001"
presses = "inputs/hop.title.001.presses"
frames = 100
level = "title"

[[clip]]
id = "hop.l1.001"
frames = 200
level = "field"

[[clip]]
id = "hop.l2.001"
frames = 300
level = "field"
"#,
        );
        put(&g.join("inputs/hop.title.001.presses"), "a@0-1\n");
        d
    }

    fn board_file(games: &Path, s: &str) {
        put(&games.join("hop").join(FILE), s);
    }

    #[test]
    fn a_line_names_its_clip_or_takes_its_level_s_first() {
        let d = games();
        let g = d.path().join("hop");
        assert!(lines(&g).is_none(), "no file, no board");
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nlevel = \"title\"\n\n[[line]]\nname = \"level 2\"\nclip = \"hop.l2.001\"\n",
        );
        let l = lines(&g).unwrap().unwrap();
        assert_eq!(
            l,
            vec![
                Line {
                    name: "the title".into(),
                    clip: "hop.title.001".into(),
                    level: "title".into(),
                    own_draws: false,
                },
                Line {
                    name: "level 2".into(),
                    clip: "hop.l2.001".into(),
                    level: "field".into(),
                    own_draws: false,
                },
            ]
        );
        board_file(
            d.path(),
            "[[line]]\nname = \"gone\"\nclip = \"hop.l9.001\"\n",
        );
        let e = lines(&g).unwrap().unwrap_err();
        assert!(e.contains("no clip hop.l9.001"), "{e}");
    }

    /// While the runner is idle, the finder looks once at each line below
    /// full on its current key, and at nothing while the runner is busy; a
    /// new measure (a landing) lets it look again.
    #[test]
    fn the_finder_looks_at_the_lines_below_full_while_idle() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nclip = \"hop.title.001\"\n\n[[line]]\nname = \"level 2\"\nclip = \"hop.l2.001\"\n\n[[line]]\nname = \"own\"\nclip = \"hop.l1.001\"\nown_draws = true\n",
        );
        let status = |m: &str| {
            put(
                &h.join("work/refine/hop/status.json"),
                &format!(r#"{{"model": {{"hash": "{m}"}}, "clips": []}}"#),
            )
        };
        status("finder-1");
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        let measure = || {
            refresh(h, d.path(), &gp, |_, l| {
                Ok(match (l.own_draws, l.clip.as_str()) {
                    (true, _) => Numbers {
                        own: Some([10, 200]),
                        ..Numbers::default()
                    },
                    (_, "hop.title.001") => Numbers {
                        background: [100, 100],
                        sprites: [100, 100],
                        ..Numbers::default()
                    },
                    _ => Numbers {
                        background: [300, 300],
                        sprites: [280, 300],
                        ..Numbers::default()
                    },
                })
            })
        };
        measure();
        let short: Vec<String> = short_lines(h, d.path(), &gp)
            .into_iter()
            .map(|(_, l, _)| l.name)
            .collect();
        assert_eq!(short, vec!["level 2"], "full and own-draws lines are left");
        assert!(
            find_work(h, d.path(), &gp, || false, |_, _| panic!("busy")).is_empty(),
            "nothing while the runner is busy"
        );
        let mut seen = Vec::new();
        let got = find_work(
            h,
            d.path(),
            &gp,
            || true,
            |g, l| seen.push((g.to_owned(), l.clip.clone())),
        );
        assert_eq!(got, vec!["hop/level 2"]);
        assert_eq!(seen, vec![("hop".to_owned(), "hop.l2.001".to_owned())]);
        assert!(
            find_work(h, d.path(), &gp, || true, |_, _| panic!("again")).is_empty(),
            "a line is looked at once on its key"
        );
        // A landing: measured again, looked at again.
        status("finder-2");
        assert!(
            short_lines(h, d.path(), &gp).is_empty(),
            "out of date until measured"
        );
        measure();
        assert_eq!(find_work(h, d.path(), &gp, || true, |_, _| {}).len(), 1);
    }

    /// A picture line near full asks the owner to look at it, once: a
    /// confirm row at asking you with the line's clip in the game's own
    /// pictures. A short line and an own-draws line don't, and a row the
    /// quest log has already is left as it is.
    #[test]
    fn a_line_near_full_asks_the_owner_to_look() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nclip = \"hop.title.001\"\n\n[[line]]\nname = \"level 1\"\nclip = \"hop.l1.001\"\n\n[[line]]\nname = \"level 2\"\nclip = \"hop.l2.001\"\n\n[[line]]\nname = \"own\"\nclip = \"hop.l1.001\"\nown_draws = true\n",
        );
        put(
            &h.join("work/refine/hop/status.json"),
            r#"{"model": {"hash": "look-1"}, "clips": [{"id": "hop.l1.001", "frames": 200, "straight": {"pct": 100.0}}]}"#,
        );
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        refresh(h, d.path(), &gp, |_, l| {
            Ok(match (l.own_draws, l.clip.as_str()) {
                (true, _) => Numbers {
                    own: Some([200, 200]),
                    ..Numbers::default()
                },
                (_, "hop.l1.001") => Numbers {
                    background: [200, 200],
                    sprites: [199, 200],
                    ..Numbers::default()
                },
                (_, "hop.l2.001") => Numbers {
                    background: [300, 300],
                    sprites: [280, 300],
                    ..Numbers::default()
                },
                _ => Numbers {
                    background: [100, 100],
                    sprites: [100, 100],
                    ..Numbers::default()
                },
            })
        });
        let f = h.join("mechanics/hop/queue.json");
        put(
            &f,
            r#"[{"mechanic": "title-look", "status": "asking you", "note": "the owner's", "clip": {"id": "hop.title.001", "from": 0, "to": 99, "look": "game"}}]"#,
        );
        assert_eq!(ask_looks(h, d.path(), "hop"), vec!["confirm-level-1"]);
        let q = crate::mechanic::session::read_queue(&f);
        assert_eq!(
            q[0]["status"], "asking you",
            "a row asking about the title's clip is left, and no confirm-title"
        );
        let r = q
            .iter()
            .find(|r| r["mechanic"] == "confirm-level-1")
            .unwrap();
        assert_eq!(r["status"], "asking you");
        assert_eq!(r["by"], "loop");
        assert_eq!(
            r["clip"],
            json!({"id": "hop.l1.001", "from": 0, "to": 199, "look": "game"})
        );
        let note = r["note"].as_str().unwrap();
        assert!(note.contains("LOOK AT IT"), "{note}");
        let measured = "state 100%, background 200 of 200, sprites 199 of 200";
        assert!(note.contains(measured), "{note}");
        assert_eq!(q.len(), 2, "level 2 is short and own draws isn't a look");
        assert!(ask_looks(h, d.path(), "hop").is_empty(), "once");
        // A row by the name, with no clip, is left as well.
        put(
            &f,
            r#"[{"mechanic": "confirm-title", "status": "done"}, {"mechanic": "confirm-level-1", "status": "done"}]"#,
        );
        assert!(ask_looks(h, d.path(), "hop").is_empty(), "by name");
        // Its row archived, a look saved over all its frames isn't asked
        // again; one saved over fewer frames is.
        let saved = |to: u64| {
            let l = crate::approved::Look {
                row: "confirm-level-1".into(),
                clip: "hop.l1.001".into(),
                frames: [0, to],
                look: "game".into(),
                approved: "2026-11-21".into(),
                apart: vec![[0, 0, 0]],
            };
            put(
                &d.path().join("hop").join(crate::approved::FILE),
                &crate::approved::text(&[l]),
            );
        };
        put(&f, r#"[{"mechanic": "confirm-title", "status": "done"}]"#);
        saved(199);
        assert!(ask_looks(h, d.path(), "hop").is_empty(), "saved");
        saved(150);
        assert_eq!(ask_looks(h, d.path(), "hop"), vec!["confirm-level-1"]);
        assert_eq!(
            confirm_row("The first bonus round"),
            "confirm-first-bonus-round"
        );
    }

    #[test]
    fn a_measure_is_kept_and_says_how_it_moved() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nclip = \"hop.title.001\"\n",
        );
        put(
            &h.join("work/refine/hop/status.json"),
            r#"{"model": {"hash": "m1"}, "clips": [{"id": "hop.title.001", "frames": 100, "straight": {"pct": 98.0}}]}"#,
        );
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        // Nothing measured: the line is out of date and has no numbers.
        let b = board(h, d.path(), "hop").unwrap();
        assert_eq!(b["lines"][0]["stale"], json!(true));
        assert!(b["lines"][0]["background"].is_null());
        let mut runs = 0;
        refresh(h, d.path(), &gp, |_, _| {
            runs += 1;
            Ok(Numbers {
                background: [90, 100],
                sprites: [50, 100],
                ..Numbers::default()
            })
        });
        assert_eq!(runs, 1);
        // Measured and current: nothing to do again.
        refresh(h, d.path(), &gp, |_, _| panic!("measured again"));
        let b = board(h, d.path(), "hop").unwrap();
        let l = &b["lines"][0];
        assert_eq!(l["stale"], json!(false));
        assert_eq!(
            l["background"],
            json!({"matched": 90, "total": 100, "moved": ""})
        );
        assert_eq!(l["state"]["pct"], json!(98.0));
        assert_eq!(l["approved"], json!([]));
        // The page's PROGRESS: the line's confirm row, and no lock yet.
        assert_eq!(l["confirm_row"], json!("confirm-title"));
        assert!(l["locked"].is_null());
        // A landing changes the model: measured again, and the numbers say
        // how they moved.
        put(
            &h.join("work/refine/hop/status.json"),
            r#"{"model": {"hash": "m2"}, "clips": [{"id": "hop.title.001", "frames": 100, "straight": {"pct": 100.0}}]}"#,
        );
        assert_eq!(
            board(h, d.path(), "hop").unwrap()["lines"][0]["stale"],
            json!(true)
        );
        refresh(h, d.path(), &gp, |_, _| {
            Ok(Numbers {
                background: [95, 100],
                sprites: [40, 100],
                ..Numbers::default()
            })
        });
        let b = board(h, d.path(), "hop").unwrap();
        let l = &b["lines"][0];
        assert_eq!(l["background"]["moved"], json!("up"));
        assert_eq!(l["sprites"]["moved"], json!("down"));
        assert_eq!(l["state"]["moved"], json!("up"));
        let t = text(&b).join("\n");
        assert!(
            t.contains(
                "the title (hop.title.001): state 100% up, background 95 of 100 up, sprites 40 of 100 down, approved none"
            ),
            "{t}"
        );
        // The clip's presses changed: out of date again.
        put(
            &d.path().join("hop/inputs/hop.title.001.presses"),
            "b@0-1\n",
        );
        assert_eq!(out_of_date(h, d.path(), &gp).len(), 1);
        // A look the owner locked over some of the clip's frames isn't the
        // line's lock; one over all of them is, with its day.
        let look = |to: u64| {
            format!(
                "[[look]]\nrow = \"confirm-title\"\nclip = \"hop.title.001\"\nframes = [0, {to}]\nlook = \"game\"\napproved = \"2026-11-22\"\napart = [[0, 0, 0]]\n"
            )
        };
        put(&d.path().join("hop/approved.toml"), &look(50));
        assert!(board(h, d.path(), "hop").unwrap()["lines"][0]["locked"].is_null());
        put(&d.path().join("hop/approved.toml"), &look(99));
        assert_eq!(
            board(h, d.path(), "hop").unwrap()["lines"][0]["locked"],
            json!({"day": "2026-11-22", "row": "confirm-title", "frames": [0, 99]})
        );
        assert_eq!(
            target_line("The title sprites at least 99 of 100").as_deref(),
            Some("the title")
        );
        assert_eq!(target_line("no target here"), None);
    }

    #[test]
    fn a_failed_measure_keeps_the_numbers_it_had() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        record(
            h,
            "hop",
            "hop.l1.001",
            "k1",
            Ok(Numbers {
                background: [1, 2],
                sprites: [1, 2],
                ..Numbers::default()
            }),
        )
        .unwrap();
        record(h, "hop", "hop.l1.001", "k2", Err("no module".into())).unwrap();
        let e = &read_cache(h, "hop").lines["hop.l1.001"];
        assert_eq!(e.now.unwrap().background, [1, 2]);
        assert_eq!(e.error.as_deref(), Some("no module"));
        board_file(
            d.path(),
            "[[line]]\nname = \"one\"\nclip = \"hop.l1.001\"\n",
        );
        let t = text(&board(h, d.path(), "hop").unwrap()).join("\n");
        assert!(t.contains("the last measure failed: no module"), "{t}");
    }

    #[test]
    fn an_own_draws_line_is_kept_apart_from_its_clip_s_pictures() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nclip = \"hop.title.001\"\n\n[[line]]\nname = \"on its own\"\nclip = \"hop.title.001\"\nown_draws = true\n",
        );
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        // Two lines on one clip are two measures, each its own way.
        let mut asked = Vec::new();
        refresh(h, d.path(), &gp, |_, l| {
            asked.push(l.own_draws);
            Ok(if l.own_draws {
                Numbers {
                    own: Some([400, 1000]),
                    ..Numbers::default()
                }
            } else {
                Numbers {
                    background: [90, 100],
                    sprites: [80, 100],
                    ..Numbers::default()
                }
            })
        });
        assert_eq!(asked, vec![false, true]);
        let b = board(h, d.path(), "hop").unwrap();
        let (pic, own) = (&b["lines"][0], &b["lines"][1]);
        assert_eq!(pic["background"]["matched"], json!(90));
        assert!(pic["own"].is_null());
        assert_eq!(own["own_draws"], json!(true));
        assert!(own["background"].is_null() && own["sprites"].is_null());
        assert_eq!(
            own["own"],
            json!({"matched": 400, "total": 1000, "moved": ""})
        );
        assert_eq!(own["stale"], json!(false));
        let t = text(&b).join("\n");
        assert!(
            t.contains("on its own (hop.title.001): on the model's own random draws, straight through 400 of 1000"),
            "{t}"
        );
        // The numbers as gp clip score --json gives them.
        let v =
            json!({"clips": [{"clip": "hop.title.001", "straight": {"together": 7, "frames": 9}}]});
        assert_eq!(own_numbers(&v, "hop.title.001").unwrap().own, Some([7, 9]));
        assert!(own_numbers(&v, "hop.l1.001").is_none());
    }

    #[test]
    fn a_game_with_hooks_is_measured_on_its_own_hooks() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let g = d.path().join("hop");
        put(&h.join("work/modules/hop.wasm"), "an old module");
        let none = BTreeMap::new();
        // No hook code: the module kept for the game.
        assert_eq!(
            module_for(h, &g, "hop", &none),
            Some(h.join("work/modules/hop.wasm"))
        );
        // Hook code of its own: none, so picture builds it as by hand.
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        let before = key_of(h, &g, "hop", "hop.l1.001", &gp);
        put(&g.join("ext/src/lib.rs"), "// hooks\n");
        assert_eq!(module_for(h, &g, "hop", &none), None);
        // And the hooks are part of the line's key.
        let with = key_of(h, &g, "hop", "hop.l1.001", &gp);
        assert_ne!(before, with);
        put(&g.join("ext/src/lib.rs"), "// other hooks\n");
        assert_ne!(with, key_of(h, &g, "hop", "hop.l1.001", &gp));
        // A module given to gp serve is always the one.
        let given: BTreeMap<String, PathBuf> =
            [("hop".to_owned(), PathBuf::from("/given.wasm"))].into();
        assert_eq!(
            module_for(h, &g, "hop", &given),
            Some(PathBuf::from("/given.wasm"))
        );
    }

    #[test]
    fn held_out_runs_are_scored_and_say_how_they_moved() {
        let d = games();
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        board_file(
            d.path(),
            "[[line]]\nname = \"the title\"\nclip = \"hop.title.001\"\n",
        );
        put(
            &h.join("work/refine/hop/status.json"),
            r#"{"model": {"hash": "m1"}}"#,
        );
        let gp = h.join("bin/gp");
        put(&gp, "a gp");
        // No sealed runs: the board says so.
        let b = board(h, d.path(), "hop").unwrap();
        assert_eq!(b["held_out"], json!([]));
        assert!(text(&b).join("\n").contains("held out: no sealed runs yet"));
        // The page's sealed clip 1 with a note, a log at the top, and
        // another game's run, which isn't hop's.
        let top = h.join("sealed/hop");
        put(&top.join("sealed-run/clips/001.log"), "a log\n");
        put(
            &top.join("sealed-run/clips/001.toml"),
            "start = 0\nnote = \"level 1\"\n",
        );
        put(&top.join("full-l2.log"), "another log\n");
        put(&top.join("scores/1/kept.log"), "scores are not runs\n");
        put(&h.join("sealed/other/x.log"), "not hop's\n");
        let b = board(h, d.path(), "hop").unwrap();
        let names: Vec<_> = b["held_out"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["run"].clone(), r["name"].clone(), r["stale"].clone()))
            .collect();
        assert_eq!(
            names,
            vec![
                (json!("full-l2"), json!("sealed run full-l2"), json!(true)),
                (json!("1"), json!("level 1"), json!(true)),
            ]
        );
        let mut scored = Vec::new();
        refresh_held(h, d.path(), &gp, |g, r| {
            scored.push(format!("{g} {} {}", r.folder, r.run));
            Ok(Held {
                straight: [50, 100],
                resync: [80, 100],
                longest: "4.0".into(),
            })
        });
        assert_eq!(scored, vec!["hop hop full-l2", "hop hop 1"]);
        refresh_held(h, d.path(), &gp, |_, _| panic!("scored again"));
        // A landing: scored again, and the numbers say how they moved.
        put(
            &h.join("work/refine/hop/status.json"),
            r#"{"model": {"hash": "m2"}}"#,
        );
        refresh_held(h, d.path(), &gp, |_, r| {
            if r.run == "1" {
                Ok(Held {
                    straight: [70, 100],
                    resync: [80, 100],
                    longest: "6.0".into(),
                })
            } else {
                Err("no module".into())
            }
        });
        let b = board(h, d.path(), "hop").unwrap();
        let one = &b["held_out"][1];
        assert_eq!(
            one["straight"],
            json!({"together": 70, "frames": 100, "pct": 70.0, "moved": "up"})
        );
        assert_eq!(one["resync"]["moved"], json!("same"));
        assert_eq!(one["stale"], json!(false));
        let t = text(&b).join("\n");
        assert!(
            t.contains(
                "held out level 1 (sealed run 1): straight 70% (70 of 100) up, resync 80% (80 of 100) same, longest 6.0 s"
            ),
            "{t}"
        );
        assert!(t.contains("the last score failed: no module"), "{t}");
        // The run's file changed: out of date again.
        put(&top.join("full-l2.log"), "changed\n");
        put(&top.join("sealed-run/clips/001.log"), "changed\n");
        assert_eq!(held_out_of_date(h, d.path(), &gp).len(), 2);
    }

    #[test]
    fn a_sealed_score_s_json_gives_its_numbers() {
        let v = json!({"run": "1", "score": {"straight": {"together": 3, "frames": 4}, "resync": {"together": 4, "frames": 4}}, "longest": "1.5"});
        assert_eq!(
            held_of(&v),
            Some(Held {
                straight: [3, 4],
                resync: [4, 4],
                longest: "1.5".into()
            })
        );
        assert_eq!(held_of(&json!({"error": "x"})), None);
    }

    #[test]
    fn a_target_reads_line_column_and_least() {
        assert_eq!(
            parse_target("Level 1 sprites at least 330 of 336").unwrap(),
            Target {
                line: "level 1".into(),
                column: Column::Sprites,
                at_least: 330.0
            }
        );
        assert_eq!(
            parse_target("the demo state at least 99.5%").unwrap(),
            Target {
                line: "the demo".into(),
                column: Column::State,
                at_least: 99.5
            }
        );
        assert_eq!(
            parse_target("attract mode background at least 800")
                .unwrap()
                .column,
            Column::Background
        );
        assert_eq!(
            parse_target("demo own draws own at least 900 of 1801")
                .unwrap()
                .line,
            "demo own draws"
        );
        for bad in [
            "level 1 sprites at least 340 of 336",
            "level 1 sprites 330",
            "sprites at least 3",
            "the demo state at least 120%",
            "level 1 sprites at least many",
        ] {
            assert!(parse_target(bad).is_err(), "{bad}");
        }
    }

    /// A target names a line and a column that line has.
    #[test]
    fn a_target_checks_its_line_and_column() {
        let b = json!({"lines": [
            {"name": "Level 1", "clip": "a", "own_draws": false},
            {"name": "Demo own", "clip": "b", "own_draws": true},
        ]});
        assert!(check_target(Some(&b), "g", "level 1 sprites at least 3").is_ok());
        assert!(check_target(Some(&b), "g", "demo own own at least 3").is_ok());
        assert!(check_target(Some(&b), "g", "demo own state at least 90").is_ok());
        assert!(check_target(Some(&b), "g", "level 1 own at least 3").is_err());
        assert!(check_target(Some(&b), "g", "demo own sprites at least 3").is_err());
        let e = check_target(Some(&b), "g", "level 9 sprites at least 3").unwrap_err();
        assert!(e.contains("its lines: Level 1, Demo own"), "{e}");
        assert!(check_target(None, "g", "level 1 sprites at least 3").is_err());
    }

    #[test]
    fn moves_compare_shares_of_frames() {
        assert_eq!(moved([5, 10], None), "");
        assert_eq!(moved([5, 10], Some([5, 10])), "same");
        assert_eq!(moved([6, 10], Some([5, 10])), "up");
        assert_eq!(moved([10, 20], Some([5, 10])), "same");
        assert_eq!(moved([9, 20], Some([5, 10])), "down");
    }
}
